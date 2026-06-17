//! Anthropic-compatible provider (DeepSeek's `/anthropic` endpoint, or any
//! Anthropic Messages API backend).
//!
//! Unlike the OpenAI provider, the neutral schema cannot be serialized
//! directly: Anthropic puts `system` at the top level, expresses tool calls as
//! `tool_use` content blocks, returns tool results as `tool_result` blocks
//! inside a *user* turn, and streams a structured event sequence rather than
//! delta accumulation. This module translates both directions explicitly.

use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;

use anyhow::Result;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};

use super::{
  FunctionCall, LlmProvider, Message, StreamItem, StreamResult, Tool, ToolCall, UsageInfo,
};

/// Anthropic requires `max_tokens`; this is the default ceiling.
const MAX_TOKENS: u64 = 8192;

pub struct AnthropicProvider {
  client: Client,
  api_key: String,
  pub base_url: String,
}

impl AnthropicProvider {
  pub fn new(api_key: String, base_url: String) -> Self {
    let client = Client::builder()
      .no_proxy()
      .build()
      .unwrap_or_else(|_| Client::new());
    Self {
      client,
      api_key,
      base_url,
    }
  }

  /// Default DeepSeek Anthropic-compatible endpoint, overridable via env.
  pub fn default_base_url() -> String {
    std::env::var("DEEPSEEK_ANTHROPIC_BASE")
      .unwrap_or_else(|_| "https://api.deepseek.com/anthropic".to_string())
  }
}

/// Translate the neutral schema into an Anthropic Messages request body.
/// Pure and testable — no I/O.
pub fn build_body(
  model: &str,
  messages: &[Message],
  thinking_mode: &str,
  tools: Option<&[Tool]>,
) -> Value {
  // 1. Pull all system messages into the top-level `system` field.
  let mut system = String::new();
  for m in messages {
    if let Message::Simple { role, content, .. } = m
      && role == "system"
    {
      if !system.is_empty() {
        system.push_str("\n\n");
      }
      system.push_str(content);
    }
  }

  // 2. Build the messages array, merging consecutive tool results into one
  //    user turn (Anthropic requires tool_result blocks in a user message).
  let mut out: Vec<Value> = Vec::new();
  let mut pending_tool_results: Vec<Value> = Vec::new();

  let flush_results = |out: &mut Vec<Value>, results: &mut Vec<Value>| {
    if !results.is_empty() {
      out.push(json!({ "role": "user", "content": std::mem::take(results) }));
    }
  };

  for m in messages {
    match m {
      Message::ToolResponse {
        content,
        tool_call_id,
        ..
      } => {
        pending_tool_results.push(json!({
          "type": "tool_result",
          "tool_use_id": tool_call_id,
          "content": content,
        }));
      }
      Message::Simple {
        role,
        content,
        tool_calls,
        ..
      } => {
        if role == "system" {
          continue; // already hoisted
        }
        flush_results(&mut out, &mut pending_tool_results);

        if role == "assistant" {
          let mut blocks: Vec<Value> = Vec::new();
          if !content.is_empty() {
            blocks.push(json!({ "type": "text", "text": content }));
          }
          if let Some(calls) = tool_calls {
            for tc in calls {
              let input: Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
              blocks.push(json!({
                "type": "tool_use",
                "id": tc.id,
                "name": tc.function.name,
                "input": input,
              }));
            }
          }
          // An assistant turn must carry at least one block.
          if blocks.is_empty() {
            blocks.push(json!({ "type": "text", "text": "" }));
          }
          out.push(json!({ "role": "assistant", "content": blocks }));
        } else {
          // user (or any non-assistant, non-system) turn.
          out.push(json!({ "role": "user", "content": content }));
        }
      }
    }
  }
  flush_results(&mut out, &mut pending_tool_results);

  let mut body = json!({
    "model": model,
    "max_tokens": MAX_TOKENS,
    "stream": true,
    "messages": out,
  });
  if !system.is_empty() {
    body["system"] = json!(system);
  }
  if thinking_mode != "none" {
    // DeepSeek's Anthropic endpoint: enable thinking + effort via output_config.
    body["thinking"] = json!({ "type": "enabled" });
    body["output_config"] = json!({ "effort": thinking_mode });
  }
  if let Some(ts) = tools {
    let mapped: Vec<Value> = ts
      .iter()
      .map(|t| {
        json!({
          "name": t.function.name,
          "description": t.function.description,
          "input_schema": t.function.parameters,
        })
      })
      .collect();
    body["tools"] = json!(mapped);
  }
  body
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
  async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<StreamResult> {
    let body = build_body(model, &messages, thinking_mode, tools.as_deref());

    let resp = self
      .client
      .post(format!("{}/v1/messages", self.base_url))
      .header("x-api-key", &self.api_key)
      .header("anthropic-version", "2023-06-01")
      .header("content-type", "application/json")
      .json(&body)
      .send()
      .await?;

    if !resp.status().is_success() {
      let status = resp.status();
      let err_body = resp.text().await?;
      anyhow::bail!("Anthropic API Error {}: {}", status, err_body);
    }

    Ok(Box::pin(EventState {
      stream: Box::pin(resp.bytes_stream()),
      buffer: Vec::new(),
      pending: VecDeque::new(),
      finished: false,
      blocks: BTreeMap::new(),
      usage: UsageInfo::default(),
    }))
  }
}

