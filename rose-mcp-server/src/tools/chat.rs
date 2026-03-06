//! Chat tools for MCP server

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::api_client::{ApiClient, ChatRequest};
use crate::schemas::{GetChatHistoryParams, SendChatParams};
use crate::tools::{create_tool, format_error, format_response, ToolHandler};

/// Tool to send a chat message
pub struct SendChatTool {
    api_client: Arc<ApiClient>,
}

impl SendChatTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for SendChatTool {
    fn tool(&self) -> Tool {
        create_tool(
            "send_chat",
            "Send a chat message from the bot to nearby players. The message will be visible to players within range of the bot.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "message": {
                        "type": "string",
                        "description": "Chat message to send"
                    },
                    "chat_type": {
                        "type": "string",
                        "description": "Type of chat message",
                        "enum": ["local", "shout"],
                        "default": "local"
                    }
                },
                "required": ["bot_id", "message"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: SendChatParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        let request = ChatRequest {
            message: params.message.clone(),
            chat_type: params.chat_type.unwrap_or_else(|| "local".to_string()),
        };

        match self.api_client.send_chat(&bot_id, request).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} sent chat: '{}'", bot_id, params.message)
            }))),
            Err(e) => Ok(format_error(format!("Failed to send chat: {}", e))),
        }
    }
}

/// Tool to get chat history
pub struct GetChatHistoryTool {
    api_client: Arc<ApiClient>,
}

impl GetChatHistoryTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetChatHistoryTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_chat_history",
            "Get recent chat messages received by the bot. This shows what nearby players have said.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    }
                },
                "required": ["bot_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: GetChatHistoryParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_chat_history(&bot_id).await {
            Ok(history) => {
                let messages: Vec<_> = history
                    .messages
                    .into_iter()
                    .map(|m| {
                        json!({
                            "timestamp": m.timestamp,
                            "sender_name": m.sender_name,
                            "sender_entity_id": m.sender_entity_id,
                            "message": m.message,
                            "chat_type": m.chat_type
                        })
                    })
                    .collect();

                Ok(format_response(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "messages": messages,
                    "count": messages.len()
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get chat history: {}", e))),
        }
    }
}
