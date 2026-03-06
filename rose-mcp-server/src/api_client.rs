//! HTTP client for the ROSE Offline REST API

use anyhow::{Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::Config;

/// API response wrapper from the server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

/// Error response from the server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: u16,
}

/// Position in 3D game space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Position {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

/// Position with zone information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZonePosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub zone_id: u16,
}

/// Health/Mana/Stamina points
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VitalPoints {
    pub current: u32,
    pub max: u32,
}

/// Request to create a new bot
#[derive(Debug, Clone, Serialize)]
pub struct CreateBotRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    pub assigned_player: String,
}

/// Response after creating a bot
#[derive(Debug, Clone, Deserialize)]
pub struct CreateBotResponse {
    pub bot_id: Uuid,
    pub entity_id: u32,
    pub name: String,
    pub status: String,
}

/// Bot status information
#[derive(Debug, Clone, Deserialize)]
pub struct BotStatus {
    pub bot_id: Uuid,
    pub name: String,
    pub level: u16,
    pub job: String,
    pub health: VitalPoints,
    pub mana: VitalPoints,
    pub stamina: VitalPoints,
    pub position: ZonePosition,
    pub current_command: String,
    pub assigned_player: Option<String>,
    pub is_dead: bool,
    pub is_sitting: bool,
}

/// Bot summary for list
#[derive(Debug, Clone, Deserialize)]
pub struct BotSummary {
    pub bot_id: Uuid,
    pub name: String,
    pub level: u16,
    pub health: VitalPoints,
    pub position: ZonePosition,
    pub assigned_player: Option<String>,
    pub status: String,
}

/// Bot list response
#[derive(Debug, Clone, Deserialize)]
pub struct BotListResponse {
    pub bots: Vec<BotSummary>,
}

/// Move request
#[derive(Debug, Clone, Serialize)]
pub struct MoveRequest {
    pub destination: Position,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_entity_id: Option<u32>,
    #[serde(default = "default_move_mode")]
    pub move_mode: String,
}

#[allow(dead_code)]
fn default_move_mode() -> String {
    "run".to_string()
}

/// Follow player request
#[derive(Debug, Clone, Serialize)]
pub struct FollowRequest {
    pub player_name: String,
    #[serde(default = "default_follow_distance")]
    pub distance: f32,
}

#[allow(dead_code)]
fn default_follow_distance() -> f32 {
    300.0
}

/// Attack request
#[derive(Debug, Clone, Serialize)]
pub struct AttackRequest {
    pub target_entity_id: u32,
}

/// Skill target type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillTargetType {
    Entity,
    Position,
    #[serde(rename = "self")]
    SelfTarget,
}

/// Skill request
#[derive(Debug, Clone, Serialize)]
pub struct SkillRequest {
    pub skill_id: u16,
    #[serde(rename = "targetType")]
    pub target_type: SkillTargetType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_entity_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_position: Option<Position>,
}

/// Chat request
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default = "default_chat_type")]
    pub chat_type: String,
}

#[allow(dead_code)]
fn default_chat_type() -> String {
    "local".to_string()
}

/// Chat message record
#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    pub timestamp: String,
    pub sender_name: String,
    pub sender_entity_id: u32,
    pub message: String,
    pub chat_type: String,
}

/// Chat history response
#[derive(Debug, Clone, Deserialize)]
pub struct ChatHistoryResponse {
    pub messages: Vec<ChatMessage>,
}

/// Nearby entity type
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NearbyEntityType {
    Player,
    Monster,
    Npc,
    Item,
}

/// Nearby entity information
#[derive(Debug, Clone, Deserialize)]
pub struct NearbyEntity {
    pub entity_id: u32,
    pub entity_type: NearbyEntityType,
    pub name: String,
    pub level: Option<u16>,
    pub position: Position,
    pub distance: f32,
    pub health_percent: Option<u8>,
}

/// Nearby entities response
#[derive(Debug, Clone, Deserialize)]
pub struct NearbyEntitiesResponse {
    pub entities: Vec<NearbyEntity>,
}

/// Skill information
#[derive(Debug, Clone, Deserialize)]
pub struct SkillInfo {
    pub slot: u8,
    pub skill_id: u16,
    pub name: String,
    pub level: u8,
    pub mp_cost: u16,
    pub cooldown: f32,
}

/// Bot skills response
#[derive(Debug, Clone, Deserialize)]
pub struct BotSkillsResponse {
    pub skills: Vec<SkillInfo>,
}

/// Inventory item information
#[derive(Debug, Clone, Deserialize)]
pub struct InventoryItemInfo {
    pub slot: String,
    pub item_id: usize,
    pub name: String,
    pub quantity: u32,
}

/// Response for bot inventory
#[derive(Debug, Clone, Deserialize)]
pub struct BotInventoryResponse {
    pub items: Vec<InventoryItemInfo>,
}

/// Player status information
#[derive(Debug, Clone, Deserialize)]
pub struct PlayerStatus {
    pub name: String,
    pub health: VitalPoints,
    pub mana: VitalPoints,
    pub level: u16,
    pub position: ZonePosition,
    pub is_in_combat: bool,
}

