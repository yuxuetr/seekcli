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

/// Raise the interrupt the loop polls, for a caller that owns the terminal
/// rather than the flag.
///
/// The approval prompt is the one place a user can press Ctrl-C while the
/// REPL's watcher is not the thing reading the key — see
/// `approval::confirm_interactive`.
pub(crate) fn request_interrupt() {
  if let Ok(guard) = INTERRUPT.lock()
    && let Some(flag) = guard.as_ref()
  {
    flag.store(true, Ordering::SeqCst);
  }
}

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
  kill_pgid(pid);
}

/// The same kill, by pid alone.
///
/// Split out for `Cleanup`, which runs after the `Child` is gone: a `Drop`
/// impl cannot borrow something the dropped future already owns.
fn kill_pgid(pid: u32) {
  let _ = std::process::Command::new("kill")
    .arg("-KILL")
    .arg(format!("-{pid}"))
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status();
}

/// Stops the spinner and the command when this scope ends **however it ends**.
///
/// The tool deadline in `tools::mod` is
/// `tokio::time::timeout(limit, run(args))`, which on expiry *drops* this
/// future rather than returning through it. Everything written after the
/// `select!` in `run_shell` is then simply never reached — and both things it
/// does were unbounded:
///
/// * the spinner is a separate `tokio::spawn`ed task polling a flag nobody
///   would ever set, so it went on redrawing stderr every 120ms forever,
///   painting over the model's answer and over the next approval prompt;
/// * the child was never killed, so the whole process tree ran on as an orphan.
///
/// Measured, not reasoned: a `raco pkg install` whose 600s cap had already
/// fired was still alive at 17m33s, while a spinner on screen still named it.
///
/// `Drop` is the only code a dropped future is guaranteed to run, so the
/// cleanup lives here instead of in a line that has to be *reached*. The happy
/// path still stops the spinner explicitly before joining it — that ordering is
/// a different requirement, and setting the flag twice costs nothing.
struct Cleanup {
  stop: Arc<AtomicBool>,
  done: Arc<tokio::sync::Notify>,
  /// `Some` only while the child may still be running. Cleared once it is
  /// reaped, so a normal command does not pay for a `kill` that could only
  /// target a process which has already exited.
  pgid: Option<u32>,
}

impl Cleanup {
  fn reaped(&mut self) {
    self.pgid = None;
  }
}

impl Drop for Cleanup {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::SeqCst);
    self.done.notify_one();
    if let Some(pid) = self.pgid {
      kill_pgid(pid);
    }
  }
}

