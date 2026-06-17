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
  pub(crate) async fn run_benchmark(&mut self, suite_path: &Path) -> Result<()> {
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

      // AgentRun: tools resolve against process cwd, so sandbox by chdir.
      env::set_current_dir(&testbed)?;
      let run = self.run_headless(&task.prompt).await;
      env::set_current_dir(&original_cwd)?;

      let llm_calls = match run {
        Ok(c) => c,
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
      let (passed, output) = task.run_eval(&testbed).unwrap_or((false, String::new()));
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
      report.push(TaskResult {
        name: task.name.clone(),
        passed,
        duration_ms: start.elapsed().as_millis(),
        llm_calls,
        cny: self.cost.estimated_cny() - cost_before,
        note,
      });
    }

    println!("{}", report.render());
    Ok(())
  }
}
