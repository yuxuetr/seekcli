use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod agent;
mod api;
mod benchmark;
mod commands;
mod completer;
mod config;
mod engine;
mod history;
mod mcp;
mod observability;
mod proposals;
mod session;
mod skills;
mod subagents;
mod tasks;
mod tools;
mod ui;

use completer::CmdCompleter;

/// Serialises tests that touch process-global state.
///
/// Three things in this binary are process-wide by design — the working
/// directory, `tools::policy`'s mode, and `tools::approval`'s interaction mode
/// — and `cargo test` runs tests in parallel. Without a shared lock, one test
/// flipping the policy to read-only makes an unrelated test's assertion fail,
/// which reads as a bug in the code under test rather than in the harness.
#[cfg(test)]
pub(crate) mod testsync {
  use std::sync::{Mutex, MutexGuard};

  static GLOBAL: Mutex<()> = Mutex::new(());

  /// A panicking test poisons the lock. Recover instead of cascading: the
  /// invariant each test establishes for itself still holds for the next one.
  pub fn lock() -> MutexGuard<'static, ()> {
    match GLOBAL.lock() {
      Ok(g) => g,
      Err(poisoned) => poisoned.into_inner(),
    }
  }
}

#[cfg(test)]
mod loop_tests;
#[cfg(test)]
mod test;

pub use api::{LlmProvider, Message, StreamItem, ToolCall};
pub use config::Config;
pub use history::HistoryManager;
pub use session::Session;
pub use skills::{Skill, SkillManager};

#[derive(Debug, PartialEq, Clone, Copy)]
enum ThinkingMode {
  None,
  High,
  Max,
}

impl ThinkingMode {
  fn label(&self) -> &str {
    match self {
      ThinkingMode::None => "None",
      ThinkingMode::High => "High",
      ThinkingMode::Max => "Max",
    }
  }
  fn as_str(&self) -> &str {
    match self {
      ThinkingMode::None => "none",
      ThinkingMode::High => "high",
      ThinkingMode::Max => "max",
    }
  }
}

struct App {
  brain: Box<dyn LlmProvider>,
  config: Config,
  history: HistoryManager,
  skill_manager: SkillManager,
  current_session: Session,
  model: String,
  thinking_mode: ThinkingMode,
  /// When on, the agent is instructed to externalize long-task state to
  /// PLAN.md / TODO.md in the workspace. Toggled with `/plan`.
  plan_mode: bool,
  current_skill: Option<Skill>,
  last_code_blocks: Vec<String>,
  /// Token/cost accounting for the current session (decorator-style). Reset on
  /// /clear, restored on /load, persisted into the session JSON.
  cost: observability::cost::CostTracker,
  /// Decision-path tracer (opt-in via SEEKCLI_TRACE); no-op when disabled.
  tracer: observability::trace::Trace,
  /// Iteration ceiling for a run. Defaults to `agent::MAX_ITER`; a headless
  /// run can lower it with `--max-iter` so an unattended job has a bounded
  /// worst-case cost.
  max_iter: usize,
  /// Tools contributed by MCP servers. Empty when none are configured, and
  /// then it costs nothing.
  mcp: mcp::McpRegistry,
  /// Set to true by the Ctrl-C watcher task. Polled at the top of each
  /// agent loop iteration and during stream consumption to allow graceful
  /// mid-task interruption back to the REPL.
  interrupt: Arc<AtomicBool>,
}

