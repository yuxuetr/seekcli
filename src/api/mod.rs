//! Provider-neutral message/tool/stream schema and the `LlmProvider` trait.
//!
//! The schema types (`Message`, `Tool`, `StreamItem`, …) are the engine's
//! lingua franca — the agent loop only ever speaks in these. Each provider
//! translates them to and from its own wire format, so swapping providers
//! never touches `engine.rs`.
//!
//! Note: `Message`/`Tool`'s derived `Serialize` happens to be OpenAI-shaped,
//! which also serves as the session-storage format. The OpenAI provider reuses
//! it directly; the Anthropic provider translates explicitly.

use std::pin::Pin;

use anyhow::Result;
use futures_util::Stream;
use serde::{Deserialize, Serialize};

pub mod anthropic;
pub mod openai;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Message {
  Simple {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
  },
  ToolResponse {
    role: String,
    content: String,
    tool_call_id: String,
  },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCall {
  pub id: String,
  #[serde(rename = "type")]
  pub tool_type: String,
  pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionCall {
  pub name: String,
  pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tool {
  #[serde(rename = "type")]
  pub tool_type: String,
  pub function: FunctionDefinition,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionDefinition {
  pub name: String,
  pub description: String,
  pub parameters: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum StreamItem {
  Reasoning(String),
  Content(String),
  ToolCall(ToolCall),
  Finish(Option<String>),
  Usage(UsageInfo),
}

/// Token-level accounting reported at the end of a streamed response.
#[derive(Debug, Clone, Default)]
pub struct UsageInfo {
  pub prompt_tokens: u64,
  pub completion_tokens: u64,
  pub prompt_cache_hit_tokens: u64,
  pub prompt_cache_miss_tokens: u64,
}

impl Message {
  pub fn new_user_text(text: String) -> Self {
    Self::Simple {
      role: "user".to_string(),
      content: text,
      reasoning_content: None,
      tool_calls: None,
    }
  }
}

/// A streamed item or a hard error.
pub type StreamResult = Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>;

/// An LLM backend. Implementations translate the neutral schema to their wire
/// format and parse their streamed response back into `StreamItem`s.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
  /// Issue a streaming generation request. `thinking_mode` is "none" | "high"
  /// | "max"; `tools` is omitted for tools-free planning passes.
  async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<StreamResult>;
}
