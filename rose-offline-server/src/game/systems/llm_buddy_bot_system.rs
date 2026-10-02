//! Game systems for LLM Buddy Bot control
//!
//! This module contains the Bevy systems that process commands from the REST API
//! and integrate LLM-controlled buddy bots with the game loop.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bevy::{
    ecs::query::QueryData,
    prelude::*,
    math::{Vec3, Vec3Swizzles},
};
use bevy::math::Vec2;
use chrono::Utc;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::game::{
    api::{
        models::{
            ItemInfo, NearbyEntity, NearbyEntityType, ThreatInfo,
        },
        BotInfo, BotSummaryData, DeleteBotResponse, GetBotInventoryResponse, GetBotListResponse,
        GetBotSkillsResponse, GetChatHistoryResponse, GetPlayerStatusResponse, GetZoneInfoResponse,
        LlmBotCommand, ResponseSender,
    },
    bundles::{client_entity_leave_zone, skill_target_filter_allows, skill_use_requirements_met},
    components::{
        BotBehaviorMode, CharacterInfo, ChatMessage as BotChatMessage, ChatType as BotChatType,
        ClanMembership, ClientEntity, ClientEntitySector, ClientEntityType, Command,
        CommandCastSkillTarget, CommandData, Cooldowns, DamageSources, Dead, Equipment,
        ExperiencePoints, GameClient, HealthPoints, Inventory, LlmBuddyBot, ManaPoints,
        NextCommand, Party, PartyMembership, Position, SkillList, Stamina, StatusEffects, Team,
    },
    messages::server::ServerMessage,
    resources::{ClientEntityList, GameData, ServerMessages},
    storage::llm_buddy_bot::LlmBuddyBotStorage,
};

use rose_data::{AbilityType, SkillData, SkillId, SkillType, StatusEffectType, ZoneId};
use rose_game_common::{
    components::{AbilityValues, MoveMode},
    messages::PartyRejectInviteReason,
};

/// Resource holding the LLM Bot Manager for command processing
///
/// This resource provides access to the command receiver and bot entity mappings.
#[derive(Resource)]
pub struct LlmBotManagerResource {
    /// Reference to the bot manager's bots map (contains BotInfo with entity, name, assigned_player, etc.)
    pub bots_map: Arc<RwLock<std::collections::HashMap<Uuid, BotInfo>>>,
    /// Pending commands to create bots (bot_id -> creation data)
    pub pending_creates: std::collections::HashMap<Uuid, LlmBotCommand>,
    /// Pending commands to delete bots (bot_id, response_channel)
    pub pending_deletes: Vec<(Uuid, ResponseSender<DeleteBotResponse>)>,
}

impl Default for LlmBotManagerResource {
    fn default() -> Self {
        Self {
            bots_map: Arc::new(RwLock::new(std::collections::HashMap::new())),
            pending_creates: std::collections::HashMap::new(),
            pending_deletes: Vec::new(),
        }
    }
}

/// Resource containing the command receiver channel
#[derive(Resource)]
pub struct LlmBotCommandReceiver {
    /// Receiver for commands from the API
    receiver: crossbeam_channel::Receiver<LlmBotCommand>,
}

impl LlmBotCommandReceiver {
    /// Create a new command receiver resource
    pub fn new(receiver: crossbeam_channel::Receiver<LlmBotCommand>) -> Self {
        Self { receiver }
    }

    /// Try to receive a command without blocking
    pub fn try_recv(&self) -> Option<LlmBotCommand> {
        match self.receiver.try_recv() {
            Ok(cmd) => {
                log::info!("LlmBotCommandReceiver::try_recv - successfully received command");
                Some(cmd)
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                // No commands available - this is normal
                None
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                // log::error!("LlmBotCommandReceiver::try_recv - CHANNEL IS DISCONNECTED! The sender has been dropped.");
                None
            }
        }
    }

    /// True when API commands are waiting to be processed.
    pub fn has_pending(&self) -> bool {
        !self.receiver.is_empty()
    }
}

/// Run condition for `process_llm_bot_commands_system`. Its only inputs are
/// the API command channel and `pending_deletes`, which that system fills and
/// drains within the same run, so skipping it otherwise is behaviour-neutral.
pub fn llm_bot_commands_pending(
    command_receiver: Res<LlmBotCommandReceiver>,
    bot_manager: Res<LlmBotManagerResource>,
) -> bool {
    command_receiver.has_pending() || !bot_manager.pending_deletes.is_empty()
}

/// Run condition for `process_llm_bot_creations_system`, whose only input is
/// `pending_creates` (queued by `process_llm_bot_commands_system`).
pub fn llm_bot_creations_pending(bot_manager: Res<LlmBotManagerResource>) -> bool {
    !bot_manager.pending_creates.is_empty()
}

