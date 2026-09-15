//! Session state as an append-only event log.
//!
//! Sessions used to be a snapshot: `Session { messages: Vec<Message> }`
//! serialised to one JSON file. That shape cannot answer questions the product
//! needs to answer. Once compaction summarised the middle of a conversation,
//! the originals were **gone**; there was no notion of "step 7", so there was
//! nothing to fork from or resume at; and a run could not be replayed to debug
//! the harness itself.
//!
//! The invariant this module establishes is dsh's: **model-visible means
//! logged**. Everything that reaches a model request must be reconstructable
//! from `events.jsonl`, and `derive_messages` is the only way the working set
//! is produced. Fork, resume, search and titles then fall out of the log
//! rather than each needing their own mechanism.
//!
//! Compaction is recorded as an *event*, not as a destructive rewrite: the
//! projection skips the replaced range and splices in the summary, while the
//! original events stay on disk. That is what makes a compacted session still
//! auditable — and it is the piece stage 10.3 deferred.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::api::{Message, ToolCall, UsageInfo};
use crate::observability::cost::CostTracker;

/// Which prompt a `SystemPrompt` event carries. Kept distinct so the
/// projection can rebuild the exact ordering the loop relies on for prompt
/// caching: the static kernel first, workspace rules second, skill last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptKind {
  Kernel,
  Workspace,
  Skill,
  /// A compaction summary spliced in by the projection.
  Summary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPayload {
  UserMessage {
    content: String,
    /// Images the user attached, stored as **blob references, not bytes**.
    ///
    /// `#[serde(default)]` is what lets every pre-stage-41 log deserialize
    /// unchanged — no version bump, no migration chain. And keeping bytes out
    /// means one screenshot does not add hundreds of KB to `events.jsonl`
    /// (`docs/architecture/L4-memory.md` §4.6.2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<ImageRef>,
  },
  AssistantMessage {
    content: String,
    reasoning: Option<String>,
    tool_calls: Vec<ToolCall>,
  },
  ToolResult {
    call_id: String,
    content: String,
    /// Images the tool returned, as blob references. Same rule as
    /// `UserMessage::images`: `#[serde(default)]` keeps older logs readable and
    /// bytes stay out of the log.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<ImageRef>,
  },
  SystemPrompt {
    kind: PromptKind,
    content: String,
  },
  /// Replaces `[from, to)` in the projection with `summary`. The replaced
  /// events remain in the file.
  Compaction {
    from: u64,
    to: u64,
    summary: String,
  },
  SkillActivated {
    name: String,
  },
  Usage(UsageInfo),
  Interrupted,
}

/// A stored reference to an image blob.
///
/// The durable half of the pair whose transient half is `api::ImagePart`.
/// `media_type` travels with the path because the blob filename need not carry
/// a usable extension, and guessing it at replay time would be a second source
/// of truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageRef {
  pub path: String,
  pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
  pub seq: u64,
  pub ts: DateTime<Utc>,
  pub payload: EventPayload,
}

/// Everything `/history` needs, so listing sessions never parses the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
  pub id: String,
  pub title: String,
  pub model: String,
  pub created: DateTime<Utc>,
  pub updated: DateTime<Utc>,
  pub event_count: usize,
  #[serde(default)]
  pub cost: CostTracker,
  /// Set when this session was forked, so lineage is visible without
  /// diffing logs.
  #[serde(default)]
  pub forked_from: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Session {
  pub meta: SessionMeta,
  pub events: Vec<SessionEvent>,
  /// How many leading events are already on disk.
  ///
  /// The log calls itself append-only, but `save_session` used to rewrite the
  /// whole file every time, which made a claimed O(1) append O(n) and meant a
  /// crash mid-write could take the entire session with it. This watermark is
  /// what lets the writer send only what is new. It is derived, never
  /// serialized: on load it equals `events.len()`, because everything read
  /// back was by definition already written.
  persisted: usize,
}

pub const UNTITLED: &str = "New Chat";