/// Resolves once the user has asked to stop.
///
/// Deliberately does **not** clear the flag: the agent loop's own check is
/// what ends the turn, and consuming it here would kill the command while
/// leaving the loop to carry on as if nothing happened.
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
  let spinner_done = Arc::new(tokio::sync::Notify::new());
  let spinner_task =
    spawn_delayed_spinner(command.to_string(), stop_flag.clone(), spinner_done.clone());
  // Armed before the child exists so a spawn failure still stops the spinner.
  let mut cleanup = Cleanup {
    stop: stop_flag.clone(),
    done: spinner_done.clone(),
    pgid: None,
  };

  // Spawned rather than `.output()`ed so the child stays reachable: Ctrl-C
  // used to return control to the REPL while the command kept running with
  // nobody watching it -- a `sleep 60` or a `cargo build` would outlive the
  // turn that started it.
  let mut child = tokio::process::Command::new("sh")
    .arg("-c")
    .arg(command)
    // EOF, not the terminal. An inherited stdin is a tty, and a command that
    // asks a question on it waits for an answer that can never come: nobody is
    // typing, and the question itself is invisible because stdout is a pipe
    // this function only reads after the child exits. Measured on `raco pkg
    // install rackunit`, which stopped at its dependency prompt having consumed
    // 0.36 seconds of CPU.
    //
    // The wait is bounded -- `tools::mod::timeout_for` caps `run_shell` at 600s
    // -- but 600 seconds of a turn is spent for nothing, and the timeout path
    // drops this future, which orphans the child (see `kill_process_group`'s
    // caller below: it never runs on that path).
    //
    // With `/dev/null` the read returns EOF immediately and the command fails
    // with its own diagnostic, which is the one thing the model can act on. The
    // background path (`jobs::spawn`) has always done this; the difference was
    // an oversight, not a decision.
    .stdin(Stdio::null())
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
  cleanup.pgid = child.id();

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

  // Reaped: from here on the guard has nothing left to kill.
  cleanup.reaped();

  // Signal the spinner to stop and wait for it to clear cleanly. The notify is
  // what makes "wait" bounded by how long clearing takes rather than by the
  // delay the spinner was still sleeping out.
  stop_flag.store(true, Ordering::SeqCst);
  spinner_done.notify_one();
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
  finished: Arc<tokio::sync::Notify>,
) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    // The delay must be *interruptible*, not merely checked afterwards. It was
    // a plain `sleep(SPINNER_DELAY)`, and the caller joins this task before
    // returning — so every `run_shell` paid the full 800ms even when the
    // command took 3ms. It was invisible because the number looked like the
    // command's own cost: a sub-agent's trace showed `run_shell` at 805, 807,
    // 804, 830ms, a suspiciously flat line that turned out to be this.
    //
    // `notify_one` rather than `notify_waiters`: it leaves a permit if this
    // task has not reached the await yet, so a command that finishes before
    // the task is first polled still wakes it.
    tokio::select! {
      _ = tokio::time::sleep(SPINNER_DELAY) => {}
      _ = finished.notified() => return,
    }
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

  /// A fast command must return fast. The spinner is a *progress* affordance;
  /// making every caller wait for its delay turns it into a tax.
  ///
  /// Measured before the fix: a sub-agent's trace showed ten `run_shell` spans
  /// at 805/807/804/830ms — about 8s of dead time in one delegation, and
  /// ~800ms on every interactive shell command the user ran.
  #[tokio::test]
  async fn a_fast_command_does_not_wait_out_the_spinner_delay() {
    crate::tools::approval::set_interaction(crate::tools::approval::Interaction::AutoApprove);
    let started = std::time::Instant::now();
    let out = run_shell(&serde_json::json!({ "command": "echo hi" })).await;
    let elapsed = started.elapsed();

    match out {
      Ok(text) => assert!(text.contains("hi"), "command did not run: {text}"),
      Err(e) => panic!("run_shell failed: {e}"),
    }
    assert!(
      elapsed < SPINNER_DELAY / 2,
      "`echo hi` took {:?}; the spinner delay is {:?} and must not be charged \
       to commands that finish before it",
      elapsed,
      SPINNER_DELAY
    );
  }
  /// The other side of the same fix: a command that outlives the delay must
  /// still take the spinner branch, and must not be cut short by the notify.
  /// Making the wait interruptible is only correct if the interruption cannot
  /// arrive early.
  #[tokio::test]
  async fn a_slow_command_still_runs_to_completion() {
    crate::tools::approval::set_interaction(crate::tools::approval::Interaction::AutoApprove);
    let started = std::time::Instant::now();
    let out = run_shell(&serde_json::json!({ "command": "sleep 1; echo done" })).await;
    let elapsed = started.elapsed();

    match out {
      Ok(text) => assert!(text.contains("done"), "output lost: {text}"),
      Err(e) => panic!("run_shell failed: {e}"),
    }
    assert!(
      elapsed >= std::time::Duration::from_millis(900),
      "returned after {elapsed:?}; the command sleeps 1s, so this would mean \
       the wait was cut short"
    );
    // And it must not have paid the delay *on top* of its own second.
    assert!(
      elapsed < std::time::Duration::from_millis(1600),
      "took {elapsed:?} for a 1s command — the spinner delay is being charged \
       again"
    );
  }

  /// A command that reads stdin must see EOF, never the terminal.
  ///
  /// What this guards costs a full `SHELL_TIMEOUT` (600s) per occurrence, and
  /// leaves an orphan behind: the timeout lives in `tools::mod` and works by
  /// dropping this future, so the kill below never runs on that path. Measured
  /// on `raco pkg install rackunit`, which stopped at its `--deps search-ask`
  /// prompt having consumed 0.36s of CPU -- and was still running 17 minutes
  /// later, long past the 10-minute cap that had already fired. The prompt was
  /// never visible: stdout is a pipe this function only reads once the child
  /// exits, so the question and the answer deadlocked on each other.
  ///
  /// `stat` both paths and compare: device+inode differs for a pipe or a tty
  /// and matches for `/dev/null`. GNU spells it `-c`, BSD `-f`, and the release
  /// ships both platforms.
  ///
  /// **This door is blind when the runner's own stdin is already `/dev/null`**,
  /// which is the case under CI and under most non-interactive shells — the
  /// child then inherits `/dev/null` and looks correct without the fix. No
  /// black-box test can do better: inheriting and redirecting are only
  /// distinguishable when there is something to inherit. It does red, in
  /// 0.02s, whenever `cargo test` is run with a live stdin — a developer's
  /// terminal, which is where the line would get deleted in the first place.
  ///
  /// Both halves measured with the fix reverted: `sleep 20 | cargo test` reds,
  /// a plain `cargo test` here does not. An earlier version asserted on how
  /// long `cat` blocked instead; it reds in 24.86s under the pipe and is
  /// equally blind without it, so it cost 24 seconds and bought nothing.
  #[tokio::test]
  async fn a_command_reading_stdin_gets_dev_null_not_the_terminal() {
    crate::tools::approval::set_interaction(crate::tools::approval::Interaction::AutoApprove);
    let out = run_shell(&serde_json::json!({
      "command":
        "for f in /dev/fd/0 /dev/null; \
         do stat -c '%d %i' $f 2>/dev/null || stat -f '%d %i' $f; done"
    }))
    .await;

    let text = match out {
      Ok(t) => t,
      Err(e) => panic!("run_shell failed: {e}"),
    };
    let ids: Vec<&str> = text
      .lines()
      .map(str::trim)
      .filter(|l| !l.is_empty())
      .rev()
      .take(2)
      .collect();
    assert_eq!(ids.len(), 2, "probe produced no ids:\n{text}");
    assert_eq!(
      ids[0], ids[1],
      "the child's stdin is not /dev/null -- it inherited ours, and any \
       command that asks a question on it will block forever:\n{text}"
    );
  }

  /// Dropping the future must take the whole command with it.
  ///
  /// This is exactly how the tool deadline in `tools::mod` ends a command:
  /// `tokio::time::timeout` drops the future rather than returning through it,
  /// so everything written after the `select!` is skipped. Before `Cleanup`,
  /// that skipped the kill *and* the spinner stop — measured on a
  /// `raco pkg install` still alive at 17m33s against a 600s cap, with its
  /// spinner still redrawing the screen. The orphan is the half that can be
  /// observed from a test; the spinner rides on the same `Drop`.
  ///
  /// `sleep 45; echo <marker>` rather than a bare `sleep`: `sh` execs away for
  /// a simple command, and then the marker would not be in anyone's argv to
  /// look for. A compound command keeps `sh` alive, which is also the case
  /// where killing only the child would leave the grandchild running.
  #[tokio::test]
  async fn a_dropped_command_does_not_outlive_its_future() {
    crate::tools::approval::set_interaction(crate::tools::approval::Interaction::AutoApprove);
    let marker = format!("seekcli-orphan-probe-{}", std::process::id());
    let command = format!("sleep 45; echo {marker}");

    let dropped = tokio::time::timeout(
      std::time::Duration::from_millis(400),
      run_shell(&serde_json::json!({ "command": command })),
    )
    .await;
    assert!(dropped.is_err(), "the probe command returned on its own");

    // The kill happens in `Drop`, which has run by now; give the OS a moment
    // to reap before asking whether anything is left.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let out = std::process::Command::new("ps")
      .args(["-Ao", "command"])
      .output();
    let listing = match out {
      Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
      Err(e) => panic!("ps failed: {e}"),
    };
    assert!(
      !listing.contains(&marker),
      "the command outlived the future that owned it -- an orphan the agent \
       can no longer see, stop, or account for"
    );
  }

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
