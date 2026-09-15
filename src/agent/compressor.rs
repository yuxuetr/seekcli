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

use crate::api::tokens::{Heuristic, TokenCounter};
use crate::api::{LlmProvider, Message, StreamItem};
use crate::session::{self, EventPayload, Session};

/// When to compact, and how much to protect — derived from the configured
/// model window rather than hard-coded.
///
/// The two numbers this replaced were a fixed `150_000` and a fixed `8`. The
/// threshold in particular was wrong in a way nothing could observe: the
/// provider is chosen statically from config and two wire protocols are
/// supported, so the window behind that number differs per install. Point
/// SeekCLI at a model with a smaller window and compaction simply never fires
/// — the request fails on length, and no line of output connects the two.
///
/// `Budget::default()` reproduces the old constants exactly, so the change is
/// one of configurability and visibility, not of behaviour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
  /// Compaction trips at or above this many estimated tokens.
  pub threshold_tokens: usize,
  /// Trailing messages held out of compaction as protected working memory.
  pub keep_tail: usize,
  /// Kept for the log line, so the threshold can be read back as
  /// `window * ratio` instead of appearing as an unexplained number.
  pub window_tokens: usize,
  pub ratio: f64,
}

/// Ratios outside this range are refused rather than honoured: at 0 every turn
/// compacts, at 1 the threshold sits at the window itself and leaves the model
/// no room to answer. Both are configuration mistakes, not intentions.
const RATIO_BOUNDS: std::ops::RangeInclusive<f64> = 0.1..=0.95;

impl Budget {
  /// Build from config, clamping a nonsensical ratio **loudly**. Silently
  /// honouring `compact_at_ratio = 5.0` would disable compaction with no trace
  /// — the same invisible failure this type exists to end.
  pub fn from_config(cfg: &crate::config::MemoryConfig) -> Self {
    let mut ratio = cfg.compact_at_ratio;
    if !ratio.is_finite() || !RATIO_BOUNDS.contains(&ratio) {
      let clamped = if ratio.is_finite() {
        ratio.clamp(*RATIO_BOUNDS.start(), *RATIO_BOUNDS.end())
      } else {
        default_ratio()
      };
      eprintln!(
        "{} memory.compact_at_ratio = {} is out of range {:?}; using {}",
        "[Memory]".yellow(),
        ratio,
        RATIO_BOUNDS,
        clamped
      );
      ratio = clamped;
    }
    let window = cfg.context_window_tokens;
    Self {
      // `as usize` on a product of a positive usize and a bounded ratio: the
      // ratio is <= 0.95 here, so the result cannot exceed `window`.
      threshold_tokens: (window as f64 * ratio) as usize,
      keep_tail: cfg.keep_tail_messages.max(2),
      window_tokens: window,
      ratio,
    }
  }

  /// How the threshold was arrived at, for the compaction log line.
  fn derivation(&self) -> String {
    format!(
      "{} = window {} x {}",
      self.threshold_tokens, self.window_tokens, self.ratio
    )
  }
}

fn default_ratio() -> f64 {
  0.75
}

