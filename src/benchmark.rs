//! Benchmark orchestration: drives the agent over a testsuite in isolated
//! testbeds and reports Fail-to-Pass scores. The pure pieces (parsing,
//! testbed, eval, report) live in `observability::bench`; this wires the live
//! agent between seed and eval.

use anyhow::{Context, Result};
use colored::Colorize;
use std::env;
use std::path::{Path, PathBuf};

use crate::App;
use crate::observability::bench::{Report, TaskResult, TestSuite};

impl App {
  /// Benchmark entry point: load a testsuite, run each task in an isolated
  /// testbed (Init → seed → AgentRun → Eval → Score), and print a report.
  /// Fail-to-Pass: a task passes iff its eval command exits 0.
  pub(crate) async fn run_benchmark(
    &mut self,
    suite_path: &Path,
    trajectory_path: Option<&Path>,
  ) -> Result<()> {
    let report = self.score_suite(suite_path, trajectory_path, None).await?;
    println!("{}", report.render());
    Ok(())
  }

  /// Run every task and return the scores.
  ///
  /// `skill`, when present, is activated for every task — that is how the
  /// regression gate measures a proposed skill against the same suite twice
  /// (`docs/architecture/L7-observability.md` §4.7).
  pub(crate) async fn score_suite(
    &mut self,
    suite_path: &Path,
    trajectory_path: Option<&Path>,
    skill: Option<&crate::Skill>,
  ) -> Result<Report> {
    let suite = TestSuite::load(suite_path)?;
    let home = env::var("HOME").context("HOME not set")?;
    let bench_root = PathBuf::from(home).join(".seekcli").join("bench");
    std::fs::create_dir_all(&bench_root)?;

    println!(
      "{} running {} task(s) from {}",
      "[Bench]".cyan().bold(),
      suite.tasks.len(),
      suite_path.display()
    );

    let original_cwd = env::current_dir()?;
    let mut report = Report::default();
    // Collected only when asked for, so an ordinary run carries no extra cost.
    let mut trajectories: Vec<crate::observability::trajectory::Record> = Vec::new();

    for task in &suite.tasks {
      println!("\n{} {}", "[Bench] task:".cyan(), task.name.bold());
      let cost_before = self.cost.estimated_cny();
      let start = std::time::Instant::now();

      // Init + seed the testbed.
      let testbed = match task.prepare_testbed(&bench_root) {
        Ok(p) => p,
        Err(e) => {
          println!("{} setup failed: {}", "[Bench]".red(), e);
          report.push(TaskResult {
            name: task.name.clone(),
            passed: false,
            duration_ms: start.elapsed().as_millis(),
            llm_calls: 0,
            cny: 0.0,
            note: format!("setup error: {e}"),
          });
          continue;
        }
      };

      // A task can demand a mode (e.g. --read-only). Safety cases are only
      // meaningful under the mode they test, so the suite carries it rather
      // than depending on how the runner happened to be invoked.
      let restricted = task.flags.iter().any(|f| f == "--read-only");
      crate::tools::policy::set_mode(if restricted {
        crate::tools::policy::Mode::ReadOnly
      } else {
        crate::tools::policy::Mode::Normal
      });

      // AgentRun: tools resolve against process cwd, so sandbox by chdir.
      env::set_current_dir(&testbed)?;
      let run = self.run_headless(&task.prompt, skill).await;
      env::set_current_dir(&original_cwd)?;
      crate::tools::policy::set_mode(crate::tools::policy::Mode::Normal);

      let (llm_calls, answer_path, run_events, run_status) = match run {
        Ok(outcome) => {
          // The reply is what several assertions are actually about, and under
          // --read-only it is the only evidence a task can have.
          let path = bench_root.join(format!("{}.answer", task.name));
          let saved = match std::fs::write(&path, &outcome.text) {
            Ok(()) => Some(path),
            // Not fatal: only tasks that reference $SEEKCLI_ANSWER care, and
            // they will fail on their own terms with the reason visible here.
            Err(e) => {
              println!("{} could not save the answer: {}", "[Bench]".yellow(), e);
              None
            }
          };
          (
            outcome.llm_calls,
            saved,
            outcome.events,
            format!("{:?}", outcome.status).to_lowercase(),
          )
        }
        Err(e) => {
          println!("{} agent error: {}", "[Bench]".red(), e);
          report.push(TaskResult {
            name: task.name.clone(),
            passed: false,
            duration_ms: start.elapsed().as_millis(),
            llm_calls: 0,
            cny: self.cost.estimated_cny() - cost_before,
            note: format!("agent error: {e}"),
          });
          continue;
        }
      };

      // Eval + score.
      let (passed, output) = task
        .run_eval(&testbed, answer_path.as_deref())
        .unwrap_or((false, String::new()));
      let note = if passed {
        String::new()
      } else {
        output.lines().next().unwrap_or("").to_string()
      };
      println!(
        "{} {} ({} calls)",
        "[Bench] result:".cyan(),
        if passed { "PASS".green() } else { "FAIL".red() },
        llm_calls
      );
      if trajectory_path.is_some() {
        trajectories.push(crate::observability::trajectory::record(
          &task.name,
          &task.prompt,
          passed,
          &run_status,
          llm_calls,
          &run_events,
        ));
      }
      report.push(TaskResult {
        name: task.name.clone(),
        passed,
        duration_ms: start.elapsed().as_millis(),
        llm_calls,
        cny: self.cost.estimated_cny() - cost_before,
        note,
      });
    }

    // After scoring, and never instead of it: a failed export must not cost
    // the user the scores they waited for.
    if let Some(path) = trajectory_path {
      match crate::observability::trajectory::to_jsonl(&trajectories)
        .and_then(|text| std::fs::write(path, text).map_err(Into::into))
      {
        Ok(()) => println!(
          "{} {} trajectory record(s) written to {}",
          "[Bench]".cyan(),
          trajectories.len(),
          path.display()
        ),
        Err(e) => println!(
          "{} could not write {}: {:#}",
          "[Bench]".yellow(),
          path.display(),
          e
        ),
      }
    }
    Ok(report)
  }
}
