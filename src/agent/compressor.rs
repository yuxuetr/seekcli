//! Staged-degradation context compression for long-running ReAct loops.
//!
//! The cardinal rule of harness memory management: drop redundant *data* while
//! preserving *intent* and the logic chain. Naively summarizing or deleting the
//! middle of the conversation can sever a `tool_call` from its `tool` result,
//! confusing the model into re-issuing calls it already made.
//!
//! Strategy (cheapest first, escalating only if needed):
//!   Stage 0  Leading system messages are sacred — kept verbatim so the prompt
//!            cache prefix stays stable.
//!   Stage 1  MASK: in the far history (older than the working-memory tail),
//!            replace bulky `tool` result bodies with a short placeholder. The
//!            originating assistant `tool_calls` are KEPT, so the intent chain
//!            survives — the model still sees *what* it did, just not the full
//!            multi-KB output.
//!   Stage 2  HEAD-TAIL TRUNCATE: even inside the protected working-memory
//!            tail, a single oversized `tool` result is clipped to its first +
//!            last slice (errors put the cause at the top and the stack summary
//!            at the bottom; the middle is noise).
//!   Stage 3  SUMMARIZE (escalation): only if masking + truncation still leave
//!            us over the threshold, fall back to an LLM summary of the middle.
//!
//! Triggered at the top of each main-agent ReAct iteration. Idempotent: markers
//! prevent re-masking / re-truncating already-compressed messages.

use anyhow::Result;
use colored::Colorize;
use futures_util::StreamExt;

use crate::api::{ApiClient, Message, StreamItem};

/// Trigger compression when serialized messages exceed this many bytes.
/// ~4 bytes/token (English) to ~2 (Chinese); 600KB ≈ 150K~300K tokens, well
/// under DeepSeek V4's 1M cap with headroom for the rest of the loop.
pub const COMPRESSION_THRESHOLD_BYTES: usize = 600_000;

/// Number of trailing messages to keep in the protected working memory.
const KEEP_TAIL: usize = 8;

/// Only mask far-history tool results larger than this (small outputs aren't
/// worth a placeholder).
const MASK_MIN_BYTES: usize = 500;

/// Head-tail truncate working-memory tool results larger than this...
const TAIL_TOOL_LIMIT: usize = 1_000;
/// ...keeping this many bytes from each end.
const TAIL_KEEP_EACH: usize = 500;

/// Prefix marking an already-masked far-history tool result (idempotency).
const MASK_MARKER: &str = "[tool output masked";
/// Marker embedded in a head-tail-truncated body (idempotency).
const TRUNC_MARKER: &str = "[...truncated";

