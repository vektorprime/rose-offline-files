//! Chat Handler System for LLM Bot Communication
//!
//! This module handles player-to-bot chat communication, enabling players
//! to talk to bots and receive natural language responses. The system:
//! - Detects chat messages directed at bots
//! - Triggers high-priority LLM queries for immediate responses
//! - Generates contextual responses using the LLM

use std::sync::Arc;

use bevy::prelude::*;
use crossbeam_channel::Sender;
use uuid::Uuid;

use super::client::{ChatCompletionRequest, ChatMessage, LlmClient, LlmError, ToolCall};
use super::config::LlmConfig;
use super::event_queue::LlmEventQueue;
use super::event_types::{EventPriority, LlmEvent};
use super::prompts::build_conversation_prompt;
use super::tool_executor::execute_tool_call;
use super::tools::get_tool_definitions;
use crate::game::api::LlmBotCommand;
use crate::game::components::{CharacterInfo, ClientEntity, LlmBuddyBot, Position};
use crate::game::events::ChatMessageEvent;

/// Maximum distance for a player to be considered "nearby" for chat detection
pub const CHAT_PROXIMITY_DISTANCE: f32 = 500.0;

/// Resource for tracking pending chat responses.
///
/// This struct holds information about in-flight chat response requests
/// that are being processed asynchronously.
#[derive(Debug, Clone)]
pub struct PendingChatResponse {
    /// The bot ID this response is for
    pub bot_id: Uuid,
    /// The entity of the bot
    pub entity: Entity,
    /// The player who sent the original message
    pub player_name: String,
    /// The original message (for context)
    pub original_message: String,
    /// When the request was sent (game time in seconds)
    pub request_time: f64,
    /// The LLM response receiver
    pub response_rx: crossbeam_channel::Receiver<Result<Vec<ToolCall>, LlmError>>,
}

/// A completed chat response ready for processing.
///
/// This struct contains the result of an async LLM chat request that
/// has completed and is ready to have its tool calls executed.
#[derive(Debug)]
pub struct CompletedChatResponse {
    /// The bot ID this response is for
    pub bot_id: Uuid,
    /// The entity of the bot
    pub entity: Entity,
    /// The player who sent the original message
    pub player_name: String,
    /// The original message (for context)
    pub original_message: String,
    /// The result from the LLM (tool calls or error)
    pub result: Result<Vec<ToolCall>, LlmError>,
}

/// Resource containing the state of the chat handler system.
#[derive(Resource, Default)]
pub struct ChatHandlerState {
    /// Pending chat responses waiting to be processed
    pub pending_responses: Vec<PendingChatResponse>,
}

impl ChatHandlerState {
    /// Creates a new empty chat handler state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a pending chat response to track.
    pub fn add_pending_response(&mut self, response: PendingChatResponse) {
        // Limit the number of pending responses to prevent memory buildup
        if self.pending_responses.len() >= 10 {
            // Remove the oldest response
            self.pending_responses.remove(0);
        }
        self.pending_responses.push(response);
    }

    /// Removes completed pending responses and returns them as CompletedChatResponse.
    ///
    /// This method receives the channel message ONCE during the drain operation,
    /// avoiding the double-consumption bug that would occur if we partitioned first
    /// and then tried to receive again.
    pub fn drain_completed_responses(&mut self) -> Vec<CompletedChatResponse> {
        let mut completed = Vec::new();
        let mut still_pending = Vec::new();

        for response in self.pending_responses.drain(..) {
            match response.response_rx.try_recv() {
                Ok(result) => {
                    completed.push(CompletedChatResponse {
                        bot_id: response.bot_id,
                        entity: response.entity,
                        player_name: response.player_name,
                        original_message: response.original_message,
                        result,
                    });
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    still_pending.push(response);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    log::warn!(
                        "Chat response channel disconnected for bot {}",
                        response.bot_id
                    );
                }
            }
        }
        self.pending_responses = still_pending;
        completed
    }
}

