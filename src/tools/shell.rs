use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::{approval, audit, policy};

/// Delay before showing the progress spinner. Below this threshold, a
/// command finishes too quickly for the spinner to be useful.
const SPINNER_DELAY: Duration = Duration::from_millis(800);

/// How often a running command checks whether the user pressed Ctrl-C.
const CANCEL_POLL: Duration = Duration::from_millis(120);

/// The interrupt flag the REPL's Ctrl-C watcher sets.
///
/// Shared rather than passed down: cancellation has to reach the innermost
/// blocking operation, and threading an `Arc` through every tool signature to
/// serve one tool would cost more than it explains. Same shape as the policy
/// mode and the approval mode.
static INTERRUPT: std::sync::Mutex<Option<Arc<AtomicBool>>> = std::sync::Mutex::new(None);

pub fn set_interrupt(flag: Arc<AtomicBool>) {
  if let Ok(mut guard) = INTERRUPT.lock() {
    *guard = Some(flag);
  }
}

fn interrupted() -> bool {
  match INTERRUPT.lock() {
    Ok(guard) => guard.as_ref().is_some_and(|f| f.load(Ordering::SeqCst)),
    Err(_) => false,
  }
}

/// Resolves once the user has asked to stop.
///
/// Deliberately does **not** clear the flag: the agent loop's own check is
/// what ends the turn, and consuming it here would kill the command while
/// leaving the loop to carry on as if nothing happened.
/// Kill the command's whole process group.
///
/// Shelling out to `kill` rather than taking a `libc` dependency for one call:
/// the group id equals the child's pid (see `process_group(0)`), and `kill -KILL
/// -<pgid>` is exactly what a shell would do. Best effort — the child is killed
/// directly regardless.
pub(crate) fn kill_process_group(child: &tokio::process::Child) {
  let Some(pid) = child.id() else {
    return;
  };
  let _ = std::process::Command::new("kill")
    .arg("-KILL")
    .arg(format!("-{pid}"))
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status();
}

async fn cancelled() {
  loop {
    if interrupted() {
      return;
    }
    tokio::time::sleep(CANCEL_POLL).await;
  }
}

