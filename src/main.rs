use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod agent;
mod api;
mod benchmark;
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

  fn print_help(&self) {
    println!("{}", "\nAvailable Commands:".bold().yellow());
    println!("  /model [flash|pro]      Switch DeepSeek model");
    println!("  /thinking [n|h|m]       Switch thinking intensity (None/High/Max)");
    println!("  /plan [on|off]          Toggle Plan Mode (externalize state to PLAN.md/TODO.md)");
    println!("  /skill list             List active skills");
    println!("  /skill <name> [prompt]  Activate a skill (optional: send prompt immediately)");
    println!("  /skill proposals        List pending skill proposals from the agent");
    println!("  /skill accept <name>    Promote a proposal to active skill");
    println!("  /skill reject <name>    Discard a skill proposal");
    println!("  /skill migrate          Convert legacy <name>.json skills to <name>/SKILL.md");
    println!("  /copy [index]           Copy code block from last response");
    println!("  /clear                  Reset conversation");
    println!("  /history                List previous sessions");
    println!("  /load <id>              Load a previous session by id prefix");
    println!("  /help                   Show this help");
    println!("  /quit                   Exit\n");
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

  fn activate_skill(&mut self, skill: Skill) {
    // Drop any previously-activated skill's system message so we don't
    // accumulate conflicting personas when switching mid-session.
    self.current_session.messages.retain(|m| {
      !matches!(
        m,
        Message::Simple { role, content, .. }
          if role == "system" && content.starts_with("# Activated Skill: ")
      )
    });

    let prompt_text = format!(
      "# Activated Skill: {}\n\n{}",
      skill.name, skill.system_prompt
    );
    self.current_session.messages.push(Message::Simple {
      role: "system".to_string(),
      content: prompt_text,
      reasoning_content: None,
      tool_calls: None,
    });
    self.current_skill = Some(skill);
  }

  async fn handle_command(&mut self, line: &str) -> Result<bool> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
      return Ok(false);
    }
    let cmd = parts[0];

    match cmd {
      "/quit" | "/exit" => return Ok(true),
      "/help" => self.print_help(),
      "/clear" => {
        self.current_session = self.history.create_session(self.model.clone());
        self.current_skill = None;
        self.cost = observability::cost::CostTracker::new();
        println!("{}", "Conversation reset.".yellow());
      }
      "/skill" => match parts.get(1).copied() {
        None => println!(
          "{} Usage: /skill list | /skill proposals | /skill <name> | /skill accept <name> | /skill reject <name> | /skill migrate",
          "Info:".blue()
        ),
        Some("list") => {
          let skills = self.skill_manager.load_skills()?;
          if skills.is_empty() {
            println!("{} No skills installed.", "Info:".blue());
          } else {
            for s in skills {
              println!("- {}: {}", s.name.bold(), s.description);
            }
          }
        }
        Some("proposals") => {
          let proposals = self.skill_manager.list_proposals()?;
          if proposals.is_empty() {
            println!("{} No skill proposals pending.", "Info:".blue());
          } else {
            println!(
              "Pending proposals (run {} or {}):",
              "/skill accept <name>".green(),
              "/skill reject <name>".yellow()
            );
            for s in proposals {
              println!("- {}: {}", s.name.bold(), s.description);
            }
          }
        }
        Some("accept") => match parts.get(2) {
          None => println!("{} Usage: /skill accept <name>", "Info:".blue()),
          Some(name) => match self.skill_manager.accept_proposal(name) {
            Ok(()) => println!(
              "{} Promoted '{}' to active skill.",
              "Success:".green(),
              name
            ),
            Err(e) => println!("{} {}", "Error:".red(), e),
          },
        },
        Some("reject") => match parts.get(2) {
          None => println!("{} Usage: /skill reject <name>", "Info:".blue()),
          Some(name) => match self.skill_manager.reject_proposal(name) {
            Ok(()) => println!("{} Discarded proposal '{}'.", "Success:".green(), name),
            Err(e) => println!("{} {}", "Error:".red(), e),
          },
        },
        Some("migrate") => match self.skill_manager.migrate_legacy() {
          Err(e) => println!("{} migrate failed: {}", "Error:".red(), e),
          Ok(report) => {
            if report.migrated.is_empty() && report.skipped.is_empty() && report.errors.is_empty() {
              println!("{} No legacy .json skills to migrate.", "Info:".blue());
            } else {
              for name in &report.migrated {
                println!(
                  "{} migrated '{}' → {}/SKILL.md (backup: {}.json.bak)",
                  "Success:".green(),
                  name,
                  name,
                  name
                );
              }
              for s in &report.skipped {
                println!("{} skipped: {}", "Info:".blue(), s);
              }
              for e in &report.errors {
                println!("{} {}", "Error:".red(), e);
              }
              println!(
                "\nTotals: {} migrated, {} skipped, {} errors.",
                report.migrated.len(),
                report.skipped.len(),
                report.errors.len()
              );
            }
          }
        },
        Some(name) => {
          let skills = self.skill_manager.load_skills()?;
          if let Some(skill) = skills.into_iter().find(|s| s.name == name) {
            println!("{} Activated skill: {}", "✦".cyan(), skill.name.green());
            self.activate_skill(skill);
            // If the user wrote `/skill <name> rest of prompt`, treat the
            // trailing tokens as an immediate chat turn after activation.
            if parts.len() > 2 {
              let prompt = parts[2..].join(" ");
              self.chat(&prompt).await?;
            }
          } else {
            println!("{} Skill not found: {}", "Error:".red(), name);
          }
        }
      },
      "/model" => {
        if parts.len() > 1 {
          self.model = match parts[1] {
            "flash" => self.config.brain.flash_model.clone(),
            "pro" => self.config.brain.pro_model.clone(),
            _ => self.model.clone(),
          };
        }
        println!("Model: {}", self.model.cyan());
      }
      "/thinking" => {
        if parts.len() > 1 {
          self.thinking_mode = match parts[1] {
            "n" => ThinkingMode::None,
            "h" => ThinkingMode::High,
            "m" => ThinkingMode::Max,
            _ => self.thinking_mode,
          };
        }
        println!("Thinking: {:?}", self.thinking_mode);
      }
      "/plan" => {
        // Optional explicit on/off, else toggle.
        self.plan_mode = match parts.get(1).copied() {
          Some("on") => true,
          Some("off") => false,
          _ => !self.plan_mode,
        };
        if self.plan_mode {
          println!(
            "{} Plan Mode {} — agent will externalize state to PLAN.md / TODO.md",
            "✦".cyan(),
            "ON".green()
          );
        } else {
          println!("{} Plan Mode {}", "✦".cyan(), "OFF".yellow());
        }
      }
      "/history" => {
        let sessions = self.history.list_sessions()?;
        for s in sessions.iter().take(10) {
          let cost_note = if s.cost.is_empty() {
            String::new()
          } else {
            format!(" · ≈¥{:.4}", s.cost.estimated_cny())
          };
          println!(
            "- {} ({}){}",
            s.title,
            s.id.chars().take(8).collect::<String>(),
            cost_note.dimmed()
          );
        }
      }
      "/load" => {
        if parts.len() > 1 {
          let prefix = parts[1];
          let sessions = self.history.list_sessions()?;
          if let Some(s) = sessions.iter().find(|s| s.id.starts_with(prefix)) {
            self.current_session = self.history.load_session(&s.id)?;
            // Restore the loaded session's cost so the bill continues from
            // where it left off rather than mixing with the prior session.
            self.cost = self.current_session.cost.clone();
            println!("{} Loaded session: {}", "✦".cyan(), s.title);
          } else {
            println!("{} No session matching: {}", "Error:".red(), prefix);
          }
        } else {
          println!("{} Usage: /load <id>", "Info:".blue());
        }
      }
      "/copy" => self.handle_copy(&parts)?,
      _ => println!("Unknown command. Try /help"),
    }
    Ok(false)
  }

  fn handle_copy(&self, parts: &[&str]) -> Result<()> {
    if parts.len() <= 1 {
      if self.last_code_blocks.is_empty() {
        println!("{} No code blocks in last response.", "Info:".blue());
      } else {
        println!(
          "{} Specify index (1-{}) to copy.",
          "Info:".blue(),
          self.last_code_blocks.len()
        );
      }
      return Ok(());
    }
    let Ok(idx) = parts[1].parse::<usize>() else {
      println!("{} /copy expects a number.", "Error:".red());
      return Ok(());
    };
    if idx == 0 || idx > self.last_code_blocks.len() {
      println!(
        "{} Invalid index. Range: 1-{}",
        "Error:".red(),
        self.last_code_blocks.len()
      );
      return Ok(());
    }
    let code = &self.last_code_blocks[idx - 1];
    #[cfg(target_os = "macos")]
    {
      let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
      if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(code.trim().as_bytes())?;
      }
      child.wait()?;
      println!("{} Block {} copied.", "Success:".green(), idx);
    }
    #[cfg(not(target_os = "macos"))]
    {
      let _ = code;
      println!("{} /copy is only implemented on macOS.", "Info:".blue());
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
