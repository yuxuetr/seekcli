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
use std::time::Duration;

use anyhow::Result;
use futures_util::Stream;
use serde::{Deserialize, Serialize};

pub mod anthropic;
pub mod openai;
pub mod record;
pub mod resilience;
pub mod tokens;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;
pub use resilience::{Resilient, RetryPolicy};

/// A provider failure the retry layer can reason about.
///
/// Providers used to `bail!("API Error {status}: {body}")`, which reads fine
/// in a log but is opaque to a decorator: "should I retry?" depends on the
/// status code, and recovering it from a formatted string is guesswork. The
/// variants below carry exactly what `resilience::is_retryable` needs.
#[derive(Debug)]
pub enum LlmError {
  /// The server answered, but not with 2xx.
  Status {
    provider: &'static str,
    status: u16,
    /// Parsed `Retry-After`, when the server told us how long to wait.
    retry_after: Option<Duration>,
    body: String,
  },
  /// Nothing came back: connect failure, TLS error, timeout.
  Transport {
    provider: &'static str,
    source: String,
  },
}

impl std::fmt::Display for LlmError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Status {
        provider,
        status,
        body,
        ..
      } => write!(f, "{} API error {}: {}", provider, status, body),
      Self::Transport { provider, source } => {
        write!(f, "{} transport error: {}", provider, source)
      }
    }
  }
}

impl std::error::Error for LlmError {}

/// Build the configured endpoint, wrapped in the retry/timeout decorator.
///
/// Lives here rather than in `App::new` so the wire-format branch, the key
/// lookup and the resilience wrapping stay in one place — adding a third wire
/// should touch this function and nothing else.
pub fn build_provider(config: &crate::config::Config) -> Result<Box<dyn LlmProvider>> {
  // Checked before credentials: a replayed run must work with no API key at
  // all, or the tests it enables cannot run in CI. Retry/timeout are skipped
  // too — there is no network to be resilient about, and a wrapped replay
  // would only add latency to the test suite.
  if let Some(replay) = record::replay_from_env() {
    return Ok(replay);
  }

  let endpoint = config.resolve_provider()?;
  let key = endpoint.resolve_key()?;
  let inner: Box<dyn LlmProvider> = match endpoint.wire.as_str() {
    "anthropic" => Box::new(AnthropicProvider::new(key, endpoint.base_url.clone())),
    _ => Box::new(OpenAiProvider::new(key, endpoint.base_url.clone())),
  };
  let r = &config.resilience;
  let policy = RetryPolicy {
    max_attempts: r.max_attempts.max(1),
    base_delay: Duration::from_millis(r.base_delay_ms),
    max_delay: Duration::from_secs(r.max_delay_secs),
    request_timeout: Duration::from_secs(r.request_timeout_secs),
    stream_idle_timeout: Duration::from_secs(r.stream_idle_timeout_secs),
  };
  // Recording sits inside the retry layer: what gets written is the stream
  // that actually reached the loop, not the attempts that failed on the way.
  Ok(Box::new(Resilient::new(record::wrap(inner), policy)))
}

/// Read `Retry-After` (delta-seconds form only; the HTTP-date form is rare in
/// practice and parsing it would pull in a date parser for little gain).
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
  headers
    .get(reqwest::header::RETRY_AFTER)?
    .to_str()
    .ok()?
    .trim()
    .parse::<u64>()
    .ok()
    .map(Duration::from_secs)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Message {
  /// MUST stay first. `untagged` tries variants in declaration order, and
  /// `Simple` matches any `{role, content, ...}` object because serde ignores
  /// unknown fields — so with `Simple` first, a tool message deserialised into
  /// `Simple` and its `tool_call_id` was silently dropped. Reloading such a
  /// session produced a `tool` message with no call id, which the provider
  /// then rejects or mispairs. `ToolResponse` requires `tool_call_id`, so
  /// trying it first is unambiguous.
  ToolResponse {
    role: String,
    content: String,
    tool_call_id: String,
  },
  Simple {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    /// Images travelling with this message.
    ///
    /// `skip` rather than a serialized field: `Message` is **not** the wire
    /// type — `anthropic.rs::build_body` has always transformed it — and the
    /// multipart shape is assembled at the wire boundary by
    /// [`to_openai_wire`]. Keeping `content: String` is what lets this land
    /// without touching the 75 sites that construct a `Message`
    /// (`docs/architecture/L4-memory.md` §4.6.1).
    #[serde(skip)]
    images: Vec<ImagePart>,
  },
}

