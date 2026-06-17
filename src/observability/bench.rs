//! Benchmark runner: Fail-to-Pass evaluation of the agent (图2).
//!
//! Borrows SWE-bench's core idea — judge by tests, not by the agent's own
//! claim of success. Each task seeds an isolated testbed, runs the agent on a
//! prompt, then runs a verification command; the task passes iff that command
//! exits 0. Aggregated scores give a regression baseline so engine changes can
//! be measured instead of guessed at.
//!
//! This module owns the pure, testable pieces — suite parsing, testbed setup,
//! eval, scoring, and report formatting. Driving the agent between setup and
//! eval lives in `main.rs` (it needs the live `App`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// A single benchmark task loaded from the suite JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct Task {
  /// Unique, filesystem-safe task identifier.
  pub name: String,
  /// Instruction handed to the agent.
  pub prompt: String,
  /// Files written into the testbed before the agent runs (path -> contents).
  #[serde(default)]
  pub files: BTreeMap<String, String>,
  /// Shell commands run to seed the testbed (after files are written).
  #[serde(default)]
  pub setup: Vec<String>,
  /// Verification command. Task passes iff this exits 0 (Fail-to-Pass).
  pub eval: String,
}

/// A loaded benchmark suite.
#[derive(Debug, Clone, Deserialize)]
pub struct TestSuite {
  pub tasks: Vec<Task>,
}

impl TestSuite {
  /// Parse a suite from a JSON file.
  pub fn load(path: &Path) -> Result<Self> {
    let raw = std::fs::read_to_string(path)
      .with_context(|| format!("reading testsuite {}", path.display()))?;
    let suite: TestSuite = serde_json::from_str(&raw).with_context(|| "parsing testsuite JSON")?;
    if suite.tasks.is_empty() {
      anyhow::bail!("testsuite has no tasks");
    }
    Ok(suite)
  }
}

impl Task {
  /// Create and seed the testbed for this task under `root`, returning its
  /// path. Writes declared files, then runs setup commands inside it.
  pub fn prepare_testbed(&self, root: &Path) -> Result<PathBuf> {
    let testbed = root.join(&self.name);
    // Start clean so reruns are deterministic.
    if testbed.exists() {
      std::fs::remove_dir_all(&testbed).ok();
    }
    std::fs::create_dir_all(&testbed)
      .with_context(|| format!("creating testbed {}", testbed.display()))?;

    for (rel, contents) in &self.files {
      let path = testbed.join(rel);
      if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
      }
      std::fs::write(&path, contents)
        .with_context(|| format!("writing seed file {}", path.display()))?;
    }

    for cmd in &self.setup {
      let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(&testbed)
        .status()
        .with_context(|| format!("running setup `{cmd}`"))?;
      if !status.success() {
        anyhow::bail!("setup command failed (`{cmd}`): {status}");
      }
    }
    Ok(testbed)
  }

  /// Run the verification command in `testbed`. Returns (passed, combined
  /// stdout+stderr). Pass = exit 0.
  pub fn run_eval(&self, testbed: &Path) -> Result<(bool, String)> {
    let out = std::process::Command::new("sh")
      .arg("-c")
      .arg(&self.eval)
      .current_dir(testbed)
      .output()
      .with_context(|| format!("running eval `{}`", self.eval))?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), combined))
  }
}

/// Outcome of a single task run.
#[derive(Debug, Clone)]
pub struct TaskResult {
  pub name: String,
  pub passed: bool,
  pub duration_ms: u128,
  /// LLM calls the agent made (proxy for turns).
  pub llm_calls: u64,
  /// Estimated CNY spent on this task.
  pub cny: f64,
  /// First line of eval output on failure (for the report).
  pub note: String,
}

/// Aggregated benchmark report.
#[derive(Debug, Default)]
pub struct Report {
  pub results: Vec<TaskResult>,
}

impl Report {
  pub fn push(&mut self, r: TaskResult) {
    self.results.push(r);
  }

  pub fn passed(&self) -> usize {
    self.results.iter().filter(|r| r.passed).count()
  }