/// Per-content-block accumulator. `tool_use` blocks collect partial JSON until
/// `content_block_stop`.
struct BlockState {
  kind: String,
  id: String,
  name: String,
  json_buf: String,
}

struct EventState {
  stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
  buffer: Vec<u8>,
  pending: VecDeque<Result<StreamItem>>,
  finished: bool,
  blocks: BTreeMap<usize, BlockState>,
  usage: UsageInfo,
}

impl EventState {
  /// Dispatch one parsed SSE `data:` JSON object to zero or more StreamItems.
  fn handle(&mut self, v: &Value) {
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
      "message_start" => {
        if let Some(u) = v.pointer("/message/usage") {
          // Anthropic splits input: `input_tokens` is the FRESH (uncached)
          // count; cached reads/writes are reported separately. Total prompt =
          // sum of all three.
          let fresh = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
          let cache_read = u
            .get("cache_read_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
          let cache_write = u
            .get("cache_creation_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
          self.usage.prompt_tokens = fresh + cache_read + cache_write;
          self.usage.prompt_cache_hit_tokens = cache_read;
          self.usage.prompt_cache_miss_tokens = fresh + cache_write;
        }
      }
      "content_block_start" => {
        let idx = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let cb = &v["content_block"];
        let kind = cb.get("type").and_then(|x| x.as_str()).unwrap_or("text");
        self.blocks.insert(
          idx,
          BlockState {
            kind: kind.to_string(),
            id: cb
              .get("id")
              .and_then(|x| x.as_str())
              .unwrap_or("")
              .to_string(),
            name: cb
              .get("name")
              .and_then(|x| x.as_str())
              .unwrap_or("")
              .to_string(),
            json_buf: String::new(),
          },
        );
      }
      "content_block_delta" => {
        let idx = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let delta = &v["delta"];
        match delta.get("type").and_then(|x| x.as_str()).unwrap_or("") {
          "text_delta" => {
            if let Some(t) = delta.get("text").and_then(|x| x.as_str()) {
              self
                .pending
                .push_back(Ok(StreamItem::Content(t.to_string())));
            }
          }
          "thinking_delta" => {
            if let Some(t) = delta.get("thinking").and_then(|x| x.as_str()) {
              self
                .pending
                .push_back(Ok(StreamItem::Reasoning(t.to_string())));
            }
          }
          "input_json_delta" => {
            if let Some(pj) = delta.get("partial_json").and_then(|x| x.as_str())
              && let Some(b) = self.blocks.get_mut(&idx)
            {
              b.json_buf.push_str(pj);
            }
          }
          _ => {}
        }
      }
      "content_block_stop" => {
        let idx = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        if let Some(b) = self.blocks.remove(&idx)
          && b.kind == "tool_use"
        {
          let arguments = if b.json_buf.trim().is_empty() {
            "{}".to_string()
          } else {
            b.json_buf
          };
          self.pending.push_back(Ok(StreamItem::ToolCall(ToolCall {
            id: b.id,
            tool_type: "function".to_string(),
            function: FunctionCall {
              name: b.name,
              arguments,
            },
          })));
        }
      }
      "message_delta" => {
        if let Some(o) = v.pointer("/usage/output_tokens").and_then(|x| x.as_u64()) {
          self.usage.completion_tokens = o;
        }
        if let Some(reason) = v.pointer("/delta/stop_reason").and_then(|x| x.as_str()) {
          self
            .pending
            .push_back(Ok(StreamItem::Usage(self.usage.clone())));
          self
            .pending
            .push_back(Ok(StreamItem::Finish(Some(reason.to_string()))));
        }
      }
      "message_stop" => {
        self.finished = true;
      }
      _ => {}
    }
  }
}

impl Stream for EventState {
  type Item = Result<StreamItem>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Option<Self::Item>> {
    if let Some(item) = self.pending.pop_front() {
      return std::task::Poll::Ready(Some(item));
    }
    if self.finished {
      return std::task::Poll::Ready(None);
    }

    while let std::task::Poll::Ready(maybe_bytes) = self.stream.poll_next_unpin(cx) {
      match maybe_bytes {
        Some(Ok(bytes)) => {
          self.buffer.extend_from_slice(&bytes);
          let mut start = 0;
          while let Some(line_end) = self.buffer[start..].iter().position(|&b| b == b'\n') {
            let line_pos = start + line_end;
            let line = String::from_utf8_lossy(&self.buffer[start..line_pos]).into_owned();
            start = line_pos + 1;
            // Anthropic SSE interleaves `event:` and `data:` lines; the JSON
            // `type` field is self-describing, so we only need `data:`.
            if let Some(data) = line.strip_prefix("data: ") {
              let data = data.trim();
              if data.is_empty() {
                continue;
              }
              if let Ok(v) = serde_json::from_str::<Value>(data) {
                self.handle(&v);
              }
            }
          }
          self.buffer.drain(..start);

          if let Some(item) = self.pending.pop_front() {
            return std::task::Poll::Ready(Some(item));
          }
          if self.finished {
            return std::task::Poll::Ready(None);
          }
        }
        Some(Err(e)) => return std::task::Poll::Ready(Some(Err(anyhow::Error::from(e)))),
        None => {
          self.finished = true;
          return std::task::Poll::Ready(None);
        }
      }
    }
    std::task::Poll::Pending
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::{FunctionDefinition, Tool};

  fn sys(c: &str) -> Message {
    Message::Simple {
      role: "system".into(),
      content: c.into(),
      reasoning_content: None,
      tool_calls: None,
    }
  }
  fn user(c: &str) -> Message {
    Message::new_user_text(c.into())
  }

  #[test]
  fn hoists_system_and_maps_user() {
    let msgs = vec![sys("rule A"), sys("rule B"), user("hi")];
    let body = build_body("m", &msgs, "none", None);
    assert_eq!(body["system"], "rule A\n\nrule B");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body.get("thinking").is_none());
  }

