use anyhow::Result;
use serde_json::Value;

pub mod approval;
pub mod edit;
pub mod fs;
pub mod meta;
pub mod offload;
pub mod path_security;
pub mod registry;
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

    match name {
      "read_file" => fs::read_file(&args).await,
      "write_file" => fs::write_file(&args).await,
      "edit_file" => fs::edit_file(&args).await,
      "list_dir" => fs::list_dir(&args).await,
      "run_shell" => shell::run_shell(&args).await,
      "create_skill" => meta::create_skill(&args).await,
      _ => anyhow::bail!("Unknown tool: {}", name),
    }
  }
}