/// Apply staged-degradation compression in place if `messages` exceeds the
/// threshold. Returns `Ok(true)` when any compression happened.
pub async fn maybe_compress(
  client: &ApiClient,
  model: &str,
  messages: &mut Vec<Message>,
) -> Result<bool> {
  let total = estimate_bytes(messages);
  if total < COMPRESSION_THRESHOLD_BYTES {
    return Ok(false);
  }

  // Stage 0: leading run of `system` messages stays verbatim.
  let head_end = messages
    .iter()
    .take_while(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
    .count();

  if messages.len() <= head_end + KEEP_TAIL {
    // Nothing but head + protected tail; can't shed the middle safely.
    // Still head-tail truncate any oversized tail tool result (stage 2).
    let truncated = truncate_tail(messages, head_end);
    return Ok(truncated);
  }

  let tail_start = messages.len() - KEEP_TAIL;
  let mut changed = false;

  // Stage 1: mask bulky far-history tool results (preserve ToolCall intent).
  let mut masked_bytes = 0usize;
  for msg in &mut messages[head_end..tail_start] {
    if let Message::ToolResponse { content, .. } = msg
      && content.len() > MASK_MIN_BYTES
      && !content.starts_with(MASK_MARKER)
    {
      masked_bytes += content.len();
      *content = format!(
        "{} — {} bytes cleared; the originating tool call above is preserved. \
         Re-run the tool if you need the full output again.]",
        MASK_MARKER,
        content.len()
      );
      changed = true;
    }
  }

  // Stage 2: head-tail truncate oversized tool results in the working tail.
  changed |= truncate_tail(messages, tail_start);

  if changed {
    let after = estimate_bytes(messages);
    let reduction = 100usize.saturating_sub(after * 100 / total.max(1));
    println!(
      "{} staged compression: {} → {} bytes ({}% reduction, {} bytes masked)",
      "[Memory]".magenta(),
      total,
      after,
      reduction,
      masked_bytes
    );
    if after < COMPRESSION_THRESHOLD_BYTES {
      return Ok(true);
    }
  }

  // Stage 3 (escalation): masking + truncation weren't enough — summarize the
  // far-history middle and replace it with a single synthetic system message.
  let middle: Vec<Message> = messages[head_end..tail_start].to_vec();
  println!(
    "{} escalating: summarizing {} middle messages...",
    "[Memory]".magenta(),
    middle.len()
  );
  let summary = summarize_messages(client, model, &middle).await?;

  let mut rebuilt = messages[..head_end].to_vec();
  rebuilt.push(Message::Simple {
    role: "system".to_string(),
    content: format!("[Compressed earlier turns]\n\n{}", summary),
    reasoning_content: None,
    tool_calls: None,
  });
  rebuilt.extend(messages[tail_start..].iter().cloned());
  *messages = rebuilt;
  Ok(true)
}

/// Head-tail truncate any oversized `tool` result at or after `from`.
/// Returns true if anything was truncated.
fn truncate_tail(messages: &mut [Message], from: usize) -> bool {
  let mut changed = false;
  for msg in &mut messages[from..] {
    if let Message::ToolResponse { content, .. } = msg
      && content.len() > TAIL_TOOL_LIMIT
      && !content.contains(TRUNC_MARKER)
    {
      *content = head_tail_truncate(content);
      changed = true;
    }
  }
  changed
}

/// Keep the first and last `TAIL_KEEP_EACH` bytes of `content`, dropping the
/// middle. For error logs the cause is at the top and the summary at the
/// bottom; the middle is usually a repetitive stack/noise.
fn head_tail_truncate(content: &str) -> String {
  let n = content.len();
  let head_end = floor_boundary(content, TAIL_KEEP_EACH);
  let tail_start = ceil_boundary(content, n - TAIL_KEEP_EACH);
  let dropped = tail_start.saturating_sub(head_end);
  format!(
    "{}\n{} {} bytes from the middle...]\n{}",
    &content[..head_end],
    TRUNC_MARKER,
    dropped,
    &content[tail_start..]
  )
}

/// Largest char boundary <= `idx`.
fn floor_boundary(s: &str, idx: usize) -> usize {
  let mut i = idx.min(s.len());
  while i > 0 && !s.is_char_boundary(i) {
    i -= 1;
  }
  i
}

/// Smallest char boundary >= `idx`.
fn ceil_boundary(s: &str, idx: usize) -> usize {
  let mut i = idx.min(s.len());
  while i < s.len() && !s.is_char_boundary(i) {
    i += 1;
  }
  i
}

fn estimate_bytes(messages: &[Message]) -> usize {
  messages
    .iter()
    .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
    .sum()
}

async fn summarize_messages(client: &ApiClient, model: &str, middle: &[Message]) -> Result<String> {
  let middle_json = serde_json::to_string_pretty(middle)?;
  let prompt = format!(
    "You are summarizing the middle portion of a long agent conversation \
     so it can be compressed out of the context window. Preserve:\n\
     - Key facts established or discovered\n\
     - Decisions made and their rationale\n\
     - Pending tasks or unresolved questions\n\
     - File paths, function names, error messages — anything the agent might \
       reference later\n\n\
     Drop:\n\
     - Conversational filler and intermediate reasoning\n\
     - Tool call mechanics (e.g. 'I called read_file and got 5KB back')\n\n\
     Output a compact Markdown summary in third-person, under 500 words.\n\n\
     ---\n\nCONVERSATION TO SUMMARIZE:\n\n{}",
    middle_json
  );

  let summary_messages = vec![Message::Simple {
    role: "user".to_string(),
    content: prompt,
    reasoning_content: None,
    tool_calls: None,
  }];

  let mut stream = client
    .call_api_with_params(model, summary_messages, "none", None)
    .await?;
  let mut summary = String::new();
  while let Some(item) = stream.next().await {
    if let Ok(StreamItem::Content(c)) = item {
      summary.push_str(&c);
    }
  }

  if summary.trim().is_empty() {
    anyhow::bail!("Compression summary came back empty");
  }
  Ok(summary)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn make_simple(role: &str, content: &str) -> Message {
    Message::Simple {
      role: role.to_string(),
      content: content.to_string(),
      reasoning_content: None,
      tool_calls: None,
    }
  }

  fn make_tool(content: &str) -> Message {
    Message::ToolResponse {
      role: "tool".to_string(),
      content: content.to_string(),
      tool_call_id: "call_1".to_string(),
    }
  }

  #[test]
  fn estimate_bytes_nonzero() {
    let msgs = vec![make_simple("user", "hello world")];
    assert!(estimate_bytes(&msgs) > 0);
  }

  #[test]
  fn estimate_bytes_grows_with_content() {
    let small = vec![make_simple("user", "hi")];
    let large = vec![make_simple("user", &"x".repeat(10_000))];
    assert!(estimate_bytes(&large) > estimate_bytes(&small) * 100);
  }

  #[test]
  fn head_tail_truncate_keeps_ends() {
    let body = format!("HEAD{}TAIL", "x".repeat(5_000));
    let out = head_tail_truncate(&body);
    assert!(out.starts_with("HEAD"));
    assert!(out.ends_with("TAIL"));
    assert!(out.contains(TRUNC_MARKER));
    assert!(out.len() < body.len());
  }

  #[test]
  fn truncate_tail_is_idempotent() {
    let mut msgs = vec![make_tool(&"y".repeat(5_000))];
    assert!(truncate_tail(&mut msgs, 0)); // first pass truncates
    assert!(!truncate_tail(&mut msgs, 0)); // marker present -> no-op
  }

  #[test]
  fn small_tool_results_untouched() {
    let mut msgs = vec![make_tool("short output")];
    assert!(!truncate_tail(&mut msgs, 0));
    if let Message::ToolResponse { content, .. } = &msgs[0] {
      assert_eq!(content, "short output");
    } else {
      panic!("expected tool response");
    }
  }
}
