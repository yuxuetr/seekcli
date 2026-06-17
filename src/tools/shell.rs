use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::approval;

/// Delay before showing the progress spinner. Below this threshold, a
/// command finishes too quickly for the spinner to be useful.
const SPINNER_DELAY: Duration = Duration::from_millis(800);

pub async fn run_shell(args: &Value) -> Result<String> {
  let command = args
    .get("command")
    .and_then(|v| v.as_str())
    .context("Missing 'command' argument")?;

  if let Some(reason) = approval::is_dangerous(command) {
    if !approval::confirm(command, reason) {
      println!("{} command denied by user.", "[Agent]".red());
      return Ok(format!(
        "[USER DENIED] User refused to run dangerous command ({reason}): {command}\n\
         Do not retry. Suggest a safer alternative or ask the user how to proceed."
      ));
    }
    println!("{} command approved by user.", "[Agent]".green());
  }

  println!("\n{} {}", "[Agent Executing]".cyan(), command);

  // Spawn a side task that activates a progress spinner only if the command
  // takes longer than SPINNER_DELAY. The spinner clears itself when the
  // main task signals completion via the shared atomic flag.
  let stop_flag = Arc::new(AtomicBool::new(false));
  let spinner_task = spawn_delayed_spinner(command.to_string(), stop_flag.clone());

  let output_result = tokio::process::Command::new("sh")
    .arg("-c")
    .arg(command)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .output()
    .await;

  // Signal the spinner to stop and wait for it to clear cleanly.
  stop_flag.store(true, Ordering::SeqCst);
  let _ = spinner_task.await;

  let output = output_result.context("Failed to spawn shell command")?;

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

  // Offload bulky output (logs, large dumps) to a temp file, keeping a
  // head+tail preview so the context isn't flooded. Ephemeral output, so no
  // source hint.
  let result = super::offload::offload(result, None).await;

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

/// Spawn a task that, after `SPINNER_DELAY`, displays a progress spinner
/// until `stop_flag` is set. The spinner clears itself on stop.
fn spawn_delayed_spinner(
  command: String,
  stop_flag: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    tokio::time::sleep(SPINNER_DELAY).await;
    if stop_flag.load(Ordering::SeqCst) {
      // Command finished before the delay; nothing to show.
      return;
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
      ProgressStyle::default_spinner()
        .template("  {spinner:.cyan} {elapsed_precise} running: {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_message(truncate_for_spinner(&command));
    pb.enable_steady_tick(Duration::from_millis(120));

    // Poll the stop flag rather than blocking, so the spinner reacts within
    // ~100ms of the command finishing.
    while !stop_flag.load(Ordering::SeqCst) {
      tokio::time::sleep(Duration::from_millis(100)).await;
    }
    pb.finish_and_clear();
  })
}

fn truncate_for_spinner(s: &str) -> String {
  const MAX: usize = 60;
  if s.chars().count() <= MAX {
    return s.to_string();
  }
  let cut: String = s.chars().take(MAX).collect();
  format!("{}…", cut)
}