impl App {
  async fn new() -> Result<Self> {
    let loaded = Config::load()?;
    // Notices go to stderr so a future `-p --output json` keeps stdout clean.
    for notice in &loaded.notices {
      eprintln!("{}", notice.yellow());
    }
    let config = loaded.config;
    // Install the user's shell-command allow/deny policy (three-state approval).
    tools::approval::init_policy(config.security.allow.clone(), config.security.deny.clone());
    observability::cost::set_rates(observability::cost::Rates {
      cache_hit: config.pricing.cache_hit_cny_per_m,
      cache_miss: config.pricing.cache_miss_cny_per_m,
      output: config.pricing.output_cny_per_m,
    });
    // Session storage first: it does not depend on credentials, and running
    // the legacy migration before a possible "API key not set" exit means a
    // user fixing their key later does not find their history still stuck in
    // the old format.
    let history = HistoryManager::new()?;
    let skill_manager = SkillManager::new()?;
    let brain = api::build_provider(&config)?;
    let model = config.brain.flash_model.clone();
    let current_session = history.create_session(model.clone());

    // Offloaded tool output belongs to the session that produced it.
    tools::offload::set_blob_dir(history.blobs_dir(current_session.id()));

    // Servers are launched here rather than lazily: the model needs their
    // schemas in the very first request, and discovering a tool mid-turn
    // would change the tool set under prompt caching.
    let mcp = mcp::McpRegistry::connect_all(&config.mcp_servers).await;

    let interrupt = Arc::new(AtomicBool::new(false));
    spawn_interrupt_watcher(interrupt.clone());
    // Cancellation must reach the innermost blocking operation, not just the loop.
    tools::shell::set_interrupt(interrupt.clone());

    Ok(Self {
      brain,
      config,
      history,
      skill_manager,
      current_session,
      model,
      thinking_mode: ThinkingMode::None,
      max_iter: agent::MAX_ITER,
      plan_mode: false,
      current_skill: None,
      last_code_blocks: Vec::new(),
      cost: observability::cost::CostTracker::new(),
      tracer: observability::trace::Trace::from_env(),
      mcp,
      interrupt,
    })
  }

  /// Construct an App around a supplied provider, bypassing config and
  /// credentials. Exists so replay tests can drive the real
  /// `run_agent_loop` — the whole point of stage 25 is that the loop stops
  /// being the untested part of the system.
  #[cfg(test)]
  pub(crate) fn for_test(brain: Box<dyn LlmProvider>) -> Result<Self> {
    let config = Config::default();
    let history = HistoryManager::new()?;
    let skill_manager = SkillManager::new()?;
    let model = config.brain.flash_model.clone();
    let current_session = history.create_session(model.clone());
    Ok(Self {
      brain,
      config,
      history,
      skill_manager,
      current_session,
      model,
      thinking_mode: ThinkingMode::None,
      max_iter: agent::MAX_ITER,
      plan_mode: false,
      current_skill: None,
      last_code_blocks: Vec::new(),
      cost: observability::cost::CostTracker::new(),
      tracer: observability::trace::Trace::new(false),
      mcp: mcp::McpRegistry::empty(),
      interrupt: Arc::new(AtomicBool::new(false)),
    })
  }

  async fn run(&mut self) -> Result<()> {
    println!(
      "{}",
      format!("SeekCLI (DeepSeek {} Harness Agent)", self.model)
        .bold()
        .green()
    );
    if let Some(notice) = tasks::pending_digest_notice(&self.config) {
      println!("{}", notice.yellow());
    }

    let completer = CmdCompleter {
      skills_dir: self.skill_manager.skills_dir().clone(),
      proposals_dir: self.skill_manager.proposals_dir().clone(),
    };
    let mut rl: Editor<CmdCompleter, FileHistory> =
      Editor::new().context("rustyline init failed")?;
    rl.set_helper(Some(completer));

    loop {
      let skill_label = self
        .current_skill
        .as_ref()
        .map(|s| format!("|{}", s.name))
        .unwrap_or_default();
      let plan_label = if self.plan_mode { "|plan" } else { "" };
      let prompt = format!(
        "{} ({}{}{}) {} ",
        self.model.blue(),
        self.thinking_mode.label().magenta(),
        plan_label.cyan(),
        skill_label.yellow(),
        "❯".green()
      );

      match rl.readline(&prompt) {
        Ok(line) => {
          let line = line.trim();
          if line.is_empty() {
            continue;
          }
          rl.add_history_entry(line)?;
          if line.starts_with('/') {
            if self.handle_command(line).await? {
              break;
            }
          } else {
            self.chat(line).await?;
          }
        }
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
        Err(err) => {
          println!("Error: {:?}", err);
          break;
        }
      }
    }
    Ok(())
  }
}

#[derive(Parser)]
#[command(author, version, about = "DeepSeek Harness Agent for CLI", long_about = None)]
struct Cli {
  /// Run one prompt headlessly and exit. Reads extra context from stdin when
  /// it is piped. Result goes to stdout; progress goes to stderr.
  #[arg(short = 'p', long, value_name = "PROMPT")]
  prompt: Option<String>,

  /// Output format for -p. `json` is pipeable into jq: stdout carries only
  /// the JSON object.
  #[arg(long, value_name = "FORMAT", default_value = "text")]
  output: OutputFormat,

