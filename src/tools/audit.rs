//! Append-only record of what the agent actually did.
//!
//! Deliberately separate from the session event log (L4): that log answers
//! "what did the model see", this one answers "what did this process do to the
//! machine, and what did the policy say about it". They are read by different
//! people for different reasons, and a security record that lives inside the
//! artefact it audits is worth less.
//!
//! Arguments are recorded as a digest rather than verbatim, because tool
//! arguments routinely carry file contents and occasionally carry secrets, and
//! a log written for safety must not become the thing that leaks. `run_shell`
//! is the exception: the command is echoed to the terminal before it runs
//! anyway, so hiding it here would remove the log's main value without
//! removing any exposure.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
  Executed,
  Denied,
}

impl Outcome {
  fn as_str(self) -> &'static str {
    match self {
      Outcome::Executed => "executed",
      Outcome::Denied => "denied",
    }
  }
}

fn log_path() -> Option<PathBuf> {
  let home = std::env::var("HOME").ok()?;
  Some(PathBuf::from(home).join(".seekcli").join("audit.jsonl"))
}

/// Cheap content digest. Not cryptographic — it exists so two identical calls
/// are recognisably identical, not to resist an attacker.
/// FNV-1a, rendered `fnv1a:<hex>`.
///
/// Not a security primitive and not used as one: it identifies *which* content
/// a record refers to, so a reader can tell two turns apart without the record
/// carrying the content. `pub(crate)` since stage 47, where the session log
/// needs the same identity for injected system prompts.
pub(crate) fn digest(text: &str) -> String {
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for byte in text.as_bytes() {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
  }
  let mut out = String::new();
  let _ = write!(out, "fnv1a:{:016x}", hash);
  out
}

pub fn record(tool: &str, args: &Value, outcome: Outcome, note: &str) {
  let args_text = args.to_string();
  append(serde_json::json!({
    "ts": chrono::Utc::now().to_rfc3339(),
    "tool": tool,
    "outcome": outcome.as_str(),
    "note": note,
    "args_digest": digest(&args_text),
    "args_bytes": args_text.len(),
    "cwd": std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
  }));
}

/// `run_shell` variant: keeps the command verbatim (see module docs).
pub fn record_command(command: &str, outcome: Outcome, note: &str) {
  append(serde_json::json!({
    "ts": chrono::Utc::now().to_rfc3339(),
    "tool": "run_shell",
    "outcome": outcome.as_str(),
    "note": note,
    "command": command,
    "cwd": std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
  }));
}

fn append(entry: Value) {
  let Some(path) = log_path() else {
    return;
  };
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  // Best-effort: an unwritable audit log must not stop the agent, but the
  // failure is announced rather than swallowed (design-principles §4).
  match OpenOptions::new().create(true).append(true).open(&path) {
    Ok(mut f) => {
      if writeln!(f, "{}", entry).is_err() {
        eprintln!("[Audit] could not append to {}", path.display());
      }
    }
    Err(e) => eprintln!("[Audit] could not open {}: {}", path.display(), e),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn digest_is_stable_and_distinguishes_inputs() {
    assert_eq!(digest("abc"), digest("abc"));
    assert_ne!(digest("abc"), digest("abz"));
  }

  #[test]
  fn arguments_are_digested_never_echoed() {
    let args = serde_json::json!({ "content": "sk-super-secret-key" });
    let text = args.to_string();
    let d = digest(&text);
    assert!(
      !d.contains("secret"),
      "a safety log must not become the leak: {}",
      d
    );
    assert!(d.starts_with("fnv1a:"));
  }
}
