//! ROSE MCP Server - Entry point
//!
//! This MCP server provides tools for LLMs to control buddy bots
//! in the ROSE Online game server.

use anyhow::Result;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{stdin, stdout, BufReader, BufWriter};
use tokio::sync::Mutex;
use tracing::{info, warn, Level};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use rose_mcp_server::{api_client::*, config::Config};

/// State file name for persistence
const STATE_FILE: &str = "rose_mcp_state.json";

/// MCP Server for ROSE Online Buddy Bot Control
#[derive(Parser, Debug)]
#[command(name = "rose-mcp-server")]
#[command(about = "MCP Server for controlling buddy bots in ROSE Online", long_about = None)]
struct Args {
    /// Base URL for the ROSE Offline REST API
    #[arg(short, long, env = "ROSE_API_URL", default_value = "http://localhost:8080/api/v1")]
    api_url: String,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

// MCP Protocol Types

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolDefinition {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

/// Persistent state that survives process restarts
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentState {
    /// The currently active bot ID (set after create_buddy_bot or set_active_bot)
    current_bot_id: Option<Uuid>,
    /// Map of bot names to their IDs for lookup
    bot_names: std::collections::HashMap<String, Uuid>,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            current_bot_id: None,
            bot_names: std::collections::HashMap::new(),
        }
    }
}

impl PersistentState {
    /// Get the path to the state file
    fn state_file_path() -> PathBuf {
        // Try to use the same directory as the executable, or current directory
        std::env::current_exe()
            .ok()
            .and_then(|exe_path| exe_path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join(STATE_FILE)
    }

    /// Load state from file
    fn load() -> Self {
        let path = Self::state_file_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    match serde_json::from_str(&content) {
                        Ok(state) => {
                            info!("Loaded persistent state from {}", path.display());
                            return state;
                        }
                        Err(e) => {
                            warn!("Failed to parse state file {}: {}", path.display(), e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read state file {}: {}", path.display(), e);
                }
            }
        }
        Self::default()
    }

    /// Save state to file
    fn save(&self) {
        let path = Self::state_file_path();
        match serde_json::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&path, content) {
                    warn!("Failed to save state file {}: {}", path.display(), e);
                } else {
                    info!("Saved persistent state to {}", path.display());
                }
            }
            Err(e) => {
                warn!("Failed to serialize state: {}", e);
            }
        }
    }
}

/// Server state that persists between tool calls
#[derive(Debug, Default)]
struct ServerState {
    /// The persistent state (loaded from file)
    persistent: PersistentState,
}

/// ROSE MCP Server handler
struct RoseMcpServer {
    api_client: Arc<ApiClient>,
    /// Persistent state between tool calls
    state: Arc<Mutex<ServerState>>,
}

impl RoseMcpServer {
    fn new(api_client: Arc<ApiClient>) -> Self {
        // Load persistent state on startup
        let persistent = PersistentState::load();
        info!("Loaded state: current_bot_id = {:?}", persistent.current_bot_id);
        
        Self { 
            api_client,
            state: Arc::new(Mutex::new(ServerState {
                persistent,
            })),
        }
    }

    /// Get the current bot ID, either from arguments or from stored state
    async fn get_bot_id(&self, args: &Value) -> Result<Uuid, String> {
        // First try to get from arguments
        if let Some(bot_id_str) = args["bot_id"].as_str() {
            return bot_id_str.parse::<Uuid>().map_err(|e| format!("Invalid bot_id: {}", e));
        }
        
        // Then try to use stored bot ID
        let state = self.state.lock().await;
        if let Some(bot_id) = state.persistent.current_bot_id {
            return Ok(bot_id);
        }
        
        Err("No bot_id provided and no active bot set. Call create_buddy_bot first or provide bot_id parameter.".to_string())
    }

    /// Set the current active bot and persist to file
    async fn set_current_bot(&self, bot_id: Uuid, name: &str) {
        let mut state = self.state.lock().await;
        state.persistent.current_bot_id = Some(bot_id);
        state.persistent.bot_names.insert(name.to_string(), bot_id);
        info!("Set current bot: {} ({})", name, bot_id);
        
        // Save to file for persistence across process restarts
        state.persistent.save();
    }

