//! Background execution.
//!
//! `run_shell` is synchronous, so a five-minute build blocks the whole
//! conversation: the model cannot read a file, plan the next step, or answer a
//! question while it waits. Worse, a long command and a hung one look
//! identical from the outside.
//!
//! A job runs detached, streams into a log file, and is collected through
//! `job_list` / `job_output` / `job_kill`. The log is a file rather than an
//! in-memory buffer so a long build cannot grow the process without bound, and
//! so `job_output` can return a tail without holding everything.
//!
//! Jobs are **not** a daemon: they die with the process. A background command
//! that outlives the REPL would be a surprise the user never asked for, and
//! there is nothing left to collect its output with.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;

/// Longest tail `job_output` returns by default.
const DEFAULT_TAIL_LINES: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
  Running,
  Exited(i32),
  Killed,
}

impl JobState {
  fn label(self) -> String {
    match self {
      JobState::Running => "running".to_string(),
      JobState::Exited(0) => "done".to_string(),
      JobState::Exited(code) => format!("failed({code})"),
      JobState::Killed => "killed".to_string(),
    }
  }

  pub fn is_finished(self) -> bool {
    !matches!(self, JobState::Running)
  }
}

struct Job {
  id: u64,
  command: String,
  log: PathBuf,
  started: Instant,
  state: JobState,
  /// Kept so the job can be killed; `None` once it has been reaped.
  child: Option<tokio::process::Child>,
  /// Set once the completion has been reported to the model, so a finished
  /// job is announced exactly once rather than on every subsequent turn.
  announced: bool,
}

static JOBS: Mutex<Option<BTreeMap<u64, Job>>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn with_jobs<T>(f: impl FnOnce(&mut BTreeMap<u64, Job>) -> T) -> Option<T> {
  let mut guard = JOBS.lock().ok()?;
  Some(f(guard.get_or_insert_with(BTreeMap::new)))
}

fn log_dir() -> Result<PathBuf> {
  let home = std::env::var("HOME").context("Could not find HOME directory")?;
  let dir = PathBuf::from(home).join(".seekcli").join("jobs");
  std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
  Ok(dir)
}

/// Launch `command` detached. Returns the model-facing acknowledgement.
pub async fn spawn(command: &str) -> Result<String> {
  let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
  let log = log_dir()?.join(format!("{id}.log"));
  let file =
    std::fs::File::create(&log).with_context(|| format!("cannot create {}", log.display()))?;
  // stderr shares the log: a build's errors are exactly what the model needs,
  // and interleaving them preserves the order they happened in.
  let errfile = file.try_clone().context("cannot duplicate log handle")?;

  let child = tokio::process::Command::new("sh")
    .arg("-c")
    .arg(command)
    .stdin(Stdio::null())
    .stdout(Stdio::from(file))
    .stderr(Stdio::from(errfile))
    // Same reason as the foreground path: `sh` execs away for a simple command,
    // but forks for anything it cannot exec, and killing only `sh` then leaves
    // the real work running. A background build that survives `job_kill` is
    // worse than one that was never killable — the user believes it stopped.
    .process_group(0)
    .spawn()
    .with_context(|| format!("cannot start background command: {command}"))?;

  let job = Job {
    id,
    command: command.to_string(),
    log: log.clone(),
    started: Instant::now(),
    state: JobState::Running,
    child: Some(child),
    announced: false,
  };
  with_jobs(|jobs| jobs.insert(id, job));

  Ok(format!(
    "Started background job {id}: {command}\n\
     Output accumulates in the background. Use job_output({id}) to read it, \
     job_list() to see what is running, job_kill({id}) to stop it. \
     Do not poll in a tight loop — do other work and check back."
  ))
}

/// Reap finished children so `state` is accurate before it is reported.
fn refresh() {
  with_jobs(|jobs| {
    for job in jobs.values_mut() {
      if job.state != JobState::Running {
        continue;
      }
      if let Some(child) = job.child.as_mut()
        && let Ok(Some(status)) = child.try_wait()
      {
        job.state = JobState::Exited(status.code().unwrap_or(-1));
        job.child = None;
      }
    }
  });
}

