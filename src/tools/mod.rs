use anyhow::Result;
use serde_json::Value;

pub mod fs;
pub mod meta;
pub mod registry;
pub mod shell;

pub struct ToolDispatcher;

impl ToolDispatcher {
  pub fn new() -> Self {
    Self
  }

  pub async fn execute(&self, name: &str, arguments: &str) -> Result<String> {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);

    match name {
      "read_file" => fs::read_file(&args).await,
      "write_file" => fs::write_file(&args).await,
      "list_dir" => fs::list_dir(&args).await,
      "run_shell" => shell::run_shell(&args).await,
      "create_skill" => meta::create_skill(&args).await,
      _ => anyhow::bail!("Unknown tool: {}", name),
    }
  }
}