/// One image, loaded and ready to send.
///
/// The transient half of the pair: the durable half is `session::ImageRef`,
/// which stores a blob path instead of bytes so one screenshot does not add
/// hundreds of KB to `events.jsonl` (`docs/architecture/L4-memory.md` §4.6.2).
#[derive(Debug, Clone, PartialEq)]
pub struct ImagePart {
  /// e.g. `image/png`.
  pub media_type: String,
  pub data_base64: String,
}

impl ImagePart {
  /// The `data:` URI the OpenAI-shaped wire expects.
  fn data_uri(&self) -> String {
    format!("data:{};base64,{}", self.media_type, self.data_base64)
  }
}

impl Message {
  /// A user message carrying images alongside its text.
  pub fn new_user_with_images(text: String, images: Vec<ImagePart>) -> Self {
    Message::Simple {
      role: "user".to_string(),
      content: text,
      reasoning_content: None,
      tool_calls: None,
      images,
    }
  }

  /// Images attached to this message, if any.
  pub fn images(&self) -> &[ImagePart] {
    match self {
      Message::Simple { images, .. } => images,
      Message::ToolResponse { .. } => &[],
    }
  }
}

/// Serialize messages for the OpenAI-shaped wire, expanding any images into
/// multipart content.
///
/// A message with no images serializes **exactly as before** — a plain string
/// `content` — which is what keeps every recorded fixture valid.
pub fn to_openai_wire(messages: &[Message]) -> Result<Vec<serde_json::Value>> {
  messages
    .iter()
    .map(|m| {
      let mut value = serde_json::to_value(m)?;
      let images = m.images();
      if images.is_empty() {
        return Ok(value);
      }
      let text = value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
      let mut parts: Vec<serde_json::Value> = Vec::new();
      // An empty text part is not just noise: some providers reject it.
      if !text.is_empty() {
        parts.push(serde_json::json!({ "type": "text", "text": text }));
      }
      for image in images {
        parts.push(serde_json::json!({
          "type": "image_url",
          "image_url": { "url": image.data_uri() }
        }));
      }
      if let Some(object) = value.as_object_mut() {
        object.insert("content".to_string(), serde_json::Value::Array(parts));
      }
      Ok(value)
    })
    .collect()
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamItem {
  Reasoning(String),
  Content(String),
  ToolCall(ToolCall),
  Finish(Option<String>),
  Usage(UsageInfo),
}

/// Token-level accounting reported at the end of a streamed response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageInfo {
  pub prompt_tokens: u64,
  pub completion_tokens: u64,
  pub prompt_cache_hit_tokens: u64,
  pub prompt_cache_miss_tokens: u64,
}

