//! MCP tools module

pub mod bot_management;
pub mod chat;
pub mod combat;
pub mod information;
pub mod movement;

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde::Serialize;
use std::sync::Arc;

use crate::api_client::ApiClient;

/// Export all tool functions
pub use bot_management::*;
pub use chat::*;
pub use combat::*;
pub use information::*;
pub use movement::*;

/// Tool handler trait for MCP tools
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Get the tool definition
    fn tool(&self) -> Tool;
    
    /// Handle the tool call
    async fn handle(&self, arguments: serde_json::Value) -> Result<String>;
}

/// Create all available tools
pub fn create_tools(api_client: Arc<ApiClient>) -> Vec<Box<dyn ToolHandler>> {
    vec![
        Box::new(bot_management::CreateBuddyBotTool::new(api_client.clone())),
        Box::new(bot_management::GetBotStatusTool::new(api_client.clone())),
        Box::new(bot_management::GetBotContextTool::new(api_client.clone())),
        Box::new(bot_management::ListBotsTool::new(api_client.clone())),
        Box::new(bot_management::RemoveBotTool::new(api_client.clone())),
        Box::new(movement::MoveBotTool::new(api_client.clone())),
        Box::new(movement::FollowPlayerTool::new(api_client.clone())),
        Box::new(movement::StopBotTool::new(api_client.clone())),
        Box::new(combat::AttackTargetTool::new(api_client.clone())),
        Box::new(combat::UseSkillTool::new(api_client.clone())),
        Box::new(chat::SendChatTool::new(api_client.clone())),
        Box::new(chat::GetChatHistoryTool::new(api_client.clone())),
        Box::new(information::GetNearbyEntitiesTool::new(api_client.clone())),
        Box::new(information::GetBotSkillsTool::new(api_client.clone())),
        Box::new(information::GetBotInventoryTool::new(api_client.clone())),
        Box::new(information::GetPlayerStatusTool::new(api_client.clone())),
        Box::new(information::GetNearbyItemsTool::new(api_client.clone())),
        Box::new(information::GetZoneInfoTool::new(api_client.clone())),
        Box::new(movement::TeleportToPlayerTool::new(api_client.clone())),
        Box::new(combat::PickupItemTool::new(api_client.clone())),
        Box::new(combat::UseItemOnPlayerTool::new(api_client.clone())),
        Box::new(bot_management::SetBotBehaviorModeTool::new(api_client)),
    ]
}

/// Helper to format tool response as JSON
pub fn format_response<T: Serialize>(result: T) -> String {
    serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
}

/// Helper to format error response
pub fn format_error(error: impl std::fmt::Display) -> String {
    serde_json::json!({
        "success": false,
        "error": error.to_string()
    })
    .to_string()
}

/// Helper to create a Tool with proper defaults for rmcp0.17
pub fn create_tool(name: &'static str, description: &'static str, input_schema: serde_json::Value) -> Tool {
    use std::borrow::Cow;
    
    let schema = match input_schema {
        serde_json::Value::Object(map) => Arc::new(map),
        _ => Arc::new(serde_json::Map::new()),
    };
    
    Tool {
        name: Cow::Borrowed(name),
        title: Some(name.to_string()),
        description: Some(Cow::Borrowed(description)),
        input_schema: schema,
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}