/// Determines if a chat message is directed at a bot.
///
/// A chat message is considered "for a bot" if:
/// - The player is within proximity distance (CHAT_PROXIMITY_DISTANCE)
/// - OR the message contains the bot's name (@BotName or just BotName)
/// - OR the player is the bot's assigned player
///
/// # Arguments
///
/// * `message` - The chat message content
/// * `sender_name` - The name of the player who sent the message
/// * `bot_name` - The name of the bot (if available)
/// * `assigned_player_name` - The name of the bot's assigned player
/// * `distance` - Distance between player and bot (if available)
///
/// # Returns
///
/// true if the message is directed at the bot, false otherwise
pub fn is_chat_directed_at_bot(
    message: &str,
    sender_name: &str,
    bot_name: Option<&str>,
    assigned_player_name: &str,
    distance: Option<f32>,
) -> bool {
    // Check if message is from the assigned player
    if sender_name == assigned_player_name {
        return true;
    }

    // Check proximity
    if let Some(dist) = distance {
        if dist <= CHAT_PROXIMITY_DISTANCE {
            return true;
        }
    }

    // Check for @mention or name mention in message
    if let Some(name) = bot_name {
        let message_lower = message.to_lowercase();
        let name_lower = name.to_lowercase();

        // Check for @mention
        if message_lower.contains(&format!("@{}", name_lower)) {
            return true;
        }

        // Check for name mention (without @)
        if message_lower.contains(&name_lower) {
            return true;
        }
    }

    false
}

/// Checks if a chat message requires a response from the bot.
///
/// This function pre-filters messages to avoid unnecessary LLM queries.
/// Only questions and commands require responses; greetings and statements do not.
///
/// # Arguments
///
/// * `message` - The chat message content
///
/// # Returns
///
/// true if the message requires a response, false otherwise
pub fn chat_requires_response(message: &str) -> bool {
    let message_lower = message.to_lowercase();
    
    // Check for question indicators
    if message.contains('?') {
        return true;
    }
    
    // Check for question words at the start
    if message_lower.starts_with("what") || message_lower.starts_with("how") ||
       message_lower.starts_with("where") || message_lower.starts_with("when") ||
       message_lower.starts_with("why") || message_lower.starts_with("who") ||
       message_lower.starts_with("can you") || message_lower.starts_with("could you") ||
       message_lower.starts_with("would you") || message_lower.starts_with("do you") ||
       message_lower.starts_with("are you") || message_lower.starts_with("is there") {
        return true;
    }
    
    // Check for command indicators
    if message_lower.contains("follow") || message_lower.contains("come") ||
       message_lower.contains("attack") || message_lower.contains("kill") || message_lower.contains("fight") ||
       message_lower.contains("help") || message_lower.contains("heal") ||
       message_lower.contains("stop") || message_lower.contains("wait") ||
       message_lower.contains("go") || message_lower.contains("move") ||
       message_lower.contains("pick up") || message_lower.contains("pickup") ||
       message_lower.contains("use") || message_lower.contains("cast") ||
       message_lower.contains("buff") || message_lower.contains("revive") {
        return true;
    }
    
    false
}

