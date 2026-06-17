//! OpenAI-compatible provider (DeepSeek's `/chat/completions` endpoint, and any
//! other OpenAI-compatible backend). Reuses the neutral schema's OpenAI-shaped
//! `Serialize` for the request body and parses the OpenAI SSE delta stream.

use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;

use anyhow::Result;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::Client;

use super::{
  FunctionCall, LlmProvider, Message, StreamItem, StreamResult, Tool, ToolCall, UsageInfo,
};

pub struct OpenAiProvider {
  client: Client,
  api_key: String,
  pub base_url: String,
}

/// Accumulator for a single tool call as it streams in fragments. OpenAI-style
/// providers split a tool call across many SSE deltas: the first carries
/// `id`/`name`, later ones carry chunks of the JSON `arguments`. Concatenate
/// by `index`.
struct PartialToolCall {
  id: String,
  tool_type: String,
  name: String,
  arguments: String,
}

struct StreamState {
  stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
  buffer: Vec<u8>,
  pending: VecDeque<Result<StreamItem>>,
  finished: bool,
  partial_tools: BTreeMap<usize, PartialToolCall>,
}

impl StreamState {
  fn flush_tool_calls(&mut self) {
    let drained: Vec<(usize, PartialToolCall)> = std::mem::take(&mut self.partial_tools)
      .into_iter()
      .collect();
    for (_, p) in drained {
      if p.name.is_empty() {
        continue;
      }
      let id = if p.id.is_empty() {
        format!("call_{}", uuid::Uuid::new_v4())
      } else {
        p.id
      };
      let arguments = if p.arguments.is_empty() {
        "{}".to_string()
      } else {
        p.arguments
      };
      self.pending.push_back(Ok(StreamItem::ToolCall(ToolCall {
        id,
        tool_type: p.tool_type,
        function: FunctionCall {
          name: p.name,
          arguments,
        },
      })));
    }
  }
}

impl OpenAiProvider {
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

  /// Default DeepSeek OpenAI-compatible endpoint, overridable via env.
  pub fn default_base_url() -> String {
    std::env::var("DEEPSEEK_API_BASE").unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string())
  }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
  async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<StreamResult> {
    let mut body = serde_json::json!({
      "model": model,
      "messages": messages,
      "stream": true,
    });

    // DeepSeek's OpenAI-compatible thinking control: a boolean `thinking`
    // toggle plus `reasoning_effort` for intensity. (The old `{thinking:{mode}}`
    // shape was silently ignored by the server.)
    if thinking_mode != "none" {
      body["thinking"] = serde_json::json!({ "type": "enabled" });
      body["reasoning_effort"] = serde_json::json!(thinking_mode);
    }

    if let Some(t) = tools {
      body["tools"] = serde_json::json!(t);
    }

    let resp = self
      .client
      .post(format!("{}/chat/completions", self.base_url))
      .header("Authorization", format!("Bearer {}", self.api_key))
      .json(&body)
      .send()
      .await?;

    if !resp.status().is_success() {
      let status = resp.status();
      let err_body = resp.text().await?;
      anyhow::bail!("API Error {}: {}", status, err_body);
    }

    let stream = resp.bytes_stream();
    Ok(Box::pin(StreamState {
      stream: Box::pin(stream),
      buffer: Vec::new(),
      pending: VecDeque::new(),
      finished: false,
      partial_tools: BTreeMap::new(),
    }))
  }
}

impl Stream for StreamState {
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
            let line = String::from_utf8_lossy(&self.buffer[start..line_pos]);
            start = line_pos + 1;

            if let Some(data) = line.strip_prefix("data: ") {
              let data = data.trim();
              if data == "[DONE]" {
                self.flush_tool_calls();
                self.finished = true;
                break;
              }

              let val = match serde_json::from_str::<serde_json::Value>(data) {
                Ok(v) => v,
                Err(_) => continue,
              };

              // Usage info can land in a chunk with empty `choices`, so parse
              // it before the choices branch.
              if let Some(usage) = val.get("usage").and_then(|u| u.as_object()) {
                let info = UsageInfo {
                  prompt_tokens: usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                  completion_tokens: usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                  prompt_cache_hit_tokens: usage
                    .get("prompt_cache_hit_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                  prompt_cache_miss_tokens: usage
                    .get("prompt_cache_miss_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                };
                self.pending.push_back(Ok(StreamItem::Usage(info)));
              }

              if let Some(choices) = val["choices"].as_array()
                && let Some(choice) = choices.first()
              {
                let delta = &choice["delta"];

                if let Some(reasoning) = delta["reasoning_content"].as_str()
                  && !reasoning.is_empty()
                {
                  self
                    .pending
                    .push_back(Ok(StreamItem::Reasoning(reasoning.to_string())));
                }

                if let Some(content) = delta["content"].as_str()
                  && !content.is_empty()
                {
                  self
                    .pending
                    .push_back(Ok(StreamItem::Content(content.to_string())));
                }

                if let Some(tool_calls) = delta["tool_calls"].as_array() {
                  for tc in tool_calls {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    let entry = self
                      .partial_tools
                      .entry(idx)
                      .or_insert_with(|| PartialToolCall {
                        id: String::new(),
                        tool_type: "function".to_string(),
                        name: String::new(),
                        arguments: String::new(),
                      });
                    if let Some(id) = tc["id"].as_str()
                      && !id.is_empty()
                    {
                      entry.id = id.to_string();
                    }
                    if let Some(t) = tc["type"].as_str()
                      && !t.is_empty()
                    {
                      entry.tool_type = t.to_string();
                    }
                    if let Some(name) = tc["function"]["name"].as_str()
                      && !name.is_empty()
                    {
                      entry.name = name.to_string();
                    }
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                      entry.arguments.push_str(args);
                    }
                  }
                }

                if let Some(finish_reason) = choice["finish_reason"].as_str() {
                  self.flush_tool_calls();
                  self
                    .pending
                    .push_back(Ok(StreamItem::Finish(Some(finish_reason.to_string()))));
                  self.finished = true;
                }
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
