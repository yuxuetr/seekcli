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
///
/// `deny_unknown_fields` because the alternative is a suite that looks like it
/// does something it does not. A `cleanup` key that serde quietly drops leaves
/// the author believing their task tidies up after itself, and the mess only
/// shows up much later somewhere else.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
  /// A note to whoever reads the suite. Same story as `TestSuite::comment`:
  /// the convention already existed and was surviving only because unknown
  /// fields were dropped.
  #[serde(default, rename = "_note")]
  pub note: String,
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
  /// Commands run after `eval`, whatever the verdict.
  ///
  /// For the rare task whose side effects land outside its testbed — writing
  /// into the user's real memory directory, say. Failures here are reported
  /// and do not change the verdict: a task that passed did pass, and a tidy-up
  /// that could not run is a separate problem the user should hear about
  /// rather than see folded into a red result.
  #[serde(default)]
  pub cleanup: Vec<String>,
}

/// A loaded benchmark suite.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestSuite {
  /// The suite's own note to a reader. JSON has no comments, so every suite
  /// here already carried one — silently dropped until `deny_unknown_fields`
  /// made the drop visible. Declaring it keeps the convention and keeps a
  /// typo'd key an error.
  #[serde(default, rename = "_comment")]
  pub comment: String,
  pub tasks: Vec<Task>,
}

/// The smoke suite, compiled into the binary.
///
/// `CARGO_MANIFEST_DIR` is a *build-time* path. Resolving the suite through it
/// works in the source tree and nowhere else: an installed binary looks for a
/// directory that does not exist on that machine, and `examples/` is not in the
/// crate's `include` list either. So the gate stage 38 built — "a skill that
/// breaks the smoke suite does not land" — silently became no gate at all for
/// every user who did not clone the repo, while still printing a reassuring
/// line about it.
///
/// Embedding it means the gate exists wherever the binary does.
const EMBEDDED_SMOKE: &str = include_str!("../../examples/benchmarks/basic.json");

impl TestSuite {
  /// The built-in smoke suite. Cannot fail to be found; can only fail to parse,
  /// and then it failed at compile time.
  pub fn embedded_smoke() -> Result<Self> {
    let suite: TestSuite =
      serde_json::from_str(EMBEDDED_SMOKE).with_context(|| "parsing the embedded smoke suite")?;
    if suite.tasks.is_empty() {
      anyhow::bail!("the embedded smoke suite has no tasks");
    }
    Ok(suite)
  }

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

  /// Run the task's `cleanup` commands. Best-effort and loud.
  ///
  /// The verdict is already decided, so a failure here must not change it —
  /// but it must not be swallowed either, or the user finds the leftovers
  /// later with no idea where they came from.
  pub fn run_cleanup(&self, testbed: &Path) {
    for command in &self.cleanup {
      match std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(testbed)
        .output()
      {
        Ok(out) if !out.status.success() => eprintln!(
          "[Bench] cleanup for `{}` exited {}: {}",
          self.name,
          out.status,
          String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => eprintln!("[Bench] cleanup for `{}` could not run: {e}", self.name),
        Ok(_) => {}
      }
    }
  }
}

/// What the regression gate decided about a proposed skill.
///
/// A pure function of two runs, so "a proposal that breaks things must be
/// refused" is unit-testable rather than something that costs a live run every
/// time it is checked (`docs/architecture/L7-observability.md` §4.7.4).
#[derive(Debug, Clone, PartialEq)]
pub enum Regression {
  /// Nothing that passed before fails now.
  Clean,
  /// These tasks passed without the skill and fail with it.
  Broke(Vec<String>),
}

impl Regression {
  /// Compare a baseline run against the same suite with the skill active.
  ///
  /// Deliberately one-directional: a task that **starts** passing is not
  /// evidence the skill is good — the suite is generic and the skill is not —
  /// so it neither helps nor hurts the verdict. Only losing ground counts
  /// (`docs/architecture/L7-observability.md` §4.7.2).
  pub fn compare(before: &Report, after: &Report) -> Self {
    let passed_before: std::collections::HashSet<&str> = before
      .results
      .iter()
      .filter(|r| r.passed)
      .map(|r| r.name.as_str())
      .collect();
    let broke: Vec<String> = after
      .results
      .iter()
      .filter(|r| !r.passed && passed_before.contains(r.name.as_str()))
      .map(|r| r.name.clone())
      .collect();
    if broke.is_empty() {
      Self::Clean
    } else {
      Self::Broke(broke)
    }
  }

