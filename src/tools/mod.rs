use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use result::{ToolKind, ToolResult};

pub mod approval;
pub mod ask;
pub mod audit;
pub mod edit;
pub mod fs;
pub mod inspect;
pub mod jobs;
pub mod meta;
pub mod offload;
pub mod path_security;
pub mod policy;
pub mod registry;
pub mod result;
pub mod search;
pub mod shell;

/// Default ceiling for a single tool call.
///
/// A tool that hangs used to hang the whole agent: no output, no error, and
/// for an unattended `--run-task` no way to tell it apart from slow work.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// `run_shell` gets longer: builds and test suites legitimately take minutes,
/// and killing a real build at two minutes would be worse than waiting.
const SHELL_TIMEOUT: Duration = Duration::from_secs(600);

fn timeout_for(tool: &str) -> Duration {
  match tool {
    "run_shell" => SHELL_TIMEOUT,
    _ => DEFAULT_TIMEOUT,
  }
}

pub struct ToolDispatcher;

impl ToolDispatcher {
  pub fn new() -> Self {
    Self
  }

  /// The guarded pipeline every tool call passes through:
  ///
  /// ```text
  /// parse args -> policy gate -> deadline -> execute -> audit
  /// ```
  ///
  /// Keeping it as one ordered function rather than a composable chain is
  /// deliberate: with eight tools and five stages, a plugin-style middleware
  /// stack would add indirection without ever being reconfigured. What matters
  /// is that there is exactly *one* path, so no tool can bypass a stage.
  pub async fn execute(&self, name: &str, arguments: &str) -> ToolResult {
    self
      .execute_with(name, arguments, None, |args| async move {
        Self::run(name, &args).await
      })
      .await
  }

  /// The pipeline, with the execution step supplied by the caller.
  ///
  /// Exists so MCP tools traverse the *same* gate, deadline and audit path as
  /// built-ins. A capability that arrives from configuration must not become a
  /// way around `--read-only`; giving foreign tools their own execute path is
  /// exactly how that happens.
  pub async fn execute_with<F, Fut>(
    &self,
    name: &str,
    arguments: &str,
    declared_read_only: Option<bool>,
    run: F,
  ) -> ToolResult
  where
    F: FnOnce(Value) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
  {
    // A malformed arguments payload used to be silently coerced to `Null`,
    // which then surfaced as a confusing "missing argument" error. Surface it
    // explicitly so Error Recovery can hand the model an actionable hint.
    let args: Value = match serde_json::from_str(arguments) {
      Ok(v) => v,
      Err(e) => {
        return ToolResult::bad_args(format!("arguments for `{}` is not valid JSON: {}", name, e));
      }
    };

    // Single gate for every tool: mode -> path -> command.
    // `run_shell` re-reads the command verdict inside shell.rs to drive the
    // interactive prompt; `check` here is what guarantees no tool bypasses it.
    match policy::check_with(name, &args, declared_read_only) {
      policy::Verdict::Allow => {}
      policy::Verdict::Deny(reason) => {
        audit::record(name, &args, audit::Outcome::Denied, &reason);
        return ToolResult::denied(format!("[MODE DENIED] {reason}"));
      }
      // Ask is resolved where the interaction lives (shell.rs), so the
      // prompt can show the actual command.
      policy::Verdict::Ask(_) => {}
    }

    let limit = timeout_for(name);
    let result = match tokio::time::timeout(limit, run(args.clone())).await {
      Ok(outcome) => ToolResult::from_legacy(outcome),
      Err(_) => ToolResult::timed_out(format!(
        "`{}` exceeded {:?}. If this is legitimately long-running, run it in \
         the background instead of waiting for it inline.",
        name, limit
      )),
    };

    if result.kind != ToolKind::Ok {
      audit::record(
        name,
        &args,
        audit::Outcome::Denied,
        result.kind.prefix().trim(),
      );
    }
    result
  }

