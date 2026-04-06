//! HTTP request handlers for the LLM Buddy Bot REST API

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use super::channels::LlmBotCommand;
use super::models::*;
use super::state::ApiState;

/// Query parameters for nearby entities endpoint
#[derive(Debug, Deserialize)]
pub struct NearbyQuery {
    /// Search radius
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// Entity types to include (comma-separated)
    #[serde(default)]
    pub entity_types: Option<String>,
}

fn default_radius() -> f32 {
    1000.0
}

/// Create a new bot
pub async fn create_bot(
    State(state): State<ApiState>,
    Json(request): Json<CreateBotRequest>,
) -> Result<Json<ApiResponse<CreateBotResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let bot_id = Uuid::new_v4();
    let level = request.level.unwrap_or(1);
    let class = request.build.unwrap_or_else(|| "knight".to_string());
    let gender = request.gender.map(|g| match g {
        BotGender::Male => "male".to_string(),
        BotGender::Female => "female".to_string(),
    });

    let command = LlmBotCommand::CreateBot {
        bot_id,
        name: request.name.clone(),
        level,
        class: class.clone(),
        gender: gender.clone(),
        assigned_player: request.assigned_player.clone(),
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    // Register the bot immediately in the API state so it's discoverable
    // The entity_id will be updated by the game world when the bot is actually created
    let assigned_player = if request.assigned_player.is_empty() {
        None
    } else {
        Some(request.assigned_player.clone())
    };
    state.register_bot_full(
        bot_id,
        bevy::prelude::Entity::from_raw_u32(0)
            .unwrap_or(bevy::prelude::Entity::PLACEHOLDER),
        request.name.clone(),
        assigned_player,
        level,
        class,
    );

    Ok(Json(ApiResponse::success(CreateBotResponse {
        bot_id,
        entity_id: 0, // Will be updated by game world
        name: request.name,
        status: "created".to_string(),
    })))
}

/// Delete a bot
/// 
/// This endpoint waits for confirmation from the game thread before returning.
/// Returns:
/// - 200 OK if bot was successfully deleted
/// - 404 NOT FOUND if bot doesn't exist
/// - 500 INTERNAL SERVER ERROR if deletion failed
pub async fn delete_bot(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    log::info!("delete_bot called for bot_id: {}", bot_id);
    
    if !state.bot_exists(&bot_id) {
        // Bot doesn't exist - return404
        log::warn!("delete_bot: bot {} not found in ApiState", bot_id);
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    // Create response channel for confirmation from game thread
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    log::info!("delete_bot: created response channel for bot {}", bot_id);
    
    // Send delete command to game thread with response channel
    let command = LlmBotCommand::DeleteBot { bot_id, response_tx };
    log::info!("delete_bot: sending DeleteBot command for bot {} to channel", bot_id);
    state
        .send_command(command)
        .map_err(|e| {
            log::error!("delete_bot: failed to send command for bot {}: {}", bot_id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;
    log::info!("delete_bot: DeleteBot command sent successfully for bot {}", bot_id);

    // Wait for confirmation from game thread (with timeout)
    log::info!("delete_bot: waiting for confirmation from game thread for bot {}", bot_id);
    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                log::info!("delete_bot: successfully deleted bot {}", bot_id);
                Ok(Json(ApiResponse::success(Empty {})))
            } else {
                log::error!("delete_bot: deletion failed for bot {}: {:?}", bot_id, response.error);
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Deletion failed".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            log::error!("delete_bot: timeout waiting for confirmation for bot {}", bot_id);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Timeout waiting for deletion confirmation", 500)),
            ))
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            log::error!("delete_bot: response channel disconnected for bot {}", bot_id);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Game thread disconnected", 500)),
            ))
        }
    }
}

/// List all bots
pub async fn list_bots(
    State(state): State<ApiState>,
) -> Result<Json<ApiResponse<BotListResponse>>, (StatusCode, Json<ErrorResponse>)> {
    log::info!("list_bots called");
    
    // Create response channel for bot list from game thread
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    
    let command = LlmBotCommand::GetBotList { response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    // Wait for response from game thread (with timeout)
    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                // Convert BotSummaryData to BotSummary
                let bots: Vec<BotSummary> = response.bots.into_iter().map(|bot| BotSummary {
                    bot_id: bot.bot_id,
                    name: bot.name,
                    level: bot.level,
                    health: bot.health,
                    position: bot.position,
                    assigned_player: bot.assigned_player,
                    status: bot.status,
                }).collect();
                
                Ok(Json(ApiResponse::success(BotListResponse { bots })))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get bot list".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Timeout waiting for bot list", 500)),
            ))
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Game thread disconnected", 500)),
            ))
        }
    }
}

