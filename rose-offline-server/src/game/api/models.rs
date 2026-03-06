//! Request and Response DTOs for the LLM Buddy Bot REST API

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

impl ZonePosition {
    pub fn new(x: f32, y: f32, z: f32, zone_id: u16) -> Self {
        Self { x, y, z, zone_id }
    }
}

/// Health and Mana points structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VitalPoints {
    pub current: u32,
    pub max: u32,
}

impl VitalPoints {
    pub fn new(current: u32, max: u32) -> Self {
        Self { current, max }
    }

    pub fn percent(&self) -> f32 {
        if self.max == 0 {
            0.0
        } else {
            (self.current as f32 / self.max as f32) * 100.0
        }
    }
}

/// Gender type for bot creation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotGender {
    Male,
    Female,
}

impl Default for BotGender {
    fn default() -> Self {
        Self::Male
    }
}

/// Request to create a new bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBotRequest {
    /// Name for the bot character
    pub name: String,
    /// Level for the bot (optional, defaults based on assigned player)
    #[serde(default)]
    pub level: Option<u16>,
    /// Build/class type for the bot
    #[serde(default)]
    pub build: Option<String>,
    /// Gender for the bot (optional, defaults to male)
    #[serde(default)]
    pub gender: Option<BotGender>,
    /// Player name to assign the bot to follow
    pub assigned_player: String,
}

/// Response after creating a bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBotResponse {
    pub bot_id: Uuid,
    pub entity_id: u32,
    pub name: String,
    pub status: String,
}

/// Bot class/build types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotClass {
    Knight,
    Champion,
    Mage,
    Cleric,
    Raider,
    Scout,
    Bourgeois,
    Artisan,
}

impl Default for BotClass {
    fn default() -> Self {
        Self::Knight
    }
}

/// Current status of a bot
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Request to move a bot to a position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveRequest {
    /// Target destination
    pub destination: Position,
    /// Optional entity to follow
    #[serde(default)]
    pub target_entity_id: Option<u32>,
    /// Movement mode (walk or run)
    #[serde(default = "default_move_mode")]
    pub move_mode: String,
}

fn default_move_mode() -> String {
    "run".to_string()
}

/// Request to follow a player
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowRequest {
    /// Player name to follow
    pub player_name: String,
    /// Distance to maintain from the player
    #[serde(default = "default_follow_distance")]
    pub distance: f32,
}

fn default_follow_distance() -> f32 {
    300.0
}

/// Request to attack a target
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackRequest {
    /// Entity ID of the target to attack
    pub target_entity_id: u32,
}

/// Target type for skill usage
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTargetType {
    Entity,
    Position,
    SelfTarget,
}

/// Request to use a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRequest {
    /// Skill ID to use
    pub skill_id: u16,
    /// Type of targeting
    #[serde(rename = "targetType")]
    pub target_type: SkillTargetType,
    /// Target entity ID (required if target_type is Entity)
    #[serde(default)]
    pub target_entity_id: Option<u32>,
    /// Target position (required if target_type is Position)
    #[serde(default)]
    pub target_position: Option<Position>,
}

/// Request to send a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Message content
    pub message: String,
    /// Chat type (local or shout)
    #[serde(default = "default_chat_type")]
    pub chat_type: String,
}

fn default_chat_type() -> String {
    "local".to_string()
}

/// Request to pickup an item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickupRequest {
    /// Entity ID of the item to pickup
    pub item_entity_id: u32,
}

/// Request to perform an emote
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmoteRequest {
    /// Emote/motion ID
    pub emote_id: u16,
    /// Whether this is a stop emote
    #[serde(default)]
    pub is_stop: bool,
}

/// Chat message record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Timestamp of the message
    pub timestamp: String,
    /// Name of the sender
    pub sender_name: String,
    /// Entity ID of the sender
    pub sender_entity_id: u32,
    /// Message content
    pub message: String,
    /// Type of chat (local, shout, etc.)
    pub chat_type: String,
}

/// Response for chat history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistoryResponse {
    pub messages: Vec<ChatMessage>,
}

/// Entity type for nearby entity responses
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NearbyEntityType {
    Player,
    Monster,
    Npc,
    Item,
}

