//! Feedback System for LLM Bot Control
//!
//! This module provides the main feedback loop system that orchestrates the entire
//! LLM bot control flow: gathering context, calling the LLM, and executing tool calls.
//!
//! The system runs periodically and is designed to be non-blocking to avoid freezing
//! the game loop.
//!
//! # High-Priority Event Handling
//!
//! Chat events are treated as high-priority and bypass the normal poll interval
//! to provide immediate responses. When a high-priority event (like PlayerChat)
//! is detected, the system immediately queries the LLM regardless of when the
//! last query was sent.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use crossbeam_channel::Sender;
use parking_lot::RwLock;
use uuid::Uuid;

use rose_data::{ItemDatabase, NpcDatabase, SkillDatabase, ZoneDatabase};

use super::client::{ChatCompletionRequest, ChatMessage, LlmClient, LlmError, ToolCall};
use super::config::LlmConfig;
use super::context_builder::LlmContext;
use super::event_queue::LlmEventQueue;
use super::event_types::{EventPriority, LlmEvent, TimestampedLlmEvent};
use super::llm_context_query_system::{
    build_bot_context_from_data, extract_bot_data, extract_nearby_item, extract_nearby_monster,
    extract_nearby_player, extract_skills, BotData,
};
use super::prompts::{build_user_message, SYSTEM_PROMPT};
use super::tool_executor::execute_tool_call;
use super::tools::get_tool_definitions;
use crate::game::api::LlmBotCommand;
use crate::game::components::{
    AbilityValues, CharacterInfo, ClientEntity, Command, Dead, HealthPoints, ItemDrop, Level, LlmBuddyBot, ManaPoints,
    Npc, Position, SkillList,
};
use crate::game::resources::GameData;

/// Resource for tracking pending async LLM responses.
///
/// This struct holds information about in-flight LLM requests that
/// are being processed asynchronously.
#[derive(Debug, Clone)]
pub struct PendingResponse {
    /// The bot ID this response is for
    pub bot_id: Uuid,
    /// When the request was sent (game time in seconds)
    pub request_time: f64,
    /// The LLM response receiver (using channel for async communication)
    pub response_rx: crossbeam_channel::Receiver<Result<Vec<ToolCall>, LlmError>>,
}

/// A completed response with its result.
///
/// This struct holds the bot ID and the actual result from the LLM
/// after the async task has completed.
#[derive(Debug)]
pub struct CompletedResponse {
    /// The bot ID this response is for
    pub bot_id: Uuid,
    /// The result from the LLM (tool calls or error)
    pub result: Result<Vec<ToolCall>, LlmError>,
}

/// Resource containing the state of the LLM feedback system.
///
/// This resource tracks timing information and pending responses
/// for the feedback loop.
#[derive(Resource, Default)]
pub struct LlmFeedbackState {
    /// Last query time per bot (game time in seconds)
    pub last_query_times: HashMap<Uuid, f64>,
    /// Pending async responses waiting to be processed
    pub pending_responses: Vec<PendingResponse>,
    /// Whether the system is currently processing (for debugging)
    pub is_processing: bool,
}

impl LlmFeedbackState {
    /// Creates a new empty feedback state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates the last query time for a bot.
    pub fn update_query_time(&mut self, bot_id: Uuid, time: f64) {
        self.last_query_times.insert(bot_id, time);
    }

    /// Gets the time since the last query for a bot.
    pub fn time_since_last_query(&self, bot_id: Uuid, current_time: f64) -> Option<f64> {
        self.last_query_times.get(&bot_id).map(|&t| current_time - t)
    }

    /// Adds a pending response to track.
    pub fn add_pending_response(&mut self, response: PendingResponse) {
        self.pending_responses.push(response);
    }