impl Session {
  pub fn new(id: String, model: String) -> Self {
    let now = Utc::now();
    Self {
      meta: SessionMeta {
        id,
        title: UNTITLED.to_string(),
        model,
        created: now,
        updated: now,
        event_count: 0,
        cost: CostTracker::new(),
        forked_from: None,
      },
      events: Vec::new(),
      persisted: 0,
    }
  }

  /// Rebuild a session that was read back from disk: every event present is,
  /// by definition, already persisted.
  pub fn restored(meta: SessionMeta, events: Vec<SessionEvent>) -> Self {
    let persisted = events.len();
    Self {
      meta,
      events,
      persisted,
    }
  }

  /// Events not yet on disk.
  pub fn unpersisted(&self) -> &[SessionEvent] {
    self.events.get(self.persisted..).unwrap_or(&[])
  }

  /// Called by the writer once `unpersisted()` has been appended.
  pub fn mark_persisted(&mut self) {
    self.persisted = self.events.len();
  }

  pub fn id(&self) -> &str {
    &self.meta.id
  }

  /// Append one event, assigning the next sequence number.
  pub fn record(&mut self, payload: EventPayload) {
    let seq = self.events.len() as u64;
    self.events.push(SessionEvent {
      seq,
      ts: Utc::now(),
      payload,
    });
    self.meta.event_count = self.events.len();
    self.meta.updated = Utc::now();
  }

  pub fn extend(&mut self, payloads: impl IntoIterator<Item = EventPayload>) {
    for p in payloads {
      self.record(p);
    }
  }

  /// The model-visible conversation. The single source of the working set.
  pub fn messages(&self) -> Vec<Message> {
    derive_messages(&self.events)
  }

  /// Truncate to the first `count` events, producing an independent session.
  pub fn fork(&self, new_id: String, count: usize) -> Self {
    let count = count.min(self.events.len());
    let now = Utc::now();
    Self {
      meta: SessionMeta {
        id: new_id,
        title: format!("fork of {}", short_id(&self.meta.id)),
        model: self.meta.model.clone(),
        created: now,
        updated: now,
        event_count: count,
        // Cost belongs to the run that spent it, not to the copy.
        cost: CostTracker::new(),
        forked_from: Some(self.meta.id.clone()),
      },
      events: self.events[..count].to_vec(),
      // A fork is a new file with nothing in it yet, whatever the parent had
      // already written.
      persisted: 0,
    }
  }
}

pub fn short_id(id: &str) -> &str {
  id.get(..8).unwrap_or(id)
}

/// Project events into the message list a request is built from.
///
/// A `Compaction` event replaces the range it names: the projection skips
/// those events and splices in the summary as a system message. The skipped
/// events remain in `events.jsonl`, which is the whole point — a compacted
/// session is still fully auditable and replayable.
/// Rebuild a user message, loading any referenced image blobs.
///
/// A blob that is gone — the 30-day sweep reclaims them — leaves a visible line
/// in the text rather than disappearing. Per [design-principles §4] degradation
/// beats interruption, but never silently: the model must not be told the image
/// was there when it was not, and `/resume` must not fail over an expired
/// screenshot (`docs/architecture/L4-memory.md` §4.6.2).
fn rebuild_user_message(content: &str, images: &[ImageRef]) -> Message {
  if images.is_empty() {
    return Message::new_user_text(content.to_string());
  }
  let (text, loaded) = load_images(content, images);
  Message::new_user_with_images(text, loaded)
}

/// Load referenced blobs, degrading visibly for any that are gone.
///
/// Shared by the user and tool arms so the degradation rule has exactly one
/// implementation — two copies would drift, and the one that drifted would be
/// the one that silently lied about an image being present.
fn load_images(content: &str, images: &[ImageRef]) -> (String, Vec<crate::api::ImagePart>) {
  let mut text = content.to_string();
  let mut loaded = Vec::new();
  for image in images {
    match std::fs::read(&image.path) {
      Ok(bytes) => loaded.push(crate::api::ImagePart {
        media_type: image.media_type.clone(),
        data_base64: base64_encode(&bytes),
      }),
      Err(_) => {
        if !text.is_empty() {
          text.push('\n');
        }
        text.push_str(&format!(
          "[image no longer available: {} — offload blobs are swept after 30 days]",
          image.path
        ));
      }
    }
  }
  (text, loaded)
}

