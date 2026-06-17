//! Token / cost accounting.
//!
//! DeepSeek reports a [`UsageInfo`] block at the end of each streamed response.
//! Rather than threading billing logic through the engine, the main loop hands
//! every usage block to a [`CostTracker`] that accumulates a running total for
//! the session and can print a bill on demand. This mirrors the harness
//! "decorator / interceptor" pattern: the core loop stays oblivious to money.
//!
//! The CNY figures are deliberately labeled estimates — DeepSeek's published
//! rates change and vary by model/time; treat the token counts as exact and the
//! yuan as a ballpark. Adjust [`CNY_PER_M_CACHE_HIT`] etc. if rates move.

use colored::Colorize;

use crate::api::UsageInfo;

/// Estimated CNY per 1M cache-hit input tokens (cheapest tier).
const CNY_PER_M_CACHE_HIT: f64 = 0.5;
/// Estimated CNY per 1M cache-miss (fresh) input tokens.
const CNY_PER_M_CACHE_MISS: f64 = 2.0;
/// Estimated CNY per 1M output (completion) tokens.
const CNY_PER_M_OUTPUT: f64 = 3.0;

/// Running token/cost total across a session. One per `App`; reset on restart.
#[derive(Debug, Default, Clone)]
pub struct CostTracker {
  pub prompt_tokens: u64,
  pub completion_tokens: u64,
  pub cache_hit_tokens: u64,
  pub cache_miss_tokens: u64,
  /// Number of LLM responses that reported usage (turns + planning + sub-agents).
  pub api_calls: u64,
}

impl CostTracker {
  pub fn new() -> Self {
    Self::default()
  }

  /// Fold one usage block into the running total.
  pub fn record(&mut self, u: &UsageInfo) {
    self.prompt_tokens += u.prompt_tokens;
    self.completion_tokens += u.completion_tokens;
    self.cache_hit_tokens += u.prompt_cache_hit_tokens;
    self.cache_miss_tokens += u.prompt_cache_miss_tokens;
    self.api_calls += 1;
  }

  /// Whether anything has been recorded yet.
  pub fn is_empty(&self) -> bool {
    self.api_calls == 0
  }

  /// Estimated spend in CNY. Uses cache hit/miss split when available, else
  /// falls back to treating all prompt tokens as cache misses.
  pub fn estimated_cny(&self) -> f64 {
    let (hit, miss) = if self.cache_hit_tokens + self.cache_miss_tokens > 0 {
      (self.cache_hit_tokens, self.cache_miss_tokens)
    } else {
      (0, self.prompt_tokens)
    };
    (hit as f64 / 1_000_000.0) * CNY_PER_M_CACHE_HIT
      + (miss as f64 / 1_000_000.0) * CNY_PER_M_CACHE_MISS
      + (self.completion_tokens as f64 / 1_000_000.0) * CNY_PER_M_OUTPUT
  }

  /// Cache hit rate over all prompt tokens, as a percentage (0 when no input).
  pub fn cache_hit_pct(&self) -> u64 {
    let total = self.cache_hit_tokens + self.cache_miss_tokens;
    (self.cache_hit_tokens * 100)
      .checked_div(total)
      .unwrap_or(0)
  }

  /// One-line session bill for printing at the end of a chat turn.
  pub fn summary(&self) -> String {
    format!(
      "{} {} calls · prompt={} (cache {}%) · completion={} · ≈¥{:.4}",
      "[Cost]".dimmed(),
      self.api_calls,
      self.prompt_tokens,
      self.cache_hit_pct(),
      self.completion_tokens,
      self.estimated_cny(),
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn usage(prompt: u64, completion: u64, hit: u64, miss: u64) -> UsageInfo {
    UsageInfo {
      prompt_tokens: prompt,
      completion_tokens: completion,
      prompt_cache_hit_tokens: hit,
      prompt_cache_miss_tokens: miss,
    }
  }

  #[test]
  fn accumulates_across_calls() {
    let mut t = CostTracker::new();
    assert!(t.is_empty());
    t.record(&usage(100, 50, 80, 20));
    t.record(&usage(200, 60, 150, 50));
    assert_eq!(t.api_calls, 2);
    assert_eq!(t.prompt_tokens, 300);
    assert_eq!(t.completion_tokens, 110);
    assert_eq!(t.cache_hit_tokens, 230);
    assert!(!t.is_empty());
  }

  #[test]
  fn cache_hit_pct_computes() {
    let mut t = CostTracker::new();
    t.record(&usage(100, 0, 75, 25));
    assert_eq!(t.cache_hit_pct(), 75);
  }

  #[test]
  fn estimated_cny_uses_hit_miss_split() {
    let mut t = CostTracker::new();
    // 1M cache-miss input + 1M output = 2.0 + 3.0 = 5.0 CNY.
    t.record(&usage(1_000_000, 1_000_000, 0, 1_000_000));
    let cny = t.estimated_cny();
    assert!((cny - 5.0).abs() < 1e-6, "got {cny}");
  }

  #[test]
  fn estimated_cny_falls_back_to_prompt_as_miss() {
    let mut t = CostTracker::new();
    // No hit/miss split reported: treat all prompt as miss → 2.0 CNY.
    t.record(&usage(1_000_000, 0, 0, 0));
    assert!((t.estimated_cny() - 2.0).abs() < 1e-6);
  }
}
