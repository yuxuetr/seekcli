//! Trajectory export: hand a bench run to something outside this repository.
//!
//! SeekCLI is the **environment and evaluator** for RL / evolution experiments,
//! not the trainer (`docs/evaluation/2026-09-12-self-evolution-baseline.md` §5).
//! The three things such a pipeline needs were all already here — a resettable
//! testbed, a decidable reward, a replayable trajectory — but the third was
//! only ever captured under `#[cfg(test)]`, so nothing outside could read it.
//!
//! This module is the pure part: events plus a verdict in, records out. It
//! imports nothing from the agent loop and writes no files.
//!
//! Format contract: `docs/architecture/L7-observability.md` §4.6.3.

use serde::Serialize;

use crate::session::EventPayload;

/// How `reward` was decided. Written out so that adding another verdict style
/// later cannot make old data read as if it used the new one.
const FAIL_TO_PASS: &str = "fail_to_pass";

/// One tool call the model asked for.
#[derive(Debug, Serialize, PartialEq)]
pub struct Action {
  pub text: String,
  pub tool_calls: Vec<ActionCall>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ActionCall {
  pub name: String,
  pub arguments: String,
}

/// One step: what the model saw, and what it did about it.
#[derive(Debug, Serialize, PartialEq)]
pub struct Step {
  pub index: usize,
  /// Everything that arrived since the previous action — the user's prompt on
  /// the first step, tool results after that.
  pub observation: String,
  pub action: Action,
}

/// One task's trajectory. One JSONL line.
#[derive(Debug, Serialize, PartialEq)]
pub struct Record {
  pub task: String,
  pub prompt: String,
  /// `1` passed / `0` did not. The `expect_fail` inversion is already applied,
  /// so a consumer never needs to know which tasks are stated backwards.
  pub reward: u8,
  pub reward_kind: &'static str,
  /// `completed` / `max_iterations` / `interrupted`. A trajectory that hit the
  /// iteration ceiling is not the same thing as one that finished, and training
  /// on them interchangeably would be a mistake.
  pub status: String,
  pub llm_calls: u64,
  pub steps: Vec<Step>,
}

/// Build one record from a task's run.
///
/// Reward stays terminal — exactly one verdict per task — because that is what
/// Fail-to-Pass produces. Spreading it over the steps here would bake a credit
/// assignment decision into the data, and that belongs to whoever consumes it
/// (`docs/architecture/L7-observability.md` §4.6.2).
pub fn record(
  task: &str,
  prompt: &str,
  passed: bool,
  status: &str,
  llm_calls: u64,
  events: &[EventPayload],
) -> Record {
  Record {
    task: task.to_string(),
    prompt: prompt.to_string(),
    reward: u8::from(passed),
    reward_kind: FAIL_TO_PASS,
    status: status.to_string(),
    llm_calls,
    steps: steps_of(prompt, events),
  }
}

/// Fold the event log into (observation, action) pairs.
///
/// A step closes at each assistant message; everything seen since the previous
/// one is its observation. Events that are not model-visible input or output —
/// usage accounting, skill activation — are skipped rather than flattened into
/// the observation, which would put harness bookkeeping in the training data.
/// `prompt` seeds the first observation when the log does not carry it.
///
/// An asymmetry this export surfaced: `chat()` records the user's message into
/// the session itself, while `run_headless` — the path `--bench` uses — leaves
/// that to its caller, so a bench log begins at the first assistant message.
/// Seeding here rather than changing what the loop logs is deliberate: the
/// event log's ordering is the load-bearing L4 invariant, and reordering it to
/// tidy up an export would be a bad trade. The prompt is genuinely what the
/// model saw entering step 0, and `Record::prompt` remains the authority.
fn steps_of(prompt: &str, events: &[EventPayload]) -> Vec<Step> {
  let mut steps = Vec::new();
  let logged_input = events
    .iter()
    .any(|e| matches!(e, EventPayload::UserMessage { .. }));
  let mut observation = if logged_input {
    String::new()
  } else {
    prompt.to_string()
  };

  for event in events {
    match event {
      EventPayload::UserMessage { content, .. } => push_line(&mut observation, content),
      EventPayload::ToolResult { content, .. } => push_line(&mut observation, content),
      // A compaction replaced part of the projection; the summary is what the
      // model actually saw next, so it belongs in the observation.
      EventPayload::Compaction { summary, .. } => push_line(&mut observation, summary),
      EventPayload::SystemPrompt { .. }
      | EventPayload::SkillActivated { .. }
      | EventPayload::Usage(_)
      | EventPayload::Interrupted => {}
      EventPayload::AssistantMessage {
        content,
        tool_calls,
        ..
      } => {
        steps.push(Step {
          index: steps.len(),
          observation: std::mem::take(&mut observation),
          action: Action {
            text: content.clone(),
            tool_calls: tool_calls
              .iter()
              .map(|c| ActionCall {
                name: c.function.name.clone(),
                arguments: c.function.arguments.clone(),
              })
              .collect(),
          },
        });
      }
    }
  }
  steps
}

fn push_line(buffer: &mut String, text: &str) {
  if !buffer.is_empty() {
    buffer.push('\n');
  }
  buffer.push_str(text);
}

/// Serialize records as JSONL. One line per task, in run order.
pub fn to_jsonl(records: &[Record]) -> anyhow::Result<String> {
  let mut out = String::new();
  for r in records {
    out.push_str(&serde_json::to_string(r)?);
    out.push('\n');
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::{FunctionCall, ToolCall};

  fn call(name: &str, arguments: &str) -> ToolCall {
    ToolCall {
      id: "c1".to_string(),
      tool_type: "function".to_string(),
      function: FunctionCall {
        name: name.to_string(),
        arguments: arguments.to_string(),
      },
    }
  }

  fn log() -> Vec<EventPayload> {
    vec![
      EventPayload::SystemPrompt {
        kind: crate::session::PromptKind::Kernel,
        content: "you are an agent".into(),
      },
      EventPayload::UserMessage {
        images: Vec::new(),
        content: "read notes.md".into(),
      },
      EventPayload::AssistantMessage {
        content: String::new(),
        reasoning: None,
        tool_calls: vec![call("read_file", r#"{"path":"notes.md"}"#)],
      },
      EventPayload::ToolResult {
        call_id: "c1".into(),
        content: "no such file".into(),
      },
      EventPayload::AssistantMessage {
        content: "It does not exist yet.".into(),
        reasoning: None,
        tool_calls: vec![],
      },
    ]
  }

  #[test]
  fn a_step_closes_at_each_assistant_message() {
    let steps = steps_of("read notes.md", &log());
    assert_eq!(steps.len(), 2);
    // The prompt is the first observation; the tool result is the second.
    assert_eq!(steps[0].observation, "read notes.md");
    assert_eq!(steps[0].action.tool_calls[0].name, "read_file");
    assert_eq!(steps[1].observation, "no such file");
    assert!(steps[1].action.tool_calls.is_empty());
  }

  /// Harness bookkeeping must not leak into the observation: a consumer would
  /// train on text the model never saw as input.
  #[test]
  fn the_system_prompt_and_accounting_events_are_not_observations() {
    let steps = steps_of("read notes.md", &log());
    for s in &steps {
      assert!(!s.observation.contains("you are an agent"), "{s:?}");
    }
  }

  #[test]
  fn reward_is_terminal_and_already_inverted() {
    let r = record("t", "p", true, "completed", 3, &log());
    assert_eq!(r.reward, 1);
    assert_eq!(r.reward_kind, "fail_to_pass");
    // Exactly one verdict for the whole task -- no per-step reward field to
    // mistake for one.
    let json = match serde_json::to_string(&r) {
      Ok(j) => j,
      Err(e) => panic!("{e}"),
    };
    assert_eq!(json.matches("\"reward\"").count(), 1, "{json}");

    let failed = record("t", "p", false, "max_iterations", 9, &log());
    assert_eq!(failed.reward, 0);
    assert_eq!(failed.status, "max_iterations");
  }

  #[test]
  fn jsonl_is_one_line_per_task() {
    let records = vec![
      record("a", "p", true, "completed", 1, &log()),
      record("b", "p", false, "completed", 2, &log()),
    ];
    let out = match to_jsonl(&records) {
      Ok(o) => o,
      Err(e) => panic!("{e}"),
    };
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    for line in lines {
      let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
      assert!(parsed.is_ok(), "{line}");
    }
  }

  /// The bench path does not log the prompt, so the export seeds it; the REPL
  /// path does, so it must not be duplicated.
  #[test]
  fn the_prompt_seeds_step_zero_only_when_the_log_lacks_it() {
    let unlogged: Vec<EventPayload> = vec![EventPayload::AssistantMessage {
      content: "hi".into(),
      reasoning: None,
      tool_calls: vec![],
    }];
    let seeded = steps_of("do the thing", &unlogged);
    assert_eq!(seeded[0].observation, "do the thing");

    // `log()` already opens with a UserMessage, so seeding would duplicate it.
    let from_log = steps_of("do the thing", &log());
    assert_eq!(from_log[0].observation, "read notes.md");
  }

  #[test]
  fn a_run_with_no_assistant_message_yields_no_steps() {
    // A task the agent never answered must not produce a phantom step whose
    // action is empty -- that reads as "it chose to say nothing".
    let only_input = vec![EventPayload::UserMessage {
      images: Vec::new(),
      content: "hello".into(),
    }];
    assert!(steps_of("hello", &only_input).is_empty());
  }
}
