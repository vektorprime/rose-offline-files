//! HTTP Client for OpenAI-compatible LLM API
//!
//! This module provides the HTTP client and data structures for communicating
//! with an OpenAI-compatible LLM server (e.g., llama.cpp, vLLM, etc.).

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use super::LlmConfig;

/// Errors that can occur during LLM API communication.
#[derive(Debug, Error)]
pub enum LlmError {
    /// Failed to build the HTTP client
    #[error("Failed to build HTTP client: {0}")]
    HttpClientBuildError(#[source] reqwest::Error),

    /// Failed to send the request to the LLM server
    #[error("Request failed: {0}")]
    RequestFailed(#[source] reqwest::Error),

    /// The LLM server returned an error status code
    #[error("API error: HTTP {0}")]
    ApiError(u16),

    /// Failed to parse the response from the LLM server
    #[error("Failed to parse response: {0}")]
    ParseError(#[source] reqwest::Error),

    /// The request timed out
    #[error("Request timed out")]
    Timeout,

    /// Failed to connect to the LLM server
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// No tool call was returned in the response
    #[error("No tool call in response")]
    NoToolCall,

    /// Failed to serialize the request
    #[error("Failed to serialize request: {0}")]
    SerializationError(#[source] serde_json::Error),
}

/// HTTP client for OpenAI-compatible LLM API.
///
/// This client handles communication with LLM servers that implement the
/// OpenAI chat completions API, such as llama.cpp, vLLM, or OpenAI itself.
pub struct LlmClient {
    /// The underlying HTTP client
    http_client: reqwest::Client,
    /// Configuration for the LLM connection
    config: LlmConfig,
}

impl LlmClient {
    /// Creates a new LLM client with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(config: &LlmConfig) -> Result<Self, LlmError> {
        let timeout_secs = 30; // Default 30 second timeout
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(LlmError::HttpClientBuildError)?;

        Ok(Self {
            http_client,
            config: config.clone(),
        })
    }

    /// Sends a chat completion request to the LLM server.
    ///
    /// # Arguments
    ///
    /// * `request` - The chat completion request to send
    ///
    /// # Returns
    ///
    /// The chat completion response on success, or an error.
    pub async fn complete(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, LlmError> {
        let url = format!("{}/v1/chat/completions", self.config.server_url);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(LlmError::RequestFailed)?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            // Try to get error body for more context
            let _body = response.text().await.unwrap_or_default();
            return Err(LlmError::ApiError(status));
        }

        let completion = response
            .json::<ChatCompletionResponse>()
            .await
            .map_err(LlmError::ParseError)?;

        Ok(completion)
    }
}

// ============================================================================
// OpenAI API Request/Response Types
// ============================================================================

/// A chat completion request for the OpenAI API.
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionRequest {
    /// The model to use for completion
    pub model: String,
    /// The messages in the conversation
    pub messages: Vec<ChatMessage>,
    /// The tools available for the LLM to call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    /// How to select tools: "auto", "none", or {"type": "function", "function": {"name": "..."}}
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    /// Maximum tokens in the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Temperature for response randomness (0.0 - 2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

impl ChatCompletionRequest {
    /// Creates a new chat completion request with the given model and messages.
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: None,
            tool_choice: None,
            max_tokens: None,
            temperature: None,
        }
    }

    /// Sets the tools available for the LLM to call.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = Some(tools);
        self.tool_choice = Some(serde_json::json!("auto"));
        self
    }

    /// Sets the maximum tokens in the response.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}

/// A message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The role of the message author: "system", "user", "assistant", or "tool"
    pub role: String,
    /// The content of the message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Tool calls made by the assistant (for assistant role)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The tool call ID this message is responding to (for tool role)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    /// Creates a new system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Creates a new user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Creates a new assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Creates an assistant message with tool calls.
    pub fn assistant_with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    /// Creates a tool response message.
    pub fn tool_response(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A tool definition for the OpenAI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The type of tool (always "function" for now)
    #[serde(rename = "type")]
    pub tool_type: String,
    /// The function definition
    pub function: FunctionDefinition,
}

impl ToolDefinition {
    /// Creates a new function tool definition.
    pub fn function(definition: FunctionDefinition) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: definition,
        }
    }
}

/// A function definition for a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    /// The name of the function
    pub name: String,
    /// A description of what the function does
    pub description: String,
    /// The parameters the function accepts (JSON Schema)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

impl FunctionDefinition {
    /// Creates a new function definition.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: None,
        }
    }

    /// Sets the parameters schema for the function.
    pub fn with_parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = Some(parameters);
        self
    }
}

/// A tool call in an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// The ID of the tool call
    pub id: String,
    /// The type of tool (always "function" for now)
    #[serde(rename = "type")]
    pub call_type: String,
    /// The function call details
    pub function: FunctionCall,
}

impl ToolCall {
    /// Creates a new tool call.
    pub fn new(id: impl Into<String>, function: FunctionCall) -> Self {
        Self {
            id: id.into(),
            call_type: "function".to_string(),
            function,
        }
    }
}