    /// Removes completed pending responses and returns the actual results.
    ///
    /// This method receives from the channel only once per response,
    /// preserving the actual result data for processing.
    pub fn drain_completed_responses(&mut self) -> Vec<CompletedResponse> {
        let mut completed = Vec::new();
        let mut still_pending = Vec::new();
        
        for response in self.pending_responses.drain(..) {
            match response.response_rx.try_recv() {
                Ok(result) => {
                    // Got a result - add it to completed responses
                    completed.push(CompletedResponse {
                        bot_id: response.bot_id,
                        result,
                    });
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    // No result yet - keep it pending
                    still_pending.push(response);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    // Channel disconnected without sending - log warning and drop
                    log::warn!(
                        "Response channel disconnected without result for bot {}",
                        response.bot_id
                    );
                }
            }
        }
        
        self.pending_responses = still_pending;
        completed
    }
}

/// Resource holding the LLM client for async operations.
///
/// This resource wraps the LLM client in an Arc for thread-safe sharing
/// with async tasks.
#[derive(Resource)]
pub struct LlmClientResource {
    /// The LLM client wrapped for thread safety
    pub client: Arc<LlmClient>,
    /// The configuration
    pub config: LlmConfig,
}

impl LlmClientResource {
    /// Creates a new LLM client resource.
    pub fn new(config: &LlmConfig) -> Result<Self, LlmError> {
        let client = LlmClient::new(config)?;
        Ok(Self {
            client: Arc::new(client),
            config: config.clone(),
        })
    }
}

/// Resource holding the command sender for tool execution.
///
/// This resource provides access to the command channel for sending
/// LLM bot commands after tool calls are converted.
#[derive(Resource)]
pub struct LlmCommandSenderResource {
    /// The command sender channel
    pub sender: Sender<LlmBotCommand>,
}

impl LlmCommandSenderResource {
    /// Creates a new command sender resource.
    pub fn new(sender: Sender<LlmBotCommand>) -> Self {
        Self { sender }
    }
}