/// Get bot status
pub async fn get_bot_status(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<BotStatus>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let bot_info = state.get_bot_info(&bot_id);
    let class = bot_info.as_ref().map(|i| i.class.clone()).unwrap_or_else(|| "Knight".to_string());

    // Create response channel for bot list from game thread (which includes status)
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    
    let command = LlmBotCommand::GetBotList { response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    // Wait for response from game thread (with timeout)
    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                // Find the specific bot in the list
                if let Some(bot) = response.bots.into_iter().find(|b| b.bot_id == bot_id) {
                    Ok(Json(ApiResponse::success(BotStatus {
                        bot_id,
                        name: bot.name,
                        level: bot.level,
                        job: class.clone(),
                        health: bot.health,
                        mana: bot.mana,
                        stamina: bot.stamina,
                        position: bot.position,
                        current_command: bot.status.clone(),
                        assigned_player: bot.assigned_player,
                        is_dead: bot.status == "dead",
                        is_sitting: bot.status == "sitting",
                    })))
                } else {
                    Err((
                        StatusCode::NOT_FOUND,
                        Json(ErrorResponse::new("Bot not found in game world", 404)),
                    ))
                }
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get bot status".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Timeout waiting for bot status", 500)),
            ))
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Game thread disconnected", 500)),
            ))
        }
    }
}

/// Move bot to a position
pub async fn move_bot(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<MoveRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Move {
        bot_id,
        destination: request.destination,
        target_entity: request.target_entity_id,
        move_mode: request.move_mode,
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Follow a player
pub async fn follow_player(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<FollowRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    log::info!("follow_player called for bot_id: {}", bot_id);
    if !state.bot_exists(&bot_id) {
        log::warn!("follow_player: bot {} not found in ApiState", bot_id);
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Follow {
        bot_id,
        player_name: request.player_name,
        distance: request.distance,
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Stop bot movement
pub async fn stop_bot(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Stop { bot_id };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Attack a target
pub async fn attack_target(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<AttackRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Attack {
        bot_id,
        target_entity_id: request.target_entity_id,
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Use a skill
pub async fn use_skill(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<SkillRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let target_entity_id = if request.target_type == SkillTargetType::Entity {
        request.target_entity_id
    } else {
        None
    };

    let target_position = if request.target_type == SkillTargetType::Position {
        request.target_position
    } else {
        None
    };

    let command = LlmBotCommand::UseSkill {
        bot_id,
        skill_id: request.skill_id,
        target_entity_id,
        target_position,
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Send a chat message
pub async fn send_chat(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<ChatRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Chat {
        bot_id,
        message: request.message,
        chat_type: request.chat_type,
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Get chat history
pub async fn get_chat_history(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<ChatHistoryResponse>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    // Create response channel for chat history from game thread
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    
    let command = LlmBotCommand::GetChatHistory { bot_id, response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    // Wait for response from game thread (with timeout)
    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                Ok(Json(ApiResponse::success(ChatHistoryResponse {
                    messages: response.messages,
                })))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get chat history".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Timeout waiting for chat history", 500)),
            ))
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Game thread disconnected", 500)),
            ))
        }
    }
}

/// Get nearby entities
pub async fn get_nearby_entities(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Query(query): Query<NearbyQuery>,
) -> Result<Json<ApiResponse<NearbyEntitiesResponse>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    // Create response channel for bot context from game thread (which includes nearby entities)
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    
    let command = LlmBotCommand::GetBotContext { bot_id, response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    // Wait for response from game thread (with timeout)
    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                Ok(Json(ApiResponse::success(NearbyEntitiesResponse {
                    entities: response.entities,
                })))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get nearby entities".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Timeout waiting for nearby entities", 500)),
            ))
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Game thread disconnected", 500)),
            ))
        }
    }
}

/// Get bot skills
pub async fn get_bot_skills(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<BotSkillsResponse>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    // Create response channel for skills from game thread
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    
    let command = LlmBotCommand::GetBotSkills { bot_id, response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    // Wait for response from game thread (with timeout)
    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                Ok(Json(ApiResponse::success(BotSkillsResponse {
                    skills: response.skills,
                })))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get skills".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Timeout waiting for skills", 500)),
            ))
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("Game thread disconnected", 500)),
            ))
        }
    }
}

