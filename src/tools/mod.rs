use anyhow::Result;
use serde_json::Value;

pub mod approval;
pub mod audit;
pub mod edit;
pub mod fs;
pub mod meta;
pub mod offload;
pub mod path_security;
pub mod policy;
pub mod registry;
pub mod search;
pub mod shell;

pub struct ToolDispatcher;

impl ToolDispatcher {
  pub fn new() -> Self {
    Self
  }

  pub async fn execute(&self, name: &str, arguments: &str) -> Result<String> {
    // A malformed arguments payload used to be silently coerced to `Null`,
    // which then surfaced as a confusing "missing argument" error. Surface it
    // explicitly so Error Recovery can hand the model an actionable hint.
    let args: Value = match serde_json::from_str(arguments) {
      Ok(v) => v,
      Err(e) => {
        return Ok(format!(
          "[BAD ARGS] arguments for `{}` is not valid JSON: {}",
          name, e
        ));
      }
    };

    // Single gate for every tool: mode -> path -> command.
    // `run_shell` re-reads the command verdict inside shell.rs to drive the
    // interactive prompt; `check` here is what guarantees no tool bypasses it.
    match policy::check(name, &args) {
      policy::Verdict::Allow => {}
      policy::Verdict::Deny(reason) => {
        let outcome = audit::Outcome::Denied;
        audit::record(name, &args, outcome, &reason);
        return Ok(format!("[MODE DENIED] {reason}"));
      }
      // Ask is resolved where the interaction lives (shell.rs), so the
      // prompt can show the actual command.
      policy::Verdict::Ask(_) => {}
    }

    match name {
      "read_file" => fs::read_file(&args).await,
      "write_file" => fs::write_file(&args).await,
      "edit_file" => fs::edit_file(&args).await,
      "list_dir" => fs::list_dir(&args).await,
      "glob" => search::glob(&args).await,
      "grep" => search::grep(&args).await,
      "run_shell" => shell::run_shell(&args).await,
      "create_skill" => meta::create_skill(&args).await,
      _ => anyhow::bail!("Unknown tool: {}", name),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn restricted_mode_denial_is_reported_to_the_model_not_raised_as_an_error() {
    policy::set_mode(policy::Mode::ReadOnly);
    let d = ToolDispatcher::new();
    let out = d
      .execute("write_file", r#"{"path":"x","content":"y"}"#)
      .await;
    policy::set_mode(policy::Mode::Normal);

    // A hard Err would abort the turn; the model should instead see the
    // refusal and adapt, the same way it does for [USER DENIED].
    let text = match out {
      Ok(t) => t,
      Err(e) => panic!("denial must be a tool result, not an error: {}", e),
    };
    assert!(text.starts_with("[MODE DENIED]"), "got: {}", text);
  }

  #[tokio::test]
  async fn malformed_arguments_are_reported_rather_than_coerced() {
    policy::set_mode(policy::Mode::Normal);
    let d = ToolDispatcher::new();
    let out = match d.execute("read_file", "not json at all").await {
      Ok(t) => t,
      Err(e) => panic!("bad args must be a tool result: {}", e),
    };
    assert!(out.starts_with("[BAD ARGS]"), "got: {}", out);
  }
}