/// System that processes incoming LLM bot commands
///
/// This system reads commands from the API channel and applies them to bot entities.
/// It handles Move, Follow, Attack, UseSkill, Chat, Stop, Sit, Stand, and Pickup commands.
pub fn process_llm_bot_commands_system(
    mut commands: Commands,
    command_receiver: Res<LlmBotCommandReceiver>,
    mut bot_manager: ResMut<LlmBotManagerResource>,
    mut client_entity_list: ResMut<ClientEntityList>,
    mut bot_query: Query<(
        &mut LlmBuddyBot,
        &mut Command,
        &mut NextCommand,
        &Position,
        Entity,
    )>,
    bot_entity_query: Query<&ClientEntity, With<LlmBuddyBot>>,
    bot_delete_query: Query<(&ClientEntity, &ClientEntitySector, &Position), With<LlmBuddyBot>>,
    skill_query: Query<&rose_game_common::components::SkillList>,
    entity_query: Query<(
        Entity,
        &ClientEntity,
        Option<&Position>,
        Option<&crate::game::components::CharacterInfo>,
        Option<&crate::game::components::Npc>,
        Option<&crate::game::components::ItemDrop>,
        Option<&HealthPoints>,
        Option<&ManaPoints>,
        Option<&AbilityValues>,
        Option<&crate::game::components::Level>,
        Option<&crate::game::components::Command>,
    ), Without<LlmBuddyBot>>,
    player_query: Query<(&ClientEntity, &crate::game::components::CharacterInfo, &Position, Option<&ManaPoints>, Option<&AbilityValues>, Option<&crate::game::components::Level>, Option<&crate::game::components::Command>), Without<LlmBuddyBot>>,
    bot_status_query: Query<(&Position, Option<&HealthPoints>, Option<&ManaPoints>, Option<&Stamina>, Option<&AbilityValues>, Option<&Dead>), With<LlmBuddyBot>>,
    inventory_query: Query<&Inventory>,
    mut server_messages: ResMut<ServerMessages>,
    mut use_item_events: MessageWriter<crate::game::events::UseItemEvent>,
    mut chat_message_events: MessageWriter<crate::game::events::ChatMessageEvent>,
    game_data: Res<GameData>,
) {
    // Log that the system is running (for debugging channel issues)
    static LOG_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = LOG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    // Process all pending commands
    let mut command_count = 0;
    while let Some(command) = command_receiver.try_recv() {
        command_count += 1;
        log::info!("Received command #{} from channel, processing...", command_count);
        match command {
            LlmBotCommand::GetBotInventory { bot_id, response_tx } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    if let Ok(inventory) = inventory_query.get(entity) {
                        let mut items = Vec::new();
                        for item_opt in inventory.iter() {
                            if let Some(item) = item_opt {
                                let item_ref = item.get_item_reference();
                                let name = game_data.items.get_base_item(item_ref)
                                    .map(|i| i.name.to_string())
                                    .unwrap_or_else(|| "Unknown Item".to_string());
                                
                                items.push(crate::game::api::models::InventoryItemInfo {
                                    slot: "Inventory".to_string(), // Simplified for now
                                    item_id: item.get_item_number(),
                                    name,
                                    quantity: item.get_quantity(),
                                });
                            }
                        }
                        let _ = response_tx.send(GetBotInventoryResponse {
                            success: true,
                            error: None,
                            items,
                        });
                    } else {
                        let _ = response_tx.send(GetBotInventoryResponse {
                            success: false,
                            error: Some("Bot inventory not found".to_string()),
                            items: vec![],
                        });
                    }
                } else {
                    let _ = response_tx.send(GetBotInventoryResponse {
                        success: false,
                        error: Some("Bot not found".to_string()),
                        items: vec![],
                    });
                }
            }
            LlmBotCommand::GetPlayerStatus { bot_id, response_tx } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    if let Some(player_name) = bot_info.assigned_player {
                        // Find player with all components needed
                        if let Some((client_ent, char_info, pos, mp_opt, ability_opt, level_opt, command_opt)) =
                            player_query.iter().find(|(_, info, _, _, _, _, _)| info.name == player_name) {
                            // Get HP from ability values
                            let hp = ability_opt.map(|a| a.max_health + a.adjust.max_health).unwrap_or(100);
                            let max_hp = hp;
                            // Get MP from ManaPoints component
                            let mp = mp_opt.map(|m| m.mp).unwrap_or(0);
                            let max_mp = ability_opt.map(|a| a.max_mana + a.adjust.max_mana).unwrap_or(100);
                            // Get level from Level component
                            let level = level_opt.map(|l| l.level as u16).unwrap_or(1);
                            // Check combat status from Command component
                            let is_in_combat = command_opt.map(|c| !c.is_stop()).unwrap_or(false);

                            let status = crate::game::api::models::PlayerStatus {
                                name: player_name,
                                health: crate::game::api::models::VitalPoints::new(hp as u32, max_hp as u32),
                                mana: crate::game::api::models::VitalPoints::new(mp as u32, max_mp as u32),
                                level,
                                position: crate::game::api::models::ZonePosition::new(pos.position.x, pos.position.y, pos.position.z, pos.zone_id.get()),
                                is_in_combat,
                            };

                            let _ = response_tx.send(GetPlayerStatusResponse {
                                success: true,
                                error: None,
                                status: Some(status),
                            });
                        } else {
                            let _ = response_tx.send(GetPlayerStatusResponse {
                                success: false,
                                error: Some("Player not found in world".to_string()),
                                status: None,
                            });
                        }
                    } else {
                        let _ = response_tx.send(GetPlayerStatusResponse {
                            success: false,
                            error: Some("Bot has no assigned player".to_string()),
                            status: None,
                        });
                    }
                } else {
                    let _ = response_tx.send(GetPlayerStatusResponse {
                        success: false,
                        error: Some("Bot not found".to_string()),
                        status: None,
                    });
                }
            }
            LlmBotCommand::TeleportToPlayer { bot_id } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    if let Some(player_name) = bot_info.assigned_player {
                        if let Some((_, _, player_pos, _, _, _, _)) = player_query.iter().find(|(_, info, _, _, _, _, _)| info.name == player_name) {
                            if let Ok((_, _, mut next_command, _, _)) = bot_query.get_mut(bot_info.entity) {
                                next_command.command = Some(CommandData::Move {
                                    destination: player_pos.position,
                                    target: None,
                                    move_mode: Some(MoveMode::Run),
                                });
                                next_command.has_sent_server_message = false;
                                log::info!("Teleporting bot {} to player {}", bot_id, player_name);
                            }
                        }
                    }
                }
            }
            LlmBotCommand::CreateBot {
                bot_id,
                name,
                level,
                class,
                gender,
                assigned_player,
            } => {
                // Store for later creation (creation needs access to more systems)
                let name_for_log = name.clone();
                bot_manager.pending_creates.insert(
                    bot_id,
                    LlmBotCommand::CreateBot {
                        bot_id,
                        name,
                        level,
                        class,
                        gender,
                        assigned_player,
                    },
                );
                log::info!("Queued LLM buddy bot creation: {} ({})", name_for_log, bot_id);
            }
            LlmBotCommand::DeleteBot { bot_id, response_tx } => {
                // Queue for deletion with response channel
                log::info!("DeleteBot command received for bot {} - adding to pending_deletes queue", bot_id);
                bot_manager.pending_deletes.push((bot_id, response_tx));
                log::info!("DeleteBot: pending_deletes queue now has {} items", bot_manager.pending_deletes.len());
            }
            LlmBotCommand::Move {
                bot_id,
                destination,
                target_entity,
                move_mode,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    // log::info!("[LLM_DEBUG] Processing Move command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((mut buddy_bot, _, mut next_command, bot_position, _)) = bot_query.get_mut(entity) {
                        // Stop following when manually moving
                        buddy_bot.is_following = false;

                        let dest = Vec3::new(destination.x, destination.y, destination.z);
                        // ClientEntityIds are allocated per zone: only an entity in
                        // the bot's zone is the one the caller saw.
                        let bot_zone = bot_position.zone_id;
                        let target = target_entity.and_then(|id| {
                            entity_query
                                .iter()
                                .find(|(_, client_entity, pos_opt, _, _, _, _, _, _, _, _)| {
                                    client_entity.id.0 == id as usize
                                        && pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                                })
                                .map(|(entity, _, _, _, _, _, _, _, _, _, _)| entity)
                        });
                        let mode = parse_move_mode(&move_mode);

                        // log::info!("[LLM_DEBUG] Move command details: dest={:?}, target={:?}, mode={:?}", dest, target, mode);

                        next_command.command = Some(CommandData::Move {
                            destination: dest,
                            target,
                            move_mode: mode,
                        });
                        next_command.has_sent_server_message = false;

                        // log::info!("[LLM_DEBUG] LLM bot {} moving to {:?}", bot_id, dest);
                    } else {
                        log::warn!("[LLM_DEBUG] Move command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("[LLM_DEBUG] Move command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Follow {
                bot_id,
                player_name,
                distance,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    // log::info!("[LLM_DEBUG] Processing Follow command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((mut buddy_bot, _, _, _, _)) = bot_query.get_mut(entity) {
                        // Look up the player's entity ID by name
                        let player_id = player_query.iter()
                            .find(|(_, char_info, _, _, _, _, _)| char_info.name == player_name)
                            .map(|(client_entity, _, _, _, _, _, _)| client_entity.id.0 as u32);
                        
                        buddy_bot.assigned_player_name = player_name.clone();
                        buddy_bot.follow_distance = distance;
                        buddy_bot.is_following = true;
                        
                        if let Some(id) = player_id {
                            buddy_bot.assigned_player_id = id;
                            // log::info!("[LLM_DEBUG] LLM bot {} now following '{}' (id: {}) with distance {}", bot_id, player_name, id, distance);
                        } else {
                            log::warn!("[LLM_DEBUG] LLM bot {} set to follow '{}' but player not found - will follow when player is found", bot_id, player_name);
                        }
                    } else {
                        log::warn!("[LLM_DEBUG] Follow command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("[LLM_DEBUG] Follow command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Attack {
                bot_id,
                target_entity_id,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing Attack command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((mut buddy_bot, mut command, mut next_command, bot_position, _)) = bot_query.get_mut(entity) {
                        // Stop following when attacking
                        buddy_bot.is_following = false;

                        let bot_zone = bot_position.zone_id;

                        // Find the entity by its ClientEntityId AND zone (must be in same zone as bot)
                        // Also verify it's a monster (not a player, NPC, or item)
                        let target = entity_query
                            .iter()
                            .find(|(_, client_entity, pos_opt, _, _, _, _, _, _, _, _)| {
                                client_entity.id.0 == target_entity_id as usize &&
                                client_entity.is_monster() &&
                                pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                            })
                            .map(|(entity, _, _, _, _, _, _, _, _, _, _)| entity);

                        match target {
                            Some(target_entity) => {
                                // log::info!("[LLM_DEBUG] Bot {} attacking monster target {:?} (ClientEntityId {})", bot_id, target_entity, target_entity_id);
                                // Set next_command to ensure it's processed and broadcasted by command_system
                                next_command.command = Some(CommandData::Attack { target: target_entity });
                                next_command.has_sent_server_message = false;
                                // log::info!("[LLM_DEBUG] Attack command queued in next_command for bot {}", bot_id);
                            }
                            None => {
                                log::warn!("[LLM_DEBUG] LLM bot {} cannot find monster with ClientEntityId {} in zone {:?}. Target must be a monster in the same zone.",
                                    bot_id, target_entity_id, bot_zone);
                                
                                // DIAGNOSTIC: List all monsters in the same zone to help debug entity ID issues
                                let monsters_in_zone: Vec<(usize, Option<&str>)> = entity_query
                                    .iter()
                                    .filter(|(_, client_entity, pos_opt, _, npc_opt, _, _, _, _, _, _)| {
                                        client_entity.is_monster() &&
                                        pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                                    })
                                    .map(|(_, client_entity, _, _, npc_opt, _, _, _, _, _, _)| {
                                        let name = npc_opt.and_then(|npc| game_data.npcs.get_npc(npc.id).map(|n| n.name.as_str()));
                                        (client_entity.id.0, name)
                                    })
                                    .take(10) // Limit to first 10 to avoid log spam
                                    .collect();
                                
                                log::warn!("[LLM_DEBUG] DIAGNOSTIC - Monsters in zone {:?} (showing up to 10): {:?}", bot_zone, monsters_in_zone);
                                // Do not set attack command - invalid target
                            }
                        }
                    } else {
                        log::warn!("Attack command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("Attack command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::UseSkill {
                bot_id,
                skill_id,
                target_entity_id,
                target_position,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing UseSkill command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((mut buddy_bot, _, mut next_command, bot_position, _)) = bot_query.get_mut(entity) {
                        // Stop following when using skill
                        buddy_bot.is_following = false;

                        let bot_zone = bot_position.zone_id;
                        let skill_target = if let Some(entity_id) = target_entity_id {
                            // Find target entity in the same zone (can be enemy, ally, or NPC)
                            let target_entity = entity_query
                                .iter()
                                .find(|(_, client_entity, pos_opt, _, _, _, _, _, _, _, _)| {
                                    client_entity.id.0 == entity_id as usize &&
                                    pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                                })
                                .map(|(entity, _, _, _, _, _, _, _, _, _, _)| entity);
                            
                            match target_entity {
                                Some(target) => {
                                    log::info!("LLM bot {} using skill {} on target {:?}", bot_id, skill_id, target);
                                    Some(CommandCastSkillTarget::Entity(target))
                                }
                                None => {
                                    log::warn!("LLM bot {} cannot find target with ClientEntityId {} in zone {:?} for skill {}",
                                        bot_id, entity_id, bot_zone, skill_id);
                                    
                                    // DIAGNOSTIC: List all entities in the same zone to help debug entity ID issues
                                    let entities_in_zone: Vec<(usize, NearbyEntityType, Option<&str>)> = entity_query
                                        .iter()
                                        .filter(|(_, _, pos_opt, _, _, _, _, _, _, _, _)| {
                                            pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                                        })
                                        .map(|(_, client_entity, _, char_info_opt, npc_opt, item_opt, _, _, _, _, _)| {
                                            let (entity_type, name) = if npc_opt.is_some() {
                                                (NearbyEntityType::Monster, npc_opt.and_then(|npc| game_data.npcs.get_npc(npc.id).map(|n| n.name.as_str())))
                                            } else if item_opt.is_some() {
                                                (NearbyEntityType::Item, None)
                                            } else if char_info_opt.is_some() {
                                                (NearbyEntityType::Player, char_info_opt.map(|c| c.name.as_str()))
                                            } else {
                                                (NearbyEntityType::Monster, None) // Default fallback
                                            };
                                            (client_entity.id.0, entity_type, name)
                                        })
                                        .take(10) // Limit to first 10 to avoid log spam
                                        .collect();
                                    
                                    log::warn!("[LLM_DEBUG] DIAGNOSTIC - Entities in zone {:?} (showing up to 10): {:?}", bot_zone, entities_in_zone);
                                    None // Don't use skill if target not found
                                }
                            }
                        } else if let Some(pos) = target_position {
                            Some(CommandCastSkillTarget::Position(Vec2::new(pos.x, pos.z)))
                        } else {
                            None
                        };

                        // Only proceed if we have a valid target (or no target needed for self-buffs)
                        if skill_target.is_some() || target_entity_id.is_none() {
                            // SkillId::new returns Option<SkillId>, handle invalid skill IDs
                            if let Some(skill) = SkillId::new(skill_id) {
                                next_command.command = Some(CommandData::CastSkill {
                                    skill_id: skill,
                                    skill_target,
                                    use_item: None,
                                    cast_motion_id: None,
                                    action_motion_id: None,
                                });
                                next_command.has_sent_server_message = false;
                                log::info!("LLM bot {} using skill {}", bot_id, skill_id);
                            } else {
                                log::warn!("LLM bot {} tried to use invalid skill ID {}", bot_id, skill_id);
                            }
                        }
                    } else {
                        log::warn!("UseSkill command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("UseSkill command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Chat {
                bot_id,
                message,
                chat_type,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing Chat command for bot {}: entity {:?}", bot_id, entity);
                    log::info!(
                        "LLM bot {} [{}] says: {}",
                        bot_id,
                        chat_type,
                        message
                    );

                    // Get the bot's ClientEntity to broadcast chat to nearby players
                    if let Ok(client_entity) = bot_entity_query.get(entity) {
                        // Broadcast chat message to nearby players based on chat_type
                        match chat_type.as_str() {
                            "shout" => {
                                server_messages.send_entity_message(
                                    client_entity,
                                    ServerMessage::ShoutChat {
                                        name: bot_info.name.clone(),
                                        text: message.clone(),
                                    },
                                );

                                chat_message_events.write(crate::game::events::ChatMessageEvent {
                                    sender_entity: entity,
                                    sender_name: bot_info.name.clone(),
                                    zone_id: client_entity.zone_id,
                                    message: message.clone(),
                                    chat_type: crate::game::components::ChatType::Shout,
                                });
                            }
                            "announce" => {
                                server_messages.send_entity_message(
                                    client_entity,
                                    ServerMessage::AnnounceChat {
                                        name: Some(bot_info.name.clone()),
                                        text: message.clone(),
                                    },
                                );

                                chat_message_events.write(crate::game::events::ChatMessageEvent {
                                    sender_entity: entity,
                                    sender_name: bot_info.name.clone(),
                                    zone_id: client_entity.zone_id,
                                    message: message.clone(),
                                    chat_type: crate::game::components::ChatType::Announce,
                                });
                            }
                            _ => {
                                // Default to local chat
                                server_messages.send_entity_message(
                                    client_entity,
                                    ServerMessage::LocalChat {
                                        entity_id: client_entity.id,
                                        text: message.clone(),
                                    },
                                );

                                chat_message_events.write(crate::game::events::ChatMessageEvent {
                                    sender_entity: entity,
                                    sender_name: bot_info.name.clone(),
                                    zone_id: client_entity.zone_id,
                                    message: message.clone(),
                                    chat_type: crate::game::components::ChatType::Local,
                                });
                            }
                        }
                        log::info!("Broadcast chat message from bot {} to nearby players", bot_id);
                    } else {
                        log::warn!("Chat command: bot {} entity {:?} has no ClientEntity component", bot_id, entity);
                    }

                    // Store the message in the bot's chat history
                    if let Ok((mut buddy_bot, _, _, _, _)) = bot_query.get_mut(entity) {
                        let bot_id_for_msg = buddy_bot.id;
                        let sender_entity_id = bot_entity_query.get(entity)
                            .map(|ce| ce.id.0 as u32)
                            .unwrap_or_else(|_| entity.index_u32());

                        buddy_bot.add_chat_message(BotChatMessage {
                            timestamp: Utc::now(),
                            sender_name: format!("Bot:{}", bot_id_for_msg),
                            sender_entity_id,
                            message,
                            chat_type: BotChatType::Local,
                        });
                    } else {
                        log::warn!("Chat command: bot {} entity {:?} not found in bot_query for history storage", bot_id, entity);
                    }
                } else {
                    log::warn!("Chat command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Stop { bot_id } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing Stop command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((mut buddy_bot, _, mut next_command, _, _)) = bot_query.get_mut(entity) {
                        buddy_bot.is_following = false;
                        next_command.command = Some(CommandData::Stop { send_message: true });
                        next_command.has_sent_server_message = false;

                        log::info!("LLM bot {} stopped", bot_id);
                    } else {
                        log::warn!("Stop command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("Stop command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Sit { bot_id } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing Sit command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((_, _, mut next_command, _, _)) = bot_query.get_mut(entity) {
                        next_command.command = Some(CommandData::Sitting);
                        next_command.has_sent_server_message = false;

                        log::info!("LLM bot {} sitting", bot_id);
                    } else {
                        log::warn!("Sit command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("Sit command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Stand { bot_id } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing Stand command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((_, _, mut next_command, _, _)) = bot_query.get_mut(entity) {
                        next_command.command = Some(CommandData::Standing);
                        next_command.has_sent_server_message = false;

                        log::info!("LLM bot {} standing", bot_id);
                    } else {
                        log::warn!("Stand command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("Stand command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Pickup {
                bot_id,
                item_entity_id,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing Pickup command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((_, _, mut next_command, bot_position, _)) = bot_query.get_mut(entity) {
                        // ClientEntityIds are per zone; an id that is not in the
                        // bot's zone must not resolve to some unrelated entity.
                        let bot_zone = bot_position.zone_id;
                        let target = entity_query
                            .iter()
                            .find(|(_, client_entity, pos_opt, _, _, _, _, _, _, _, _)| {
                                client_entity.id.0 == item_entity_id as usize
                                    && pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                            })
                            .map(|(entity, _, _, _, _, _, _, _, _, _, _)| entity);

                        if let Some(target) = target {
                            next_command.command = Some(CommandData::PickupItemDrop { target });
                            next_command.has_sent_server_message = false;

                            log::info!("LLM bot {} picking up item {}", bot_id, item_entity_id);
                        } else {
                            log::warn!(
                                "LLM bot {} cannot find item with ClientEntityId {} in zone {:?}",
                                bot_id,
                                item_entity_id,
                                bot_zone
                            );
                        }
                    } else {
                        log::warn!("Pickup command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("Pickup command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::AttackNearest { bot_id } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing AttackNearest command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((mut buddy_bot, _, mut next_command, bot_position, _)) = bot_query.get_mut(entity) {
                        // Stop following when attacking
                        buddy_bot.is_following = false;

                        let bot_zone = bot_position.zone_id;
                        let bot_pos = bot_position.position;

                        // Find the nearest monster (NPCs that are not players)
                        let nearest_monster = entity_query
                            .iter()
                            .filter(|(_, _, pos_opt, _, npc_opt, _, _, _, _, _, _)| {
                                // Must be an NPC (monster) and in same zone
                                npc_opt.is_some() && pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                            })
                            .min_by(|(_, _, pos_a, _, _, _, _, _, _, _, _), (_, _, pos_b, _, _, _, _, _, _, _, _)| {
                                let dist_a = pos_a.map_or(f32::MAX, |p| (p.position - bot_pos).length());
                                let dist_b = pos_b.map_or(f32::MAX, |p| (p.position - bot_pos).length());
                                dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
                            });

                        if let Some((target_entity, _, _, _, _, _, _, _, _, _, _)) = nearest_monster {
                            next_command.command = Some(CommandData::Attack { target: target_entity });
                            next_command.has_sent_server_message = false;
                            log::info!("LLM bot {} attacking nearest monster entity {:?}", bot_id, target_entity);
                        } else {
                            log::info!("LLM bot {} found no nearby monsters to attack", bot_id);
                        }
                    } else {
                        log::warn!("AttackNearest command failed: bot {} entity {:?} not found in bot_query", bot_id, entity);
                    }
                } else {
                    log::warn!("AttackNearest command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::PickupNearestItem { bot_id } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing PickupNearestItem command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((_, _, mut next_command, bot_position, _)) = bot_query.get_mut(entity) {
                        let bot_zone = bot_position.zone_id;
                        let bot_pos = bot_position.position;

                        // Find the nearest item drop in the same zone
                        let nearest_item = entity_query
                            .iter()
                            .filter(|(_, _, pos_opt, _, _, item_opt, _, _, _, _, _)| {
                                // Must be an ItemDrop and in same zone
                                item_opt.is_some() && pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                            })
                            .min_by(|(_, _, pos_a, _, _, _, _, _, _, _, _), (_, _, pos_b, _, _, _, _, _, _, _, _)| {
                                let dist_a = pos_a.map_or(f32::MAX, |p| (p.position - bot_pos).length());
                                let dist_b = pos_b.map_or(f32::MAX, |p| (p.position - bot_pos).length());
                                dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
                            });

                        if let Some((target_entity, _, _, _, _, _, _, _, _, _, _)) = nearest_item {
                            next_command.command = Some(CommandData::PickupItemDrop { target: target_entity });
                            next_command.has_sent_server_message = false;
                            log::info!("LLM bot {} picking up nearest item entity {:?}", bot_id, target_entity);
                        } else {
                            log::info!("LLM bot {} found no nearby items to pickup", bot_id);
                        }
                    } else {
                        log::warn!("PickupNearestItem command failed: bot {} entity {:?} not found in bot_query", bot_id, entity);
                    }
                } else {
                    log::warn!("PickupNearestItem command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::Emote {
                bot_id,
                emote_id,
                is_stop,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!("Processing Emote command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((_, _, mut next_command, _, _)) = bot_query.get_mut(entity) {
                        next_command.command = Some(CommandData::Emote {
                            motion_id: rose_data::MotionId::new(emote_id),
                            is_stop,
                        });
                        next_command.has_sent_server_message = false;

                        log::info!("LLM bot {} performing emote {}", bot_id, emote_id);
                    } else {
                        log::warn!("Emote command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("Emote command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::GetBotContext { bot_id, response_tx } => {
                // Get nearby threats and items for this bot
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    if let Ok((_, _, _, bot_position, _)) = bot_query.get(entity) {
                        let bot_pos = bot_position.position;
                        let bot_zone = bot_position.zone_id;
                        
                        // Query nearby monsters (threats) and items
                        let mut entities: Vec<crate::game::api::models::NearbyEntity> = vec![];

                        for (other_ent, other_client_ent, other_pos_opt, char_info_opt, npc_opt, item_opt, hp_opt, _mp_opt, ability_opt, level_opt, _command_opt) in entity_query.iter() {
                            if other_ent == entity { continue; }
                            if let Some(other_pos) = other_pos_opt {
                                if other_pos.zone_id != bot_zone { continue; }
                                
                                let distance = bot_pos.distance(other_pos.position);
                                if distance > 200000.0 { continue; } // 200m radius (increased by 10x)

                                if let Some(npc) = npc_opt {
                                    // It's an NPC or Monster
                                    let name = game_data.npcs.get_npc(npc.id)
                                        .map(|n| n.name.to_string())
                                        .unwrap_or_else(|| format!("NPC {}", npc.id.get()));
                                    
                                    // Get level from Level component, or fallback to NPC data
                                    let level = level_opt.map(|l| l.level as u16).unwrap_or_else(|| {
                                        game_data.npcs.get_npc(npc.id)
                                            .map(|n| n.level as u16)
                                            .unwrap_or(1)
                                    });
                                    let health_percent = match (hp_opt, ability_opt) {
                                        (Some(hp), Some(ability)) => {
                                            let max_hp = ability.max_health + ability.adjust.max_health;
                                            if max_hp > 0 {
                                                Some((hp.hp as f32 / max_hp as f32 * 100.0) as u8)
                                            } else {
                                                Some(100)
                                            }
                                        }
                                        (Some(_), None) => Some(100),
                                        _ => None,
                                    };

                                    entities.push(crate::game::api::models::NearbyEntity {
                                        entity_id: other_client_ent.id.0 as u32,
                                        entity_type: crate::game::api::models::NearbyEntityType::Monster,
                                        name,
                                        level: Some(level),
                                        position: crate::game::api::models::Position::new(other_pos.position.x, other_pos.position.y, other_pos.position.z),
                                        distance,
                                        health_percent,
                                    });
                                } else if let Some(item_drop) = item_opt {
                                    // It's an item drop
                                    let name = item_drop.item.as_ref().and_then(|dropped_item| {
                                        let item_ref = match dropped_item {
                                            crate::game::components::DroppedItem::Item(item) => item.get_item_reference(),
                                            crate::game::components::DroppedItem::Money(_) => return Some("Money".to_string()),
                                        };
                                        game_data.items.get_base_item(item_ref).map(|i| i.name.to_string())
                                    }).unwrap_or_else(|| "Dropped Item".to_string());

                                    entities.push(crate::game::api::models::NearbyEntity {
                                        entity_id: other_client_ent.id.0 as u32,
                                        entity_type: crate::game::api::models::NearbyEntityType::Item,
                                        name,
                                        level: None,
                                        position: crate::game::api::models::Position::new(other_pos.position.x, other_pos.position.y, other_pos.position.z),
                                        distance,
                                        health_percent: None,
                                    });
                                } else if let Some(char_info) = char_info_opt {
                                    // It's a player
                                    let health_percent = match (hp_opt, ability_opt) {
                                        (Some(hp), Some(ability)) => {
                                            let max_hp = ability.max_health + ability.adjust.max_health;
                                            if max_hp > 0 {
                                                Some((hp.hp as f32 / max_hp as f32 * 100.0) as u8)
                                            } else {
                                                Some(100)
                                            }
                                        }
                                        (Some(_), None) => Some(100),
                                        _ => None,
                                    };
                                    
                                    // Get level from Level component
                                    let level = level_opt.map(|l| l.level as u16);

                                    entities.push(crate::game::api::models::NearbyEntity {
                                        entity_id: other_client_ent.id.0 as u32,
                                        entity_type: crate::game::api::models::NearbyEntityType::Player,
                                        name: char_info.name.clone(),
                                        level,
                                        position: crate::game::api::models::Position::new(other_pos.position.x, other_pos.position.y, other_pos.position.z),
                                        distance,
                                        health_percent,
                                    });
                                }
                            }
                        }

                        let _ = response_tx.send(crate::game::api::GetBotContextResponse {
                            success: true,
                            error: None,
                            entities,
                        });
                    } else {
                        log::warn!("GetBotContext failed: bot {} entity {:?} not found", bot_id, entity);
                        let _ = response_tx.send(crate::game::api::GetBotContextResponse {
                            success: false,
                            error: Some("Bot entity not found".to_string()),
                            entities: vec![],
                        });
                    }
                } else {
                    log::warn!("GetBotContext failed: bot {} not found in bots_map", bot_id);
                    let _ = response_tx.send(crate::game::api::GetBotContextResponse {
                        success: false,
                        error: Some("Bot not found".to_string()),
                        entities: vec![],
                    });
                }
            }
            LlmBotCommand::GetBotSkills { bot_id, response_tx } => {
                // Get skills for this bot
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    if let Ok(skill_list) = skill_query.get(entity) {
                        let mut skills: Vec<crate::game::api::models::SkillInfo> = Vec::new();
                        let mut slot_index: u8 = 0;
                        
                        for page in &skill_list.pages {
                            for skill_id_opt in &page.skills {
                                if let Some(skill_id) = skill_id_opt {
                                    if let Some(skill_data) = game_data.skills.get_skill(*skill_id) {
                                        // Get MP cost from use_ability where type is Mana
                                        let mp_cost = skill_data.use_ability.iter()
                                            .find_map(|(ability_type, value)| {
                                                if *ability_type == rose_data::AbilityType::Mana {
                                                    Some(*value as u16)
                                                } else {
                                                    None
                                                }
                                            })
                                            .unwrap_or(0);
                                        // Get cooldown duration from SkillCooldown enum
                                        let cooldown = match skill_data.cooldown {
                                            rose_data::SkillCooldown::Skill { duration } => duration.as_secs_f32(),
                                            rose_data::SkillCooldown::Group { duration, .. } => duration.as_secs_f32(),
                                        };
                                        skills.push(crate::game::api::models::SkillInfo {
                                            slot: slot_index,
                                            skill_id: skill_id.get(),
                                            name: skill_data.name.to_string(),
                                            level: skill_data.level as u8,
                                            mp_cost,
                                            cooldown,
                                        });
                                    }
                                }
                                slot_index = slot_index.saturating_add(1);
                            }
                        }
                        
                        log::info!("LLM bot {} skills query: {} skills found", bot_id, skills.len());
                        
                        let _ = response_tx.send(GetBotSkillsResponse {
                            success: true,
                            error: None,
                            skills,
                        });
                    } else {
                        log::warn!("GetBotSkills failed: bot {} entity {:?} not found or has no SkillList", bot_id, entity);
                        let _ = response_tx.send(GetBotSkillsResponse {
                            success: false,
                            error: Some("Bot entity not found or has no skills".to_string()),
                            skills: vec![],
                        });
                    }
                } else {
                    log::warn!("GetBotSkills failed: bot {} not found in bots_map", bot_id);
                    let _ = response_tx.send(GetBotSkillsResponse {
                        success: false,
                        error: Some("Bot not found".to_string()),
                        skills: vec![],
                    });
                }
            }
            LlmBotCommand::GetBotList { response_tx } => {
                // Get all bots with their current position and status
                let mut bots: Vec<BotSummaryData> = Vec::new();
                
                // Snapshot (same iteration order) so queries and logging run
                // without holding the lock the API threads also take.
                let bots_snapshot: Vec<(Uuid, BotInfo)> = bot_manager
                    .bots_map
                    .read()
                    .iter()
                    .map(|(bot_id, bot_info)| (*bot_id, bot_info.clone()))
                    .collect();
                for (bot_id, bot_info) in bots_snapshot.iter() {
                    let entity = bot_info.entity;
                    
                    // Try to get position and health from the entity
                    if let Ok((position, health_opt, mana_opt, stamina_opt, ability_values_opt, dead_opt)) = bot_status_query.get(entity) {
                        let (x, y, z) = (position.position.x, position.position.y, position.position.z);
                        let zone_id = position.zone_id.get();
                        
                        // Get health points - current from HealthPoints, max from AbilityValues
                        let health = match (health_opt, ability_values_opt) {
                            (Some(hp), Some(ability_values)) => {
                                crate::game::api::models::VitalPoints::new(
                                    hp.hp as u32,
                                    (ability_values.max_health + ability_values.adjust.max_health) as u32,
                                )
                            }
                            (Some(hp), None) => {
                                crate::game::api::models::VitalPoints::new(hp.hp as u32, hp.hp as u32)
                            }
                            _ => crate::game::api::models::VitalPoints::new(100, 100)
                        };
                        
                        // Get mana points - current from ManaPoints, max from AbilityValues
                        let mana = match (mana_opt, ability_values_opt) {
                            (Some(mp), Some(ability_values)) => {
                                crate::game::api::models::VitalPoints::new(
                                    mp.mp as u32,
                                    (ability_values.max_mana + ability_values.adjust.max_mana) as u32,
                                )
                            }
                            (Some(mp), None) => {
                                crate::game::api::models::VitalPoints::new(mp.mp as u32, mp.mp as u32)
                            }
                            _ => crate::game::api::models::VitalPoints::new(100, 100)
                        };
                        
                        // Get stamina points - current from Stamina, max is typically 100
                        let stamina = stamina_opt.map(|s| {
                            crate::game::api::models::VitalPoints::new(s.stamina as u32, 100)
                        }).unwrap_or_else(|| crate::game::api::models::VitalPoints::new(100, 100));
                        
                        // Determine status based on whether bot is dead
                        let status = if dead_opt.is_some() {
                            "dead".to_string()
                        } else {
                            "active".to_string()
                        };
                        
                        log::info!("GetBotList: bot {} entity {:?} found at {:?} in zone {}", bot_id, entity, position.position, zone_id);
                        bots.push(BotSummaryData {
                            bot_id: *bot_id,
                            name: bot_info.name.clone(),
                            level: bot_info.level,
                            health,
                            mana,
                            stamina,
                            position: crate::game::api::models::ZonePosition::new(x, y, z, zone_id),
                            assigned_player: bot_info.assigned_player.clone(),
                            status,
                        });
                    } else {
                        // Bot entity not found in query - use placeholder data
                        log::debug!("GetBotList: bot {} entity {:?} not in query, using placeholder", bot_id, entity);
                        bots.push(BotSummaryData {
                            bot_id: *bot_id,
                            name: bot_info.name.clone(),
                            level: bot_info.level,
                            health: crate::game::api::models::VitalPoints::new(100, 100),
                            mana: crate::game::api::models::VitalPoints::new(100, 100),
                            stamina: crate::game::api::models::VitalPoints::new(100, 100),
                            position: crate::game::api::models::ZonePosition::new(0.0, 0.0, 0.0, 0),
                            assigned_player: bot_info.assigned_player.clone(),
                            status: "spawning".to_string(),
                        });
                    }
                }
                
                log::info!("GetBotList: returning {} bots", bots.len());
                let _ = response_tx.send(GetBotListResponse {
                    success: true,
                    error: None,
                    bots,
                });
            }
            LlmBotCommand::GetChatHistory { bot_id, response_tx } => {
                // Get chat history for this bot
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    if let Ok((buddy_bot, _, _, _, _)) = bot_query.get(entity) {
                        // Convert chat messages from component to API format
                        let messages: Vec<crate::game::api::models::ChatMessage> = buddy_bot
                            .chat_messages
                            .iter()
                            .map(|msg| crate::game::api::models::ChatMessage {
                                timestamp: msg.timestamp.to_rfc3339(),
                                sender_name: msg.sender_name.clone(),
                                sender_entity_id: msg.sender_entity_id,
                                message: msg.message.clone(),
                                chat_type: format!("{:?}", msg.chat_type).to_lowercase(),
                            })
                            .collect();
                        
                        log::info!("LLM bot {} chat history query: {} messages found", bot_id, messages.len());
                        
                        let _ = response_tx.send(crate::game::api::GetChatHistoryResponse {
                            success: true,
                            error: None,
                            messages,
                        });
                    } else {
                        log::warn!("GetChatHistory failed: bot {} entity {:?} not found or has no LlmBuddyBot", bot_id, entity);
                        let _ = response_tx.send(crate::game::api::GetChatHistoryResponse {
                            success: false,
                            error: Some("Bot entity not found".to_string()),
                            messages: vec![],
                        });
                    }
                } else {
                    log::warn!("GetChatHistory failed: bot {} not found in bots_map", bot_id);
                    let _ = response_tx.send(crate::game::api::GetChatHistoryResponse {
                        success: false,
                        error: Some("Bot not found".to_string()),
                        messages: vec![],
                    });
                }
            }
            LlmBotCommand::UseItem {
                bot_id,
                item_slot,
                target_entity_id,
            } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!(
                        "Processing UseItem command for bot {}: entity {:?}",
                        bot_id,
                        entity
                    );

                    let target_entity = if let Some(entity_id) = target_entity_id {
                        // ClientEntityIds are per zone; resolve within the bot's zone
                        // and never fall back to an unrelated entity.
                        let bot_zone = bot_status_query
                            .get(entity)
                            .ok()
                            .map(|(position, _, _, _, _, _)| position.zone_id);
                        let target = bot_zone.and_then(|bot_zone| {
                            entity_query
                                .iter()
                                .find(|(_, client_entity, pos_opt, _, _, _, _, _, _, _, _)| {
                                    client_entity.id.0 == entity_id as usize
                                        && pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
                                })
                                .map(|(entity, _, _, _, _, _, _, _, _, _, _)| entity)
                        });
                        if target.is_none() {
                            log::warn!(
                                "LLM bot {} cannot find item target with ClientEntityId {} in its zone {:?}",
                                bot_id,
                                entity_id,
                                bot_zone
                            );
                        }
                        target
                    } else {
                        // Default to self if no target specified
                        Some(entity)
                    };

                    if let Some(target_entity) = target_entity {
                        use_item_events.write(crate::game::events::UseItemEvent::from_inventory(
                            entity,
                            rose_game_common::components::ItemSlot::Inventory(
                                rose_game_common::components::InventoryPageType::Consumables,
                                item_slot as usize,
                            ),
                            Some(target_entity),
                        ));
                    }
                }
            }
            LlmBotCommand::SetBehaviorMode { bot_id, mode } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    log::info!(
                        "Processing SetBehaviorMode command for bot {}: entity {:?}, mode {:?}",
                        bot_id,
                        entity,
                        mode
                    );

                    if let Ok((mut buddy_bot, _, _, _, _)) = bot_query.get_mut(entity) {
                        buddy_bot.behavior_mode = mode;
                        log::info!("LLM bot {} behavior mode set to {:?}", bot_id, mode);
                    }
                }
            }
            LlmBotCommand::GetZoneInfo { bot_id, response_tx } => {
                let bot_info = bot_manager.bots_map.read().get(&bot_id).cloned();
                if let Some(bot_info) = bot_info {
                    let entity = bot_info.entity;
                    if let Ok((_, _, _, bot_position, _)) = bot_query.get(entity) {
                        let zone_id = bot_position.zone_id;
                        let zone_data = game_data.zones.get_zone(zone_id);

                        let zone_name = zone_data
                            .map(|z| z.name.to_string())
                            .unwrap_or_else(|| format!("Zone {}", zone_id.get()));
                        
                        // Calculate recommended level range from monster spawns in the zone
                        let (recommended_level_min, recommended_level_max) = if let Some(zone) = zone_data {
                            let mut min_level = u16::MAX;
                            let mut max_level = 0u16;
                            let mut found_monsters = false;
                            
                            for spawn in &zone.monster_spawns {
                                // Check basic spawns
                                for (npc_id, _count) in &spawn.basic_spawns {
                                    if let Some(npc) = game_data.npcs.get_npc(*npc_id) {
                                        found_monsters = true;
                                        min_level = min_level.min(npc.level as u16);
                                        max_level = max_level.max(npc.level as u16);
                                    }
                                }
                                // Check tactic spawns
                                for (npc_id, _count) in &spawn.tactic_spawns {
                                    if let Some(npc) = game_data.npcs.get_npc(*npc_id) {
                                        found_monsters = true;
                                        min_level = min_level.min(npc.level as u16);
                                        max_level = max_level.max(npc.level as u16);
                                    }
                                }
                            }
                            
                            if found_monsters {
                                (min_level, max_level)
                            } else {
                                (1, 100) // Default range if no monsters found
                            }
                        } else {
                            (1, 100) // Default range if zone not found
                        };

                        let _ = response_tx.send(GetZoneInfoResponse {
                            success: true,
                            error: None,
                            zone_name,
                            zone_id: zone_id.get(),
                            recommended_level_min,
                            recommended_level_max,
                        });
                    } else {
                        let _ = response_tx.send(GetZoneInfoResponse {
                            success: false,
                            error: Some("Bot entity not found".to_string()),
                            zone_name: "".to_string(),
                            zone_id: 0,
                            recommended_level_min: 0,
                            recommended_level_max: 0,
                        });
                    }
                } else {
                    let _ = response_tx.send(GetZoneInfoResponse {
                        success: false,
                        error: Some("Bot not found".to_string()),
                        zone_name: "".to_string(),
                        zone_id: 0,
                        recommended_level_min: 0,
                        recommended_level_max: 0,
                    });
                }
            }
        }
    }

    // Suppress unused variable warning
    let _ = game_data;

    // Process pending deletions
    let deletes: Vec<(Uuid, ResponseSender<DeleteBotResponse>)> = bot_manager.pending_deletes.drain(..).collect();
    if !deletes.is_empty() {
        log::info!("Processing {} pending bot deletions", deletes.len());
    }
    for (bot_id, response_tx) in deletes {
        log::info!("Processing deletion for bot {}", bot_id);
        
        // Check if bot exists in the map
        let bot_info_opt = bot_manager.bots_map.write().remove(&bot_id);
        
        if let Some(bot_info) = bot_info_opt {
            let entity_to_despawn = bot_info.entity;
            let bot_name = bot_info.name.clone();
            
            log::info!("Found bot {} in map: entity {:?}, name '{}'", bot_id, entity_to_despawn, bot_name);
            
            // Check if entity is valid (not a placeholder with index 0)
            if entity_to_despawn.index_u32() == 0 && entity_to_despawn.generation().to_bits() == 0 {
                log::warn!("Bot {} has placeholder entity (0, 0), skipping despawn", bot_id);
            } else {
                // First, properly remove the bot from the zone so clients are notified
                if let Ok((client_entity, client_entity_sector, position)) = bot_delete_query.get(entity_to_despawn) {
                    client_entity_leave_zone(
                        &mut commands,
                        &mut client_entity_list,
                        entity_to_despawn,
                        client_entity,
                        client_entity_sector,
                        position,
                    );
                    log::info!("Removed LLM buddy bot from zone: {} (entity: {:?})", bot_id, entity_to_despawn);
                } else {
                    log::warn!("Bot {} entity {:?} missing ClientEntity/ClientEntitySector/Position components", bot_id, entity_to_despawn);
                }
                
                // Despawn the entity
                commands.entity(entity_to_despawn).despawn();
                log::info!("Despawned LLM buddy bot: {} (entity: {:?})", bot_id, entity_to_despawn);
            }
            
            // Delete from persistent storage
            match LlmBuddyBotStorage::delete(&bot_name) {
                Ok(_) => {
                    log::info!("Deleted LLM buddy bot '{}' from persistent storage", bot_name);
                    // Send success response
                    match response_tx.send(DeleteBotResponse {
                        success: true,
                        error: None,
                    }) {
                        Ok(_) => log::info!("Sent success response for bot {} deletion", bot_id),
                        Err(_) => log::error!("Failed to send success response for bot {} deletion: requester no longer waiting", bot_id),
                    }
                }
                Err(e) => {
                    log::error!("Failed to delete LLM buddy bot '{}' from storage: {:?}", bot_name, e);
                    // Send failure response
                    match response_tx.send(DeleteBotResponse {
                        success: false,
                        error: Some(format!("Failed to delete from storage: {:?}", e)),
                    }) {
                        Ok(_) => log::info!("Sent storage failure response for bot {} deletion", bot_id),
                        Err(_) => log::error!("Failed to send failure response for bot {} deletion: requester no longer waiting", bot_id),
                    }
                }
            }
        } else {
            log::warn!("LLM buddy bot {} not found in map for deletion", bot_id);
            // Send failure response - bot not found
            match response_tx.send(DeleteBotResponse {
                success: false,
                error: Some(format!("Bot {} not found in bots map", bot_id)),
            }) {
                Ok(_) => log::info!("Sent not-found response for bot {} deletion", bot_id),
                Err(_) => log::error!("Failed to send not-found response for bot {} deletion: requester no longer waiting", bot_id),
            }
        }
    }
}

/// Parse move mode from string
fn parse_move_mode(mode: &str) -> Option<MoveMode> {
    match mode.to_lowercase().as_str() {
        "walk" => Some(MoveMode::Walk),
        "run" => Some(MoveMode::Run),
        _ => Some(MoveMode::Run), // Default to run
    }
}

/// How often a buddy bot re-evaluates heals, buffs and combat.
const BUDDY_THINK_INTERVAL: Duration = Duration::from_millis(400);
/// Heal the assigned player below this fraction of their max health.
const BUDDY_HEAL_PLAYER_HEALTH_RATIO: f32 = 0.7;
/// Heal itself below this fraction of its max health.
const BUDDY_HEAL_SELF_HEALTH_RATIO: f32 = 0.5;
/// Buffs are only cast while this fraction of mana is left for heals.
const BUDDY_BUFF_MIN_MANA_RATIO: f32 = 0.35;
/// A player further away than this is not chased down to be buffed.
const BUDDY_BUFF_RANGE_CM: f32 = 1500.0;
/// Monsters that hit the player or the bot this recently are fought back.
const BUDDY_RECENT_ATTACK_SECS: f64 = 5.0;
/// Only monsters this close to the player are fought.
const BUDDY_ASSIST_RANGE_CM: f32 = 1500.0;
/// Aggressive bots also engage any monster this close to the player.
const BUDDY_AGGRESSIVE_RANGE_CM: f32 = 1000.0;
/// A fighting bot breaks off and runs back once the player is this far away.
/// Larger than the assist range plus a ranged attack, so it does not re-engage at once.
const BUDDY_LEASH_CM: f32 = 3000.0;
/// A running follow move is re-aimed once the player is this far from its destination.
const BUDDY_FOLLOW_RETARGET_MIN_CM: f32 = 400.0;

/// A buddy bot, as driven by the built-in follow / support behaviour.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct BuddyBotQuery<'w> {
    entity: Entity,
    buddy: &'w mut LlmBuddyBot,
    next_command: &'w mut NextCommand,
    command: &'w Command,
    position: &'w Position,
    client_entity: &'w ClientEntity,
    ability_values: &'w AbilityValues,
    health_points: &'w HealthPoints,
    mana_points: &'w ManaPoints,
    move_mode: &'w MoveMode,
    team: &'w Team,
    skill_list: &'w SkillList,
    status_effects: &'w StatusEffects,
    clan_membership: Option<&'w ClanMembership>,
    party_membership: Option<&'w PartyMembership>,
    cooldowns: Option<&'w Cooldowns>,
    equipment: Option<&'w Equipment>,
    experience_points: Option<&'w ExperiencePoints>,
    inventory: Option<&'w Inventory>,
    stamina: Option<&'w Stamina>,
    damage_sources: Option<&'w DamageSources>,
}

/// The player a buddy bot is assigned to.
#[derive(QueryData)]
pub struct BuddyPlayerQuery<'w> {
    entity: Entity,
    character_info: &'w CharacterInfo,
    position: &'w Position,
    client_entity: &'w ClientEntity,
    command: &'w Command,
    ability_values: &'w AbilityValues,
    health_points: &'w HealthPoints,
    status_effects: &'w StatusEffects,
    team: &'w Team,
    clan_membership: Option<&'w ClanMembership>,
    party_membership: Option<&'w PartyMembership>,
    damage_sources: Option<&'w DamageSources>,
    dead: Option<&'w Dead>,
}

/// Built-in behaviour of LLM buddy bots. It needs neither the REST API nor an LLM:
/// each bot follows its assigned player, heals and buffs them with its own skills and,
/// in Defensive / Aggressive mode, fights what attacks them.
///
/// Bots under manual API control (`is_following == false`) are left alone. Bringing a
/// bot into the player's zone, catch-up teleports, revives and the auto-party are done
/// by `llm_bot_teleport_to_player_on_login_system`.
pub fn llm_buddy_follow_system(
    mut bot_query: Query<BuddyBotQuery, (Without<Dead>, Without<GameClient>)>,
    player_query: Query<BuddyPlayerQuery, (With<GameClient>, Without<LlmBuddyBot>)>,
    target_query: Query<(&ClientEntity, &Position, &HealthPoints, &Team), Without<Dead>>,
    client_entity_list: Res<ClientEntityList>,
    game_data: Res<GameData>,
    time: Res<Time>,
) {
    let now = Instant::now();
    let now_seconds = time.elapsed_secs_f64();

    for mut bot in bot_query.iter_mut() {
        if !bot.buddy.is_following {
            continue;
        }

        let Some(player) = player_query
            .iter()
            .find(|player| player.character_info.name == bot.buddy.assigned_player_name)
        else {
            continue;
        };

        if player.position.zone_id != bot.position.zone_id {
            // The teleport system brings the bot over once the player has joined the zone.
            continue;
        }

        if bot
            .buddy
            .next_think_at
            .map_or(true, |think_at| now >= think_at)
        {
            bot.buddy.next_think_at = Some(now + BUDDY_THINK_INTERVAL);

            let action = buddy_choose_heal(now, &game_data, &bot, &player)
                .or_else(|| buddy_choose_buff(now, &game_data, &bot, &player))
                .or_else(|| {
                    buddy_choose_attack(
                        &bot,
                        &player,
                        &target_query,
                        &client_entity_list,
                        now_seconds,
                    )
                });
            if let Some(action) = action {
                // follow_goal is kept: a cast restores the follow move it interrupted.
                *bot.next_command = action;
                continue;
            }
        }

        buddy_follow_player(&mut bot, &player);
    }
}

/// Moves the bot back to the player when it is further away than its follow distance,
/// stopping on its own side of the player at half that distance.
fn buddy_follow_player(bot: &mut BuddyBotQueryItem, player: &BuddyPlayerQueryItem) {
    let follow_distance = bot.buddy.follow_distance_cm();
    let bot_xy = bot.position.position.xy();
    let player_xy = player.position.position.xy();
    let distance = bot_xy.distance(player_xy);

    let fighting = matches!(bot.command.command, CommandData::Attack { .. })
        || matches!(bot.next_command.command, Some(CommandData::Attack { .. }));
    if fighting {
        if distance <= BUDDY_LEASH_CM {
            return;
        }
    } else if !buddy_is_free(bot) {
        // Casting, picking something up, standing up...
        return;
    }

    let running_follow_move = bot.buddy.follow_goal.filter(|_| {
        matches!(bot.command.command, CommandData::Move { target: None, .. })
            || matches!(
                bot.next_command.command,
                Some(CommandData::Move { target: None, .. })
            )
    });
    let should_move = match running_follow_move {
        Some(goal) if !fighting => {
            goal.xy().distance(player_xy) > follow_distance.max(BUDDY_FOLLOW_RETARGET_MIN_CM)
        }
        _ => distance > follow_distance,
    };

    if !should_move {
        if running_follow_move.is_none() && bot.buddy.follow_goal.is_some() {
            bot.buddy.follow_goal = None;
        }
        return;
    }

    let away_from_player = (bot_xy - player_xy).try_normalize().unwrap_or(Vec2::NEG_Y);
    let stop_xy = player_xy + away_from_player * (follow_distance * 0.5);
    let destination = Vec3::new(stop_xy.x, stop_xy.y, player.position.position.z);

    *bot.next_command = NextCommand::with_move(destination, None, Some(MoveMode::Run));
    bot.buddy.follow_goal = Some(destination);
}

/// True when the bot is idle or running its own follow move, so it can take new orders.
fn buddy_is_free(bot: &BuddyBotQueryItem) -> bool {
    let following = bot.buddy.follow_goal.is_some();

    let command_free = match &bot.command.command {
        CommandData::Stop { .. }
        | CommandData::Sit
        | CommandData::Sitting
        | CommandData::Emote { .. } => true,
        CommandData::Move { target: None, .. } => following,
        _ => false,
    };
    let next_command_free = match &bot.next_command.command {
        None | Some(CommandData::Stop { .. }) => true,
        Some(CommandData::Move { target: None, .. }) => following,
        _ => false,
    };

    command_free && next_command_free
}

fn buddy_is_casting(bot: &BuddyBotQueryItem) -> bool {
    matches!(bot.command.command, CommandData::CastSkill { .. })
        || matches!(
            bot.next_command.command,
            Some(CommandData::CastSkill { .. })
        )
}

fn health_ratio(health_points: &HealthPoints, ability_values: &AbilityValues) -> f32 {
    let max_health = ability_values.get_max_health();
    if max_health > 0 {
        health_points.hp as f32 / max_health as f32
    } else {
        1.0
    }
}

fn mana_ratio(mana_points: &ManaPoints, ability_values: &AbilityValues) -> f32 {
    let max_mana = ability_values.get_max_mana();
    if max_mana > 0 {
        mana_points.mp as f32 / max_mana as f32
    } else {
        0.0
    }
}

/// Every skill the bot has learned.
fn buddy_skills<'a>(
    game_data: &'a GameData,
    skill_list: &'a SkillList,
) -> impl Iterator<Item = &'a SkillData> + 'a {
    skill_list
        .pages
        .iter()
        .flat_map(|page| page.skills.iter())
        .filter_map(|skill_slot| skill_slot.as_ref())
        .filter_map(move |skill_id| game_data.skills.get_skill(*skill_id))
}

fn skill_status_effect_types<'a>(
    game_data: &'a GameData,
    skill_data: &'a SkillData,
) -> impl Iterator<Item = StatusEffectType> + 'a {
    skill_data
        .status_effects
        .iter()
        .flatten()
        .filter_map(move |&status_effect_id| {
            game_data.status_effects.get_status_effect(status_effect_id)
        })
        .map(|status_effect_data| status_effect_data.status_effect_type)
}

fn skill_heals_instantly(skill_data: &SkillData) -> bool {
    skill_data
        .add_ability
        .iter()
        .flatten()
        .any(|add_ability| add_ability.ability_type == AbilityType::Health)
}

/// Whether `skill_data` restores health, instantly or over time.
fn skill_heals(game_data: &GameData, skill_data: &SkillData) -> bool {
    skill_heals_instantly(skill_data)
        || skill_status_effect_types(game_data, skill_data)
            .any(|status_effect_type| matches!(status_effect_type, StatusEffectType::IncreaseHp))
}

/// A heal over time does nothing on a target that already has it running.
fn heal_takes_effect(
    game_data: &GameData,
    skill_data: &SkillData,
    target_status_effects: &StatusEffects,
) -> bool {
    skill_heals_instantly(skill_data)
        || skill_status_effect_types(game_data, skill_data).any(|status_effect_type| {
            target_status_effects
                .get_status_effect_value(status_effect_type)
                .is_none()
        })
}

fn is_buff_status_effect(status_effect_type: StatusEffectType) -> bool {
    matches!(
        status_effect_type,
        StatusEffectType::IncreaseMaxHp
            | StatusEffectType::IncreaseMaxMp
            | StatusEffectType::IncreaseMoveSpeed
            | StatusEffectType::IncreaseAttackSpeed
            | StatusEffectType::IncreaseAttackPower
            | StatusEffectType::IncreaseDefence
            | StatusEffectType::IncreaseResistance
            | StatusEffectType::IncreaseHit
            | StatusEffectType::IncreaseCritical
            | StatusEffectType::IncreaseAvoid
            | StatusEffectType::ShieldDamage
            | StatusEffectType::AdditionalDamageRate
    )
}

/// Whether `skill_data` only grants buffs. Cures and other clears are excluded: their
/// effect never shows as active, so they would be recast forever.
fn skill_is_buff(game_data: &GameData, skill_data: &SkillData) -> bool {
    if !matches!(
        skill_data.skill_type,
        SkillType::SelfBoundDuration
            | SkillType::SelfStateDuration
            | SkillType::TargetBoundDuration
            | SkillType::TargetStateDuration
            | SkillType::SelfBound
            | SkillType::TargetBound
    ) || skill_heals(game_data, skill_data)
    {
        return false;
    }

    let mut status_effect_types = skill_status_effect_types(game_data, skill_data).peekable();
    status_effect_types.peek().is_some() && status_effect_types.all(is_buff_status_effect)
}

fn lacks_skill_effects(
    game_data: &GameData,
    skill_data: &SkillData,
    status_effects: &StatusEffects,
) -> bool {
    !skill_status_effect_types(game_data, skill_data).any(|status_effect_type| {
        status_effects
            .get_status_effect_value(status_effect_type)
            .is_some()
    })
}

/// The use requirements command_system enforces; a cast it refuses stops the bot.
fn buddy_can_use_skill(
    now: Instant,
    game_data: &GameData,
    bot: &BuddyBotQueryItem,
    skill_data: &SkillData,
) -> bool {
    skill_use_requirements_met(
        now,
        game_data,
        bot.client_entity,
        bot.ability_values,
        bot.health_points,
        bot.move_mode,
        bot.cooldowns,
        bot.equipment,
        bot.experience_points,
        bot.inventory,
        Some(bot.mana_points),
        bot.stamina,
        skill_data,
    )
}

/// The target filter command_system checks for a cast on the player.
fn buddy_can_target_player(
    bot: &BuddyBotQueryItem,
    player: &BuddyPlayerQueryItem,
    skill_data: &SkillData,
) -> bool {
    skill_target_filter_allows(
        bot.entity,
        bot.team,
        bot.clan_membership,
        bot.party_membership,
        player.entity,
        player.team,
        player.clan_membership,
        player.party_membership,
        player.health_points.hp,
        player.client_entity,
        skill_data,
    )
}

/// The check command_system makes for a cast with no target (applied to the caster).
fn buddy_can_target_self(bot: &BuddyBotQueryItem, skill_data: &SkillData) -> bool {
    skill_data.skill_type.is_self_skill()
        || skill_target_filter_allows(
            bot.entity,
            bot.team,
            bot.clan_membership,
            bot.party_membership,
            bot.entity,
            bot.team,
            bot.clan_membership,
            bot.party_membership,
            bot.health_points.hp,
            bot.client_entity,
            skill_data,
        )
}

/// Heals the player (or the bot itself) when they are low, the lower one first.
fn buddy_choose_heal(
    now: Instant,
    game_data: &GameData,
    bot: &BuddyBotQueryItem,
    player: &BuddyPlayerQueryItem,
) -> Option<NextCommand> {
    if buddy_is_casting(bot) {
        return None;
    }

    let player_alive = player.dead.is_none() && player.health_points.hp > 0;
    let player_ratio = if player_alive {
        health_ratio(player.health_points, player.ability_values)
    } else {
        1.0
    };
    let self_ratio = health_ratio(bot.health_points, bot.ability_values);
    let heal_player = player_ratio < BUDDY_HEAL_PLAYER_HEALTH_RATIO;
    let heal_self = self_ratio < BUDDY_HEAL_SELF_HEALTH_RATIO;
    if !heal_player && !heal_self {
        return None;
    }

    let player_distance = bot
        .position
        .position
        .xy()
        .distance(player.position.position.xy());
    let mut player_cast = None;
    let mut self_cast = None;

    for skill_data in buddy_skills(game_data, bot.skill_list) {
        if !skill_heals(game_data, skill_data)
            || !buddy_can_use_skill(now, game_data, bot, skill_data)
        {
            continue;
        }

        if heal_player
            && player_cast.is_none()
            && heal_takes_effect(game_data, skill_data, player.status_effects)
            && buddy_can_target_player(bot, player, skill_data)
        {
            if skill_data.skill_type.is_target_skill() {
                player_cast = Some(NextCommand::with_cast_skill_target_entity(
                    skill_data.id,
                    player.entity,
                    None,
                ));
            } else if skill_data.scope > 0
                && player_distance <= skill_data.scope as f32
                && buddy_can_target_self(bot, skill_data)
            {
                // An area heal centred on the bot reaches the player.
                player_cast = Some(NextCommand::with_cast_skill_target_self(
                    skill_data.id,
                    None,
                ));
            }
        }

        if heal_self
            && self_cast.is_none()
            && heal_takes_effect(game_data, skill_data, bot.status_effects)
            && buddy_can_target_self(bot, skill_data)
        {
            self_cast = Some(NextCommand::with_cast_skill_target_self(
                skill_data.id,
                None,
            ));
        }
    }

    if heal_self && self_ratio < player_ratio {
        self_cast.or(player_cast)
    } else {
        player_cast.or(self_cast)
    }
}

/// Keeps the bot's buffs up on the player and itself while it has mana to spare.
fn buddy_choose_buff(
    now: Instant,
    game_data: &GameData,
    bot: &BuddyBotQueryItem,
    player: &BuddyPlayerQueryItem,
) -> Option<NextCommand> {
    if buddy_is_casting(bot)
        || !buddy_is_free(bot)
        || mana_ratio(bot.mana_points, bot.ability_values) < BUDDY_BUFF_MIN_MANA_RATIO
    {
        return None;
    }

    let player_distance = bot
        .position
        .position
        .xy()
        .distance(player.position.position.xy());
    let player_in_range = player.dead.is_none()
        && player.health_points.hp > 0
        && player_distance <= BUDDY_BUFF_RANGE_CM;

    for skill_data in buddy_skills(game_data, bot.skill_list) {
        if !skill_is_buff(game_data, skill_data)
            || !buddy_can_use_skill(now, game_data, bot, skill_data)
        {
            continue;
        }

        let player_needs_buff = player_in_range
            && lacks_skill_effects(game_data, skill_data, player.status_effects)
            && buddy_can_target_player(bot, player, skill_data);
        if player_needs_buff && skill_data.skill_type.is_target_skill() {
            return Some(NextCommand::with_cast_skill_target_entity(
                skill_data.id,
                player.entity,
                None,
            ));
        }

        if buddy_can_target_self(bot, skill_data) {
            let self_needs_buff = lacks_skill_effects(game_data, skill_data, bot.status_effects);
            let reaches_player = player_needs_buff
                && skill_data.scope > 0
                && player_distance <= skill_data.scope as f32;
            if self_needs_buff || reaches_player {
                return Some(NextCommand::with_cast_skill_target_self(
                    skill_data.id,
                    None,
                ));
            }
        }
    }

    None
}

/// Defensive / Aggressive: attack what the player is fighting, else the latest monster to
/// hit the player or the bot; Aggressive bots also engage monsters near the player.
fn buddy_choose_attack(
    bot: &BuddyBotQueryItem,
    player: &BuddyPlayerQueryItem,
    target_query: &Query<(&ClientEntity, &Position, &HealthPoints, &Team), Without<Dead>>,
    client_entity_list: &ClientEntityList,
    now_seconds: f64,
) -> Option<NextCommand> {
    if !matches!(
        bot.buddy.behavior_mode,
        BotBehaviorMode::Defensive | BotBehaviorMode::Aggressive
    ) || !buddy_is_free(bot)
    {
        return None;
    }

    let zone_id = player.position.zone_id;
    let player_xy = player.position.position.xy();
    let is_valid_target = |target_entity: Entity| {
        target_query.get(target_entity).is_ok_and(
            |(target_client_entity, target_position, target_health_points, target_team)| {
                target_client_entity.is_monster()
                    && target_health_points.hp > 0
                    && target_team.id != bot.team.id
                    && target_position.zone_id == zone_id
                    && target_position.position.xy().distance(player_xy) <= BUDDY_ASSIST_RANGE_CM
            },
        )
    };

    if let Some(target) = player
        .command
        .target_entity()
        .filter(|&target| is_valid_target(target))
    {
        return Some(NextCommand::with_attack(target));
    }

    let recent_attacker = player
        .damage_sources
        .into_iter()
        .chain(bot.damage_sources)
        .flat_map(|damage_sources| damage_sources.damage_sources.iter())
        .filter(|damage_source| {
            now_seconds - damage_source.last_damage_seconds <= BUDDY_RECENT_ATTACK_SECS
                && is_valid_target(damage_source.entity)
        })
        .max_by(|a, b| a.last_damage_seconds.total_cmp(&b.last_damage_seconds))
        .map(|damage_source| damage_source.entity);
    if let Some(target) = recent_attacker {
        return Some(NextCommand::with_attack(target));
    }

    if matches!(bot.buddy.behavior_mode, BotBehaviorMode::Aggressive) {
        let bot_xy = bot.position.position.xy();
        let nearest_monster = client_entity_list.get_zone(zone_id).and_then(|zone| {
            zone.iter_entity_type_within_distance(
                player_xy,
                BUDDY_AGGRESSIVE_RANGE_CM,
                &[ClientEntityType::Monster],
            )
            .filter(|(entity, _)| is_valid_target(*entity))
            .min_by(|(_, a), (_, b)| {
                a.xy()
                    .distance_squared(bot_xy)
                    .total_cmp(&b.xy().distance_squared(bot_xy))
            })
            .map(|(entity, _)| entity)
        });
        if let Some(target) = nearest_monster {
            return Some(NextCommand::with_attack(target));
        }
    }

    None
}

/// System that captures nearby chat messages for buddy bots
///
/// This system listens for chat messages near buddy bots and stores them
/// in the bot's chat history for LLM context.
pub fn llm_buddy_chat_capture_system(
    mut bot_query: Query<(&Position, &mut LlmBuddyBot), Without<Dead>>,
    mut chat_events: MessageReader<crate::game::events::ChatMessageEvent>,
    client_entity_query: Query<&ClientEntity>,
) {
    // Chat capture radius
    // Increased by 10x to allow bots to capture chat from further away.
    const CHAT_CAPTURE_RADIUS: f32 = 20000.0;

    for event in chat_events.read() {
        for (bot_pos, mut buddy_bot) in bot_query.iter_mut() {
            // Skip if in different zones
            if bot_pos.zone_id != event.zone_id {
                continue;
            }

            // Check distance (we don't have the sender's position in the event yet,
            // but we can assume it's nearby if it's a local chat)
            // Actually, we should probably add sender_position to ChatMessageEvent
            
            let sender_entity_id = client_entity_query.get(event.sender_entity)
                .map(|ce| ce.id.0 as u32)
                .unwrap_or_else(|_| event.sender_entity.index_u32());

            // For now, just capture all local chat in the same zone
            buddy_bot.add_chat_message(BotChatMessage {
                timestamp: Utc::now(),
                sender_name: event.sender_name.clone(),
                sender_entity_id,
                message: event.message.clone(),
                chat_type: event.chat_type,
            });
        }
    }
}

/// Register a bot entity with the manager
pub fn register_llm_bot(
    bot_manager: &mut ResMut<LlmBotManagerResource>,
    bot_id: Uuid,
    entity: Entity,
    name: String,
    assigned_player: Option<String>,
    level: u16,
    class: String,
) {
    let info = BotInfo::new(entity, name, assigned_player, level, class);
    bot_manager.bots_map.write().insert(bot_id, info);
    log::info!("Registered LLM buddy bot {} -> entity {:?}", bot_id, entity);
}

/// Unregister a bot entity from the manager
pub fn unregister_llm_bot(bot_manager: &mut ResMut<LlmBotManagerResource>, bot_id: &Uuid) {
    bot_manager.bots_map.write().remove(bot_id);
    log::info!("Unregistered LLM buddy bot {}", bot_id);
}

/// System that auto-accepts party invites for LLM buddy bots
///
/// A buddy bot accepts an invite from its assigned player, or from the leader of the party
/// that player is in, and turns down everyone else, so it always ends up in its player's
/// party. The invites themselves are sent by `llm_bot_teleport_to_player_on_login_system`
/// (and bot creation) whenever the player logs in or changes zone.
pub fn llm_buddy_bot_auto_accept_party_system(
    mut bot_query: Query<(Entity, &LlmBuddyBot, &mut PartyMembership)>,
    character_query: Query<
        (
            Entity,
            &CharacterInfo,
            Option<&PartyMembership>,
            Has<ClientEntity>,
        ),
        Without<LlmBuddyBot>,
    >,
    party_query: Query<&Party>,
    mut party_events: MessageWriter<crate::game::events::PartyEvent>,
) {
    for (bot_entity, buddy_bot, mut party_membership) in bot_query.iter_mut() {
        if party_membership.party.is_some() || party_membership.pending_invites.is_empty() {
            continue;
        }

        let assigned_player = character_query.iter().find(|(_, character_info, _, _)| {
            character_info.name == buddy_bot.assigned_player_name
        });
        let assigned_player_entity = assigned_player.map(|(entity, _, _, _)| entity);
        let assigned_party_leader = assigned_player
            .and_then(|(_, _, membership, _)| membership.and_then(|membership| membership.party))
            .and_then(|party_entity| party_query.get(party_entity).ok())
            .map(|party| party.owner);

        let mut accepted = false;
        let mut stale_invites = Vec::new();
        for &owner_entity in party_membership.pending_invites.iter() {
            let Ok((_, _, _, owner_in_zone)) = character_query.get(owner_entity) else {
                // The inviter is gone; party_system cannot resolve (or reject) this invite.
                stale_invites.push(owner_entity);
                continue;
            };

            if !owner_in_zone {
                // party_system only resolves an answer while the inviter is in a zone.
                continue;
            }

            let allowed = Some(owner_entity) == assigned_player_entity
                || Some(owner_entity) == assigned_party_leader;
            if !allowed {
                party_events.write(crate::game::events::PartyEvent::RejectInvite {
                    reason: PartyRejectInviteReason::Busy,
                    owner_entity,
                    invited_entity: bot_entity,
                });
            } else if !accepted {
                log::info!(
                    "LLM buddy bot auto-accepting party invite from entity {:?}",
                    owner_entity
                );
                party_events.write(crate::game::events::PartyEvent::AcceptInvite {
                    owner_entity,
                    invited_entity: bot_entity,
                });
                accepted = true;
            }
        }

        if !stale_invites.is_empty() {
            party_membership
                .pending_invites
                .retain(|owner_entity| !stale_invites.contains(owner_entity));
        }
    }
}

/// System that handles admin commands for LLM bots
pub fn llm_bot_admin_command_system(
    mut chat_events: MessageReader<crate::game::events::ChatMessageEvent>,
    bot_manager: Res<LlmBotManagerResource>,
    mut bot_query: Query<(&mut LlmBuddyBot, &mut NextCommand, &Position, Entity)>,
    entity_query: Query<(Entity, &ClientEntity, &Position)>,
    game_data: Res<GameData>,
) {
    for event in chat_events.read() {
        if event.message == "@admin test all tools" {
            log::info!("[LLM_ADMIN] Received test command from {}", event.sender_name);
            
            // Find the first available bot (lock released before testing)
            let first_bot = bot_manager
                .bots_map
                .read()
                .iter()
                .next()
                .map(|(bot_id, bot_info)| (*bot_id, bot_info.clone()));
            if let Some((bot_id, bot_info)) = first_bot {
                let bot_entity = bot_info.entity;
                log::info!("[LLM_ADMIN] Testing tools for bot {} (entity {:?})", bot_id, bot_entity);
                
                if let Ok((mut buddy_bot, mut next_command, bot_pos, _)) = bot_query.get_mut(bot_entity) {
                    // 1. Test Chat
                    log::info!("[LLM_ADMIN] Test 1: Chat");
                    buddy_bot.add_chat_message(BotChatMessage {
                        timestamp: Utc::now(),
                        sender_name: "ADMIN_TEST".to_string(),
                        sender_entity_id: 0,
                        message: "Testing all tools...".to_string(),
                        chat_type: BotChatType::Local,
                    });

                    // 2. Test Move (move slightly)
                    log::info!("[LLM_ADMIN] Test 2: Move");
                    let test_dest = bot_pos.position + Vec3::new(100.0, 100.0, 0.0);
                    next_command.command = Some(CommandData::Move {
                        destination: test_dest,
                        target: None,
                        move_mode: Some(MoveMode::Run),
                    });
                    next_command.has_sent_server_message = false;

                    // 3. Test Follow (follow the sender)
                    log::info!("[LLM_ADMIN] Test 3: Follow");
                    buddy_bot.assigned_player_name = event.sender_name.clone();
                    buddy_bot.is_following = true;
                    buddy_bot.follow_distance = 50.0;

                    // 4. Test Attack (find nearest monster)
                    log::info!("[LLM_ADMIN] Test 4: Attack");
                    let bot_zone = bot_pos.zone_id;
                    let nearest_monster = entity_query.iter()
                        .filter(|(_, ce, p)| ce.is_monster() && p.zone_id == bot_zone)
                        .min_by(|(_, _, p1), (_, _, p2)| {
                            let d1 = bot_pos.position.distance(p1.position);
                            let d2 = bot_pos.position.distance(p2.position);
                            d1.partial_cmp(&d2).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    
                    if let Some((m_entity, m_ce, _)) = nearest_monster {
                        log::info!("[LLM_ADMIN] Found monster to attack: {:?} (ClientEntityId {})", m_entity, m_ce.id.0);
                        // We don't set it now because it would override the Move test
                        // But we log that we found it.
                    } else {
                        log::warn!("[LLM_ADMIN] No monster found to test Attack");
                    }

                    log::info!("[LLM_ADMIN] Tool tests initiated. Check logs for execution details.");
                } else {
                    log::error!("[LLM_ADMIN] Failed to get bot components for testing");
                }
            } else {
                log::warn!("[LLM_ADMIN] No LLM bots available to test");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_move_mode() {
        assert_eq!(parse_move_mode("walk"), Some(MoveMode::Walk));
        assert_eq!(parse_move_mode("run"), Some(MoveMode::Run));
        assert_eq!(parse_move_mode("WALK"), Some(MoveMode::Walk));
        assert_eq!(parse_move_mode("invalid"), Some(MoveMode::Run)); // defaults to run
    }
}