  /// The sentence the user (and, on refusal, the model) reads.
  pub fn explain(&self) -> String {
    match self {
      Self::Clean => "no task that passed before fails with it".to_string(),
      Self::Broke(names) => format!(
        "it breaks {} task(s) that passed without it: {}. Fix the skill's \
         instructions, or accept it anyway with --skip-eval",
        names.len(),
        names.join(", ")
      ),
    }
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

  fn report(rows: &[(&str, bool)]) -> Report {
    let mut r = Report::default();
    for (name, passed) in rows {
      r.push(TaskResult {
        name: (*name).to_string(),
        passed: *passed,
        duration_ms: 0,
        llm_calls: 0,
        cny: 0.0,
        note: String::new(),
      });
    }
    r
  }

  #[test]
  fn a_skill_that_breaks_nothing_is_clean() {
    let before = report(&[("a", true), ("b", true)]);
    let after = report(&[("a", true), ("b", true)]);
    assert_eq!(Regression::compare(&before, &after), Regression::Clean);
  }

  /// A suite key serde quietly drops is a suite that looks like it does
  /// something it does not — the author believes their task tidies up after
  /// itself and the mess surfaces much later, somewhere else.
  #[test]
  fn an_unknown_suite_key_is_refused_rather_than_ignored() {
    let json = r#"{"tasks":[{"name":"t","prompt":"p","eval":"true","clenup":["rm x"]}]}"#;
    let err = match serde_json::from_str::<TestSuite>(json) {
      Err(e) => e.to_string(),
      Ok(_) => panic!("a typo'd key must not parse silently"),
    };
    assert!(
      err.contains("clenup"),
      "the error must name the typo: {err}"
    );
  }

  /// The scenario suite is the one place a task can touch state outside its
  /// testbed, so its cleanup has to actually be a thing the runner knows about.
  #[test]
  fn cleanup_commands_are_parsed_not_discarded() {
    let json = r#"{"tasks":[{"name":"t","prompt":"p","eval":"true","cleanup":["rm -f x"]}]}"#;
    let suite = match serde_json::from_str::<TestSuite>(json) {
      Ok(s) => s,
      Err(e) => panic!("{e}"),
    };
    assert_eq!(
      suite.tasks.first().map(|t| t.cleanup.len()),
      Some(1),
      "cleanup must survive parsing"
    );
  }

  /// The gate's own precondition. Before this, the suite was resolved through
  /// `CARGO_MANIFEST_DIR` — a build-time path — so an installed binary looked
  /// for a directory that does not exist on that machine, printed a warning,
  /// and accepted the proposal unmeasured. A gate that is only a gate on the
  /// developer's laptop is not a gate.
  #[test]
  fn the_embedded_smoke_suite_is_present_and_usable() {
    let suite = match TestSuite::embedded_smoke() {
      Ok(s) => s,
      Err(e) => panic!("the built-in smoke suite must always load: {e:#}"),
    };
    assert!(
      !suite.tasks.is_empty(),
      "an empty smoke suite would pass everything"
    );
    // Every task needs the two things the runner cannot invent.
    for t in &suite.tasks {
      assert!(!t.name.trim().is_empty(), "a task needs a name");
      assert!(
        !t.prompt.trim().is_empty(),
        "a task needs a prompt: {}",
        t.name
      );
    }
  }

  /// The case the gate exists for: a badly written skill prompt degrades the
  /// agent on work it could already do.
  #[test]
  fn losing_ground_names_exactly_which_tasks() {
    let before = report(&[("a", true), ("b", true), ("c", false)]);
    let after = report(&[("a", true), ("b", false), ("c", false)]);
    match Regression::compare(&before, &after) {
      Regression::Broke(names) => assert_eq!(names, vec!["b".to_string()]),
      other => panic!("expected a regression, got {other:?}"),
    }
  }

  /// A task that was already failing must not be blamed on the skill.
  #[test]
  fn a_task_that_was_already_failing_is_not_a_regression() {
    let before = report(&[("a", false)]);
    let after = report(&[("a", false)]);
    assert_eq!(Regression::compare(&before, &after), Regression::Clean);
  }

  /// Newly passing tasks are not evidence: the suite is generic, the skill is
  /// not. Counting them would let a skill "buy" a regression with an unrelated
  /// win.
  #[test]
  fn a_newly_passing_task_does_not_offset_a_broken_one() {
    let before = report(&[("a", true), ("b", false)]);
    let after = report(&[("a", false), ("b", true)]);
    match Regression::compare(&before, &after) {
      Regression::Broke(names) => assert_eq!(names, vec!["a".to_string()]),
      other => panic!("a win must not cancel a loss: {other:?}"),
    }
  }

  #[test]
  fn the_refusal_names_the_tasks_and_the_way_out() {
    let text = Regression::Broke(vec!["fix_bug".into()]).explain();
    assert!(text.contains("fix_bug"), "{text}");
    assert!(text.contains("--skip-eval"), "{text}");
  }

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
      cleanup: Vec::new(),
      note: String::new(),
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
      cleanup: Vec::new(),
      note: String::new(),
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
      cleanup: Vec::new(),
      note: String::new(),
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
      cleanup: Vec::new(),
      note: String::new(),
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