/// A function call within a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    /// The name of the function to call
    pub name: String,
    /// The arguments to pass to the function (JSON string)
    pub arguments: String,
}

impl FunctionCall {
    /// Creates a new function call.
    pub fn new(name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    /// Creates a new function call with JSON arguments.
    pub fn with_json_arguments(name: impl Into<String>, arguments: &impl Serialize) -> Self {
        Self {
            name: name.into(),
            arguments: serde_json::to_string(arguments).unwrap_or_default(),
        }
    }

    /// Parses the arguments as a specific type.
    pub fn parse_arguments<T: for<'de> Deserialize<'de>>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.arguments)
    }
}

/// A chat completion response from the OpenAI API.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionResponse {
    /// The unique ID for this completion
    pub id: String,
    /// The object type (e.g., "chat.completion")
    pub object: String,
    /// The Unix timestamp of when the completion was created
    pub created: u64,
    /// The model used for the completion
    pub model: String,
    /// The completion choices
    pub choices: Vec<ChatChoice>,
    /// Token usage information
    #[serde(default)]
    pub usage: Option<Usage>,
}

impl ChatCompletionResponse {
    /// Returns the first choice, if any.
    pub fn first_choice(&self) -> Option<&ChatChoice> {
        self.choices.first()
    }

    /// Returns the message from the first choice, if any.
    pub fn first_message(&self) -> Option<&ChatMessage> {
        self.choices.first().map(|c| &c.message)
    }

    /// Returns the first tool call from the first choice, if any.
    pub fn first_tool_call(&self) -> Option<&ToolCall> {
        self.choices
            .first()
            .and_then(|c| c.message.tool_calls.as_ref())
            .and_then(|calls| calls.first())
    }

    /// Returns all tool calls from the first choice, if any.
    pub fn tool_calls(&self) -> Option<&Vec<ToolCall>> {
        self.choices
            .first()
            .and_then(|c| c.message.tool_calls.as_ref())
    }
}

/// A single completion choice in a response.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatChoice {
    /// The index of this choice
    pub index: u32,
    /// The message generated by the LLM
    pub message: ChatMessage,
    /// The reason the completion finished
    pub finish_reason: Option<String>,
}

/// Token usage information.
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    /// Number of tokens in the prompt
    pub prompt_tokens: u64,
    /// Number of tokens in the completion
    pub completion_tokens: u64,
    /// Total tokens used
    pub total_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_system() {
        let msg = ChatMessage::system("You are a helpful assistant.");
        assert_eq!(msg.role, "system");
        assert_eq!(msg.content, Some("You are a helpful assistant.".to_string()));
        assert!(msg.tool_calls.is_none());
    }

    #[test]
    fn test_chat_message_user() {
        let msg = ChatMessage::user("Hello!");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, Some("Hello!".to_string()));
    }

    #[test]
    fn test_function_call_parse_arguments() {
        #[derive(Deserialize)]
        struct TestArgs {
            name: String,
            value: i32,
        }

        let call = FunctionCall::new("test", r#"{"name":"test","value":42}"#);
        let args: TestArgs = call.parse_arguments().unwrap();
        assert_eq!(args.name, "test");
        assert_eq!(args.value, 42);
    }

    #[test]
    fn test_function_call_with_json_arguments() {
        #[derive(Serialize)]
        struct TestArgs {
            name: String,
            value: i32,
        }

        let args = TestArgs {
            name: "test".to_string(),
            value: 42,
        };
        let call = FunctionCall::with_json_arguments("test_fn", &args);
        assert_eq!(call.name, "test_fn");
        assert!(call.arguments.contains("test"));
        assert!(call.arguments.contains("42"));
    }

    #[test]
    fn test_chat_completion_request_serialization() {
        let request = ChatCompletionRequest::new(
            "local-model",
            vec![
                ChatMessage::system("You are a bot."),
                ChatMessage::user("Hello!"),
            ],
        )
        .with_max_tokens(100);

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"model\":\"local-model\""));
        assert!(json.contains("\"max_tokens\":100"));
        // Temperature is no longer sent - LLM uses its native temperature
        assert!(!json.contains("\"temperature\""));
    }

    #[test]
    fn test_tool_definition_serialization() {
        let tool = ToolDefinition::function(FunctionDefinition::new(
            "test_function",
            "A test function",
        ).with_parameters(serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            }
        })));

        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("\"type\":\"function\""));
        assert!(json.contains("\"name\":\"test_function\""));
    }

    #[test]
    fn test_response_first_tool_call() {
        let response = ChatCompletionResponse {
            id: "test".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "local".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage::assistant_with_tool_calls(vec![ToolCall::new(
                    "call_123",
                    FunctionCall::new("test_fn", r#"{"arg":1}"#),
                )]),
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };

        let tool_call = response.first_tool_call().unwrap();
        assert_eq!(tool_call.id, "call_123");
        assert_eq!(tool_call.function.name, "test_fn");
    }
}