pub async fn run_shell(args: &Value) -> Result<String> {
  let command = args
    .get("command")
    .and_then(|v| v.as_str())
    .context("Missing 'command' argument")?;

  // Sub-command aware: `ls; rm -rf /tmp/x` is judged by the `rm`, not the `ls`.
  // Background dispatch happens before the spinner and the blocking wait, but
  // after nothing else -- a backgrounded command still passes every gate.
  let background = args
    .get("background")
    .and_then(Value::as_bool)
    .unwrap_or(false);

  match policy::classify_command(command) {
    approval::Decision::Allow => {}
    approval::Decision::Deny(reason) => {
      eprintln!("{} command blocked by policy: {}", "[Agent]".red(), reason);
      return Ok(format!(
        "[USER DENIED] Command blocked by policy ({reason}): {command}\n\
         This command is not permitted. Do not retry; propose a safer alternative."
      ));
    }
    approval::Decision::Ask(reason) => {
      if !approval::confirm(command, &reason) {
        eprintln!("{} command denied by user.", "[Agent]".red());
        return Ok(format!(
          "[USER DENIED] User refused to run dangerous command ({reason}): {command}\n\
           Do not retry. Suggest a safer alternative or ask the user how to proceed."
        ));
      }
      eprintln!("{} command approved by user.", "[Agent]".green());
    }
  }

  // Writing outside the workspace was previously unchecked for shell, so the
  // file-tool whitelist only ever stopped honest mistakes. Not a full shell
  // parse -- see security-model.md -- but a redirect or a write verb aimed at
  // an absolute / `~` path now costs a prompt.
  let escaping = policy::escaping_write_targets(command);
  if !escaping.is_empty() {
    let reason = format!("writes outside the workspace: {}", escaping.join(", "));
    if !approval::confirm(command, &reason) {
      audit::record_command(command, audit::Outcome::Denied, &reason);
      eprintln!("{} out-of-workspace write denied.", "[Agent]".red());
      return Ok(format!(
        "[PATH DENIED] Refused ({reason}): {command}\n\
         Do not retry. Write inside the working directory, or ask the user to \
         run SeekCLI from the intended directory."
      ));
    }
  }

  audit::record_command(command, audit::Outcome::Executed, "");

  if background {
    return super::jobs::spawn(command).await;
  }

  eprintln!("\n{} {}", "[Agent Executing]".cyan(), command);

  // Spawn a side task that activates a progress spinner only if the command
  // takes longer than SPINNER_DELAY. The spinner clears itself when the
  // main task signals completion via the shared atomic flag.
  let stop_flag = Arc::new(AtomicBool::new(false));
  let spinner_task = spawn_delayed_spinner(command.to_string(), stop_flag.clone());

  // Spawned rather than `.output()`ed so the child stays reachable: Ctrl-C
  // used to return control to the REPL while the command kept running with
  // nobody watching it -- a `sleep 60` or a `cargo build` would outlive the
  // turn that started it.
  let mut child = tokio::process::Command::new("sh")
    .arg("-c")
    .arg(command)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    // Its own process group, so cancelling can reach the whole command tree.
    // `sh` execs away for a simple command, so killing the child was enough
    // there — but anything `sh` cannot exec (a pipeline, `&&`, `time`, a loop)
    // leaves it forking, and killing only `sh` orphans the grandchild. Measured:
    // `time sleep 20` kept the stdout pipe open for the full 19 seconds after
    // its `sh` was killed, and went on running afterwards.
    .process_group(0)
    .spawn()
    .context("Failed to spawn shell command")?;

  // Drain the pipes concurrently. A child that fills a pipe buffer blocks
  // forever if nobody is reading, so this cannot wait for exit first.
  let mut child_stdout = child.stdout.take();
  let mut child_stderr = child.stderr.take();
  let out_task = tokio::spawn(async move {
    let mut buf = Vec::new();
    if let Some(pipe) = child_stdout.as_mut() {
      let _ = tokio::io::AsyncReadExt::read_to_end(pipe, &mut buf).await;
    }
    buf
  });
  let err_task = tokio::spawn(async move {
    let mut buf = Vec::new();
    if let Some(pipe) = child_stderr.as_mut() {
      let _ = tokio::io::AsyncReadExt::read_to_end(pipe, &mut buf).await;
    }
    buf
  });

  let mut was_cancelled = false;
  let status = tokio::select! {
    status = child.wait() => status.context("waiting for shell command")?,
    _ = cancelled() => {
      was_cancelled = true;
      kill_process_group(&child);
      // Still kill the child itself: `kill(1)` may be missing, and the group
      // kill is the addition rather than the replacement.
      let _ = child.start_kill();
      child.wait().await.context("reaping interrupted shell command")?
    }
  };

  // Signal the spinner to stop and wait for it to clear cleanly.
  stop_flag.store(true, Ordering::SeqCst);
  let _ = spinner_task.await;

  // Draining is bounded when the command was cancelled. The group kill should
  // have released the pipes, but "should" is what the previous version also
  // assumed — and a stuck drain is indistinguishable, from the user's side,
  // from Ctrl-C not working at all.
  let (stdout_bytes, stderr_bytes) = if was_cancelled {
    let drain = tokio::time::Duration::from_millis(500);
    (
      tokio::time::timeout(drain, out_task)
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default(),
      tokio::time::timeout(drain, err_task)
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default(),
    )
  } else {
    (
      out_task.await.unwrap_or_default(),
      err_task.await.unwrap_or_default(),
    )
  };

  if was_cancelled {
    eprintln!("{} command interrupted by user.", "[Agent]".yellow());
    let partial = String::from_utf8_lossy(&stdout_bytes);
    return Ok(format!(
      "[USER DENIED] Command interrupted by the user: {command}\n\
       Do not retry it. Partial output before it was stopped:\n{}",
      super::offload::offload(partial.into_owned(), Some("interrupted command")).await
    ));
  }

  let output = std::process::Output {
    status,
    stdout: stdout_bytes,
    stderr: stderr_bytes,
  };

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

#[cfg(test)]
mod tests {
  use super::*;
  /// Platform probe kept as a test: `process_group(0)` must actually put the
  /// child in its own group, or `kill -KILL -<pid>` targets the wrong thing.
  #[tokio::test]
  async fn spawned_commands_get_their_own_process_group() {
    let child = tokio::process::Command::new("sh")
      .arg("-c")
      .arg("sleep 2")
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .process_group(0)
      .spawn();
    let mut child = match child {
      Ok(c) => c,
      Err(e) => panic!("spawn failed: {e}"),
    };
    let pid = child.id().unwrap_or(0);
    let out = std::process::Command::new("ps")
      .args(["-o", "pgid=", "-p", &pid.to_string()])
      .output();
    let pgid: u32 = match out {
      Ok(o) => String::from_utf8_lossy(&o.stdout)
        .trim()
        .parse()
        .unwrap_or(0),
      Err(e) => panic!("ps failed: {e}"),
    };
    let _ = child.kill().await;
    assert_eq!(
      pgid, pid,
      "process_group(0) did not take effect: child pid {pid} is in group {pgid}"
    );
  }
}
