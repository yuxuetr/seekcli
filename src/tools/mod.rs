use anyhow::Result;
use serde_json::Value;

pub mod approval;
pub mod edit;
pub mod fs;
pub mod meta;
pub mod offload;
pub mod path_security;
pub mod registry;
pub mod search;
pub mod shell;

/// Whether mutating tools are permitted in this run.
///
/// A narrow, tool-name-based gate rather than the full `PolicyGate` designed
/// in `docs/architecture/L3-security.md` §4.1 — that lands in stage 24 and
/// will absorb this along with Plan Mode. Implemented here now so
/// `--read-only` actually enforces something instead of being a flag that
/// merely documents an intention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
  Normal,
  ReadOnly,
}

static EXEC_MODE: std::sync::Mutex<ExecMode> = std::sync::Mutex::new(ExecMode::Normal);

pub fn set_exec_mode(mode: ExecMode) {
  if let Ok(mut guard) = EXEC_MODE.lock() {
    *guard = mode;
  }
}

fn exec_mode() -> ExecMode {
  match EXEC_MODE.lock() {
    Ok(g) => *g,
    // A poisoned lock must not silently unlock writes.
    Err(_) => ExecMode::ReadOnly,
  }
}

/// Tools that can change something outside the process.
///
/// `run_shell` counts: a shell command's effects cannot be known without
/// parsing it, so read-only mode refuses the whole tool rather than guessing.
/// That is stricter than ideal, and deliberately so — the alternative is a
/// gate that a `>` redirect walks straight through.
fn is_mutating(tool: &str) -> bool {
  matches!(
    tool,
    "write_file" | "edit_file" | "run_shell" | "create_skill"
  )
}

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

    if exec_mode() == ExecMode::ReadOnly && is_mutating(name) {
      return Ok(format!(
        "[MODE DENIED] `{}` is not available in read-only mode. \
         Investigate and report instead of changing anything; do not retry this call.",
        name
      ));
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

  #[test]
  fn read_only_mode_refuses_every_mutating_tool() {
    // run_shell is included on purpose: a command's effects cannot be known
    // without parsing it, and a gate a `>` redirect walks through is no gate.
    for t in ["write_file", "edit_file", "run_shell", "create_skill"] {
      assert!(is_mutating(t), "{} must be gated in read-only mode", t);
    }
    for t in ["read_file", "list_dir", "glob", "grep"] {
      assert!(!is_mutating(t), "{} is a pure read", t);
    }
  }

  #[tokio::test]
  async fn read_only_denial_is_reported_to_the_model_not_raised_as_an_error() {
    set_exec_mode(ExecMode::ReadOnly);
    let d = ToolDispatcher::new();
    let out = d
      .execute("write_file", r#"{"path":"x","content":"y"}"#)
      .await;
    set_exec_mode(ExecMode::Normal);

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
    set_exec_mode(ExecMode::Normal);
    let d = ToolDispatcher::new();
    let out = match d.execute("read_file", "not json at all").await {
      Ok(t) => t,
      Err(e) => panic!("bad args must be a tool result: {}", e),
    };
    assert!(out.starts_with("[BAD ARGS]"), "got: {}", out);
  }
}