  pub fn total(&self) -> usize {
    self.results.len()
  }

  pub fn total_cny(&self) -> f64 {
    self.results.iter().map(|r| r.cny).sum()
  }

  pub fn total_ms(&self) -> u128 {
    self.results.iter().map(|r| r.duration_ms).sum()
  }

  /// Multi-line human-readable report.
  pub fn render(&self) -> String {
    let mut out = String::new();
    out.push_str("\n=== Benchmark Report ===\n");
    for r in &self.results {
      let mark = if r.passed { "PASS" } else { "FAIL" };
      out.push_str(&format!(
        "[{}] {:<24} {:>6}ms  {} calls  ≈¥{:.4}",
        mark, r.name, r.duration_ms, r.llm_calls, r.cny
      ));
      if !r.passed && !r.note.is_empty() {
        out.push_str(&format!("  — {}", r.note));
      }
      out.push('\n');
    }
    let pct = if self.total() == 0 {
      0
    } else {
      self.passed() * 100 / self.total()
    };
    out.push_str(&format!(
      "------------------------\nScore: {}/{} ({}%)  ·  total ≈¥{:.4}  ·  {}ms\n",
      self.passed(),
      self.total(),
      pct,
      self.total_cny(),
      self.total_ms(),
    ));
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn suite_json() -> &'static str {
    r#"{
      "tasks": [
        {
          "name": "create_hello",
          "prompt": "create hello.txt with text hello",
          "eval": "grep -q hello hello.txt"
        }
      ]
    }"#
  }

  #[test]
  fn parses_suite() {
    let s: TestSuite = serde_json::from_str(suite_json()).expect("parse");
    assert_eq!(s.tasks.len(), 1);
    assert_eq!(s.tasks[0].name, "create_hello");
    assert!(s.tasks[0].files.is_empty());
    assert!(s.tasks[0].setup.is_empty());
  }

  #[test]
  fn prepare_testbed_writes_files_and_runs_setup() {
    let task = Task {
      name: "prep_test".to_string(),
      prompt: "x".to_string(),
      files: BTreeMap::from([("a.txt".to_string(), "seed".to_string())]),
      setup: vec!["echo more > b.txt".to_string()],
      eval: "true".to_string(),
    };
    let root = std::env::temp_dir().join(format!("seekcli_bench_{}", uuid::Uuid::new_v4()));
    let bed = task.prepare_testbed(&root).expect("prepare");
    assert_eq!(std::fs::read_to_string(bed.join("a.txt")).unwrap(), "seed");
    assert!(bed.join("b.txt").exists());
    std::fs::remove_dir_all(&root).ok();
  }

  #[test]
  fn run_eval_reflects_exit_code() {
    let root = std::env::temp_dir().join(format!("seekcli_bench_{}", uuid::Uuid::new_v4()));
    let pass = Task {
      name: "p".to_string(),
      prompt: String::new(),
      files: BTreeMap::new(),
      setup: vec![],
      eval: "true".to_string(),
    };
    let bed = pass.prepare_testbed(&root).unwrap();
    assert!(pass.run_eval(&bed).unwrap().0);

    let fail = Task {
      eval: "false".to_string(),
      ..pass.clone()
    };
    assert!(!fail.run_eval(&bed).unwrap().0);
    std::fs::remove_dir_all(&root).ok();
  }

  #[test]
  fn report_scores_and_renders() {
    let mut rep = Report::default();
    rep.push(TaskResult {
      name: "t1".to_string(),
      passed: true,
      duration_ms: 100,
      llm_calls: 2,
      cny: 0.01,
      note: String::new(),
    });
    rep.push(TaskResult {
      name: "t2".to_string(),
      passed: false,
      duration_ms: 200,
      llm_calls: 5,
      cny: 0.02,
      note: "eval failed".to_string(),
    });
    assert_eq!(rep.passed(), 1);
    assert_eq!(rep.total(), 2);
    let text = rep.render();
    assert!(text.contains("Score: 1/2 (50%)"));
    assert!(text.contains("PASS"));
    assert!(text.contains("FAIL"));
  }
}