  async fn run(name: &str, args: &Value) -> Result<String> {
    match name {
      "read_file" => fs::read_file(args).await,
      "write_file" => fs::write_file(args).await,
      "edit_file" => fs::edit_file(args).await,
      "list_dir" => fs::list_dir(args).await,
      "glob" => search::glob(args).await,
      "grep" => search::grep(args).await,
      "run_shell" => shell::run_shell(args).await,
      "job_list" => jobs::job_list(args).await,
      "job_output" => jobs::job_output(args).await,
      "job_kill" => jobs::job_kill(args).await,
      "create_skill" => meta::create_skill(args).await,
      "ask_user_question" => ask::ask_user_question(args).await,
      _ => anyhow::bail!("{}", registry::unknown_tool_message(name)),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // Holding the guard across the await is the point: the global policy mode
  // must stay set for the whole call. Safe here because only tests take this
  // lock and the test runtime cannot deadlock on it.
  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn restricted_mode_denial_is_reported_to_the_model_not_raised_as_an_error() {
    let _guard = crate::testsync::lock();
    policy::set_mode(policy::Mode::ReadOnly);
    let d = ToolDispatcher::new();
    let out = d
      .execute("write_file", r#"{"path":"x","content":"y"}"#)
      .await;
    policy::set_mode(policy::Mode::Normal);

    // The refusal reaches the model as a classified result, not as an error
    // that would abort the turn -- and `Denied` is not `is_failure`, so it
    // does not drag the loop into Two-Stage replanning around the policy.
    assert_eq!(out.kind, ToolKind::Denied);
    assert!(!out.kind.is_failure());
    assert!(out.render().starts_with("[MODE DENIED]"), "got: {}", out);
  }

  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn malformed_arguments_are_reported_rather_than_coerced() {
    let _guard = crate::testsync::lock();
    policy::set_mode(policy::Mode::Normal);
    let d = ToolDispatcher::new();
    let out = d.execute("read_file", "not json at all").await;
    assert_eq!(out.kind, ToolKind::BadArgs);
    assert!(
      out.kind.is_failure(),
      "the model must be told to fix its JSON"
    );
    assert!(out.render().starts_with("[BAD ARGS]"), "got: {}", out);
  }

  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn an_unknown_tool_is_a_failure_not_a_panic() {
    let _guard = crate::testsync::lock();
    policy::set_mode(policy::Mode::Normal);
    let out = ToolDispatcher::new().execute("no_such_tool", "{}").await;
    assert_eq!(out.kind, ToolKind::Failed);
  }

  /// The deadline must actually fire. A tool that hangs used to hang the whole
  /// agent: no output, no error, and for an unattended run no way to tell it
  /// apart from slow work.
  #[allow(clippy::await_holding_lock)]
  #[tokio::test(start_paused = true)]
  async fn a_hanging_tool_hits_its_deadline_instead_of_hanging_the_agent() {
    let _guard = crate::testsync::lock();
    policy::set_mode(policy::Mode::Normal);
    approval::set_interaction(approval::Interaction::AutoApprove);

    // `sleep` well past the shell ceiling. With a paused clock tokio advances
    // time itself, so this costs no wall-clock seconds.
    let out = ToolDispatcher::new()
      .execute("run_shell", r#"{"command":"sleep 3600"}"#)
      .await;

    approval::set_interaction(approval::Interaction::Prompt);
    assert_eq!(out.kind, ToolKind::TimedOut);
    assert!(
      out.kind.is_failure(),
      "a deadline miss should trigger recovery"
    );
    assert!(
      out.render().contains("background"),
      "the model needs to be told what to do instead: {}",
      out
    );
  }

  #[test]
  fn shell_gets_a_longer_deadline_than_the_rest() {
    // Builds and test suites legitimately take minutes; killing a real build
    // at the default two minutes would be worse than waiting for it.
    assert!(timeout_for("run_shell") > timeout_for("read_file"));
    assert_eq!(timeout_for("read_file"), DEFAULT_TIMEOUT);
  }
}