/// Information about a nearby entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyEntity {
    pub entity_id: u32,
    pub entity_type: NearbyEntityType,
    pub name: String,
    pub level: Option<u16>,
    pub position: Position,
    pub distance: f32,
    pub health_percent: Option<u8>,
}

/// Response for nearby entities query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyEntitiesResponse {
    pub entities: Vec<NearbyEntity>,
}

/// Skill information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub slot: u8,
    pub skill_id: u16,
    pub name: String,
    pub level: u8,
    pub mp_cost: u16,
    pub cooldown: f32,
}

/// Response for bot skills
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotSkillsResponse {
    pub skills: Vec<SkillInfo>,
}

/// Inventory item information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryItemInfo {
    pub slot: String,
    pub item_id: usize,
    pub name: String,
    pub quantity: u32,
}

/// Response for bot inventory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotInventoryResponse {
    pub items: Vec<InventoryItemInfo>,
}

/// Player status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub name: String,
    pub health: VitalPoints,
    pub mana: VitalPoints,
    pub level: u16,
    pub position: ZonePosition,
    pub is_in_combat: bool,
}

/// Response for player status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatusResponse {
    pub status: PlayerStatus,
}

/// Response for zone info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfoResponse {
    pub zone_name: String,
    pub zone_id: u16,
    pub recommended_level_min: u16,
    pub recommended_level_max: u16,
}

/// Assigned player information for LLM context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignedPlayerInfo {
    pub name: String,
    pub distance: f32,
    pub health_percent: u8,
    pub is_in_combat: bool,
}

/// Threat information for LLM context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatInfo {
    pub name: String,
    pub level: u16,
    pub distance: f32,
}

/// Item information for LLM context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemInfo {
    pub name: String,
    pub distance: f32,
}

/// Recent chat for LLM context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentChatInfo {
    pub sender: String,
    pub message: String,
}

/// Bot context optimized for LLM consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotContext {
    pub bot: BotContextBot,
    pub assigned_player: Option<AssignedPlayerInfo>,
    pub nearby_threats: Vec<ThreatInfo>,
    pub nearby_items: Vec<ItemInfo>,
    pub recent_chat: Vec<RecentChatInfo>,
    pub available_actions: Vec<String>,
}

/// Bot information within context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotContextBot {
    pub name: String,
    pub level: u16,
    pub job: String,
    pub health_percent: u8,
    pub mana_percent: u8,
    pub position: Position,
    pub zone: String,
}

/// Action type for LLM execute endpoint
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmActionType {
    FollowPlayer,
    MoveTo,
    AttackNearest,
    AttackTarget,
    UseSkill,
    UseItem,
    SetBehaviorMode,
    Say,
    PickupItems,
    Sit,
    Stand,
    Wait,
}

/// Parameters for LLM execute endpoint
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmExecuteParameters {
    /// Player name (for follow_player)
    #[serde(default)]
    pub player_name: Option<String>,
    /// Target position (for move_to)
    #[serde(default)]
    pub position: Option<Position>,
    /// Target entity ID (for attack_target)
    #[serde(default)]
    pub target_entity_id: Option<u32>,
    /// Skill ID (for use_skill)
    #[serde(default)]
    pub skill_id: Option<u16>,
    /// Message content (for say)
    #[serde(default)]
    pub message: Option<String>,
    /// Item slot index (for use_item)
    #[serde(default)]
    pub item_slot: Option<u16>,
    /// Behavior mode (for set_behavior_mode)
    #[serde(default)]
    pub behavior_mode: Option<String>,
    /// Wait duration in seconds (for wait)
    #[serde(default)]
    pub duration: Option<f32>,
}

/// Request for LLM execute endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmExecuteRequest {
    pub action: LlmActionType,
    #[serde(default)]
    pub parameters: LlmExecuteParameters,
}

/// Bot summary for list endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotSummary {
    pub bot_id: Uuid,
    pub name: String,
    pub level: u16,
    pub health: VitalPoints,
    pub position: ZonePosition,
    pub assigned_player: Option<String>,
    pub status: String,
}

/// Response for list bots endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotListResponse {
    pub bots: Vec<BotSummary>,
}

/// Generic API response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// Empty response for simple operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Empty {}

/// Error response type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: u16,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>, code: u16) -> Self {
        Self {
            error: error.into(),
            code,
        }
    }
}
