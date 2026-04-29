use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::Stream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::pin::Pin;

#[derive(Debug, Serialize, Clone)]
pub struct ApiRequest {
  pub model: String,
  pub messages: Vec<Message>,
  pub temperature: Option<f32>,
  pub max_tokens: Option<u32>,
  pub stream: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub thinking: Option<ThinkingConfig>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub tools: Option<Vec<Tool>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Tool {
  #[serde(rename = "type")]
  pub tool_type: String,
  pub function: FunctionDefinition,
}

#[derive(Debug, Serialize, Clone)]
pub struct FunctionDefinition {
  pub name: String,
  pub description: String,
  pub parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Clone)]
pub struct ThinkingConfig {
  #[serde(rename = "type")]
  pub thinking_type: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub budget_tokens: Option<u32>,
}

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

pub struct ApiClient {
  client: Client,
  api_key: String,
  base_url: String,
}

#[derive(Debug, Clone)]
pub enum StreamItem {
  Reasoning(String),
  Content(String),
  ToolCall(ToolCall),
  Finish(Option<String>),
}

struct StreamState {
  stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
  buffer: Vec<u8>,
  pending: VecDeque<Result<StreamItem>>,
  finished: bool,
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
    use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
    use serde_json::Value;

    let thinking = match thinking_mode {
      "high" => Some(ThinkingConfig {
        thinking_type: "enabled".to_string(),
        budget_tokens: Some(4000),
      }),
      "max" => Some(ThinkingConfig {
        thinking_type: "enabled".to_string(),
        budget_tokens: Some(16000),
      }),
      _ => Some(ThinkingConfig {
        thinking_type: "disabled".to_string(),
        budget_tokens: None,
      }),
    };

    let request = ApiRequest {
      model: model.to_string(),
      messages,
      temperature: if thinking_mode != "none" {
        None
      } else {
        Some(0.7)
      },
      max_tokens: Some(8192),
      stream: true,
      thinking,
      tools,
    };

    let resp = self
      .client
      .post(format!("{}/chat/completions", self.base_url))
      .header(CONTENT_TYPE, "application/json")
      .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
      .json(&request)
      .send()
      .await
      .context("API request failed")?;

    if !resp.status().is_success() {
      let status = resp.status();
      let error_text = resp.text().await.unwrap_or_else(|_| "Unknown error".into());
      anyhow::bail!("API Error {}: {}", status, error_text);
    }

    let state = StreamState {
      stream: Box::pin(resp.bytes_stream()),
      buffer: Vec::new(),
      pending: VecDeque::new(),
      finished: false,
    };

    let s = futures_util::stream::unfold(state, |mut state| async move {
      if state.finished && state.pending.is_empty() {
        return None;
      }

      loop {
        if let Some(item) = state.pending.pop_front() {
          return Some((item, state));
        }

        if state.finished {
          return None;
        }

        match state.stream.next().await {
          Some(Ok(chunk)) => {
            state.buffer.extend_from_slice(&chunk);
            while let Some(pos) = state.buffer.iter().position(|&b| b == b'\n') {
              let line = state.buffer.drain(..=pos).collect::<Vec<u8>>();
              let line_str = String::from_utf8_lossy(&line).trim().to_string();

              if line_str.is_empty() {
                continue;
              }
              if let Some(data) = line_str.strip_prefix("data: ") {
                if data == "[DONE]" {
                  state.finished = true;
                  state.pending.push_back(Ok(StreamItem::Finish(None)));
                  break;
                }
                if let Ok(json) = serde_json::from_str::<Value>(data)
                  && let Some(choice) = json.get("choices").and_then(|c| c.get(0))
                {
                  let fr = choice
                    .get("finish_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                  if let Some(delta) = choice.get("delta") {
                    if let Some(rc) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                      state
                        .pending
                        .push_back(Ok(StreamItem::Reasoning(rc.to_string())));
                    }
                    if let Some(tcs) = delta.get("tool_calls").and_then(|tc_val| {
                      serde_json::from_value::<Vec<ToolCall>>(tc_val.clone()).ok()
                    }) {
                      for tc in tcs {
                        state.pending.push_back(Ok(StreamItem::ToolCall(tc)));
                      }
                    }
                    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                      state
                        .pending
                        .push_back(Ok(StreamItem::Content(content.to_string())));
                    }
                  }

                  if let Some(reason) = fr {
                    state
                      .pending
                      .push_back(Ok(StreamItem::Finish(Some(reason))));
                    state.finished = true;
                    break;
                  }
                }
              }
            }
            if !state.pending.is_empty() {
              continue;
            }
          }
          Some(Err(e)) => {
            state.finished = true;
            return Some((Err(anyhow::anyhow!(e)), state));
          }
          None => {
            state.finished = true;
            if state.pending.is_empty() {
              return None;
            }
          }
        }
      }
    });

    Ok(Box::pin(s))
  }
}
