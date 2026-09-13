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
  /// Invert the verdict: the task passes when `eval` **fails**.
  ///
  /// Safety cases are stated backwards — "the agent must NOT have deleted
  /// this" — and writing them as a negated shell expression buries the intent
  /// in `!` and `test`. Naming the inversion keeps the eval command a plain
  /// statement of what the agent was supposed to be blocked from doing.
  #[serde(default)]
  pub expect_fail: bool,
  /// Extra flags for the agent run, e.g. `--read-only`.
  ///
  /// A safety case is only meaningful under the mode it tests, and encoding
  /// that in the suite keeps the whole scenario in one file instead of
  /// splitting it between the JSON and how the runner was invoked.
  #[serde(default)]
  pub flags: Vec<String>,
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
  ///
  /// `answer` is the agent's final reply, exposed to the eval command as
  /// `$SEEKCLI_ANSWER`. Without it a task can only assert on files the agent
  /// wrote, which rules out the whole class of cases where the interesting
  /// outcome *is* what the agent said — and rules them out completely under
  /// `--read-only`, where it is not allowed to write anything at all
  /// (`docs/architecture/L1-engine.md` §4.6.7).
  ///
  /// It lives outside the testbed on purpose: a task asserting the agent
  /// created no files does so with `ls -A`, which would see a file planted
  /// here.
  pub fn run_eval(&self, testbed: &Path, answer: Option<&Path>) -> Result<(bool, String)> {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&self.eval).current_dir(testbed);
    // Always set, so a task referring to it never silently tests the empty
    // string against a stale value inherited from the environment.
    cmd.env(
      "SEEKCLI_ANSWER",
      answer.map(Path::to_path_buf).unwrap_or_default(),
    );
    let out = cmd
      .output()
      .with_context(|| format!("running eval `{}`", self.eval))?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    // `expect_fail` inverts the verdict, not the output: the note still
    // describes what actually happened, which is what a failing report needs.
    let raw = out.status.success();
    Ok((raw != self.expect_fail, combined))
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
      expect_fail: false,
      flags: Vec::new(),
    };
    let root = std::env::temp_dir().join(format!("seekcli_bench_{}", uuid::Uuid::new_v4()));
    let bed = task.prepare_testbed(&root).expect("prepare");
    assert_eq!(std::fs::read_to_string(bed.join("a.txt")).unwrap(), "seed");
    assert!(bed.join("b.txt").exists());
    std::fs::remove_dir_all(&root).ok();
  }

  /// Every shipped suite must parse. A hand-written JSON typo would otherwise
  /// only surface when someone spends real tokens running the benchmark.
  #[test]
  fn every_shipped_suite_parses_and_is_well_formed() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/benchmarks");
    let mut total = 0usize;
    let mut suites = 0usize;
    let entries = match std::fs::read_dir(&dir) {
      Ok(e) => e,
      Err(e) => panic!("cannot read {}: {}", dir.display(), e),
    };
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().and_then(|s| s.to_str()) != Some("json") {
        continue;
      }
      let suite = match TestSuite::load(&path) {
        Ok(s) => s,
        Err(e) => panic!("{} does not parse: {:#}", path.display(), e),
      };
      assert!(!suite.tasks.is_empty(), "{} has no tasks", path.display());
      for task in &suite.tasks {
        assert!(!task.name.is_empty(), "unnamed task in {}", path.display());
        assert!(!task.eval.is_empty(), "task `{}` has no eval", task.name);
        // A reverse assertion only means something under the mode it tests;
        // without flags it would silently pass in normal mode.
        if task.expect_fail {
          assert!(
            !task.flags.is_empty(),
            "task `{}` inverts its verdict but names no mode to test",
            task.name
          );
        }
        total += 1;
      }
      suites += 1;
    }
    assert!(suites >= 5, "expected the full suite set, found {}", suites);
    assert!(total >= 20, "eval coverage regressed to {} tasks", total);
  }

  /// Safety cases assert the agent was *prevented* from doing something, so
  /// the runner has to be able to say "passing means this command fails".
  #[test]
  fn expect_fail_inverts_the_verdict() {
    let dir = std::env::temp_dir().join("seekcli-bench-expectfail");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::remove_file(dir.join("forbidden.txt"));

    let guard = Task {
      name: "no_write".into(),
      prompt: String::new(),
      files: BTreeMap::new(),
      setup: Vec::new(),
      // Reads as "the file exists" -- and the task passes when it does NOT.
      eval: "test -f forbidden.txt".into(),
      expect_fail: true,
      flags: Vec::new(),
    };
    match guard.run_eval(&dir, None) {
      Ok((passed, _)) => assert!(passed, "absent file means the guard held"),
      Err(e) => panic!("eval failed: {}", e),
    }

    let _ = std::fs::write(dir.join("forbidden.txt"), "leaked");
    match guard.run_eval(&dir, None) {
      Ok((passed, _)) => assert!(!passed, "the file exists, so the guard was breached"),
      Err(e) => panic!("eval failed: {}", e),
    }
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
      expect_fail: false,
      flags: Vec::new(),
    };
    let bed = pass.prepare_testbed(&root).unwrap();
    assert!(pass.run_eval(&bed, None).unwrap().0);

    let fail = Task {
      eval: "false".to_string(),
      ..pass.clone()
    };
    assert!(!fail.run_eval(&bed, None).unwrap().0);
    std::fs::remove_dir_all(&root).ok();
  }

  /// The agent's reply has to be assertable. Without it a read-only task can
  /// assert nothing at all: the agent is not allowed to write the evidence.
  #[test]
  fn the_agent_answer_is_exposed_to_the_eval_command() {
    let root = std::env::temp_dir().join(format!("seekcli_bench_{}", uuid::Uuid::new_v4()));
    let task = Task {
      name: "answer".to_string(),
      prompt: String::new(),
      files: BTreeMap::new(),
      setup: vec![],
      eval: "grep -q 42 \"$SEEKCLI_ANSWER\"".to_string(),
      expect_fail: false,
      flags: Vec::new(),
    };
    let bed = match task.prepare_testbed(&root) {
      Ok(b) => b,
      Err(e) => panic!("testbed: {e}"),
    };
    let answer = root.join("answer.txt");
    let _ = std::fs::write(&answer, "the count is 42\n");

    match task.run_eval(&bed, Some(&answer)) {
      Ok((passed, _)) => assert!(passed, "the eval must see the reply"),
      Err(e) => panic!("eval failed: {e}"),
    }

    // Unset rather than inherited: a task referring to it must not silently
    // pass against a stale value from the environment.
    match task.run_eval(&bed, None) {
      Ok((passed, _)) => assert!(!passed, "no answer means the assertion cannot hold"),
      Err(e) => panic!("eval failed: {e}"),
    }
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
