use anyhow::{Context, Result};
use colored::Colorize;
use serde_json::Value;
use std::process::Stdio;

pub async fn run_shell(args: &Value) -> Result<String> {
  let command = args
    .get("command")
    .and_then(|v| v.as_str())
    .context("Missing 'command' argument")?;

  println!("\n{} {}", "[Agent Executing]".cyan(), command);

  let output = tokio::process::Command::new("sh")
    .arg("-c")
    .arg(command)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .output()
    .await
    .context("Failed to spawn shell command")?;

  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  let mut result = String::new();
  if !stdout.is_empty() {
    result.push_str("STDOUT:\n");
    result.push_str(&stdout);
    result.push('\n');
  }
  if !stderr.is_empty() {
    result.push_str("STDERR:\n");
    result.push_str(&stderr);
    result.push('\n');
  }

  if result.is_empty() {
    result.push_str("Command executed successfully with no output.");
  }

  if output.status.success() {
    Ok(result)
  } else {
    // Even if it failed, we return Ok(result) so the LLM gets the stderr and can retry
    Ok(format!(
      "Command failed with exit code: {}.\n{}",
      output.status, result
    ))
  }
}