impl Default for Budget {
  fn default() -> Self {
    Self::from_config(&crate::config::MemoryConfig::default())
  }
}

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
  client: &dyn LlmProvider,
  model: &str,
  messages: &mut Vec<Message>,
  budget: Budget,
) -> Result<bool> {
  let total = estimate_tokens(messages);
  if total < budget.threshold_tokens {
    return Ok(false);
  }

  // Stage 0: leading run of `system` messages stays verbatim.
  let head_end = messages
    .iter()
    .take_while(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
    .count();

  if messages.len() <= head_end + budget.keep_tail {
    // Nothing but head + protected tail; can't shed the middle safely.
    // Still head-tail truncate any oversized tail tool result (stage 2).
    let truncated = truncate_tail(messages, head_end);
    return Ok(truncated);
  }

  let tail_start = messages.len() - budget.keep_tail;
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
    let after = estimate_tokens(messages);
    let reduction = 100usize.saturating_sub(after * 100 / total.max(1));
    eprintln!(
      "{} staged compression: {} → {} tokens ({}% reduction, {} bytes masked); \
       threshold {}",
      "[Memory]".magenta(),
      total,
      after,
      reduction,
      masked_bytes,
      budget.derivation()
    );
    if after < budget.threshold_tokens {
      return Ok(true);
    }
  }

  // Stage 3 (escalation): masking + truncation weren't enough — summarize the
  // far-history middle and replace it with a single synthetic system message.
  let middle: Vec<Message> = messages[head_end..tail_start].to_vec();
  eprintln!(
    "{} escalating: summarizing {} middle messages...",
    "[Memory]".magenta(),
    middle.len()
  );
  let summary = summarize_messages(client, model, &middle).await?;

  let mut rebuilt = messages[..head_end].to_vec();
  rebuilt.push(Message::Simple {
    images: Vec::new(),
    role: "system".to_string(),
    content: format!("[Compressed earlier turns]\n\n{}", summary),
    reasoning_content: None,
    tool_calls: None,
  });
  rebuilt.extend(messages[tail_start..].iter().cloned());
  *messages = rebuilt;
  Ok(true)
}

