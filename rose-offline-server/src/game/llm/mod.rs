//! LLM Feedback Loop Module
//!
//! This module provides the event system foundation for LLM-controlled bot feedback.
//! It captures game events and makes them available for the LLM feedback system.
//!
//! ## Feature Flag
//!
//! This module is only available when the `llm-feedback` feature is enabled.
//!
//! ## Components
//!
//! - **config**: Configuration for the LLM feedback system
//! - **event_queue**: Queue for buffering events before LLM processing
//! - **event_types**: Event types relevant to LLM decision-making
//! - **client**: HTTP client for OpenAI-compatible LLM API
//! - **prompts**: System prompts and message building functions
//! - **tools**: Tool definitions for LLM bot control
//! - **context_builder**: Context builder for gathering bot state and nearby entities
//! - **tool_executor**: Tool executor for converting LLM tool calls to game commands
//! - **feedback_system**: Main feedback loop system that orchestrates LLM queries
//! - **event_collector_system**: System that collects game events and converts to LLM events
//! - **llm_context_query_system**: System that queries ECS for bot context
//! - **chat_handler**: System for handling player-to-bot chat communication

mod client;
mod config;
mod context_builder;
mod event_queue;
mod event_types;
mod prompts;
mod tool_executor;
mod tools;

// These modules are only available with the llm-feedback feature
#[cfg(feature = "llm-feedback")]
mod feedback_system;
#[cfg(feature = "llm-feedback")]
mod event_collector_system;
#[cfg(feature = "llm-feedback")]
mod llm_context_query_system;
#[cfg(feature = "llm-feedback")]
mod chat_handler;

pub use client::{
    ChatCompletionRequest, ChatCompletionResponse, ChatChoice, ChatMessage, FunctionCall,
    FunctionDefinition, LlmClient, LlmError, ToolCall, ToolDefinition, Usage,
};
pub use config::LlmConfig;
pub use context_builder::{
    AssignedPlayerContext, LlmContext, LlmContextBuilder, NearbyEntityContext, NearbyItemContext,
    NearbyPlayerContext, SkillContext,
};
pub use event_queue::LlmEventQueue;
pub use event_types::{EventPriority, LlmEvent, TimestampedLlmEvent};
pub use prompts::{
    build_chat_response_prompt, build_combat_prompt, build_conversation_prompt,
    build_follow_prompt, build_low_health_prompt, build_user_message, SYSTEM_PROMPT,
};
pub use tool_executor::{
    execute_multiple_tool_calls, execute_tool_call, parse_behavior_mode, ToolExecutionError,
    ToolExecutionResult,
};
pub use tools::{
    get_tool_definitions, AttackTargetArgs, BehaviorMode, Destination, FollowPlayerArgs,
    MoveBotArgs, PickupItemArgs, SendChatArgs, SetBehaviorModeArgs, UseSkillArgs,
};

// Export feedback system types when feature is enabled
#[cfg(feature = "llm-feedback")]
pub use feedback_system::{
    LlmClientResource, LlmCommandSenderResource, LlmFeedbackState, PendingResponse,
    llm_feedback_system, llm_process_responses_system, llm_cleanup_stale_responses_system,
};
#[cfg(feature = "llm-feedback")]
pub use event_collector_system::llm_event_collector_system;
#[cfg(feature = "llm-feedback")]
pub use llm_context_query_system::{
    gather_bot_context_system, build_bot_context, build_quick_context,
};
#[cfg(feature = "llm-feedback")]
pub use chat_handler::{
    ChatHandlerState, PendingChatResponse,
    llm_chat_handler_system, llm_chat_response_processor_system, llm_cleanup_stale_chat_responses_system,
    is_chat_directed_at_bot, spawn_chat_response_task, CHAT_PROXIMITY_DISTANCE,
};