/// Get bot inventory
pub async fn get_bot_inventory(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<BotInventoryResponse>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    let command = LlmBotCommand::GetBotInventory { bot_id, response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                Ok(Json(ApiResponse::success(BotInventoryResponse {
                    items: response.items,
                })))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get inventory".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse::new("Game world response timeout", 504)),
        )),
    }
}

/// Get player status
pub async fn get_player_status(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<PlayerStatusResponse>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    let command = LlmBotCommand::GetPlayerStatus { bot_id, response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                if let Some(status) = response.status {
                    Ok(Json(ApiResponse::success(PlayerStatusResponse { status })))
                } else {
                    Err((
                        StatusCode::NOT_FOUND,
                        Json(ErrorResponse::new("Assigned player not found", 404)),
                    ))
                }
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get player status".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse::new("Game world response timeout", 504)),
        )),
    }
}

/// Teleport bot to player
pub async fn teleport_to_player(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::TeleportToPlayer { bot_id };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Get zone info
pub async fn get_zone_info(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<ZoneInfoResponse>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    let command = LlmBotCommand::GetZoneInfo { bot_id, response_tx };
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                Ok(Json(ApiResponse::success(ZoneInfoResponse {
                    zone_name: response.zone_name,
                    zone_id: response.zone_id,
                    recommended_level_min: response.recommended_level_min,
                    recommended_level_max: response.recommended_level_max,
                })))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        &response.error.unwrap_or_else(|| "Failed to get zone info".to_string()),
                        500,
                    )),
                ))
            }
        }
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse::new("Game world response timeout", 504)),
        )),
    }
}