  /// Iteration ceiling for this run. Bounds the worst-case cost of an
  /// unattended job.
  #[arg(long, value_name = "N")]
  max_iter: Option<usize>,

  /// Refuse every mutating tool (write_file / edit_file / run_shell /
  /// create_skill) for this run.
  #[arg(long)]
  read_only: bool,

  /// Approve dangerous commands without asking. Only meaningful headless,
  /// where the default is to deny them.
  #[arg(long)]
  yes: bool,

  /// Working directory to run in.
  #[arg(long, value_name = "DIR")]
  cwd: Option<PathBuf>,

  /// Run a benchmark testsuite (JSON) headlessly instead of the REPL.
  #[arg(long, value_name = "TESTSUITE.json")]
  bench: Option<PathBuf>,

  /// With --bench: also write each task's trajectory as JSONL here.
  ///
  /// Opt-in rather than automatic: exporting is an act of handing data to
  /// something outside this repository, and a file nobody asked for on every
  /// benchmark run is noise. Format: docs/architecture/L7-observability.md §4.6.
  #[arg(long, value_name = "FILE.jsonl", requires = "bench")]
  trajectory: Option<PathBuf>,

  /// Run a named scheduled task (e.g. "reminders") headlessly instead of the
  /// REPL. Intended for launchd/cron invocation, not interactive use.
  #[arg(long, value_name = "TASK_NAME")]
  run_task: Option<String>,

  #[command(subcommand)]
  task: Option<TaskCommand>,
}

#[derive(clap::Subcommand)]
enum TaskCommand {
  /// Inspect scheduled tasks.
  Task {
    #[command(subcommand)]
    action: TaskAction,
  },
}

