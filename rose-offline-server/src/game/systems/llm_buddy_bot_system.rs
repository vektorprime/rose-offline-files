//! Game systems for LLM Buddy Bot control
//!
//! This module contains the Bevy systems that process commands from the REST API
//! and integrate LLM-controlled buddy bots with the game loop.

use std::sync::Arc;

use bevy::{
    prelude::*,
    math::Vec3,
};
use bevy::math::Vec2;
use chrono::Utc;
use crossbeam_channel::Sender;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::game::{
    api::{
        models::{
            ItemInfo, NearbyEntity, NearbyEntityType, ThreatInfo,
        },
        BotInfo, BotSummaryData, DeleteBotResponse, GetBotInventoryResponse, GetBotListResponse,
        GetBotSkillsResponse, GetChatHistoryResponse, GetPlayerStatusResponse, GetZoneInfoResponse,
        LlmBotCommand,
    },
    bundles::client_entity_leave_zone,
    components::{
        ChatMessage as BotChatMessage, ChatType as BotChatType, ClientEntity, ClientEntitySector,
        Command, CommandCastSkillTarget, CommandData, Dead, HealthPoints, Inventory, LlmBuddyBot,
        ManaPoints, NextCommand, Position, Stamina,
    },
    messages::server::ServerMessage,
    resources::{ClientEntityList, GameData, ServerMessages},
    storage::llm_buddy_bot::LlmBuddyBotStorage,
};