  #[test]
  fn assistant_tool_call_becomes_tool_use_block() {
    let msgs = vec![
      user("read it"),
      Message::Simple {
        role: "assistant".into(),
        content: "ok".into(),
        reasoning_content: None,
        tool_calls: Some(vec![ToolCall {
          id: "t1".into(),
          tool_type: "function".into(),
          function: FunctionCall {
            name: "read_file".into(),
            arguments: "{\"path\":\"a\"}".into(),
          },
        }]),
      },
      Message::ToolResponse {
        role: "tool".into(),
        content: "FILE BODY".into(),
        tool_call_id: "t1".into(),
      },
    ];
    let body = build_body("m", &msgs, "none", None);
    let arr = body["messages"].as_array().unwrap();
    // user, assistant(text+tool_use), user(tool_result)
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[1]["role"], "assistant");
    assert_eq!(arr[1]["content"][1]["type"], "tool_use");
    assert_eq!(arr[1]["content"][1]["name"], "read_file");
    assert_eq!(arr[1]["content"][1]["input"]["path"], "a");
    assert_eq!(arr[2]["role"], "user");
    assert_eq!(arr[2]["content"][0]["type"], "tool_result");
    assert_eq!(arr[2]["content"][0]["tool_use_id"], "t1");
  }

  #[test]
  fn consecutive_tool_results_merge_into_one_user_turn() {
    let msgs = vec![
      Message::ToolResponse {
        role: "tool".into(),
        content: "r1".into(),
        tool_call_id: "a".into(),
      },
      Message::ToolResponse {
        role: "tool".into(),
        content: "r2".into(),
        tool_call_id: "b".into(),
      },
    ];
    let body = build_body("m", &msgs, "none", None);
    let arr = body["messages"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["content"].as_array().unwrap().len(), 2);
  }

  #[test]
  fn tools_map_to_input_schema_and_thinking_sets_effort() {
    let tools = vec![Tool {
      tool_type: "function".into(),
      function: FunctionDefinition {
        name: "list_dir".into(),
        description: "list".into(),
        parameters: json!({"type":"object"}),
      },
    }];
    let body = build_body("m", &[user("x")], "high", Some(&tools));
    assert_eq!(body["tools"][0]["name"], "list_dir");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    assert!(body["tools"][0].get("function").is_none());
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["output_config"]["effort"], "high");
  }
}
