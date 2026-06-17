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

/// Drop per-message `reasoning_content` before sending to a provider.
///
/// Reasoning is single-turn scratch — the final answer already lives in
/// `content`. Replaying history reasoning wastes tokens, and reasoning models
/// don't expect it back (Anthropic outright rejects unsigned thinking blocks).
/// Persisted sessions keep their reasoning; only the wire payload is stripped.
pub fn strip_reasoning(messages: &[Message]) -> Vec<Message> {
  messages
    .iter()
    .map(|m| match m {
      Message::Simple {
        role,
        content,
        tool_calls,
        ..
      } => Message::Simple {
        role: role.clone(),
        content: content.clone(),
        reasoning_content: None,
        tool_calls: tool_calls.clone(),
      },
      other => other.clone(),
    })
    .collect()
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn strip_reasoning_drops_reasoning_keeps_rest() {
    let msgs = vec![
      Message::Simple {
        role: "assistant".into(),
        content: "answer".into(),
        reasoning_content: Some("long chain of thought".into()),
        tool_calls: Some(vec![ToolCall {
          id: "t1".into(),
          tool_type: "function".into(),
          function: FunctionCall {
            name: "read_file".into(),
            arguments: "{}".into(),
          },
        }]),
      },
      Message::ToolResponse {
        role: "tool".into(),
        content: "result".into(),
        tool_call_id: "t1".into(),
      },
    ];
    let out = strip_reasoning(&msgs);
    match &out[0] {
      Message::Simple {
        content,
        reasoning_content,
        tool_calls,
        ..
      } => {
        assert_eq!(content, "answer");
        assert!(reasoning_content.is_none(), "reasoning must be stripped");
        assert!(tool_calls.is_some(), "tool_calls must be preserved");
      }
      _ => panic!("expected Simple"),
    }
    // ToolResponse passes through untouched.
    assert!(matches!(out[1], Message::ToolResponse { .. }));
  }
}
