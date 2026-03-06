//! ROSE MCP Server - MCP Server for LLM Buddy Bot Control
//!
//! This crate provides an MCP (Model Context Protocol) server that allows
//! LLMs like Claude to interact with the ROSE Offline REST API and control
//! buddy bot characters in the game.
//!
//! # Features
//!
//! - Bot management (create, list, status, remove)
//! - Movement control (move, follow, stop)
//! - Combat actions (attack, use skills)
//! - Chat functionality (send and receive messages)
//! - Information gathering (nearby entities, bot skills)
//!
//! # Usage
//!
//! ```bash
//! rose-mcp-server --api-url http://localhost:8080/api/v1
//! ```
//!
//! Or set the `ROSE_API_URL` environment variable.

pub mod api_client;
pub mod config;
pub mod schemas;

pub use api_client::ApiClient;
pub use config::Config;