#[derive(clap::Subcommand)]
enum TaskAction {
  /// List defined tasks.
  List,
  /// Print a launchd plist for a task.
  ///
  /// Prints only — installing it is left to the user, because writing into
  /// someone's LaunchAgents should be an explicit act.
  Install { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum OutputFormat {
  Text,
  Json,
}

/// Exit codes, so a caller can branch without parsing output.
///
/// The distinction that matters is 2 vs 1: "the agent ran fine but did not
/// finish within its iteration budget" is a different operational signal from
/// "something broke", and a CI job usually wants to treat them differently.
mod exit {
  pub const OK: i32 = 0;
  pub const RUNTIME_ERROR: i32 = 1;
  pub const NOT_CONVERGED: i32 = 2;
  pub const POLICY_REFUSED: i32 = 3;
}

/// Background task that flips `flag` to `true` on each Ctrl-C. Rustyline
/// catches Ctrl-C at the readline prompt directly (returns `Interrupted`),
/// so the stale flag there is reset at the start of every `chat()` call.
/// During agent execution, the loop polls this flag and breaks out
/// gracefully instead of letting the signal kill the whole process.
fn spawn_interrupt_watcher(flag: Arc<AtomicBool>) {
  tokio::spawn(async move {
    loop {
      if tokio::signal::ctrl_c().await.is_err() {
        // OS not delivering signals — stop trying.
        break;
      }
      flag.store(true, Ordering::SeqCst);
    }
  });
}

/// How long to wait for piped stdin before giving up on it.
///
/// A pipe that is open but silent is indistinguishable from one whose data is
/// still coming. Waiting forever is the worse failure: `seekcli -p ...`
/// launched from a script that leaves stdin open would hang with no output and
/// no error — exactly the failure mode stage 23 set out to eliminate, and one
/// that turned up while verifying stage 29. Two seconds is far longer than any
/// real producer needs to write its first byte.
const STDIN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Assemble the effective prompt: the `-p` text, plus stdin when it is piped.
///
/// Only read stdin when it is NOT a terminal — reading an interactive stdin
/// would block waiting for an EOF the user has no reason to send.
fn read_piped_stdin() -> Result<Option<String>> {
  use std::io::{IsTerminal, Read};
  if io::stdin().is_terminal() {
    return Ok(None);
  }

  // Read on a side thread so a silent pipe costs a bounded wait instead of
  // the whole run. The thread is abandoned rather than joined: it is blocked
  // on a read that may never return, and the process is about to do useful
  // work regardless.
  let (tx, rx) = std::sync::mpsc::channel();
  std::thread::spawn(move || {
    let mut buf = String::new();
    let outcome = io::stdin().read_to_string(&mut buf).map(|_| buf);
    let _ = tx.send(outcome);
  });

  let text = match rx.recv_timeout(STDIN_WAIT) {
    Ok(Ok(text)) => text,
    Ok(Err(e)) => return Err(anyhow::Error::from(e).context("cannot read stdin")),
    Err(_) => {
      eprintln!(
        "{}",
        format!(
          "[Input] stdin stayed open with no data for {:?}; continuing without it",
          STDIN_WAIT
        )
        .yellow()
      );
      return Ok(None);
    }
  };
  let trimmed = text.trim();
  if trimmed.is_empty() {
    Ok(None)
  } else {
    Ok(Some(trimmed.to_string()))
  }
}

async fn run_prompt(app: &mut App, prompt: String, format: OutputFormat) -> Result<i32> {
  let outcome = match app.run_headless(&prompt, None).await {
    Ok(o) => o,
    Err(e) => {
      // Errors go to stderr so stdout stays machine-readable even on failure.
      eprintln!("{} {:#}", "Error:".red(), e);
      return Ok(exit::RUNTIME_ERROR);
    }
  };

  match format {
    OutputFormat::Text => ui::result(&outcome.text),
    OutputFormat::Json => {
      let payload = serde_json::json!({
        "final": outcome.text,
        "status": match outcome.status {
          engine::LoopStatus::Completed => "completed",
          engine::LoopStatus::MaxIterations => "max_iterations",
          engine::LoopStatus::Interrupted => "interrupted",
        },
        "iterations": outcome.iterations,
        "llm_calls": outcome.llm_calls,
        "usage": {
          "prompt_tokens": app.cost.prompt_tokens,
          "completion_tokens": app.cost.completion_tokens,
          "cache_hit_pct": app.cost.cache_hit_pct(),
        },
        "cost_cny": app.cost.estimated_cny(),
      });
      ui::result(&serde_json::to_string_pretty(&payload)?);
    }
  }

  Ok(match outcome.status {
    engine::LoopStatus::Completed => exit::OK,
    engine::LoopStatus::MaxIterations => exit::NOT_CONVERGED,
    engine::LoopStatus::Interrupted => exit::POLICY_REFUSED,
  })
}

#[tokio::main]
async fn main() -> Result<()> {
  let cli = Cli::parse();

  if let Some(dir) = &cli.cwd {
    std::env::set_current_dir(dir).with_context(|| format!("cannot enter {}", dir.display()))?;
  }

  // Every non-REPL entry point is headless: no TTY to prompt, and stdout is
  // reserved for the result.
  let headless = cli.prompt.is_some() || cli.bench.is_some() || cli.run_task.is_some();
  ui::set_headless(headless);
  if headless {
    tools::approval::set_interaction(if cli.yes {
      tools::approval::Interaction::AutoApprove
    } else {
      tools::approval::Interaction::AutoDeny
    });
  }
  if cli.read_only {
    tools::policy::set_mode(tools::policy::Mode::ReadOnly);
  }

  // Task inspection needs config, not a provider or a model.
  if let Some(TaskCommand::Task { action }) = &cli.task {
    let loaded = Config::load()?;
    let text = match action {
      TaskAction::List => tasks::list(&loaded.config)?,
      TaskAction::Install { name } => tasks::render_plist(&loaded.config, name)?,
    };
    print!("{}", text);
    return Ok(());
  }

  let mut app = App::new().await?;
  if let Some(n) = cli.max_iter {
    app.max_iter = n.max(1);
  }

  if let Some(prompt) = cli.prompt {
    let prompt = match read_piped_stdin()? {
      Some(stdin) => format!("{}\n\n--- piped stdin ---\n{}", prompt, stdin),
      None => prompt,
    };
    let code = run_prompt(&mut app, prompt, cli.output).await?;
    tools::jobs::kill_all();
    std::process::exit(code);
  }
  if let Some(path) = cli.bench {
    let outcome = app.run_benchmark(&path, cli.trajectory.as_deref()).await;
    tools::jobs::kill_all();
    return outcome;
  }
  if let Some(name) = cli.run_task {
    let outcome = tasks::run_task(&mut app, &name).await;
    tools::jobs::kill_all();
    return outcome;
  }
  let outcome = app.run().await;
  // Jobs are children of this process: leaving them running after the REPL
  // exits would strand work nobody can collect the output of any more.
  tools::jobs::kill_all();
  outcome
}