/// Standard base64, written out rather than pulled in as a dependency.
///
/// One short encoder on a path that already reads a file does not justify a
/// crate, and `cargo deny` has one fewer thing to have an opinion about — the
/// same call as the hand-written edit distance in `tools/registry.rs`.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
  const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let b0 = chunk[0] as u32;
    let b1 = *chunk.get(1).unwrap_or(&0) as u32;
    let b2 = *chunk.get(2).unwrap_or(&0) as u32;
    let triple = (b0 << 16) | (b1 << 8) | b2;
    out.push(TABLE[(triple >> 18) as usize & 63] as char);
    out.push(TABLE[(triple >> 12) as usize & 63] as char);
    out.push(if chunk.len() > 1 {
      TABLE[(triple >> 6) as usize & 63] as char
    } else {
      '='
    });
    out.push(if chunk.len() > 2 {
      TABLE[triple as usize & 63] as char
    } else {
      '='
    });
  }
  out
}

pub fn derive_messages(events: &[SessionEvent]) -> Vec<Message> {
  // Later compactions win, so build the skip set first.
  let mut skip_until: Vec<(u64, u64, &str)> = Vec::new();
  for e in events {
    if let EventPayload::Compaction { from, to, summary } = &e.payload {
      skip_until.push((*from, *to, summary.as_str()));
    }
  }

  let mut out = Vec::new();
  let mut index = 0usize;
  while index < events.len() {
    let event = &events[index];
    if let Some((_, to, summary)) = skip_until
      .iter()
      .find(|(from, to, _)| event.seq >= *from && event.seq < *to)
    {
      out.push(Message::Simple {
        images: Vec::new(),
        role: "system".to_string(),
        content: format!("[Compressed earlier turns]\n\n{}", summary),
        reasoning_content: None,
        tool_calls: None,
      });
      // Jump past the replaced range in one step.
      while index < events.len() && events[index].seq < *to {
        index += 1;
      }
      continue;
    }

    match &event.payload {
      EventPayload::UserMessage { content, images } => {
        out.push(rebuild_user_message(content, images));
      }
      EventPayload::AssistantMessage {
        content,
        reasoning,
        tool_calls,
      } => out.push(Message::Simple {
        images: Vec::new(),
        role: "assistant".to_string(),
        content: content.clone(),
        reasoning_content: reasoning.clone(),
        tool_calls: if tool_calls.is_empty() {
          None
        } else {
          Some(tool_calls.clone())
        },
      }),
      EventPayload::ToolResult {
        call_id,
        content,
        images,
      } => {
        let (text, loaded) = load_images(content, images);
        out.push(Message::new_tool_response(call_id.clone(), text, loaded));
      }
      EventPayload::SystemPrompt { content, .. } => out.push(Message::Simple {
        images: Vec::new(),
        role: "system".to_string(),
        content: content.clone(),
        reasoning_content: None,
        tool_calls: None,
      }),
      EventPayload::SkillActivated { .. }
      | EventPayload::Usage(_)
      | EventPayload::Interrupted
      | EventPayload::Compaction { .. } => {}
    }
    index += 1;
  }
  out
}