/// System that listens for chat messages and triggers LLM responses.
///
/// This system:
/// 1. Listens for chat messages from players
/// 2. Determines which bots should "hear" the message
/// 3. Creates high-priority events for chat messages directed at bots
/// 4. Triggers immediate LLM queries for chat responses
///
/// # Chat Detection Logic
///
/// A chat message triggers a bot response if:
/// - The player is within CHAT_PROXIMITY_DISTANCE units
/// - OR the message contains the bot's name (@BotName or BotName)
/// - OR the player is the bot's assigned player
#[cfg(feature = "llm-feedback")]
pub fn llm_chat_handler_system(
    time: Res<Time>,
    mut chat_events: MessageReader<ChatMessageEvent>,
    mut event_queue: ResMut<LlmEventQueue>,
    bot_query: Query<(&LlmBuddyBot, Option<&CharacterInfo>, &Position, Option<&ClientEntity>, Entity)>,
    player_query: Query<(&Position, &ClientEntity)>,
    mut chat_handler_state: ResMut<ChatHandlerState>,
    llm_client: Option<Res<super::feedback_system::LlmClientResource>>,
    command_sender: Option<Res<super::feedback_system::LlmCommandSenderResource>>,
) {
    let current_time = time.elapsed_secs_f64();

    // Process all chat events
    for chat_event in chat_events.read() {
        let message = &chat_event.message;
        let sender_name = &chat_event.sender_name;
        let sender_entity = chat_event.sender_entity;

        // Get sender position if available
        let sender_position = player_query
            .get(sender_entity)
            .ok()
            .map(|(pos, _)| pos.position);

        // Find all bots that should receive this chat message
        for (bot, bot_character_info, bot_position, _client_entity, _bot_entity) in bot_query.iter() {
            // Get actual bot name from CharacterInfo if available
            let bot_name = bot_character_info
                .map(|ci| ci.name.as_str())
                .unwrap_or_else(|| "");

            // Don't process messages from the bot itself
            if bot.assigned_player_name == *sender_name {
                // This is from the assigned player - always process
            } else if !bot_name.is_empty() && sender_name == bot_name {
                // Skip messages from this bot itself
                continue;
            }

            // Calculate distance if we have both positions
            let distance = sender_position.map(|sender_pos| {
                bot_position.position.distance(sender_pos)
            });

            // Check if this chat is directed at the bot
            // Use actual bot name from CharacterInfo if available, otherwise fallback to ID-based name
            let bot_name_for_check = bot_character_info
                .map(|ci| ci.name.clone())
                .unwrap_or_else(|| format!("Bot_{}", bot.id));
            let is_directed = is_chat_directed_at_bot(
                message,
                sender_name,
                Some(&bot_name_for_check),
                &bot.assigned_player_name,
                distance,
            );

            if is_directed {
                // Pre-filter: only trigger LLM query if message requires a response
                // This reduces unnecessary LLM calls for greetings and statements
                if !chat_requires_response(message) {
                    log::debug!(
                        "Chat message from '{}' directed at bot {} does not require response: '{}'",
                        sender_name,
                        bot.id,
                        message
                    );
                    continue;
                }

                // Add high-priority event to queue
                let llm_event = LlmEvent::PlayerChat {
                    bot_id: bot.id,
                    player_name: sender_name.clone(),
                    message: message.clone(),
                };

                event_queue.push_event(bot.id, llm_event, EventPriority::High, current_time);

                log::info!(
                    "Chat message from '{}' directed at bot {} (distance: {:?}) requires response: '{}'",
                    sender_name,
                    bot.id,
                    distance,
                    message
                );
            }
        }
    }

    // Process any pending chat responses from previous frames
    process_pending_chat_responses(
        &mut chat_handler_state,
        &command_sender,
    );
}

/// System that processes immediate chat responses from the LLM.
///
/// This system checks for completed async chat response requests and
/// executes the resulting tool calls (typically send_chat).
#[cfg(feature = "llm-feedback")]
pub fn llm_chat_response_processor_system(
    mut chat_handler_state: ResMut<ChatHandlerState>,
    command_sender: Option<Res<super::feedback_system::LlmCommandSenderResource>>,
) {
    process_pending_chat_responses(&mut chat_handler_state, &command_sender);
}