/// System that processes pending async LLM responses and executes tool calls.
///
/// This system checks for completed async LLM requests and processes
/// any tool calls that were returned. It runs in the main game loop
/// but handles responses from background tasks.
///
/// # System Ordering
///
/// This system should run after `llm_feedback_system` to process any
/// responses that completed during the frame.
pub fn llm_process_responses_system(
    mut feedback_state: ResMut<LlmFeedbackState>,
    command_sender: Option<Res<LlmCommandSenderResource>>,
) {
    if command_sender.is_none() {
        return;
    }
    let command_sender = command_sender.unwrap();

    // Process all completed responses
    let completed = feedback_state.drain_completed_responses();

    for completed_response in completed {
        match completed_response.result {
            Ok(tool_calls) => {
                log::info!(
                    "Processing {} tool calls for bot {}",
                    tool_calls.len(),
                    completed_response.bot_id
                );
                // Execute each tool call
                for tool_call in &tool_calls {
                    match execute_tool_call(completed_response.bot_id, tool_call, &command_sender.sender) {
                        Ok(result) => {
                            log::info!(
                                "Tool '{}' executed successfully for bot {}",
                                result.tool_name,
                                completed_response.bot_id
                            );
                        }
                        Err(e) => {
                            log::warn!(
                                "Tool execution failed for bot {}: {:?}",
                                completed_response.bot_id,
                                e
                            );
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("LLM request failed for bot {}: {:?}", completed_response.bot_id, e);
            }
        }
    }
}

/// The main LLM feedback system that queries the LLM for bot decisions.
///
/// This system runs periodically and:
/// 1. Iterates all LLM buddy bots
/// 2. Gathers context using the context builder
/// 3. Gets events from the event queue
/// 4. Spawns async tasks to query the LLM
/// 5. Handles responses through a separate system
///
/// # Non-Blocking Design
///
/// The system uses channels to communicate with async tasks, ensuring
/// the main game loop is not blocked by LLM API calls.
///
/// # High-Priority Event Handling
///
/// When high-priority events (like PlayerChat) are detected, the system
/// bypasses the normal poll interval to provide immediate responses.
/// This ensures chat messages get quick responses from bots.
///
/// # System Ordering
///
/// This system should run after `llm_event_collector_system` to ensure
/// events are queued before processing.
#[cfg(feature = "llm-feedback")]
pub fn llm_feedback_system(
    time: Res<Time>,
    mut feedback_state: ResMut<LlmFeedbackState>,
    mut event_queue: ResMut<LlmEventQueue>,
    llm_client: Option<Res<LlmClientResource>>,
    command_sender: Option<Res<LlmCommandSenderResource>>,
    game_data: Res<GameData>,
    bot_query: Query<(Entity, &LlmBuddyBot, Option<&CharacterInfo>, &Position, &HealthPoints, &ManaPoints, &Level, Option<&AbilityValues>, Option<&SkillList>, Option<&Dead>, Option<&Command>)>,
    monster_query: Query<(Entity, &ClientEntity, &Npc, &Position, Option<&Level>, Option<&HealthPoints>, Option<&AbilityValues>)>,
    player_query: Query<(Entity, &ClientEntity, Option<&CharacterInfo>, &Position, Option<&Level>)>,
    item_query: Query<(Entity, &ClientEntity, &ItemDrop, &Position)>,
) {
    // Check if LLM is configured and enabled
    let llm_client = match llm_client {
        Some(client) => client,
        None => {
            // LLM not configured, skip
            return;
        }
    };

    if !llm_client.config.enabled {
        return;
    }

    // Need command sender to execute tools
    if command_sender.is_none() {
        log::debug!("No command sender available, skipping LLM feedback");
        return;
    }
    let command_sender = command_sender.unwrap();

    let current_time = time.elapsed_secs_f64();
    let poll_interval = llm_client.config.poll_interval_secs;

    feedback_state.is_processing = true;

    // Iterate over all bots and process events
    for (bot_entity, bot, character_info, bot_position, hp, mp, level, ability_values, skill_list, dead, command) in bot_query.iter() {
        let bot_id = bot.id;
        
        // Check if there are pending events or high priority events
        let has_events = event_queue.event_count(bot_id) > 0;
        let has_high_priority = event_queue.has_high_priority_events(bot_id);

        // Skip if no events
        if !has_events {
            continue;
        }

        // For high-priority events (like chat), bypass the poll interval
        // to provide immediate responses
        if !has_high_priority {
            // Normal priority - check poll interval
            let time_since_last = feedback_state
                .time_since_last_query(bot_id, current_time)
                .unwrap_or(f64::MAX);

            if time_since_last < poll_interval {
                continue;
            }
        }

        // Get events for this bot (drains the queue)
        let events = event_queue.get_events(bot_id);

        // Extract bot data from ECS components with actual name from CharacterInfo
        let bot_data = extract_bot_data(
            bot_entity,
            bot,
            character_info,
            bot_position,
            hp,
            mp,
            level,
            ability_values,
            &game_data.zones,
            dead.is_some(),
            command,
        );

        // Find nearby monsters with actual NPC names from database
        let nearby_monsters: Vec<_> = monster_query
            .iter()
            .filter_map(|(entity, client_entity, npc, pos, lvl, monster_hp, av)| {
                extract_nearby_monster(entity, client_entity, npc, pos, lvl, monster_hp, av, bot_position, &game_data.npcs)
            })
            .collect();

        // Find nearby players with actual player names from CharacterInfo
        let nearby_players: Vec<_> = player_query
            .iter()
            .filter_map(|(entity, client_entity, char_info, pos, lvl)| {
                extract_nearby_player(entity, client_entity, char_info, pos, lvl, bot_position)
            })
            .collect();

        // Find nearby items with actual item names from database
        let nearby_items: Vec<_> = item_query
            .iter()
            .filter_map(|(entity, client_entity, item_drop, pos)| {
                extract_nearby_item(entity, client_entity, item_drop, pos, bot_position, &game_data.items)
            })
            .collect();

        // Extract skills if available with actual skill names from database
        let skills = skill_list
            .map(|sl| extract_skills(sl, &game_data.skills))
            .unwrap_or_default();

        // Extract LlmEvent from TimestampedLlmEvent for context building
        let events_ref: Vec<&LlmEvent> = events.iter().map(|e| &e.event).collect();

        // Build context for this bot using actual ECS data
        let context = build_bot_context_from_data(
            bot_id,
            &events_ref.iter().map(|e| (*e).clone()).collect::<Vec<_>>(),
            bot_data,
            nearby_monsters,
            nearby_players,
            nearby_items,
            skills,
        );

        // Build the user message
        let context_summary = context.format_context_summary();
        let user_message = build_user_message(&events, &context_summary);

        // Get tool definitions
        let tools = get_tool_definitions();

        // Create the request
        let request = ChatCompletionRequest::new(
            &llm_client.config.model,
            vec![
                ChatMessage::system(SYSTEM_PROMPT),
                ChatMessage::user(&user_message),
            ],
        )
        .with_tools(tools)
        .with_max_tokens(llm_client.config.max_tokens);

        // Clone what we need for the async task
        let client = Arc::clone(&llm_client.client);
        let sender = command_sender.sender.clone();
        let bot_id_for_task = bot_id;

        // Create response channel
        let (response_tx, response_rx) = crossbeam_channel::bounded(1);

        // Spawn async task
        // Note: In a full implementation, we would use bevy::tasks::AsyncComputeTaskPool
        // For simplicity, we use std::thread
        std::thread::spawn(move || {
            // Create tokio runtime for this thread
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");

            rt.block_on(async {
                let result = client.complete(request).await;
                match result {
                    Ok(response) => {
                        // Extract tool calls from response
                        let tool_calls: Vec<ToolCall> = response
                            .tool_calls()
                            .map(|calls| calls.clone())
                            .unwrap_or_default();

                        let _ = response_tx.send(Ok(tool_calls));
                    }
                    Err(e) => {
                        let _ = response_tx.send(Err(e));
                    }
                }
            });
        });

        // Track the pending response
        feedback_state.add_pending_response(PendingResponse {
            bot_id,
            request_time: current_time,
            response_rx,
        });

        // Update last query time
        feedback_state.update_query_time(bot_id, current_time);

        log::debug!(
            "Started LLM query for bot {} with {} events (high priority: {})",
            bot_id,
            events.len(),
            has_high_priority
        );
    }

    feedback_state.is_processing = false;
}

/// System that cleans up old pending responses that have timed out.
///
/// This system prevents memory leaks from abandoned async tasks.
pub fn llm_cleanup_stale_responses_system(
    mut feedback_state: ResMut<LlmFeedbackState>,
    time: Res<Time>,
) {
    let current_time = time.elapsed_secs_f64();
    let timeout_secs = 60.0; // 60 second timeout

    // Remove responses older than timeout
    feedback_state
        .pending_responses
        .retain(|r| current_time - r.request_time < timeout_secs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_state_new() {
        let state = LlmFeedbackState::new();
        assert!(state.last_query_times.is_empty());
        assert!(state.pending_responses.is_empty());
        assert!(!state.is_processing);
    }

    #[test]
    fn test_update_query_time() {
        let mut state = LlmFeedbackState::new();
        let bot_id = Uuid::new_v4();

        state.update_query_time(bot_id, 10.0);
        assert_eq!(state.last_query_times.get(&bot_id), Some(&10.0));

        state.update_query_time(bot_id, 20.0);
        assert_eq!(state.last_query_times.get(&bot_id), Some(&20.0));
    }

    #[test]
    fn test_time_since_last_query() {
        let mut state = LlmFeedbackState::new();
        let bot_id = Uuid::new_v4();

        // No query yet
        assert!(state.time_since_last_query(bot_id, 10.0).is_none());

        // After query
        state.update_query_time(bot_id, 5.0);
        let elapsed = state.time_since_last_query(bot_id, 10.0);
        assert_eq!(elapsed, Some(5.0));
    }

    #[test]
    fn test_add_pending_response() {
        let mut state = LlmFeedbackState::new();
        let (tx, rx) = crossbeam_channel::bounded(1);

        let pending = PendingResponse {
            bot_id: Uuid::new_v4(),
            request_time: 10.0,
            response_rx: rx,
        };

        state.add_pending_response(pending);
        assert_eq!(state.pending_responses.len(), 1);

        // Complete the response
        let _ = tx.send(Ok(vec![]));
        let completed = state.drain_completed_responses();
        assert_eq!(completed.len(), 1);
        assert_eq!(state.pending_responses.len(), 0);
    }
}