/// Jobs that finished since the last call, as a note for the model.
///
/// Delivered through the loop's context injection rather than interrupting the
/// current step: a build finishing mid-turn should not derail whatever the
/// model is in the middle of.
pub fn drain_completions() -> Option<String> {
  refresh();
  with_jobs(|jobs| {
    let mut done = Vec::new();
    for job in jobs.values_mut() {
      if job.state.is_finished() && !job.announced {
        job.announced = true;
        done.push(format!(
          "job {} ({}) {}",
          job.id,
          job.command,
          job.state.label()
        ));
      }
    }
    if done.is_empty() {
      None
    } else {
      Some(format!(
        "[Background] {}. Use job_output(<id>) to read the output.",
        done.join("; ")
      ))
    }
  })
  .flatten()
}

pub async fn job_list(_args: &Value) -> Result<String> {
  refresh();
  let listing = with_jobs(|jobs| {
    if jobs.is_empty() {
      return "No background jobs.".to_string();
    }
    let mut out = String::new();
    for job in jobs.values() {
      out.push_str(&format!(
        "{}  {:<9} {:>4}s  {}\n",
        job.id,
        job.state.label(),
        job.started.elapsed().as_secs(),
        job.command
      ));
    }
    out
  });
  Ok(listing.unwrap_or_else(|| "No background jobs.".to_string()))
}

pub async fn job_output(args: &Value) -> Result<String> {
  refresh();
  let id = require_id(args)?;
  let tail = args
    .get("tail")
    .and_then(Value::as_u64)
    .unwrap_or(DEFAULT_TAIL_LINES as u64) as usize;

  let found = with_jobs(|jobs| {
    jobs
      .get(&id)
      .map(|j| (j.log.clone(), j.state, j.command.clone()))
  })
  .flatten();
  let Some((log, state, command)) = found else {
    return Ok(format!(
      "[FAILED] no job with id {id}. Use job_list() to see ids."
    ));
  };

  let text = std::fs::read_to_string(&log).unwrap_or_default();
  let lines: Vec<&str> = text.lines().collect();
  let shown: Vec<&str> = lines.iter().rev().take(tail).rev().copied().collect();
  let omitted = lines.len().saturating_sub(shown.len());

  let mut out = format!("job {} ({}) — {}\n", id, command, state.label());
  if omitted > 0 {
    // Never silently truncate: a partial tail that looks complete would let
    // the model conclude an error never happened.
    out.push_str(&format!(
      "[showing last {} of {} lines; raise `tail` to see more]\n",
      shown.len(),
      lines.len()
    ));
  }
  if shown.is_empty() {
    out.push_str("(no output yet)");
  } else {
    out.push_str(&shown.join("\n"));
  }
  Ok(out)
}

pub async fn job_kill(args: &Value) -> Result<String> {
  let id = require_id(args)?;
  let outcome = with_jobs(|jobs| match jobs.get_mut(&id) {
    None => format!("[FAILED] no job with id {id}. Use job_list() to see ids."),
    Some(job) if job.state.is_finished() => {
      format!("job {} already {}.", id, job.state.label())
    }
    Some(job) => {
      if let Some(child) = job.child.as_mut() {
        crate::tools::shell::kill_process_group(child);
        let _ = child.start_kill();
      }
      job.state = JobState::Killed;
      job.child = None;
      job.announced = true;
      format!("job {id} killed.")
    }
  });
  Ok(outcome.unwrap_or_else(|| "[FAILED] job registry unavailable.".to_string()))
}

/// Kill everything still running. Called when the process is shutting down.
pub fn kill_all() {
  with_jobs(|jobs| {
    for job in jobs.values_mut() {
      if let Some(child) = job.child.as_mut() {
        crate::tools::shell::kill_process_group(child);
        let _ = child.start_kill();
      }
      job.child = None;
    }
  });
}

fn require_id(args: &Value) -> Result<u64> {
  args
    .get("id")
    .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
    .context("Missing or invalid 'id' argument")
}