use rose_data::{SkillId, ZoneId};
use rose_game_common::components::{AbilityValues, MoveMode};

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
    pub pending_deletes: Vec<(Uuid, Sender<DeleteBotResponse>)>,
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
                    let entity = bot_info.entity;
                    // log::info!("[LLM_DEBUG] Processing Move command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((mut buddy_bot, _, mut next_command, _, _)) = bot_query.get_mut(entity) {
                        // Stop following when manually moving
                        buddy_bot.is_following = false;

                        let dest = Vec3::new(destination.x, destination.y, destination.z);
                        let target = target_entity.and_then(|id| {
                            entity_query
                                .iter()
                                .find(|(_, client_entity, _, _, _, _, _, _, _, _, _)| client_entity.id.0 == id as usize)
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
                    let entity = bot_info.entity;
                    log::info!("Processing Pickup command for bot {}: entity {:?}", bot_id, entity);
                    if let Ok((_, _, mut next_command, _, _)) = bot_query.get_mut(entity) {
                        let target = entity_query
                            .iter()
                            .find(|(_, client_entity, _, _, _, _, _, _, _, _, _)| client_entity.id.0 == item_entity_id as usize)
                            .map(|(entity, _, _, _, _, _, _, _, _, _, _)| entity)
                            .unwrap_or_else(|| {
                                Entity::from_raw_u32(item_entity_id).unwrap_or(Entity::PLACEHOLDER)
                            });
                        
                        next_command.command = Some(CommandData::PickupItemDrop { target });
                        next_command.has_sent_server_message = false;

                        log::info!("LLM bot {} picking up item {}", bot_id, item_entity_id);
                    } else {
                        log::warn!("Pickup command failed: bot {} entity {:?} not found in bot_query (may be placeholder or despawned)", bot_id, entity);
                    }
                } else {
                    log::warn!("Pickup command failed: bot {} not found in bots_map", bot_id);
                }
            }
            LlmBotCommand::AttackNearest { bot_id } => {
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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

                        // DIAGNOSTIC: Log the entity IDs being returned to help debug entity ID mismatch issues
                        let monster_ids: Vec<(u32, &str)> = entities.iter()
                            .filter(|e| e.entity_type == crate::game::api::models::NearbyEntityType::Monster)
                            .take(5)
                            .map(|e| (e.entity_id, e.name.as_str()))
                            .collect();
                        let player_ids: Vec<(u32, &str)> = entities.iter()
                            .filter(|e| e.entity_type == crate::game::api::models::NearbyEntityType::Player)
                            .map(|e| (e.entity_id, e.name.as_str()))
                            .collect();
                        
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                
                let bots_map = bot_manager.bots_map.read();
                for (bot_id, bot_info) in bots_map.iter() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
                    let entity = bot_info.entity;
                    log::info!(
                        "Processing UseItem command for bot {}: entity {:?}",
                        bot_id,
                        entity
                    );

                    let target_entity = if let Some(entity_id) = target_entity_id {
                        entity_query
                            .iter()
                            .find(|(_, client_entity, _, _, _, _, _, _, _, _, _)| {
                                client_entity.id.0 == entity_id as usize
                            })
                            .map(|(entity, _, _, _, _, _, _, _, _, _, _)| entity)
                            .unwrap_or_else(|| {
                                Entity::from_raw_u32(entity_id).unwrap_or(Entity::PLACEHOLDER)
                            })
                    } else {
                        // Default to self if no target specified
                        entity
                    };

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
            LlmBotCommand::SetBehaviorMode { bot_id, mode } => {
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
                if let Some(bot_info) = bot_manager.bots_map.read().get(&bot_id).cloned() {
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
    let deletes: Vec<(Uuid, Sender<DeleteBotResponse>)> = bot_manager.pending_deletes.drain(..).collect();
    if !deletes.is_empty() {
        log::info!("Processing {} pending bot deletions", deletes.len());
    }
    for (bot_id, response_tx) in deletes {
        log::info!("Processing deletion for bot {} - response_tx is_valid: {}", bot_id, !response_tx.is_empty());
        
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
                        Err(e) => log::error!("Failed to send success response for bot {} deletion: {}", bot_id, e),
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
                        Err(e2) => log::error!("Failed to send failure response for bot {} deletion: {}", bot_id, e2),
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
                Err(e) => log::error!("Failed to send not-found response for bot {} deletion: {}", bot_id, e),
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

/// System that makes LLM buddy bots follow their assigned player
///
/// This system checks each buddy bot's distance from their assigned player
/// and issues move commands if they're too far away.
pub fn llm_buddy_follow_system(
    mut bot_query: Query<
        (&LlmBuddyBot, &Position, &mut Command, &mut NextCommand),
        (Without<Dead>,),
    >,
    player_query: Query<(&Position, &ClientEntity, &crate::game::components::CharacterInfo), Without<LlmBuddyBot>>,
    bot_manager: Res<LlmBotManagerResource>,
) {
    // Only process if there are bots registered
    let bots_map = bot_manager.bots_map.read();
    if bots_map.is_empty() {
        return;
    }

    // For each bot that is in follow mode
    for (&bot_id, bot_info) in bots_map.iter() {
        let entity = bot_info.entity;
        // Get bot's data
        let Ok((buddy_bot, bot_position, mut command, mut next_command)) = bot_query.get_mut(entity)
        else {
            continue;
        };

        // Skip if not following
        if !buddy_bot.is_following {
           //  log::debug!("[LLM_DEBUG] FOLLOW_DIAGNOSTIC: Bot {} not in follow mode (is_following=false)", bot_id);
            continue;
        }

        // Skip if currently executing a command (let it finish)
        if !command.is_stop() {
           //  log::debug!("[LLM_DEBUG] FOLLOW_DIAGNOSTIC: Bot {} has active command, skipping follow (command={:?})", bot_id, command);
            continue;
        }

        // Find the assigned player by name or ID
        let mut player_data: Option<(Vec3, rose_data::ZoneId)> = None;
        
        // Try to find by name first (more reliable if ID changes)
        for (pos, _, char_info) in player_query.iter() {
            if char_info.name == buddy_bot.assigned_player_name {
                player_data = Some((pos.position, pos.zone_id));
                break;
            }
        }
        
        // If not found by name, try by ID
        if player_data.is_none() {
            for (pos, client_entity, _) in player_query.iter() {
                if client_entity.id.0 == buddy_bot.assigned_player_id as usize {
                    player_data = Some((pos.position, pos.zone_id));
                    break;
                }
            }
        }

        // If we couldn't find the player, skip
        let Some((player_pos, player_zone)) = player_data else {
           //  log::debug!("[LLM_DEBUG] FOLLOW_DIAGNOSTIC: Bot {} cannot find player '{}' (id: {}) in world",
            //    bot_id, buddy_bot.assigned_player_name, buddy_bot.assigned_player_id);
            continue;
        };
        
        // Skip if in different zones
        if player_zone != bot_position.zone_id {
           //  log::debug!("[LLM_DEBUG] FOLLOW_DIAGNOSTIC: Bot {} in zone {:?} but player '{}' in zone {:?}, skipping",
            //    bot_id, bot_position.zone_id, buddy_bot.assigned_player_name, player_zone);
            continue;
        }

        // Calculate distance in XY plane only (2D distance for ground movement)
        let bot_pos = bot_position.position;
        let player_pos_2d = Vec2::new(player_pos.x, player_pos.y);
        let bot_pos_2d = Vec2::new(bot_pos.x, bot_pos.y);
        let distance_2d = bot_pos_2d.distance(player_pos_2d);

        // Convert follow_distance from meters (client coordinates) to centimeters (server coordinates)
        // LLM uses client coordinates (meters), but server uses world coordinates (centimeters)
        let follow_distance_cm = buddy_bot.follow_distance * 100.0;

        // If too far, move toward player
        if distance_2d > follow_distance_cm {
            // Calculate direction toward player (2D in XY plane only)
            let direction_2d = (player_pos_2d - bot_pos_2d).normalize();
            
            // Calculate target position, preserving the bot's Z coordinate
            // This prevents the bot from "running away" due to Z coordinate differences
            let target_offset = distance_2d - buddy_bot.follow_distance * 0.8;
            let target_pos = Vec3::new(
                bot_pos.x + direction_2d.x * target_offset,
                bot_pos.y + direction_2d.y * target_offset,
                bot_pos.z, // Keep bot's Z coordinate unchanged
            );

            // log::info!(
            //     "[LLM_DEBUG] FOLLOW_DIAGNOSTIC: Bot {} following player '{}': bot_pos={:?}, player_pos={:?}, distance_2d={}m, follow_distance={}m, target_pos={:?}",
            //     bot_id,
            //     buddy_bot.assigned_player_name,
            //     bot_pos,
            //     player_pos,
            //     distance_2d / 100.0,  // Convert to meters for logging
            //     buddy_bot.follow_distance,
            //     target_pos
            // );

            next_command.command = Some(CommandData::Move {
                destination: target_pos,
                target: None,
                move_mode: Some(MoveMode::Run),
            });
            next_command.has_sent_server_message = false;
        } else {
            // log::debug!(
            //     "[LLM_DEBUG] FOLLOW_DIAGNOSTIC: Bot {} within follow distance of player '{}' (distance_2d={}m, follow_distance={}m)",
            //     bot_id,
            //     buddy_bot.assigned_player_name,
            //     distance_2d / 100.0,
            //     buddy_bot.follow_distance
            // );
        }
    }
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

/// System that updates status information for API queries
///
/// This system periodically updates cached status information that can be
/// queried through the REST API.
pub fn llm_buddy_status_update_system(
    bot_query: Query<
        (
            &LlmBuddyBot,
            &Position,
            &HealthPoints,
            &ManaPoints,
            &Stamina,
            &AbilityValues,
            &Command,
            Option<&crate::game::components::CharacterInfo>,
            Option<&crate::game::components::Level>,
        ),
        Without<Dead>,
    >,
    bot_manager: Res<LlmBotManagerResource>,
) {
    // This system could update a cached status structure that the API can read
    // For now, the API queries the components directly
    let _ = bot_query;
    let _ = bot_manager;
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
/// When a party invite is sent to an LLM buddy bot, this system automatically
/// accepts it so the bot can party with its assigned player.
pub fn llm_buddy_bot_auto_accept_party_system(
    mut bot_query: Query<(Entity, &mut crate::game::components::PartyMembership), With<LlmBuddyBot>>,
    mut party_events: MessageWriter<crate::game::events::PartyEvent>,
) {
    for (bot_entity, mut party_membership) in bot_query.iter_mut() {
        // Skip if already in a party
        if party_membership.party.is_some() {
            continue;
        }

        // If there are pending invites, accept the first one
        if let Some(&owner_entity) = party_membership.pending_invites.first() {
            log::info!(
                "LLM buddy bot auto-accepting party invite from entity {:?}",
                owner_entity
            );
            party_events.write(crate::game::events::PartyEvent::AcceptInvite {
                owner_entity,
                invited_entity: bot_entity,
            });
            
            // NOTE: We do NOT clear pending_invites here because the party_system
            // processes events in the next frame and needs to verify the invite exists.
            // The handle_party_accept_invite function will remove the specific invite
            // from the list after successfully processing the accept.
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
            
            // Find the first available bot
            let bots_map = bot_manager.bots_map.read();
            if let Some((bot_id, bot_info)) = bots_map.iter().next() {
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