/// Response for player status
#[derive(Debug, Clone, Deserialize)]
pub struct PlayerStatusResponse {
    pub status: PlayerStatus,
}

/// Zone info information
#[derive(Debug, Clone, Deserialize)]
pub struct ZoneInfo {
    pub zone_name: String,
    pub zone_id: u16,
    pub recommended_level_min: u16,
    pub recommended_level_max: u16,
}

/// Response for zone info
#[derive(Debug, Clone, Deserialize)]
pub struct ZoneInfoResponse {
    pub zone_name: String,
    pub zone_id: u16,
    pub recommended_level_min: u16,
    pub recommended_level_max: u16,
}

/// Assigned player info for context
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssignedPlayerInfo {
    pub name: String,
    pub distance: f32,
    pub health_percent: u8,
    pub is_in_combat: bool,
}

/// Threat info for context
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThreatInfo {
    pub name: String,
    pub level: u16,
    pub distance: f32,
}

/// Item info for context
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ItemInfo {
    pub name: String,
    pub distance: f32,
}

/// Recent chat for context
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RecentChatInfo {
    pub sender: String,
    pub message: String,
}

/// Bot context bot info
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BotContextBot {
    pub name: String,
    pub level: u16,
    pub job: String,
    pub health_percent: u8,
    pub mana_percent: u8,
    pub position: Position,
    pub zone: String,
}

/// Bot context for LLM
#[derive(Debug, Clone, Deserialize)]
pub struct BotContext {
    pub bot: BotContextBot,
    pub assigned_player: Option<AssignedPlayerInfo>,
    pub nearby_threats: Vec<ThreatInfo>,
    pub nearby_items: Vec<ItemInfo>,
    pub recent_chat: Vec<RecentChatInfo>,
    pub available_actions: Vec<String>,
}

/// Empty response marker
#[derive(Debug, Clone, Deserialize)]
pub struct Empty {}

/// HTTP client for the ROSE Offline REST API
#[derive(Debug, Clone)]
pub struct ApiClient {
    client: Client,
    config: Config,
}

