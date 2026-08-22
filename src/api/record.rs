//! Record and replay LLM traffic, so the agent loop itself becomes testable.
//!
//! Before this, all 87 unit tests were pure logic: `run_agent_loop` had ~14%
//! line coverage and `tools/shell.rs`, `tools/fs.rs`, `api/openai.rs` and
//! `commands.rs` had none at all. Every change to the loop was verified by
//! hand, which is why the stage 19 fake-tool-call bug survived as long as it
//! did — nothing could have caught its return.
//!
//! Two decorators over `LlmProvider`, same shape as `CostTracker` and
//! `Resilient`:
//!
//! * `SEEKCLI_RECORD=<dir>` writes each response stream to `<seq>.jsonl`
//!   alongside the request shape in `<seq>.request.json`.
//! * `SEEKCLI_REPLAY=<dir>` serves those back with no network at all, so a
//!   test can drive a complete multi-step trajectory deterministically and
//!   without an API key.
//!
//! Replay **verifies the request shape** before answering (message count, last
//! message role, tool set). A recording replayed against a request it was not
//! made for would silently drift out of alignment and produce a green test
//! that proves nothing; failing loudly is the entire point.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use super::{LlmProvider, Message, StreamItem, StreamResult, Tool};

pub const RECORD_ENV: &str = "SEEKCLI_RECORD";
pub const REPLAY_ENV: &str = "SEEKCLI_REPLAY";

/// The parts of a request a replay must agree on.
///
/// Deliberately structural rather than byte-exact: a fixture should survive
/// rewording a prompt, but must not survive the loop asking a structurally
/// different question. `digest` is recorded for diagnostics only — it tells a
/// human *what* changed without making every prompt edit break the suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestShape {
  pub model: String,
  pub message_count: usize,
  pub last_role: String,
  pub thinking_mode: String,
  pub tool_names: Vec<String>,
  pub digest: String,
}

impl RequestShape {
  fn capture(
    model: &str,
    messages: &[Message],
    thinking_mode: &str,
    tools: Option<&[Tool]>,
  ) -> Self {
    let last_role = messages
      .last()
      .map(role_of)
      .unwrap_or_else(|| "none".to_string());
    let mut tool_names: Vec<String> = tools
      .map(|t| t.iter().map(|t| t.function.name.clone()).collect())
      .unwrap_or_default();
    tool_names.sort();
    let serialized = serde_json::to_string(messages).unwrap_or_default();
    Self {
      model: model.to_string(),
      message_count: messages.len(),
      last_role,
      thinking_mode: thinking_mode.to_string(),
      tool_names,
      digest: fnv1a(&serialized),
    }
  }

  /// Structural mismatch only. Returns the first difference found so the
  /// failure message names it instead of dumping two blobs.
  fn mismatch(&self, other: &Self) -> Option<String> {
    if self.message_count != other.message_count {
      return Some(format!(
        "message count {} != recorded {}",
        other.message_count, self.message_count
      ));
    }
    if self.last_role != other.last_role {
      return Some(format!(
        "last message role `{}` != recorded `{}`",
        other.last_role, self.last_role
      ));
    }
    if self.tool_names != other.tool_names {
      return Some(format!(
        "tool set {:?} != recorded {:?}",
        other.tool_names, self.tool_names
      ));
    }
    None
  }
}

fn role_of(m: &Message) -> String {
  match m {
    Message::Simple { role, .. } => role.clone(),
    Message::ToolResponse { role, .. } => role.clone(),
  }
}

fn fnv1a(text: &str) -> String {
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for byte in text.as_bytes() {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
  }
  format!("{:016x}", hash)
}

fn stream_file(dir: &Path, seq: usize) -> PathBuf {
  dir.join(format!("{:03}.jsonl", seq))
}

fn request_file(dir: &Path, seq: usize) -> PathBuf {
  dir.join(format!("{:03}.request.json", seq))
}

// ---------------------------------------------------------------- recording

pub struct Recording {
  inner: Box<dyn LlmProvider>,
  dir: PathBuf,
  seq: AtomicUsize,
}

impl Recording {
  pub fn new(inner: Box<dyn LlmProvider>, dir: PathBuf) -> Self {
    Self {
      inner,
      dir,
      seq: AtomicUsize::new(0),
    }
  }
}

#[async_trait::async_trait]
impl LlmProvider for Recording {
  async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<StreamResult> {
    let seq = self.seq.fetch_add(1, Ordering::SeqCst);
    std::fs::create_dir_all(&self.dir)
      .with_context(|| format!("cannot create {}", self.dir.display()))?;

    let shape = RequestShape::capture(model, &messages, thinking_mode, tools.as_deref());
    std::fs::write(
      request_file(&self.dir, seq),
      serde_json::to_string_pretty(&shape)?,
    )?;

    let stream = self
      .inner
      .call_api_with_params(model, messages, thinking_mode, tools)
      .await?;

    let path = stream_file(&self.dir, seq);
    // Truncate up front so a re-record does not append to the previous take.
    std::fs::write(&path, "")?;
    Ok(Box::pin(stream.map(move |item| {
      if let Ok(value) = &item
        && let Ok(line) = serde_json::to_string(value)
      {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&path) {
          let _ = writeln!(f, "{}", line);
        }
      }
      item
    })))
  }
}

