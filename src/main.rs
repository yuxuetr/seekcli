use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use std::env;
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
mod observability;
mod skills;
mod subagents;
mod tools;

use completer::CmdCompleter;

#[cfg(test)]
mod test;

pub use api::{ApiClient, Message, StreamItem, ToolCall};
pub use config::Config;
pub use history::{HistoryManager, Session};
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
  brain: ApiClient,
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
  /// Set to true by the Ctrl-C watcher task. Polled at the top of each
  /// agent loop iteration and during stream consumption to allow graceful
  /// mid-task interruption back to the REPL.
  interrupt: Arc<AtomicBool>,
}

impl App {
  fn new() -> Result<Self> {
    let config = Config::load()?;
    // Install the user's shell-command allow/deny policy (three-state approval).
    tools::approval::init_policy(config.security.allow.clone(), config.security.deny.clone());
    let deepseek_key = env::var("DEEPSEEK_API_KEY").context("Please set DEEPSEEK_API_KEY")?;
    let brain = ApiClient::new(deepseek_key);
    let history = HistoryManager::new()?;
    let skill_manager = SkillManager::new()?;
    let model = config.brain.flash_model.clone();
    let current_session = history.create_session(model.clone());

    let interrupt = Arc::new(AtomicBool::new(false));
    spawn_interrupt_watcher(interrupt.clone());

    Ok(Self {
      brain,
      config,
      history,
      skill_manager,
      current_session,
      model,
      thinking_mode: ThinkingMode::None,
      plan_mode: false,
      current_skill: None,
      last_code_blocks: Vec::new(),
      cost: observability::cost::CostTracker::new(),
      tracer: observability::trace::Trace::from_env(),
      interrupt,
    })
  }

  async fn run(&mut self) -> Result<()> {
    println!(
      "{}",
      format!("SeekCLI (DeepSeek {} Harness Agent)", self.model)
        .bold()
        .green()
    );

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
#[command(author, version, about = "DeepSeek V4 Harness Agent for CLI", long_about = None)]
struct Cli {
  /// Run a benchmark testsuite (JSON) headlessly instead of the REPL.
  #[arg(long, value_name = "TESTSUITE.json")]
  bench: Option<PathBuf>,
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

#[tokio::main]
async fn main() -> Result<()> {
  let cli = Cli::parse();
  let mut app = App::new()?;
  match cli.bench {
    Some(path) => app.run_benchmark(&path).await,
    None => app.run().await,
  }
}
