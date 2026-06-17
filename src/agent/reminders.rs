//! System Reminders: runtime intervention to break doom loops.
//!
//! A static system prompt cannot reliably stop a model that has fixated on a
//! failing action: recency bias ("lost in the middle") means the most recent
//! repeated tool result dominates the next decision far more than rules at the
//! top of the context. The fix is to detect repetition by hashing each turn's
//! tool-call trajectory and, once it repeats too many times, inject a
//! high-priority **user** message right at the point of decision — close enough
//! to the model's attention to actually redirect it.
//!
//! Scoped to the main agent's loop; sub-agents have their own short `max_iter`
//! caps and isolated contexts.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::api::ToolCall;

/// Number of *consecutive repeats* (beyond the first occurrence) of an
/// identical tool-call trajectory before a reminder fires. 2 means the
/// trajectory must appear 3 times in a row.
const REPEAT_THRESHOLD: u32 = 2;

/// Tracks the recent tool-call trajectory to detect doom loops.
#[derive(Default)]
pub struct ReminderInjector {
  last_hash: Option<u64>,
  repeat_count: u32,
}

impl ReminderInjector {
  pub fn new() -> Self {
    Self::default()
  }

  /// Record the tool-call trajectory of a just-completed turn. Returns a
  /// reminder string to inject as a user message when the same trajectory has
  /// repeated past the threshold; otherwise `None`.
  ///
  /// Turns that emitted no tool calls reset the tracker (the model made
  /// progress / answered), so they never trigger a reminder.
  pub fn observe(&mut self, tool_calls: &[ToolCall]) -> Option<String> {
    if tool_calls.is_empty() {
      self.last_hash = None;
      self.repeat_count = 0;
      return None;
    }

    let hash = trajectory_hash(tool_calls);
    if self.last_hash == Some(hash) {
      self.repeat_count += 1;
    } else {
      self.last_hash = Some(hash);
      self.repeat_count = 0;
    }

    if self.repeat_count >= REPEAT_THRESHOLD {
      // Reset so the next reminder only fires after a fresh run of repeats,
      // rather than every subsequent turn.
      self.repeat_count = 0;
      Some(
        "⚠️ [System] You have repeated the same tool call(s) several times with \
         the same result. This path is not working. STOP retrying it. Step \
         back and either (a) try a fundamentally different approach, (b) gather \
         new information with a different tool, or (c) report the obstacle to \
         the user and ask how to proceed."
          .to_string(),
      )
    } else {
      None
    }
  }
}

/// Hash a turn's tool calls by (name, arguments) so that an identical batch of
/// calls produces an identical fingerprint. `id` is intentionally excluded —
/// it is unique per call and would defeat repetition detection.
fn trajectory_hash(tool_calls: &[ToolCall]) -> u64 {
  let mut hasher = DefaultHasher::new();
  for tc in tool_calls {
    tc.function.name.hash(&mut hasher);
    tc.function.arguments.hash(&mut hasher);
  }
  hasher.finish()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::{FunctionCall, ToolCall};

  fn call(name: &str, args: &str) -> ToolCall {
    ToolCall {
      id: "x".to_string(),
      tool_type: "function".to_string(),
      function: FunctionCall {
        name: name.to_string(),
        arguments: args.to_string(),
      },
    }
  }

  #[test]
  fn fires_after_three_identical_turns() {
    let mut inj = ReminderInjector::new();
    let batch = vec![call("read_file", "{\"path\":\"a.rs\"}")];
    assert!(inj.observe(&batch).is_none()); // 1st
    assert!(inj.observe(&batch).is_none()); // 2nd (repeat_count=1)
    assert!(inj.observe(&batch).is_some()); // 3rd (repeat_count=2 -> fire)
  }

  #[test]
  fn different_calls_do_not_fire() {
    let mut inj = ReminderInjector::new();
    assert!(
      inj
        .observe(&[call("read_file", "{\"path\":\"a\"}")])
        .is_none()
    );
    assert!(
      inj
        .observe(&[call("read_file", "{\"path\":\"b\"}")])
        .is_none()
    );
    assert!(
      inj
        .observe(&[call("list_dir", "{\"path\":\".\"}")])
        .is_none()
    );
  }

  #[test]
  fn empty_turn_resets_tracker() {
    let mut inj = ReminderInjector::new();
    let batch = vec![call("run_shell", "{\"command\":\"ls\"}")];
    inj.observe(&batch);
    inj.observe(&batch);
    assert!(inj.observe(&[]).is_none()); // progress made, reset
    assert!(inj.observe(&batch).is_none()); // counting restarts
  }

  #[test]
  fn fires_again_only_after_fresh_run() {
    let mut inj = ReminderInjector::new();
    let batch = vec![call("read_file", "{\"path\":\"a.rs\"}")];
    inj.observe(&batch);
    inj.observe(&batch);
    assert!(inj.observe(&batch).is_some()); // fires, resets counter
    assert!(inj.observe(&batch).is_none()); // repeat_count=1 again
    assert!(inj.observe(&batch).is_some()); // fires again on next run
  }
}