/// Like `derive_messages`, but also returns which event produced each
/// message.
///
/// Needed to express compaction as an event: the compressor decides in
/// *message* space ("summarize everything before the last 8 messages"), while
/// a `Compaction` event names a *sequence* range. Without this mapping the
/// only expressible compaction would be "replace everything", which would
/// discard the recent tail the compressor is careful to keep.
pub fn derive_messages_indexed(events: &[SessionEvent]) -> (Vec<Message>, Vec<u64>) {
  let messages = derive_messages(events);
  let mut seqs = Vec::with_capacity(messages.len());
  let mut skips: Vec<(u64, u64)> = Vec::new();
  for e in events {
    if let EventPayload::Compaction { from, to, .. } = &e.payload {
      skips.push((*from, *to));
    }
  }
  let mut index = 0usize;
  while index < events.len() {
    let event = &events[index];
    if let Some((_, to)) = skips
      .iter()
      .find(|(from, to)| event.seq >= *from && event.seq < *to)
    {
      // The spliced summary is attributed to the start of the replaced range.
      seqs.push(event.seq);
      while index < events.len() && events[index].seq < *to {
        index += 1;
      }
      continue;
    }
    if produces_message(&event.payload) {
      seqs.push(event.seq);
    }
    index += 1;
  }
  debug_assert_eq!(
    messages.len(),
    seqs.len(),
    "projection and index map diverged"
  );
  (messages, seqs)
}

fn produces_message(payload: &EventPayload) -> bool {
  matches!(
    payload,
    EventPayload::UserMessage { .. }
      | EventPayload::AssistantMessage { .. }
      | EventPayload::ToolResult { .. }
      | EventPayload::SystemPrompt { .. }
  )
}

/// Turn a working-set message back into the event that would produce it.
///
/// Used where the loop still hands back messages rather than events (the
/// planning-pass bridge, reminder injections). Keeping the conversion in one
/// place is what stops the log and the working set drifting apart.
pub fn event_for(message: &Message) -> Option<EventPayload> {
  match message {
    Message::Simple {
      role,
      content,
      reasoning_content,
      tool_calls,
      images,
    } => match role.as_str() {
      // `images` is deliberately not carried here: this direction converts a
      // transient `Message` whose bytes have no blob on disk yet. The one path
      // that attaches an image (`/paste`) writes the blob first and records the
      // event with its `ImageRef` directly, so nothing reaches the log by this
      // route (`docs/architecture/L4-memory.md` §4.6.2).
      "user" if images.is_empty() => Some(EventPayload::UserMessage {
        content: content.clone(),
        images: Vec::new(),
      }),
      // Loud rather than lossy: silently dropping would break
      // "model-visible means logged".
      "user" => {
        eprintln!(
          "[Session] {} image(s) on a converted user message cannot be logged; \
           attach images through the path that writes blobs first",
          images.len()
        );
        Some(EventPayload::UserMessage {
          content: content.clone(),
          images: Vec::new(),
        })
      }
      "assistant" => Some(EventPayload::AssistantMessage {
        content: content.clone(),
        reasoning: reasoning_content.clone(),
        tool_calls: tool_calls.clone().unwrap_or_default(),
      }),
      "system" => Some(EventPayload::SystemPrompt {
        kind: PromptKind::Kernel,
        content: content.clone(),
      }),
      _ => None,
    },
    Message::ToolResponse {
      content,
      tool_call_id,
      ..
    } => Some(EventPayload::ToolResult {
      call_id: tool_call_id.clone(),
      content: content.clone(),
      // Same reason as the user arm: this direction converts a transient
      // message whose bytes have no blob yet. The dispatcher persists tool
      // images and records the event directly.
      images: Vec::new(),
    }),
  }
}

/// Serialise the log, one event per line.
pub fn to_jsonl(events: &[SessionEvent]) -> Result<String> {
  let mut out = String::new();
  for e in events {
    out.push_str(&serde_json::to_string(e)?);
    out.push('\n');
  }
  Ok(out)
}

/// Parse a log, skipping unreadable lines rather than losing the session.
///
/// A single corrupt line — a half-written last record after a crash — must not
/// make the whole history unreadable. The loss is announced, not swallowed.
pub fn from_jsonl(text: &str) -> Vec<SessionEvent> {
  let mut out = Vec::new();
  for (n, line) in text.lines().enumerate() {
    if line.trim().is_empty() {
      continue;
    }
    match serde_json::from_str::<SessionEvent>(line) {
      Ok(e) => out.push(e),
      Err(e) => eprintln!(
        "[Session] skipping unreadable event at line {}: {}",
        n + 1,
        e
      ),
    }
  }
  out
}

#[cfg(test)]
mod tests {

