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
  },
  AssistantMessage {
    content: String,
    reasoning: Option<String>,
    tool_calls: Vec<ToolCall>,
  },
  ToolResult {
    call_id: String,
    content: String,
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
    }
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
      EventPayload::UserMessage { content } => {
        out.push(Message::new_user_text(content.clone()));
      }
      EventPayload::AssistantMessage {
        content,
        reasoning,
        tool_calls,
      } => out.push(Message::Simple {
        role: "assistant".to_string(),
        content: content.clone(),
        reasoning_content: reasoning.clone(),
        tool_calls: if tool_calls.is_empty() {
          None
        } else {
          Some(tool_calls.clone())
        },
      }),
      EventPayload::ToolResult { call_id, content } => out.push(Message::ToolResponse {
        role: "tool".to_string(),
        content: content.clone(),
        tool_call_id: call_id.clone(),
      }),
      EventPayload::SystemPrompt { content, .. } => out.push(Message::Simple {
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
    } => match role.as_str() {
      "user" => Some(EventPayload::UserMessage {
        content: content.clone(),
      }),
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
  use super::*;

  fn session() -> Session {
    let mut s = Session::new("test-session-id".into(), "m".into());
    s.record(EventPayload::SystemPrompt {
      kind: PromptKind::Kernel,
      content: "kernel".into(),
    });
    s.record(EventPayload::UserMessage {
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