// ----------------------------------------------------------------- replaying

pub struct Replaying {
  dir: PathBuf,
  seq: AtomicUsize,
}

impl Replaying {
  pub fn new(dir: PathBuf) -> Self {
    Self {
      dir,
      seq: AtomicUsize::new(0),
    }
  }
}

#[async_trait::async_trait]
impl LlmProvider for Replaying {
  async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<StreamResult> {
    let seq = self.seq.fetch_add(1, Ordering::SeqCst);
    let req_path = request_file(&self.dir, seq);
    let stream_path = stream_file(&self.dir, seq);

    if !stream_path.exists() {
      anyhow::bail!(
        "replay exhausted: the loop made request #{} but {} has only {} recorded. \
         The trajectory diverged from the fixture — re-record it, or fix what \
         made the loop take an extra step.",
        seq + 1,
        self.dir.display(),
        seq
      );
    }

    let recorded: RequestShape = serde_json::from_str(
      &std::fs::read_to_string(&req_path)
        .with_context(|| format!("cannot read {}", req_path.display()))?,
    )
    .with_context(|| format!("cannot parse {}", req_path.display()))?;

    let actual = RequestShape::capture(model, &messages, thinking_mode, tools.as_deref());
    if let Some(diff) = recorded.mismatch(&actual) {
      anyhow::bail!(
        "replay request #{} does not match the recording: {}\n\
         (recorded digest {}, actual {}). A silently misaligned replay would \
         produce a green test that proves nothing, so this is a hard failure.",
        seq,
        diff,
        recorded.digest,
        actual.digest
      );
    }

    let body = std::fs::read_to_string(&stream_path)
      .with_context(|| format!("cannot read {}", stream_path.display()))?;
    let mut items: Vec<Result<StreamItem>> = Vec::new();
    for (n, line) in body.lines().enumerate() {
      if line.trim().is_empty() {
        continue;
      }
      let item: StreamItem = serde_json::from_str(line)
        .with_context(|| format!("{}:{} is not a StreamItem", stream_path.display(), n + 1))?;
      items.push(Ok(item));
    }
    Ok(Box::pin(futures_util::stream::iter(items)))
  }
}

/// Wrap `inner` per the environment. Returns `inner` untouched when neither
/// variable is set, so the normal path costs nothing.
pub fn wrap(inner: Box<dyn LlmProvider>) -> Box<dyn LlmProvider> {
  match std::env::var(RECORD_ENV) {
    Ok(dir) if !dir.is_empty() => Box::new(Recording::new(inner, PathBuf::from(dir))),
    _ => inner,
  }
}

/// A replay provider when `SEEKCLI_REPLAY` is set.
///
/// Checked before credentials are resolved: a replayed run must work with no
/// API key at all, or the tests it enables cannot run in CI.
pub fn replay_from_env() -> Option<Box<dyn LlmProvider>> {
  match std::env::var(REPLAY_ENV) {
    Ok(dir) if !dir.is_empty() => Some(Box::new(Replaying::new(PathBuf::from(dir)))),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn shape(count: usize, role: &str, tools: &[&str]) -> RequestShape {
    RequestShape {
      model: "m".into(),
      message_count: count,
      last_role: role.into(),
      thinking_mode: "none".into(),
      tool_names: tools.iter().map(|s| s.to_string()).collect(),
      digest: "0".into(),
    }
  }

  #[test]
  fn identical_shapes_match() {
    let a = shape(3, "user", &["read_file"]);
    assert!(a.mismatch(&a.clone()).is_none());
  }

  #[test]
  fn structural_differences_are_named_not_just_flagged() {
    let recorded = shape(3, "user", &["read_file"]);

    let diff = recorded.mismatch(&shape(4, "user", &["read_file"]));
    assert!(diff.is_some_and(|d| d.contains("message count")), "count");

    let diff = recorded.mismatch(&shape(3, "tool", &["read_file"]));
    assert!(diff.is_some_and(|d| d.contains("role")), "role");

    let diff = recorded.mismatch(&shape(3, "user", &["read_file", "write_file"]));
    assert!(diff.is_some_and(|d| d.contains("tool set")), "tools");
  }

  #[test]
  fn wording_changes_do_not_break_a_fixture() {
    // Only the digest differs; a reworded prompt of the same shape must still
    // replay, or every prompt edit would invalidate the whole suite.
    let mut other = shape(3, "user", &["read_file"]);
    other.digest = "deadbeef".into();
    assert!(shape(3, "user", &["read_file"]).mismatch(&other).is_none());
  }

  #[tokio::test]
  async fn replay_reports_exhaustion_with_a_diagnosis() {
    let dir = std::env::temp_dir().join("seekcli-replay-empty");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::remove_file(stream_file(&dir, 0));
    let p = Replaying::new(dir);
    let err = match p.call_api_with_params("m", vec![], "none", None).await {
      Ok(_) => panic!("expected exhaustion"),
      Err(e) => format!("{:#}", e),
    };
    assert!(err.contains("replay exhausted"), "got: {}", err);
    assert!(
      err.contains("diverged"),
      "error should say what to do: {}",
      err
    );
  }
}
