//! LLM Buddy Bot REST API Module
//!
//! This module provides a REST API for controlling bot players in the game
//! via HTTP endpoints. It's designed to allow LLMs to interact with and
//! control bot characters.

mod channels;
mod handlers;
pub mod models;
mod routes;
mod server;
mod state;

pub use channels::{
    BotSummaryData, DeleteBotResponse, GetBotContextResponse, GetBotInventoryResponse,
    GetBotListResponse, GetBotSkillsResponse, GetChatHistoryResponse, GetPlayerStatusResponse,
    GetZoneInfoResponse, LlmBotCommand, LlmBotManager,
};
pub use routes::create_router;
pub use server::{start_api_server, ApiServerConfig};
pub use state::{ApiState, BotInfo};