/// Processes pending chat responses and executes tool calls.
fn process_pending_chat_responses(
    chat_handler_state: &mut ChatHandlerState,
    command_sender: &Option<Res<super::feedback_system::LlmCommandSenderResource>>,
) {
    let command_sender = match command_sender {
        Some(sender) => sender,
        None => return,
    };

    // Process all completed responses
    let completed = chat_handler_state.drain_completed_responses();

    for completed_response in completed {
        // The result is already received in drain_completed_responses(),
        // so we just need to process it directly
        match completed_response.result {
            Ok(tool_calls) => {
                // Execute each tool call
                for tool_call in &tool_calls {
                    match execute_tool_call(completed_response.bot_id, tool_call, &command_sender.sender) {
                        Ok(result) => {
                            log::info!(
                                "Chat tool '{}' executed successfully for bot {} (responding to '{}')",
                                result.tool_name,
                                completed_response.bot_id,
                                completed_response.player_name
                            );
                        }
                        Err(e) => {
                            log::warn!(
                                "Chat tool execution failed for bot {}: {:?}",
                                completed_response.bot_id,
                                e
                            );
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "Chat LLM request failed for bot {} (responding to '{}'): {:?}",
                    completed_response.bot_id,
                    completed_response.player_name,
                    e
                );
            }
        }
    }
}

/// Spawns an async task to generate a chat response.
///
/// This function creates an async task that:
/// 1. Builds a conversation prompt with context
/// 2. Calls the LLM API
/// 3. Returns tool calls (typically send_chat) to be executed
///
/// # Arguments
///
/// * `client` - The LLM client
/// * `config` - The LLM configuration
/// * `bot_id` - The bot's unique ID
/// * `bot_name` - The bot's name
/// * `player_name` - The player who sent the message
/// * `message` - The chat message content
/// * `context_summary` - Optional context about the current situation
/// * `chat_handler_state` - State to track the pending response
pub fn spawn_chat_response_task(
    client: Arc<LlmClient>,
    config: &LlmConfig,
    bot_id: Uuid,
    bot_entity: Entity,
    bot_name: &str,
    player_name: &str,
    message: &str,
    context_summary: Option<&str>,
    chat_handler_state: &mut ChatHandlerState,
) {
    // Build the conversation prompt
    let user_message = build_conversation_prompt(
        bot_name,
        player_name,
        message,
        context_summary.unwrap_or(""),
    );

    // Get tool definitions
    let tools = get_tool_definitions();

    // Create the request
    let request = ChatCompletionRequest::new(
        &config.model,
        vec![
            ChatMessage::system(super::prompts::SYSTEM_PROMPT),
            ChatMessage::user(&user_message),
        ],
    )
    .with_tools(tools)
    .with_max_tokens(config.max_tokens);

    // Create response channel
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);

    // Spawn async task
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
    chat_handler_state.add_pending_response(PendingChatResponse {
        bot_id,
        entity: bot_entity,
        player_name: player_name.to_string(),
        original_message: message.to_string(),
        request_time: 0.0, // Will be set by caller if needed
        response_rx,
    });

    log::debug!(
        "Spawned chat response task for bot {} to respond to '{}'",
        bot_id,
        player_name
    );
}

/// Cleans up stale chat responses that have timed out.
pub fn llm_cleanup_stale_chat_responses_system(
    mut chat_handler_state: ResMut<ChatHandlerState>,
    time: Res<Time>,
) {
    let current_time = time.elapsed_secs_f64();
    let timeout_secs = 30.0; // 30 second timeout for chat responses

    // Remove responses older than timeout
    chat_handler_state
        .pending_responses
        .retain(|r| current_time - r.request_time < timeout_secs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_chat_directed_at_bot_assigned_player() {
        let result = is_chat_directed_at_bot(
            "Hello!",
            "TestPlayer",
            Some("TestBot"),
            "TestPlayer",
            None,
        );
        assert!(result, "Should be directed when from assigned player");
    }

    #[test]
    fn test_is_chat_directed_at_bot_proximity() {
        let result = is_chat_directed_at_bot(
            "Hello!",
            "OtherPlayer",
            Some("TestBot"),
            "TestPlayer",
            Some(300.0),
        );
        assert!(result, "Should be directed when within proximity");
    }

    #[test]
    fn test_is_chat_directed_at_bot_mention() {
        let result = is_chat_directed_at_bot(
            "@TestBot hello!",
            "OtherPlayer",
            Some("TestBot"),
            "TestPlayer",
            Some(1000.0), // Far away
        );
        assert!(result, "Should be directed when @mentioned");
    }

    #[test]
    fn test_is_chat_directed_at_bot_name_in_message() {
        let result = is_chat_directed_at_bot(
            "TestBot can you help?",
            "OtherPlayer",
            Some("TestBot"),
            "TestPlayer",
            Some(1000.0), // Far away
        );
        assert!(result, "Should be directed when name is in message");
    }

    #[test]
    fn test_is_chat_not_directed() {
        let result = is_chat_directed_at_bot(
            "Hello world!",
            "OtherPlayer",
            Some("TestBot"),
            "TestPlayer",
            Some(1000.0), // Far away
        );
        assert!(!result, "Should not be directed when no criteria met");
    }

    #[test]
    fn test_chat_handler_state() {
        let mut state = ChatHandlerState::new();
        assert!(state.pending_responses.is_empty());

        // Create a test world and entity for testing
        let mut world = World::new();
        let test_entity = world.spawn_empty().id();

        let (tx, rx) = crossbeam_channel::bounded(1);
        let pending = PendingChatResponse {
            bot_id: Uuid::new_v4(),
            entity: test_entity,
            player_name: "TestPlayer".to_string(),
            original_message: "Hello!".to_string(),
            request_time: 0.0,
            response_rx: rx,
        };

        state.add_pending_response(pending);
        assert_eq!(state.pending_responses.len(), 1);

        // Complete the response
        let _ = tx.send(Ok(vec![]));
        let completed = state.drain_completed_responses();
        assert_eq!(completed.len(), 1);
        assert!(state.pending_responses.is_empty());
    }
}