/// Sit down
pub async fn sit_bot(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Sit { bot_id };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Stand up
pub async fn stand_bot(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Stand { bot_id };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Pickup an item
pub async fn pickup_item(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<PickupRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Pickup {
        bot_id,
        item_entity_id: request.item_entity_id,
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Perform an emote
pub async fn perform_emote(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<EmoteRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = LlmBotCommand::Emote {
        bot_id,
        emote_id: request.emote_id,
        is_stop: request.is_stop,
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Get bot context for LLM
pub async fn get_bot_context(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
) -> Result<Json<ApiResponse<BotContext>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let bot_info = state.get_bot_info(&bot_id);
    let name = bot_info.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| "Unknown".to_string());
    let assigned_player = bot_info.as_ref().and_then(|i| i.assigned_player.clone());
    let level = bot_info.as_ref().map(|i| i.level).unwrap_or(1);
    let class = bot_info.as_ref().map(|i| i.class.clone()).unwrap_or_else(|| "Knight".to_string());

    // Query the game world for nearby threats and items
    let (response_tx, response_rx) = crossbeam_channel::bounded(1);
    let command = LlmBotCommand::GetBotContext { bot_id, response_tx };
    
    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    // Wait for response from game thread (with timeout)
    let entities: Vec<NearbyEntity> = match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(response) => {
            if response.success {
                response.entities
            } else {
                log::warn!("GetBotContext query failed for bot {}: {:?}", bot_id, response.error);
                vec![]
            }
        }
        Err(e) => {
            log::warn!("GetBotContext timeout or error for bot {}: {:?}", bot_id, e);
            vec![]
        }
    };

    // Convert NearbyEntity to ThreatInfo and ItemInfo for the context response
    let threats: Vec<ThreatInfo> = entities.iter()
        .filter(|e| e.entity_type == NearbyEntityType::Monster || e.entity_type == NearbyEntityType::Player)
        .map(|e| ThreatInfo {
            name: e.name.clone(),
            level: e.level.unwrap_or(1),
            distance: e.distance,
        }).collect();

    let items: Vec<ItemInfo> = entities.iter()
        .filter(|e| e.entity_type == NearbyEntityType::Item)
        .map(|e| ItemInfo {
            name: e.name.clone(),
            distance: e.distance,
        }).collect();

    Ok(Json(ApiResponse::success(BotContext {
        bot: BotContextBot {
            name,
            level,
            job: class,
            health_percent: 100,
            mana_percent: 100,
            position: Position::new(0.0, 0.0, 0.0),
            zone: "Unknown".to_string(),
        },
        assigned_player: assigned_player.map(|name| {
            let player_name = name.clone();
            AssignedPlayerInfo {
                name,
                // Distance calculation requires GetBotContext to return bot position and player position
                // For now, search nearby entities for the assigned player
                distance: entities.iter()
                    .find(|e| e.entity_type == NearbyEntityType::Player && e.name == player_name)
                    .map(|e| e.distance)
                    .unwrap_or(0.0),
                health_percent: entities.iter()
                    .find(|e| e.entity_type == NearbyEntityType::Player && e.name == player_name)
                    .and_then(|e| e.health_percent)
                    .unwrap_or(100),
                is_in_combat: false,
            }
        }),
        nearby_threats: threats,
        nearby_items: items,
        recent_chat: vec![],
        available_actions: vec![
            "move".to_string(),
            "attack".to_string(),
            "follow".to_string(),
            "chat".to_string(),
            "use_skill".to_string(),
        ],
    })))
}

/// Execute LLM command
pub async fn execute_llm_command(
    State(state): State<ApiState>,
    Path(bot_id): Path<Uuid>,
    Json(request): Json<LlmExecuteRequest>,
) -> Result<Json<ApiResponse<Empty>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.bot_exists(&bot_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Bot not found", 404)),
        ));
    }

    let command = match request.action {
        LlmActionType::FollowPlayer => {
            let player_name = request.parameters.player_name.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("player_name is required for follow_player", 400)),
                )
            })?;
            LlmBotCommand::Follow {
                bot_id,
                player_name,
                distance: request.parameters.duration.unwrap_or(300.0),
            }
        }
        LlmActionType::MoveTo => {
            let position = request.parameters.position.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("position is required for move_to", 400)),
                )
            })?;
            LlmBotCommand::Move {
                bot_id,
                destination: position,
                target_entity: None,
                move_mode: "run".to_string(),
            }
        }
        LlmActionType::AttackTarget => {
            let target_entity_id = request.parameters.target_entity_id.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("target_entity_id is required for attack_target", 400)),
                )
            })?;
            LlmBotCommand::Attack {
                bot_id,
                target_entity_id,
            }
        }
        LlmActionType::UseSkill => {
            let skill_id = request.parameters.skill_id.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("skill_id is required for use_skill", 400)),
                )
            })?;
            LlmBotCommand::UseSkill {
                bot_id,
                skill_id,
                target_entity_id: request.parameters.target_entity_id,
                target_position: request.parameters.position,
            }
        }
        LlmActionType::UseItem => {
            let item_slot = request.parameters.item_slot.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("item_slot is required for use_item", 400)),
                )
            })?;
            LlmBotCommand::UseItem {
                bot_id,
                item_slot,
                target_entity_id: request.parameters.target_entity_id,
            }
        }
        LlmActionType::SetBehaviorMode => {
            let mode_str = request.parameters.behavior_mode.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("behavior_mode is required for set_behavior_mode", 400)),
                )
            })?;
            let mode = match mode_str.to_lowercase().as_str() {
                "passive" => crate::game::components::BotBehaviorMode::Passive,
                "defensive" => crate::game::components::BotBehaviorMode::Defensive,
                "aggressive" => crate::game::components::BotBehaviorMode::Aggressive,
                "support" => crate::game::components::BotBehaviorMode::Support,
                _ => return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("Invalid behavior_mode. Must be passive, defensive, aggressive, or support", 400)),
                )),
            };
            LlmBotCommand::SetBehaviorMode { bot_id, mode }
        }
        LlmActionType::Say => {
            let message = request.parameters.message.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("message is required for say", 400)),
                )
            })?;
            LlmBotCommand::Chat {
                bot_id,
                message,
                chat_type: "local".to_string(),
            }
        }
        LlmActionType::Sit => LlmBotCommand::Sit { bot_id },
        LlmActionType::Stand => LlmBotCommand::Stand { bot_id },
        LlmActionType::AttackNearest => {
            LlmBotCommand::AttackNearest { bot_id }
        }
        LlmActionType::PickupItems => {
            LlmBotCommand::PickupNearestItem { bot_id }
        }
        LlmActionType::Wait => {
            // Wait is a no-op for now
            return Ok(Json(ApiResponse::success(Empty {})));
        }
    };

    state
        .send_command(command)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e, 500)),
            )
        })?;

    Ok(Json(ApiResponse::success(Empty {})))
}

/// Health check endpoint
pub async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "llm-buddy-bot-api"
    }))
}