/// Sweep job logs older than `max_age`.
pub fn sweep_logs(max_age: Duration) -> usize {
  let Ok(dir) = log_dir() else {
    return 0;
  };
  super::offload::sweep(&dir, max_age)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn state_labels_distinguish_success_from_failure() {
    assert_eq!(JobState::Running.label(), "running");
    assert_eq!(JobState::Exited(0).label(), "done");
    assert_eq!(JobState::Exited(3).label(), "failed(3)");
    assert_eq!(JobState::Killed.label(), "killed");
    assert!(!JobState::Running.is_finished());
    assert!(JobState::Exited(0).is_finished());
  }

  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn a_background_job_does_not_block_and_can_be_collected() {
    // The job registry is process-global; without this, a completion from
    // one test leaks into another's assertions (and into the replay tests).
    let _guard = crate::testsync::lock();
    let ack = match spawn("printf 'line1\\nline2\\n'; exit 0").await {
      Ok(a) => a,
      Err(e) => panic!("spawn failed: {}", e),
    };
    assert!(ack.contains("Started background job"), "got: {}", ack);
    // The point of a background job: control returns immediately.
    let id: u64 = ack
      .split_whitespace()
      .nth(3)
      .and_then(|s| s.trim_end_matches(':').parse().ok())
      .unwrap_or(0);
    assert!(id > 0, "ack should name the id: {}", ack);

    // Give the child a moment; poll rather than sleep a fixed time.
    for _ in 0..50 {
      refresh();
      let finished = with_jobs(|jobs| jobs.get(&id).is_some_and(|j| j.state.is_finished()));
      if finished == Some(true) {
        break;
      }
      tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let out = match job_output(&json!({ "id": id })).await {
      Ok(o) => o,
      Err(e) => panic!("job_output failed: {}", e),
    };
    assert!(out.contains("line1"), "got: {}", out);
    assert!(out.contains("line2"), "got: {}", out);
    assert!(out.contains("done"), "state should be reported: {}", out);
  }

  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn stderr_lands_in_the_same_log_as_stdout() {
    // The job registry is process-global; without this, a completion from
    // one test leaks into another's assertions (and into the replay tests).
    let _guard = crate::testsync::lock();
    // A build's errors are exactly what the model needs; splitting the streams
    // would hide them and lose the order things happened in.
    let ack = match spawn("echo out; echo err >&2").await {
      Ok(a) => a,
      Err(e) => panic!("spawn failed: {}", e),
    };
    let id: u64 = ack
      .split_whitespace()
      .nth(3)
      .and_then(|s| s.trim_end_matches(':').parse().ok())
      .unwrap_or(0);
    for _ in 0..50 {
      refresh();
      if with_jobs(|jobs| jobs.get(&id).is_some_and(|j| j.state.is_finished())) == Some(true) {
        break;
      }
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let out = match job_output(&json!({ "id": id })).await {
      Ok(o) => o,
      Err(e) => panic!("job_output failed: {}", e),
    };
    assert!(out.contains("out") && out.contains("err"), "got: {}", out);
  }

  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn a_completion_is_announced_exactly_once() {
    // The job registry is process-global; without this, a completion from
    // one test leaks into another's assertions (and into the replay tests).
    let _guard = crate::testsync::lock();
    let ack = match spawn("true").await {
      Ok(a) => a,
      Err(e) => panic!("spawn failed: {}", e),
    };
    let id: u64 = ack
      .split_whitespace()
      .nth(3)
      .and_then(|s| s.trim_end_matches(':').parse().ok())
      .unwrap_or(0);
    for _ in 0..50 {
      refresh();
      if with_jobs(|jobs| jobs.get(&id).is_some_and(|j| j.state.is_finished())) == Some(true) {
        break;
      }
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let first = drain_completions();
    assert!(
      first.is_some_and(|n| n.contains(&id.to_string())),
      "the completion should be reported once"
    );
    // Repeating it every turn would be nagging, not information.
    let again = drain_completions();
    assert!(
      again.is_none_or(|n| !n.contains(&format!("job {} ", id))),
      "a completion must not be announced twice"
    );
  }

  #[tokio::test]
  async fn unknown_ids_are_reported_not_panicked_on() {
    let out = match job_output(&json!({ "id": 99999 })).await {
      Ok(o) => o,
      Err(e) => panic!("must not error: {}", e),
    };
    assert!(out.starts_with("[FAILED]"), "got: {}", out);
    let out = match job_kill(&json!({ "id": 99999 })).await {
      Ok(o) => o,
      Err(e) => panic!("must not error: {}", e),
    };
    assert!(out.starts_with("[FAILED]"), "got: {}", out);
  }

  #[tokio::test]
  async fn a_missing_id_argument_is_an_error() {
    assert!(job_output(&json!({})).await.is_err());
    assert!(job_kill(&json!({})).await.is_err());
  }
}
