use std::collections::VecDeque;
use std::pin::Pin;

use anyhow::Result;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

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

pub struct ApiClient {
  client: Client,
  api_key: String,
  pub base_url: String,
}

#[derive(Debug, Clone)]
pub enum StreamItem {
  Reasoning(String),
  Content(String),
  ToolCall(ToolCall),
  Finish(Option<String>),
}

/// Accumulator for a single tool call as it streams in fragments.
/// OpenAI-style providers split a tool call across many SSE deltas:
/// the first delta carries `id` / `name`, subsequent deltas carry only
/// chunks of the JSON `arguments` string. We must concatenate by `index`.
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
  partial_tools: std::collections::BTreeMap<usize, PartialToolCall>,
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

impl ApiClient {
  pub fn new(api_key: String) -> Self {
    let base_url = std::env::var("DEEPSEEK_API_BASE")
      .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string());
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

  pub async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>> {
    let mut body = serde_json::json!({
      "model": model,
      "messages": messages,
      "stream": true,
    });

    if thinking_mode != "none" {
      body["thinking"] = serde_json::json!({
        "mode": thinking_mode,
      });
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
      partial_tools: std::collections::BTreeMap::new(),
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

              if let Ok(val) = serde_json::from_str::<serde_json::Value>(data)
                && let Some(choices) = val["choices"].as_array()
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