    /// Try to find a bot by name from stored state
    async fn find_bot_by_name(&self, name: &str) -> Option<Uuid> {
        let state = self.state.lock().await;
        state.persistent.bot_names.get(name).copied()
    }

    fn get_tools() -> Vec<ToolDefinition> {
        vec![
            // Bot Management Tools
            ToolDefinition {
                name: "create_buddy_bot".to_string(),
                description: "Create a new buddy bot assigned to a player".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Name for the bot character" },
                        "assigned_player": { "type": "string", "description": "Name of the player this bot will assist" },
                        "level": { "type": "integer", "description": "Starting level (optional, default 1)" },
                        "build": { "type": "string", "description": "Character build type: soldier, muse, hawker, dealer (optional)" }
                    },
                    "required": ["name", "assigned_player"]
                }),
            },
            ToolDefinition {
                name: "get_bot_status".to_string(),
                description: "Get the current status of a bot (health, position, etc.)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" }
                    },
                    "required": ["bot_id"]
                }),
            },
            ToolDefinition {
                name: "list_bots".to_string(),
                description: "List all active buddy bots".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "remove_bot".to_string(),
                description: "Remove/delete a buddy bot".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot to remove" }
                    },
                    "required": ["bot_id"]
                }),
            },
            ToolDefinition {
                name: "get_bot_context".to_string(),
                description: "Get comprehensive context about a bot for LLM decision-making".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" }
                    },
                    "required": ["bot_id"]
                }),
            },
            // Movement Tools
            ToolDefinition {
                name: "move_bot".to_string(),
                description: "Move a bot to a position".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" },
                        "x": { "type": "number", "description": "X coordinate" },
                        "y": { "type": "number", "description": "Y coordinate" },
                        "z": { "type": "number", "description": "Z coordinate" },
                        "move_mode": { "type": "string", "description": "Movement mode: walk, run (optional, default run)" }
                    },
                    "required": ["bot_id", "x", "y", "z"]
                }),
            },
            ToolDefinition {
                name: "follow_player".to_string(),
                description: "Make a bot follow a player".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" },
                        "player_name": { "type": "string", "description": "Name of the player to follow" },
                        "distance": { "type": "number", "description": "Follow distance (optional, default 300)" }
                    },
                    "required": ["bot_id", "player_name"]
                }),
            },
            ToolDefinition {
                name: "stop_bot".to_string(),
                description: "Stop a bot's current action".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" }
                    },
                    "required": ["bot_id"]
                }),
            },
            // Combat Tools
            ToolDefinition {
                name: "attack_target".to_string(),
                description: "Make a bot attack a target".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" },
                        "target_entity_id": { "type": "integer", "description": "Entity ID of the target to attack" }
                    },
                    "required": ["bot_id", "target_entity_id"]
                }),
            },
            ToolDefinition {
                name: "use_skill".to_string(),
                description: "Make a bot use a skill on a target".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" },
                        "skill_id": { "type": "integer", "description": "ID of the skill to use" },
                        "target_type": { "type": "string", "description": "Type of target: entity, position, self" },
                        "target_entity_id": { "type": "integer", "description": "Entity ID of target (if target_type is entity)" }
                    },
                    "required": ["bot_id", "skill_id", "target_type"]
                }),
            },
            // Chat Tools
            ToolDefinition {
                name: "send_chat".to_string(),
                description: "Send a chat message from the bot".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" },
                        "message": { "type": "string", "description": "Chat message to send" },
                        "chat_type": { "type": "string", "description": "Chat type: local, party, shout (optional, default local)" }
                    },
                    "required": ["bot_id", "message"]
                }),
            },
            ToolDefinition {
                name: "get_chat_history".to_string(),
                description: "Get recent chat messages for a bot".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" }
                    },
                    "required": ["bot_id"]
                }),
            },
            // Information Tools
            ToolDefinition {
                name: "get_nearby_entities".to_string(),
                description: "Get entities near the bot with optional filtering by type. Use this to find players, monsters, NPCs, or items in the area.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" },
                        "radius": { "type": "number", "description": "Search radius (optional, default 1000)" },
                        "entity_types": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["players", "monsters", "npcs", "items"]
                            },
                            "description": "Types of entities to include (optional, defaults to all types)"
                        }
                    },
                    "required": ["bot_id"]
                }),
            },
            ToolDefinition {
                name: "get_bot_skills".to_string(),
                description: "Get available skills for a bot".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_id": { "type": "string", "description": "UUID of the bot" }
                    },
                    "required": ["bot_id"]
                }),
            },
        ]
    }

    async fn handle_tool_call(&self, name: &str, arguments: Value) -> Result<Value, String> {
        match name {
            // Bot Management
            "create_buddy_bot" => self.create_buddy_bot(arguments).await,
            "get_bot_status" => self.get_bot_status(arguments).await,
            "list_bots" => self.list_bots(arguments).await,
            "remove_bot" => self.remove_bot(arguments).await,
            "get_bot_context" => self.get_bot_context(arguments).await,
            // Movement
            "move_bot" => self.move_bot(arguments).await,
            "follow_player" => self.follow_player(arguments).await,
            "stop_bot" => self.stop_bot(arguments).await,
            // Combat
            "attack_target" => self.attack_target(arguments).await,
            "use_skill" => self.use_skill(arguments).await,
            // Chat
            "send_chat" => self.send_chat(arguments).await,
            "get_chat_history" => self.get_chat_history(arguments).await,
            // Information
            "get_nearby_entities" => self.get_nearby_entities(arguments).await,
            "get_bot_skills" => self.get_bot_skills(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        }
    }

    // Bot Management Tools Implementation

    async fn create_buddy_bot(&self, args: Value) -> Result<Value, String> {
        let name = args["name"].as_str().ok_or("Missing 'name' parameter")?.to_string();
        let assigned_player = args["assigned_player"].as_str().ok_or("Missing 'assigned_player' parameter")?.to_string();
        let level = args["level"].as_u64().map(|l| l as u16);
        let build = args["build"].as_str().map(|s| s.to_string());

        let request = CreateBotRequest {
            name: name.clone(),
            level,
            build,
            assigned_player,
        };

        match self.api_client.create_bot(request).await {
            Ok(response) => {
                // Store the bot ID as the current active bot
                self.set_current_bot(response.bot_id, &response.name).await;
                
                Ok(json!({
                    "success": true,
                    "bot_id": response.bot_id.to_string(),
                    "entity_id": response.entity_id,
                    "name": response.name,
                    "status": response.status,
                    "message": format!("Bot '{}' created and set as active. You can now use other tools without specifying bot_id.", response.name)
                }))
            },
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn get_bot_status(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        match self.api_client.get_bot_status(&bot_id).await {
            Ok(status) => Ok(json!({
                "success": true,
                "bot": {
                    "bot_id": status.bot_id.to_string(),
                    "name": status.name,
                    "level": status.level,
                    "job": status.job,
                    "health": {
                        "current": status.health.current,
                        "max": status.health.max
                    },
                    "mana": {
                        "current": status.mana.current,
                        "max": status.mana.max
                    },
                    "position": {
                        "x": status.position.x,
                        "y": status.position.y,
                        "z": status.position.z,
                        "zone_id": status.position.zone_id
                    },
                    "current_command": status.current_command,
                    "assigned_player": status.assigned_player,
                    "is_dead": status.is_dead,
                    "is_sitting": status.is_sitting
                }
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn list_bots(&self, _args: Value) -> Result<Value, String> {
        match self.api_client.list_bots().await {
            Ok(response) => {
                // Update local state with bot names
                {
                    let mut state = self.state.lock().await;
                    for b in &response.bots {
                        state.persistent.bot_names.insert(b.name.clone(), b.bot_id);
                    }
                    // Save updated state
                    state.persistent.save();
                }
                
                let bots: Vec<_> = response.bots.into_iter().map(|b| json!({
                    "bot_id": b.bot_id.to_string(),
                    "name": b.name,
                    "level": b.level,
                    "health": { "current": b.health.current, "max": b.health.max },
                    "position": { "x": b.position.x, "y": b.position.y, "z": b.position.z, "zone_id": b.position.zone_id },
                    "assigned_player": b.assigned_player,
                    "status": b.status
                })).collect();

                Ok(json!({
                    "success": true,
                    "bots": bots,
                    "count": bots.len()
                }))
            }
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn remove_bot(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        match self.api_client.delete_bot(&bot_id).await {
            Ok(_) => {
                // Always clear from local state on success (which includes "not found" cases
                // since the delete endpoint is now idempotent)
                let was_current = {
                    let mut state = self.state.lock().await;
                    let was_current = state.persistent.current_bot_id == Some(bot_id);
                    if was_current {
                        state.persistent.current_bot_id = None;
                    }
                    // Remove from names map
                    state.persistent.bot_names.retain(|_, &mut id| id != bot_id);
                    // Save updated state
                    state.persistent.save();
                    was_current
                };
                
                Ok(json!({
                    "success": true,
                    "message": format!("Bot {} removed successfully (or was already removed)", bot_id),
                    "was_current_bot": was_current
                }))
            },
            Err(e) => {
                // Even on error, clear stale state if the bot doesn't exist
                // This handles cases where the API returns an error but the bot is stale
                let error_str = e.to_string();
                if error_str.contains("not found") || error_str.contains("404") {
                    let mut state = self.state.lock().await;
                    if state.persistent.current_bot_id == Some(bot_id) {
                        state.persistent.current_bot_id = None;
                    }
                    state.persistent.bot_names.retain(|_, &mut id| id != bot_id);
                    state.persistent.save();
                    
                    return Ok(json!({
                        "success": true,
                        "message": format!("Bot {} was not found (may have been from a previous session) - cleared from local state", bot_id)
                    }));
                }
                
                Ok(json!({
                    "success": false,
                    "error": error_str
                }))
            }
        }
    }

    async fn get_bot_context(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        match self.api_client.get_bot_context(&bot_id).await {
            Ok(context) => Ok(json!({
                "success": true,
                "context": {
                    "bot": {
                        "name": context.bot.name,
                        "level": context.bot.level,
                        "job": context.bot.job,
                        "health_percent": context.bot.health_percent,
                        "mana_percent": context.bot.mana_percent,
                        "zone": context.bot.zone
                    },
                    "assigned_player": context.assigned_player,
                    "nearby_threats": context.nearby_threats,
                    "nearby_items": context.nearby_items,
                    "recent_chat": context.recent_chat,
                }
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    // Movement Tools Implementation

    async fn move_bot(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        let x = args["x"].as_f64().ok_or("Missing 'x' parameter")? as f32;
        let y = args["y"].as_f64().ok_or("Missing 'y' parameter")? as f32;
        let z = args["z"].as_f64().ok_or("Missing 'z' parameter")? as f32;
        let move_mode = args["move_mode"].as_str().unwrap_or("run").to_string();

        let request = MoveRequest {
            destination: Position::new(x, y, z),
            target_entity_id: None,
            move_mode,
        };

        match self.api_client.move_bot(&bot_id, request).await {
            Ok(_) => Ok(json!({
                "success": true,
                "message": format!("Bot {} moving to ({}, {}, {})", bot_id, x, y, z)
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn follow_player(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        let player_name = args["player_name"].as_str().ok_or("Missing 'player_name' parameter")?.to_string();
        let distance = args["distance"].as_f64().unwrap_or(300.0) as f32;

        let request = FollowRequest {
            player_name,
            distance,
        };

        match self.api_client.follow_player(&bot_id, request).await {
            Ok(_) => Ok(json!({
                "success": true,
                "message": format!("Bot {} following player", bot_id)
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn stop_bot(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        match self.api_client.stop_bot(&bot_id).await {
            Ok(_) => Ok(json!({
                "success": true,
                "message": format!("Bot {} stopped", bot_id)
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    // Combat Tools Implementation

    async fn attack_target(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        let target_entity_id = args["target_entity_id"].as_u64().ok_or("Missing 'target_entity_id' parameter")? as u32;

        let request = AttackRequest { target_entity_id };

        match self.api_client.attack_target(&bot_id, request).await {
            Ok(_) => Ok(json!({
                "success": true,
                "message": format!("Bot {} attacking target {}", bot_id, target_entity_id)
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn use_skill(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        let skill_id = args["skill_id"].as_u64().ok_or("Missing 'skill_id' parameter")? as u16;
        let target_type_str = args["target_type"].as_str().ok_or("Missing 'target_type' parameter")?;
        let target_entity_id = args["target_entity_id"].as_u64().map(|id| id as u32);

        let target_type = match target_type_str.to_lowercase().as_str() {
            "entity" => SkillTargetType::Entity,
            "position" => SkillTargetType::Position,
            "self" => SkillTargetType::SelfTarget,
            other => return Err(format!("Invalid target_type: {}", other)),
        };

        let request = SkillRequest {
            skill_id,
            target_type,
            target_entity_id,
            target_position: None,
        };

        match self.api_client.use_skill(&bot_id, request).await {
            Ok(_) => Ok(json!({
                "success": true,
                "message": format!("Bot {} using skill {}", bot_id, skill_id)
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    // Chat Tools Implementation

    async fn send_chat(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        let message = args["message"].as_str().ok_or("Missing 'message' parameter")?.to_string();
        let chat_type = args["chat_type"].as_str().unwrap_or("local").to_string();

        let request = ChatRequest {
            message,
            chat_type,
        };

        match self.api_client.send_chat(&bot_id, request).await {
            Ok(_) => Ok(json!({
                "success": true,
                "message": "Chat sent successfully"
            })),
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn get_chat_history(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        match self.api_client.get_chat_history(&bot_id).await {
            Ok(history) => {
                let messages: Vec<_> = history.messages.into_iter().map(|m| json!({
                    "sender_name": m.sender_name,
                    "sender_entity_id": m.sender_entity_id,
                    "message": m.message,
                    "chat_type": m.chat_type,
                    "timestamp": m.timestamp
                })).collect();
                
                Ok(json!({
                    "success": true,
                    "messages": messages,
                    "count": messages.len()
                }))
            }
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    // Information Tools Implementation

    async fn get_bot_skills(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        match self.api_client.get_bot_skills(&bot_id).await {
            Ok(response) => {
                let skills: Vec<_> = response.skills.into_iter().map(|s| json!({
                    "slot": s.slot,
                    "skill_id": s.skill_id,
                    "name": s.name,
                    "level": s.level,
                    "mp_cost": s.mp_cost,
                    "cooldown": s.cooldown
                })).collect();

                Ok(json!({
                    "success": true,
                    "skills": skills,
                    "count": skills.len()
                }))
            }
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }

    async fn get_nearby_entities(&self, args: Value) -> Result<Value, String> {
        let bot_id = self.get_bot_id(&args).await?;

        let radius = args["radius"].as_f64().map(|r| r as f32);
        
        // Convert entity_types array to comma-separated string for API
        let entity_types_filter = args["entity_types"].as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(",")
        });

        match self.api_client.get_nearby_entities(&bot_id, radius, entity_types_filter.as_deref()).await {
            Ok(response) => {
                // Group entities by type
                let mut players = Vec::new();
                let mut monsters = Vec::new();
                let mut npcs = Vec::new();
                let mut items = Vec::new();

                for e in response.entities {
                    let entity_type = match e.entity_type {
                        NearbyEntityType::Player => "player",
                        NearbyEntityType::Monster => "monster",
                        NearbyEntityType::Npc => "npc",
                        NearbyEntityType::Item => "item",
                    };
                    
                    match e.entity_type {
                        NearbyEntityType::Player => {
                            players.push(json!({
                                "entity_id": e.entity_id,
                                "entity_type": entity_type,
                                "name": e.name,
                                "level": e.level,
                                "distance": e.distance
                            }));
                        }
                        NearbyEntityType::Monster => {
                            monsters.push(json!({
                                "entity_id": e.entity_id,
                                "entity_type": entity_type,
                                "name": e.name,
                                "level": e.level,
                                "distance": e.distance
                            }));
                        }
                        NearbyEntityType::Npc => {
                            npcs.push(json!({
                                "entity_id": e.entity_id,
                                "entity_type": entity_type,
                                "name": e.name,
                                "distance": e.distance
                            }));
                        }
                        NearbyEntityType::Item => {
                            items.push(json!({
                                "entity_id": e.entity_id,
                                "entity_type": entity_type,
                                "name": e.name,
                                "distance": e.distance
                            }));
                        }
                    }
                }

                Ok(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "players": players,
                    "monsters": monsters,
                    "npcs": npcs,
                    "items": items,
                    "counts": {
                        "players": players.len(),
                        "monsters": monsters.len(),
                        "npcs": npcs.len(),
                        "items": items.len()
                    }
                }))
            }
            Err(e) => Ok(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    }
}

impl RoseMcpServer {
    async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => {
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": "rose-mcp-server",
                            "version": "0.1.0"
                        },
                        "instructions": "ROSE Online Buddy Bot Control Server. Use this server to create and control buddy bots in the ROSE Online game. Available tools allow you to create bots, move them around, attack monsters, chat with players, and gather information about the game world."
                    })),
                    error: None,
                }
            }
            "tools/list" => {
                let tools = Self::get_tools();
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(json!({
                        "tools": tools
                    })),
                    error: None,
                }
            }
            "tools/call" => {
                let params = request.params.unwrap_or(json!({}));
                let tool_name = params["name"].as_str().unwrap_or("");
                let arguments = if params["arguments"].is_null() {
                    json!({})
                } else {
                    params["arguments"].clone()
                };

                info!("Tool call: {} with arguments: {:?}", tool_name, arguments);

                match self.handle_tool_call(tool_name, arguments).await {
                    Ok(result) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        result: Some(json!({
                            "content": [{
                                "type": "text",
                                "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                            }]
                        })),
                        error: None,
                    },
                    Err(error_msg) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        result: Some(json!({
                            "content": [{
                                "type": "text",
                                "text": serde_json::to_string_pretty(&json!({
                                    "success": false,
                                    "error": error_msg
                                })).unwrap_or(error_msg)
                            }],
                            "isError": true
                        })),
                        error: None,
                    },
                }
            }
            "ping" => {
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(json!({})),
                    error: None,
                }
            }
            _ => {
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32601,
                        message: format!("Method not found: {}", request.method),
                        data: None,
                    }),
                }
            }
        }
    }

    async fn run(&self) -> Result<()> {
        let reader = BufReader::new(stdin());
        let mut writer = BufWriter::new(stdout());

        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let mut lines = reader.lines();
        
        info!("MCP Server started, waiting for requests on stdin...");

        while let Some(line) = lines.next_line().await? {
            if line.is_empty() {
                continue;
            }

            info!("Received: {}", line);

            match serde_json::from_str::<JsonRpcRequest>(&line) {
                Ok(request) => {
                    let response = self.handle_request(request).await;
                    let response_str = serde_json::to_string(&response)?;
                    info!("Sending: {}", response_str);
                    writer.write_all(response_str.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
                Err(e) => {
                    warn!("Failed to parse request: {}", e);
                    let error_response = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: None,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32700,
                            message: format!("Parse error: {}", e),
                            data: None,
                        }),
                    };
                    let response_str = serde_json::to_string(&error_response)?;
                    writer.write_all(response_str.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
            }
        }

        info!("MCP Server shutting down");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging to stderr (stdout is used for MCP communication)
    let log_level = if args.verbose { Level::DEBUG } else { Level::INFO };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(log_level.into())
                .from_env_lossy(),
        )
        .with_writer(std::io::stderr)  // Important: log to stderr, not stdout
        .init();

    info!("Starting ROSE MCP Server...");
    info!("API URL: {}", args.api_url);

    // Create configuration and API client
    let config = Config::new(&args.api_url);
    let api_client = Arc::new(ApiClient::new(config)?);

    // Create the MCP server
    let server = RoseMcpServer::new(api_client);

    info!("MCP Server initialized with {} tools", RoseMcpServer::get_tools().len());

    // Run the MCP server
    server.run().await
}
