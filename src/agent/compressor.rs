//! Lossy context compression for long-running ReAct loops.
//!
//! Strategy:
//! 1. Preserve leading system messages verbatim (agent prompt + optional
//!    skill prompts) so DeepSeek's prompt cache prefix stays stable.
//! 2. Preserve the most recent `KEEP_TAIL` messages so immediate context
//!    (current task + recent tool results) is intact.
//! 3. Replace the middle with a single synthetic system message containing
//!    a model-generated summary.
//!
//! Triggered at the top of each main-agent ReAct iteration when serialized
//! message bytes exceed `COMPRESSION_THRESHOLD_BYTES`. Sub-agents are not
//! compressed (they have their own `max_iter` cap and short contexts).

use anyhow::Result;
use colored::Colorize;
use futures_util::StreamExt;

use crate::api::{ApiClient, Message, StreamItem};

/// Trigger compression when serialized messages exceed this many bytes.
/// Rough heuristic: ~4 bytes/token for English, ~2 for Chinese. 600KB lands
/// somewhere around 150K~300K tokens — well under DeepSeek V4's 1M cap, with
/// headroom for the rest of the loop's tool results.
pub const COMPRESSION_THRESHOLD_BYTES: usize = 600_000;

/// Number of trailing messages to keep uncompressed.
const KEEP_TAIL: usize = 8;

/// If `messages` exceeds the threshold, summarize the middle portion and
/// replace it in place. Returns `Ok(true)` on actual compression.
pub async fn maybe_compress(
  client: &ApiClient,
  model: &str,
  messages: &mut Vec<Message>,
) -> Result<bool> {
  let total = estimate_bytes(messages);
  if total < COMPRESSION_THRESHOLD_BYTES {
    return Ok(false);
  }

  // Leading run of `system` messages stays verbatim to keep cache prefix stable.
  let head_end = messages
    .iter()
    .take_while(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
    .count();

  if messages.len() <= head_end + KEEP_TAIL {
    // Not enough middle to be worth compressing.
    return Ok(false);
  }

  let tail_start = messages.len() - KEEP_TAIL;
  let middle: Vec<Message> = messages[head_end..tail_start].to_vec();

  println!(
    "{} compressing {} middle messages ({} bytes total)...",
    "[Memory]".magenta(),
    middle.len(),
    total
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

  let new_total = estimate_bytes(&rebuilt);
  let reduction_pct = 100usize.saturating_sub(new_total * 100 / total.max(1));
  println!(
    "{} compressed: {} → {} bytes ({}% reduction)",
    "[Memory]".magenta(),
    total,
    new_total,
    reduction_pct
  );

  *messages = rebuilt;
  Ok(true)
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
}