  /// Against the RFC 4648 vectors, including both padding lengths. A
  /// hand-written encoder with no test is how a subtly wrong data URI ships.
  #[test]
  fn base64_matches_the_standard_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    // Bytes outside ASCII must survive: a PNG is mostly these.
    assert_eq!(base64_encode(&[0xff, 0x00, 0x80]), "/wCA");
  }

  #[test]
  fn a_logged_image_is_reloaded_into_the_projection() {
    let dir = std::env::temp_dir().join(format!("seekcli_img_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    let blob = dir.join("shot.png");
    let _ = std::fs::write(&blob, b"foobar");

    let msg = rebuild_user_message(
      "what is this?",
      &[ImageRef {
        path: blob.display().to_string(),
        media_type: "image/png".into(),
      }],
    );
    assert_eq!(msg.images().len(), 1);
    assert_eq!(msg.images()[0].data_base64, "Zm9vYmFy");
    std::fs::remove_dir_all(&dir).ok();
  }

  /// Blobs are swept after 30 days. Resuming such a session must neither fail
  /// nor quietly pretend the model saw the image.
  #[test]
  fn an_expired_blob_degrades_visibly_instead_of_vanishing() {
    let msg = rebuild_user_message(
      "describe it",
      &[ImageRef {
        path: "/nonexistent/gone.png".into(),
        media_type: "image/png".into(),
      }],
    );
    assert!(msg.images().is_empty(), "no bytes should be invented");
    let text = match &msg {
      Message::Simple { content, .. } => content.clone(),
      other => panic!("unexpected variant: {other:?}"),
    };
    assert!(text.contains("describe it"), "{text}");
    assert!(text.contains("no longer available"), "{text}");
    assert!(text.contains("gone.png"), "must name which one: {text}");
  }

  /// The whole reason `images` is `#[serde(default)]`: a log written before
  /// stage 41 must still load, with no version bump and no migration.
  #[test]
  fn a_pre_stage_41_log_line_still_deserializes() {
    let line =
      r#"{"seq":1,"ts":"2026-01-01T00:00:00Z","payload":{"UserMessage":{"content":"hi"}}}"#;
    let events = from_jsonl(line);
    assert_eq!(events.len(), 1, "the old shape must still parse");
    match &events[0].payload {
      EventPayload::UserMessage { content, images } => {
        assert_eq!(content, "hi");
        assert!(images.is_empty());
      }
      other => panic!("unexpected payload: {other:?}"),
    }
  }

  /// And a text-only event must serialize back to the old shape, so a log
  /// written now stays readable by anything that predates this field.
  #[test]
  fn a_text_only_event_does_not_gain_an_images_key() {
    let events = vec![SessionEvent {
      seq: 1,
      ts: chrono::Utc::now(),
      payload: EventPayload::UserMessage {
        content: "hi".into(),
        images: Vec::new(),
      },
    }];
    let text = match to_jsonl(&events) {
      Ok(t) => t,
      Err(e) => panic!("{e}"),
    };
    assert!(!text.contains("images"), "{text}");
  }

  use super::*;

  fn session() -> Session {
    let mut s = Session::new("test-session-id".into(), "m".into());
    s.record(EventPayload::SystemPrompt {
      kind: PromptKind::Kernel,
      content: "kernel".into(),
    });
    s.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "hi".into(),
    });
    s.record(EventPayload::AssistantMessage {
      content: String::new(),
      reasoning: Some("thinking".into()),
      tool_calls: vec![ToolCall {
        id: "c1".into(),
        tool_type: "function".into(),
        function: crate::api::FunctionCall {
          name: "read_file".into(),
          arguments: "{}".into(),
        },
      }],
    });
    s.record(EventPayload::ToolResult {
      images: Vec::new(),
      call_id: "c1".into(),
      content: "contents".into(),
    });
    s.record(EventPayload::AssistantMessage {
      content: "done".into(),
      reasoning: None,
      tool_calls: vec![],
    });
    s
  }

  #[test]
  fn projection_rebuilds_the_conversation_in_order() {
    let msgs = session().messages();
    assert_eq!(msgs.len(), 5);
    let roles: Vec<String> = msgs
      .iter()
      .map(|m| match m {
        Message::Simple { role, .. } => role.clone(),
        Message::ToolResponse { role, .. } => role.clone(),
      })
      .collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant"]);
  }

  #[test]
  fn bookkeeping_events_are_not_model_visible() {
    let mut s = session();
    s.record(EventPayload::Usage(UsageInfo::default()));
    s.record(EventPayload::SkillActivated { name: "x".into() });
    s.record(EventPayload::Interrupted);
    // Three more events, zero more messages.
    assert_eq!(s.events.len(), 8);
    assert_eq!(s.messages().len(), 5);
  }

  #[test]
  fn compaction_replaces_the_range_in_the_projection_but_not_on_disk() {
    let mut s = session();
    s.record(EventPayload::Compaction {
      from: 1,
      to: 4,
      summary: "user asked, tool answered".into(),
    });

    let msgs = s.messages();
    // kernel, summary, final assistant.
    assert_eq!(msgs.len(), 3);
    match &msgs[1] {
      Message::Simple { content, role, .. } => {
        assert_eq!(role, "system");
        assert!(content.contains("user asked"), "got: {}", content);
      }
      _ => panic!("expected the summary as a system message"),
    }

    // The originals are still there — this is what stage 10.3 deferred and
    // what makes a compacted session auditable.
    assert_eq!(s.events.len(), 6);
    assert!(s.events.iter().any(|e| matches!(
      &e.payload,
      EventPayload::ToolResult { content, .. } if content == "contents"
    )));
  }

  #[test]
  fn fork_truncates_and_records_lineage_without_inheriting_cost() {
    let mut s = session();
    s.meta.cost.record(&UsageInfo {
      prompt_tokens: 100,
      completion_tokens: 10,
      ..Default::default()
    });
    let f = s.fork("child".into(), 2);
    assert_eq!(f.events.len(), 2);
    assert_eq!(f.meta.forked_from.as_deref(), Some("test-session-id"));
    assert!(
      f.meta.cost.is_empty(),
      "cost belongs to the run that spent it"
    );
    // The parent is untouched.
    assert_eq!(s.events.len(), 5);
  }

  #[test]
  fn fork_past_the_end_is_clamped_rather_than_panicking() {
    let f = session().fork("child".into(), 999);
    assert_eq!(f.events.len(), 5);
  }

  #[test]
  fn the_index_map_lines_up_with_the_projection() {
    let s = session();
    let (msgs, seqs) = derive_messages_indexed(&s.events);
    assert_eq!(msgs.len(), seqs.len());
    // Every message is attributed to the event that produced it, in order.
    assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
  }

  #[test]
  fn the_index_map_survives_a_compacted_range() {
    let mut s = session();
    s.record(EventPayload::Compaction {
      from: 1,
      to: 4,
      summary: "summary".into(),
    });
    let (msgs, seqs) = derive_messages_indexed(&s.events);
    assert_eq!(msgs.len(), seqs.len());
    // kernel(0), summary attributed to the range start(1), final assistant(4).
    assert_eq!(seqs, vec![0, 1, 4]);
  }

  #[test]
  fn jsonl_round_trips() {
    let s = session();
    let text = match to_jsonl(&s.events) {
      Ok(t) => t,
      Err(e) => panic!("serialise failed: {}", e),
    };
    let back = from_jsonl(&text);
    assert_eq!(back.len(), s.events.len());
    assert_eq!(derive_messages(&back).len(), s.messages().len());
  }

  #[test]
  fn one_corrupt_line_does_not_lose_the_session() {
    let s = session();
    let text = match to_jsonl(&s.events) {
      Ok(t) => t,
      Err(e) => panic!("serialise failed: {}", e),
    };
    let mut lines: Vec<&str> = text.lines().collect();
    lines.insert(2, "{ this is not json");
    let back = from_jsonl(&lines.join("\n"));
    assert_eq!(
      back.len(),
      s.events.len(),
      "only the bad line should be lost"
    );
  }
}