/// Compact the *session* at a turn boundary, recording a `Compaction` event.
///
/// This is what makes compression survive a turn. `maybe_compress` operates on
/// the working set, which is re-projected from the log every turn — so its
/// stage-3 summary evaporated, and a long session paid for a fresh summary on
/// every single turn. Recording the summary as an event means the projection
/// carries it forward, while the events it replaces stay on disk.
///
/// Runs before the working set is built, so the loop still gets
/// `maybe_compress` as an in-turn safety net for a single turn that balloons.
pub async fn maybe_compact_session(
  client: &dyn LlmProvider,
  model: &str,
  session: &mut Session,
  budget: Budget,
) -> Result<bool> {
  let (messages, seqs) = session::derive_messages_indexed(&session.events);
  if estimate_tokens(&messages) < budget.threshold_tokens {
    return Ok(false);
  }

  // Same shape as the working-set compressor: leading system messages stay,
  // the recent tail stays, the middle is summarized.
  let head_end = messages
    .iter()
    .take_while(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
    .count();
  if messages.len() <= head_end + budget.keep_tail {
    return Ok(false);
  }
  let tail_start = messages.len() - budget.keep_tail;

  let from = match seqs.get(head_end) {
    Some(seq) => *seq,
    None => return Ok(false),
  };
  let to = match seqs.get(tail_start) {
    Some(seq) => *seq,
    None => return Ok(false),
  };
  if to <= from {
    return Ok(false);
  }

  eprintln!(
    "{} compacting session: summarizing events {}..{} ({} messages)",
    "[Memory]".magenta(),
    from,
    to,
    tail_start - head_end
  );
  let summary = summarize_messages(client, model, &messages[head_end..tail_start]).await?;
  session.record(EventPayload::Compaction { from, to, summary });
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

/// Approximate token count for the conversation.
///
/// Replaces byte counting, which tripped compaction at roughly a third of the
/// real context budget for CJK conversations — see `api::tokens`.
fn estimate_tokens(messages: &[Message]) -> usize {
  Heuristic.count_messages(messages)
}

async fn summarize_messages(
  client: &dyn LlmProvider,
  model: &str,
  middle: &[Message],
) -> Result<String> {
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
    images: Vec::new(),
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
      images: Vec::new(),
      role: role.to_string(),
      content: content.to_string(),
      reasoning_content: None,
      tool_calls: None,
    }
  }

  fn make_tool(content: &str) -> Message {
    Message::ToolResponse {
      images: Vec::new(),
      role: "tool".to_string(),
      content: content.to_string(),
      tool_call_id: "call_1".to_string(),
    }
  }

  use crate::config::MemoryConfig;

  /// The defaults must reproduce the constants they replaced exactly. This is
  /// the whole safety argument for the change: it makes the threshold
  /// configurable and visible without moving it for anyone who does nothing.
  #[test]
  fn the_default_budget_reproduces_the_old_constants() {
    let b = Budget::default();
    assert_eq!(
      b.threshold_tokens, 150_000,
      "was COMPRESSION_THRESHOLD_TOKENS"
    );
    assert_eq!(b.keep_tail, 8, "was KEEP_TAIL");
  }

  /// The bug this fixes: a fixed 150K threshold against a model whose window
  /// is smaller means compaction never fires and the request fails on length.
  #[test]
  fn a_smaller_window_lowers_the_threshold() {
    let small = Budget::from_config(&MemoryConfig {
      context_window_tokens: 64_000,
      ..MemoryConfig::default()
    });
    assert_eq!(small.threshold_tokens, 48_000);
    assert!(
      small.threshold_tokens < Budget::default().threshold_tokens,
      "a 64K model must compact earlier than a 200K one"
    );
  }

  /// A ratio outside the sane band is a configuration mistake, not an
  /// intention — honouring `5.0` would silently disable compaction, which is
  /// the exact invisible failure this type exists to end.
  #[test]
  fn an_absurd_ratio_is_clamped_rather_than_honoured() {
    let too_big = Budget::from_config(&MemoryConfig {
      compact_at_ratio: 5.0,
      ..MemoryConfig::default()
    });
    assert!(too_big.threshold_tokens <= too_big.window_tokens);
    assert_eq!(too_big.ratio, 0.95);

    let too_small = Budget::from_config(&MemoryConfig {
      compact_at_ratio: 0.0,
      ..MemoryConfig::default()
    });
    assert_eq!(too_small.ratio, 0.1, "0 would compact on every single turn");

    let nonsense = Budget::from_config(&MemoryConfig {
      compact_at_ratio: f64::NAN,
      ..MemoryConfig::default()
    });
    assert_eq!(nonsense.ratio, 0.75, "NaN falls back to the default");
  }

  /// `keep_tail` under 2 would let a compaction split a tool call from its
  /// result — the one thing the whole module is built to prevent.
  #[test]
  fn keep_tail_has_a_floor() {
    let b = Budget::from_config(&MemoryConfig {
      keep_tail_messages: 0,
      ..MemoryConfig::default()
    });
    assert_eq!(b.keep_tail, 2);
  }

  /// The threshold must be readable back as its derivation, so a number in the
  /// log can be traced to the config that produced it.
  #[test]
  fn the_log_line_explains_where_the_threshold_came_from() {
    let d = Budget::default().derivation();
    assert!(d.contains("150000"), "{d}");
    assert!(d.contains("200000"), "{d}");
    assert!(d.contains("0.75"), "{d}");
  }

  #[test]
  fn estimate_tokens_nonzero() {
    let msgs = vec![make_simple("user", "hello world")];
    assert!(estimate_tokens(&msgs) > 0);
  }

  #[test]
  fn estimate_tokens_grows_with_content() {
    let small = vec![make_simple("user", "hi")];
    let large = vec![make_simple("user", &"x".repeat(10_000))];
    assert!(estimate_tokens(&large) > estimate_tokens(&small) * 100);
  }

  /// The reason the threshold moved off bytes: the same conversation must
  /// compact at the same point regardless of the script it is written in.
  #[test]
  fn the_threshold_no_longer_depends_on_script() {
    // Same number of characters, very different byte counts.
    let latin = vec![make_simple("user", &"a".repeat(300))];
    let cjk = vec![make_simple("user", &"中".repeat(300))];
    let latin_tokens = estimate_tokens(&latin) as f64;
    let cjk_tokens = estimate_tokens(&cjk) as f64;
    // Byte counting made the CJK version look 3x larger. Token estimation
    // should put them within a small factor of each other.
    let ratio = cjk_tokens / latin_tokens;
    assert!(
      (0.5..=5.0).contains(&ratio),
      "scripts should be comparable, got ratio {}",
      ratio
    );
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