impl Message {
  pub fn new_user_text(text: String) -> Self {
    Self::Simple {
      images: Vec::new(),
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
    .map(|m| {
      // Clear the one field rather than rebuilding the variant: a rebuild drops
      // every field it does not name, so adding one (images) would have silently
      // lost it here. Mutating a clone cannot go stale that way.
      let mut out = m.clone();
      if let Message::Simple {
        reasoning_content, ..
      } = &mut out
      {
        *reasoning_content = None;
      }
      out
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

  /// The property every recorded fixture depends on: a message without images
  /// must serialize byte-identically to before multipart existed.
  #[test]
  fn a_text_only_message_keeps_a_plain_string_content() {
    let msgs = vec![Message::new_user_text("hello".into())];
    let wire = match to_openai_wire(&msgs) {
      Ok(w) => w,
      Err(e) => panic!("{e}"),
    };
    assert_eq!(wire[0]["content"], serde_json::json!("hello"));
    // And identical to plain serde, so no fixture can drift.
    let plain = match serde_json::to_value(&msgs[0]) {
      Ok(v) => v,
      Err(e) => panic!("{e}"),
    };
    assert_eq!(wire[0], plain);
  }

  #[test]
  fn images_become_multipart_content_at_the_wire_boundary() {
    let img = ImagePart {
      media_type: "image/png".into(),
      data_base64: "QUJD".into(),
    };
    let msgs = vec![Message::new_user_with_images(
      "what is this?".into(),
      vec![img],
    )];
    let wire = match to_openai_wire(&msgs) {
      Ok(w) => w,
      Err(e) => panic!("{e}"),
    };
    let parts = match wire[0]["content"].as_array() {
      Some(p) => p,
      None => panic!("content must be an array: {}", wire[0]),
    };
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["type"], "text");
    assert_eq!(parts[1]["type"], "image_url");
    assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,QUJD");
  }

  /// Some providers reject an empty text part, so it must not be emitted --
  /// "describe this" with no words is a legitimate request.
  #[test]
  fn an_image_with_no_text_emits_no_empty_text_part() {
    let msgs = vec![Message::new_user_with_images(
      String::new(),
      vec![ImagePart {
        media_type: "image/png".into(),
        data_base64: "QQ==".into(),
      }],
    )];
    let wire = match to_openai_wire(&msgs) {
      Ok(w) => w,
      Err(e) => panic!("{e}"),
    };
    let parts = match wire[0]["content"].as_array() {
      Some(p) => p,
      None => panic!("content must be an array"),
    };
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0]["type"], "image_url");
  }

  /// `images` is `#[serde(skip)]`, so it must never reach the wire as a field.
  #[test]
  fn the_images_field_itself_is_never_serialized() {
    let msgs = vec![Message::new_user_with_images(
      "x".into(),
      vec![ImagePart {
        media_type: "image/png".into(),
        data_base64: "QQ==".into(),
      }],
    )];
    let wire = match to_openai_wire(&msgs) {
      Ok(w) => w,
      Err(e) => panic!("{e}"),
    };
    assert!(wire[0].get("images").is_none(), "{}", wire[0]);
  }

  /// Rebuilding the variant used to drop every field it did not name; adding
  /// `images` would have been lost here silently.
  #[test]
  fn stripping_reasoning_preserves_images() {
    let msgs = vec![Message::Simple {
      role: "user".into(),
      content: "x".into(),
      reasoning_content: Some("thinking".into()),
      tool_calls: None,
      images: vec![ImagePart {
        media_type: "image/png".into(),
        data_base64: "QQ==".into(),
      }],
    }];
    let out = strip_reasoning(&msgs);
    assert_eq!(out[0].images().len(), 1, "images were dropped");
    match &out[0] {
      Message::Simple {
        reasoning_content, ..
      } => assert!(reasoning_content.is_none()),
      other => panic!("unexpected variant: {other:?}"),
    }
  }

  use super::*;

  /// Regression guard for a latent bug that survived until the stage 26
  /// migration surfaced it: with `Simple` declared first, `untagged`
  /// deserialisation matched it for tool messages and dropped `tool_call_id`,
  /// so every reloaded session lost the link between a tool call and its
  /// result.
  #[test]
  fn a_tool_message_round_trips_with_its_call_id() {
    let original = Message::ToolResponse {
      role: "tool".into(),
      content: "result".into(),
      tool_call_id: "call_123".into(),
    };
    let text = match serde_json::to_string(&original) {
      Ok(t) => t,
      Err(e) => panic!("serialise failed: {}", e),
    };
    let back: Message = match serde_json::from_str(&text) {
      Ok(m) => m,
      Err(e) => panic!("deserialise failed: {}", e),
    };
    match back {
      Message::ToolResponse { tool_call_id, .. } => assert_eq!(tool_call_id, "call_123"),
      Message::Simple { .. } => panic!("tool message decoded as Simple; call id lost"),
    }
  }

  #[test]
  fn an_assistant_message_still_decodes_as_simple() {
    let text = r#"{"role":"assistant","content":"hi"}"#;
    let m: Message = match serde_json::from_str(text) {
      Ok(m) => m,
      Err(e) => panic!("deserialise failed: {}", e),
    };
    assert!(matches!(m, Message::Simple { .. }));
  }

  #[test]
  fn strip_reasoning_drops_reasoning_keeps_rest() {
    let msgs = vec![
      Message::Simple {
        images: Vec::new(),
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