impl ApiClient {
    /// Create a new API client
    pub fn new(config: Config) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, config })
    }

    /// Make a GET request to the API
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.config.endpoint(path);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context(format!("Failed to GET {}", url))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read response body")?;

        if !status.is_success() {
            anyhow::bail!("API error ({}): {}", status, body);
        }

        // Try to parse as ApiResponse first
        if let Ok(api_response) = serde_json::from_str::<ApiResponse<T>>(&body) {
            if api_response.success {
                api_response.data.ok_or_else(|| anyhow::anyhow!("API returned success but no data"))
            } else {
                anyhow::bail!("API error: {}", api_response.error.unwrap_or_else(|| "Unknown error".to_string()))
            }
        } else {
            // Try direct parse
            serde_json::from_str(&body).context("Failed to parse response JSON")
        }
    }

    /// Make a POST request to the API
    async fn post<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: &T) -> Result<R> {
        let url = self.config.endpoint(path);
        let response = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .context(format!("Failed to POST {}", url))?;

        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("Failed to read response body")?;

        if !status.is_success() {
            anyhow::bail!("API error ({}): {}", status, response_body);
        }

        // Try to parse as ApiResponse first
        if let Ok(api_response) = serde_json::from_str::<ApiResponse<R>>(&response_body) {
            if api_response.success {
                api_response.data.ok_or_else(|| anyhow::anyhow!("API returned success but no data"))
            } else {
                anyhow::bail!("API error: {}", api_response.error.unwrap_or_else(|| "Unknown error".to_string()))
            }
        } else {
            // Try direct parse
            serde_json::from_str(&response_body).context("Failed to parse response JSON")
        }
    }

    /// Make a DELETE request to the API
    async fn delete<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        let url = self.config.endpoint(path);
        let response = self
            .client
            .delete(&url)
            .send()
            .await
            .context(format!("Failed to DELETE {}", url))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read response body")?;

        if !status.is_success() {
            anyhow::bail!("API error ({}): {}", status, body);
        }

        // Try to parse as ApiResponse first
        if let Ok(api_response) = serde_json::from_str::<ApiResponse<R>>(&body) {
            if api_response.success {
                api_response.data.ok_or_else(|| anyhow::anyhow!("API returned success but no data"))
            } else {
                anyhow::bail!("API error: {}", api_response.error.unwrap_or_else(|| "Unknown error".to_string()))
            }
        } else {
            // Try direct parse
            serde_json::from_str(&body).context("Failed to parse response JSON")
        }
    }

    // ================================
    // Bot Management
    // ================================

    /// Create a new buddy bot
    pub async fn create_bot(&self, request: CreateBotRequest) -> Result<CreateBotResponse> {
        self.post("/bots", &request).await
    }

    /// List all bots
    pub async fn list_bots(&self) -> Result<BotListResponse> {
        self.get("/bots").await
    }

    /// Get bot status
    pub async fn get_bot_status(&self, bot_id: &Uuid) -> Result<BotStatus> {
        self.get(&format!("/bots/{}/status", bot_id)).await
    }

    /// Get bot context for LLM
    pub async fn get_bot_context(&self, bot_id: &Uuid) -> Result<BotContext> {
        self.get(&format!("/bots/{}/context", bot_id)).await
    }

    /// Delete a bot
    pub async fn delete_bot(&self, bot_id: &Uuid) -> Result<Empty> {
        self.delete(&format!("/bots/{}", bot_id)).await
    }

    // ================================
    // Movement
    // ================================

    /// Move bot to a position
    pub async fn move_bot(&self, bot_id: &Uuid, request: MoveRequest) -> Result<Empty> {
        self.post(&format!("/bots/{}/move", bot_id), &request).await
    }

    /// Follow a player
    pub async fn follow_player(&self, bot_id: &Uuid, request: FollowRequest) -> Result<Empty> {
        self.post(&format!("/bots/{}/follow", bot_id), &request).await
    }

    /// Stop bot movement
    pub async fn stop_bot(&self, bot_id: &Uuid) -> Result<Empty> {
        self.post(&format!("/bots/{}/stop", bot_id), &serde_json::json!({})).await
    }

    // ================================
    // Combat
    // ================================

    /// Attack a target
    pub async fn attack_target(&self, bot_id: &Uuid, request: AttackRequest) -> Result<Empty> {
        self.post(&format!("/bots/{}/attack", bot_id), &request).await
    }

    /// Use a skill
    pub async fn use_skill(&self, bot_id: &Uuid, request: SkillRequest) -> Result<Empty> {
        self.post(&format!("/bots/{}/skill", bot_id), &request).await
    }

    // ================================
    // Chat
    // ================================

    /// Send a chat message
    pub async fn send_chat(&self, bot_id: &Uuid, request: ChatRequest) -> Result<Empty> {
        self.post(&format!("/bots/{}/chat", bot_id), &request).await
    }

    /// Get chat history
    pub async fn get_chat_history(&self, bot_id: &Uuid) -> Result<ChatHistoryResponse> {
        self.get(&format!("/bots/{}/chat/history", bot_id)).await
    }

    // ================================
    // Information
    // ================================

    /// Get nearby entities
    pub async fn get_nearby_entities(&self, bot_id: &Uuid, radius: Option<f32>, entity_types: Option<&str>) -> Result<NearbyEntitiesResponse> {
        let mut query = Vec::new();
        if let Some(r) = radius {
            query.push(format!("radius={}", r));
        }
        if let Some(t) = entity_types {
            query.push(format!("entity_types={}", t));
        }
        
        let path = if query.is_empty() {
            format!("/bots/{}/nearby", bot_id)
        } else {
            format!("/bots/{}/nearby?{}", bot_id, query.join("&"))
        };
        
        self.get(&path).await
    }

    /// Get bot skills
    pub async fn get_bot_skills(&self, bot_id: &Uuid) -> Result<BotSkillsResponse> {
        self.get(&format!("/bots/{}/skills", bot_id)).await
    }

    /// Get bot inventory
    pub async fn get_bot_inventory(&self, bot_id: &Uuid) -> Result<BotInventoryResponse> {
        self.get(&format!("/bots/{}/inventory", bot_id)).await
    }

    /// Get player status
    pub async fn get_player_status(&self, bot_id: &Uuid) -> Result<PlayerStatusResponse> {
        self.get(&format!("/bots/{}/player_status", bot_id)).await
    }

    /// Teleport bot to player
    pub async fn teleport_to_player(&self, bot_id: &Uuid) -> Result<Empty> {
        self.post(&format!("/bots/{}/teleport_to_player", bot_id), &serde_json::json!({})).await
    }

    /// Get zone info
    pub async fn get_zone_info(&self, bot_id: &Uuid) -> Result<ZoneInfoResponse> {
        self.get(&format!("/bots/{}/zone", bot_id)).await
    }

    /// Pickup an item
    pub async fn pickup_item(&self, bot_id: &Uuid, item_entity_id: u32) -> Result<Empty> {
        self.post(&format!("/bots/{}/pickup", bot_id), &serde_json::json!({ "item_entity_id": item_entity_id })).await
    }

    /// Use an item
    pub async fn use_item(&self, bot_id: &Uuid, item_slot: u16, target_entity_id: Option<u32>) -> Result<Empty> {
        self.post(&format!("/bots/{}/execute", bot_id), &serde_json::json!({
            "action": "use_item",
            "parameters": {
                "item_slot": item_slot,
                "target_entity_id": target_entity_id
            }
        })).await
    }

    /// Set bot behavior mode
    pub async fn set_bot_behavior_mode(&self, bot_id: &Uuid, mode: &str) -> Result<Empty> {
        self.post(&format!("/bots/{}/execute", bot_id), &serde_json::json!({
            "action": "set_behavior_mode",
            "parameters": {
                "behavior_mode": mode
            }
        })).await
    }
}
