use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use futures_util::StreamExt;
use rustyline::Editor;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::FileHistory;
use rustyline::validate::Validator;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod agent;
mod api;
mod config;
mod history;
mod observability;
mod skills;
mod subagents;
mod tools;

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

  async fn chat(&mut self, content: &str) -> Result<()> {
    // Clear any stale interrupt flag from a previous turn (e.g. Ctrl-C
    // pressed at the readline prompt also fires the global watcher).
    self.interrupt.store(false, Ordering::SeqCst);

    self.current_session.messages.push(Message::Simple {
      role: "user".to_string(),
      content: content.to_string(),
      reasoning_content: None,
      tool_calls: None,
    });

    let tools = self.current_skill.as_ref().and_then(|s| s.to_api_tools());

    let mut messages = std::mem::take(&mut self.current_session.messages);
    Self::ensure_agent_system_prompt(&mut messages, self.plan_mode);
    let run_span = self.tracer.start_run();
    let (_final_content, updated_messages) = self
      .run_agent_loop(messages, tools, 0, agent::MAX_ITER, run_span)
      .await?;
    self.tracer.end(run_span);
    self.current_session.messages = updated_messages;

    if self.current_session.title == "New Chat" {
      self.current_session.title = content.chars().take(30).collect::<String>();
    }
    // Persist the session's running cost so it can be audited / restored later.
    self.current_session.cost = self.cost.clone();
    self.history.save_session(&self.current_session)?;

    // Print the session bill (token accounting + CNY estimate).
    if !self.cost.is_empty() {
      println!("{}", self.cost.summary());
    }
    // Flush the decision-path trace (no-op unless SEEKCLI_TRACE is set).
    match self.tracer.flush() {
      Ok(Some(path)) => println!("{} trace written to {}", "[Trace]".dimmed(), path.display()),
      Ok(None) => {}
      Err(e) => println!("{} trace write failed: {}", "[Trace]".yellow(), e),
    }
    Ok(())
  }

  /// Run the agent headlessly on a single prompt in the current working
  /// directory, returning the LLM-call count consumed (proxy for turns). Used
  /// by the benchmark runner; no session save, no REPL state.
  async fn run_headless(&mut self, prompt: &str) -> Result<u64> {
    let calls_before = self.cost.api_calls;
    let mut messages = vec![Message::new_user_text(prompt.to_string())];
    Self::ensure_agent_system_prompt(&mut messages, self.plan_mode);
    self
      .run_agent_loop(messages, None, 0, agent::MAX_ITER, None)
      .await?;
    Ok(self.cost.api_calls - calls_before)
  }

  /// Benchmark entry point: load a testsuite, run each task in an isolated
  /// testbed (Init → seed → AgentRun → Eval → Score), and print a report.
  /// Fail-to-Pass: a task passes iff its eval command exits 0.
  async fn run_benchmark(&mut self, suite_path: &std::path::Path) -> Result<()> {
    use observability::bench::{Report, TaskResult, TestSuite};

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

  /// Parse `{"subagent_type": "...", "prompt": "..."}` from a tool-call
  /// arguments string. Falls back to `("general", arguments)` if the payload
  /// is not the expected shape.
  fn parse_invoke_agent_args(arguments: &str) -> (String, String) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) {
      let subagent_type = v
        .get("subagent_type")
        .and_then(|s| s.as_str())
        .unwrap_or("general")
        .to_string();
      let prompt = v
        .get("prompt")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
      return (subagent_type, prompt);
    }
    ("general".to_string(), arguments.to_string())
  }

  fn parse_load_skill_args(arguments: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments)
      && let Some(n) = v.get("name").and_then(|s| s.as_str())
    {
      return n.to_string();
    }
    arguments.to_string()
  }

  /// Heuristic: does a tool result indicate a failure? Drives the Two-Stage
  /// ReAct micro trigger. `[Recovery]` is appended by recovery::augment on
  /// every classified failure; the other markers cover delegation/skill paths.
  fn result_is_failure(result: &str) -> bool {
    result.contains("[Recovery]")
      || result.contains("[BAD ARGS]")
      || result.starts_with("[ERROR]")
      || result.starts_with("Error executing")
      || result.contains("' failed:")
  }

  /// Two-Stage ReAct planning pass: a tools-free completion that forces the
  /// model to deliberate before acting. The plan text is appended to
  /// `messages` as an assistant message so the subsequent action call sees it.
  /// Called for the main agent only.
  async fn planning_phase(&self, messages: &mut Vec<Message>) -> Result<()> {
    println!("\n{}", "[Plan] deliberating (tools withheld)...".dimmed());
    let mut stream = self
      .brain
      .call_api_with_params(
        &self.model,
        messages.clone(),
        self.thinking_mode.as_str(),
        None,
      )
      .await?;

    let mut plan = String::new();
    let mut is_reasoning = false;
    while let Some(item) = stream.next().await {
      if self.interrupt.load(Ordering::SeqCst) {
        break;
      }
      match item? {
        StreamItem::Reasoning(r) => {
          if !is_reasoning {
            print!("\n{}", "Thinking: ".italic().bright_black());
            is_reasoning = true;
          }
          print!("{}", r.italic().bright_black());
        }
        StreamItem::Content(c) => {
          if is_reasoning {
            println!();
            is_reasoning = false;
          }
          print!("{}", c.dimmed());
          plan.push_str(&c);
        }
        _ => {}
      }
      io::stdout().flush()?;
    }
    println!();

    if !plan.trim().is_empty() {
      messages.push(Message::Simple {
        role: "assistant".to_string(),
        content: plan,
        reasoning_content: None,
        tool_calls: None,
      });
    }
    Ok(())
  }

  fn ensure_agent_system_prompt(messages: &mut Vec<Message>, plan_mode: bool) {
    // Plan Mode guidance is added/removed as the flag toggles. Marker-prefixed
    // so we can find and drop it without touching other system messages.
    let plan_msg = agent::prompt::plan_mode_rules();
    messages.retain(|m| {
      !matches!(
        m,
        Message::Simple { role, content, .. }
          if role == "system" && content.starts_with("# Plan Mode (active)")
      )
    });

    // Static kernel at index 0 — kept byte-identical so the prompt cache
    // prefix stays stable across turns/sessions.
    let target = agent::prompt::agent_system_prompt();
    let already_present = messages.first().is_some_and(|m| {
      matches!(
        m,
        Message::Simple { role, content: t, .. }
          if role == "system" && t == &target
      )
    });
    if !already_present {
      messages.insert(
        0,
        Message::Simple {
          role: "system".to_string(),
          content: target,
          reasoning_content: None,
          tool_calls: None,
        },
      );
    }

    // Dynamic Prompt Composer: inject workspace rules (AGENTS.md / CLAUDE.md)
    // as a SEPARATE system message right after the kernel, if present and not
    // already injected. Cache prefix (index 0) is unaffected.
    if let Ok(cwd) = std::env::current_dir()
      && let Some(rules) = agent::prompt::workspace_rules(&cwd)
    {
      let rules_present = messages.iter().any(|m| {
        matches!(
          m,
          Message::Simple { role, content: t, .. }
            if role == "system" && t == &rules
        )
      });
      if !rules_present {
        let insert_at = if messages
          .first()
          .is_some_and(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
        {
          1
        } else {
          0
        };
        messages.insert(
          insert_at,
          Message::Simple {
            role: "system".to_string(),
            content: rules,
            reasoning_content: None,
            tool_calls: None,
          },
        );
      }
    }

    // Plan Mode message goes after the leading run of system messages
    // (kernel + workspace rules + any active-skill prompt).
    if plan_mode {
      let head_end = messages
        .iter()
        .take_while(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
        .count();
      messages.insert(
        head_end,
        Message::Simple {
          role: "system".to_string(),
          content: plan_msg,
          reasoning_content: None,
          tool_calls: None,
        },
      );
    }
  }

  /// Extract fenced code blocks from markdown-style text. Lightweight
  /// non-regex scan; powers `/copy` after the renderer was removed.
  fn extract_code_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut current = String::new();
    for line in text.lines() {
      if line.trim_start().starts_with("```") {
        if in_block {
          blocks.push(std::mem::take(&mut current));
          in_block = false;
        } else {
          in_block = true;
        }
      } else if in_block {
        current.push_str(line);
        current.push('\n');
      }
    }
    blocks
  }

  async fn run_agent_loop(
    &mut self,
    mut messages: Vec<Message>,
    tools: Option<Vec<api::Tool>>,
    depth: usize,
    max_iter: usize,
    parent_span: Option<usize>,
  ) -> Result<(String, Vec<Message>)> {
    if depth > agent::MAX_SUBAGENT_DEPTH {
      anyhow::bail!(
        "Max sub-agent depth ({}) exceeded",
        agent::MAX_SUBAGENT_DEPTH
      );
    }

    let tool_dispatcher = tools::ToolDispatcher::new();
    let effective_tools = if depth == 0 {
      tools::registry::merge_with_skill(tools)
    } else {
      tools.unwrap_or_default()
    };

    let mut final_content = String::new();
    let mut completed = false;
    // Doom-loop detector — main agent only. Persists across iterations of this
    // chat turn so it can spot repeated tool-call trajectories.
    let mut reminder_injector = agent::reminders::ReminderInjector::new();
    // Two-Stage ReAct: set when the previous turn failed, forcing a tools-free
    // planning pass before the next action (micro trigger).
    let mut plan_next = false;

    for iter in 0..max_iter {
      // Top-of-iteration interrupt check (Ctrl-C between turns).
      if depth == 0 && self.interrupt.swap(false, Ordering::SeqCst) {
        println!("\n{}", "[Agent] interrupted by user".yellow());
        final_content = "[Interrupted by user]".to_string();
        completed = true;
        break;
      }

      let turn_span = self.tracer.begin(
        "turn",
        &format!("iter {} (depth {})", iter, depth),
        parent_span,
      );

      // Compression only at top-level main agent; sub-agents have short focused
      // contexts and their own max_iter cap.
      if depth == 0 {
        let cspan = self.tracer.begin("compaction", "maybe_compress", turn_span);
        if let Err(e) =
          agent::compressor::maybe_compress(&self.brain, &self.model, &mut messages).await
        {
          println!(
            "{} compression failed: {} (continuing without)",
            "[Memory]".yellow(),
            e
          );
        }
        self.tracer.end(cspan);
      }

      // Two-Stage ReAct (dynamic): before acting, run a tools-free planning
      // pass when (a) opening a task with thinking enabled — macro trigger,
      // or (b) the previous turn hit a tool failure — micro trigger.
      // Withholding tool schemas forces the model to deliberate instead of
      // reflexively calling a tool. Main agent only.
      if depth == 0 {
        let macro_trigger = iter == 0 && self.thinking_mode != ThinkingMode::None;
        if macro_trigger || plan_next {
          let pspan = self.tracer.begin("planning", "two-stage", turn_span);
          if let Err(e) = self.planning_phase(&mut messages).await {
            println!(
              "{} planning phase failed: {} (continuing)",
              "[Plan]".yellow(),
              e
            );
          }
          self.tracer.end(pspan);
        }
      }

      let gen_span = self.tracer.begin("generate", "llm action", turn_span);
      let mut stream = self
        .brain
        .call_api_with_params(
          &self.model,
          messages.clone(),
          self.thinking_mode.as_str(),
          Some(effective_tools.clone()),
        )
        .await?;

      let mut assistant_content = String::new();
      let mut assistant_reasoning = String::new();
      let mut tool_calls = Vec::new();
      let mut is_reasoning = false;

      while let Some(item) = stream.next().await {
        // Mid-stream interrupt check. Don't reset the flag here — let the
        // outer loop see it and exit cleanly.
        if depth == 0 && self.interrupt.load(Ordering::SeqCst) {
          println!("\n{}", "[Agent] interrupted by user (mid-stream)".yellow());
          break;
        }
        match item? {
          StreamItem::Reasoning(r) => {
            if !is_reasoning {
              print!("\n{}", "Thinking: ".italic().bright_black());
              is_reasoning = true;
            }
            print!("{}", r.italic().bright_black());
            assistant_reasoning.push_str(&r);
          }
          StreamItem::Content(c) => {
            if is_reasoning {
              println!();
              is_reasoning = false;
            }
            print!("{}", c);
            assistant_content.push_str(&c);
          }
          StreamItem::ToolCall(tc) => {
            let preview = {
              let args = &tc.function.arguments;
              if args.len() > 160 {
                let mut cut = 160;
                while cut > 0 && !args.is_char_boundary(cut) {
                  cut -= 1;
                }
                format!("{}…", &args[..cut])
              } else {
                args.clone()
              }
            };
            println!(
              "\n{} Called: {} {}",
              "Agent:".cyan(),
              tc.function.name.yellow(),
              preview.bright_black()
            );
            tool_calls.push(tc);
          }
          StreamItem::Finish(reason) => {
            println!();
            if let Some(r) = reason
              && r == "length"
            {
              println!("\n{}", "[Note: Max output limit reached.]".yellow());
            }
          }
          StreamItem::Usage(info) => {
            let pct = info
              .prompt_cache_hit_tokens
              .checked_mul(100)
              .and_then(|n| n.checked_div(info.prompt_tokens))
              .unwrap_or(0);
            println!(
              "{} prompt={} (cache hit {}%, {} miss), completion={}",
              "[Usage]".dimmed(),
              info.prompt_tokens,
              pct,
              info.prompt_cache_miss_tokens,
              info.completion_tokens
            );
            // Fold into the running session bill (decorator-style accounting).
            self.cost.record(&info);
          }
        }
        io::stdout().flush()?;
      }
      self.tracer.annotate(
        gen_span,
        serde_json::json!({ "tool_calls": tool_calls.len() }),
      );
      self.tracer.end(gen_span);

      self.last_code_blocks = Self::extract_code_blocks(&assistant_content);

      messages.push(Message::Simple {
        role: "assistant".to_string(),
        content: assistant_content.clone(),
        reasoning_content: if assistant_reasoning.is_empty() {
          None
        } else {
          Some(assistant_reasoning)
        },
        tool_calls: if tool_calls.is_empty() {
          None
        } else {
          Some(tool_calls.clone())
        },
      });

      if tool_calls.is_empty() {
        final_content = assistant_content;
        completed = true;
        self.tracer.end(turn_span);
        break;
      }

      let exec_span = self.tracer.begin(
        "execute",
        &format!("{} tool(s)", tool_calls.len()),
        turn_span,
      );
      println!("\n{} Executing tools...", "Agent:".cyan());
      // Snapshot this turn's trajectory for doom-loop detection before the
      // calls are consumed below.
      let turn_tool_calls = tool_calls.clone();
      // Tracks whether any tool failed this turn — drives the Two-Stage ReAct
      // micro trigger (force a planning pass before the next action).
      let mut turn_had_failure = false;

      // Fork-Join: if EVERY call this turn is pure read-only, run them
      // concurrently (the harness "read-concurrent, write-serial" rule). Any
      // write / shell / delegation forces the safe sequential path below.
      let parallelizable = tool_calls.len() > 1
        && tool_calls
          .iter()
          .all(|tc| tools::registry::is_parallel_readonly(&tc.function.name));

      if parallelizable {
        println!(
          "{} {} read-only tools — running concurrently",
          "Agent:".cyan(),
          tool_calls.len()
        );
        let futs = tool_calls.iter().map(|tc| {
          let disp = &tool_dispatcher;
          let name = tc.function.name.clone();
          let args = tc.function.arguments.clone();
          let id = tc.id.clone();
          async move {
            let raw = match disp.execute(&name, &args).await {
              Ok(res) => res,
              Err(e) => format!("Error executing tool {}: {}", name, e),
            };
            (id, agent::recovery::augment(&name, raw))
          }
        });
        let results = futures_util::future::join_all(futs).await;
        for (id, content) in results {
          turn_had_failure |= Self::result_is_failure(&content);
          messages.push(Message::ToolResponse {
            role: "tool".to_string(),
            content,
            tool_call_id: id,
          });
        }
      } else {
        // Side-effect system messages (e.g. from load_skill) must be appended
        // AFTER all ToolResponse messages for this turn — DeepSeek's validator
        // requires assistant{tool_calls} to be immediately followed by its
        // matching tool messages, with no system message interleaved.
        let mut deferred_system_msgs: Vec<Message> = Vec::new();
        for tc in tool_calls {
          let result_str = if tc.function.name == "invoke_agent" {
            let (subagent_type, prompt) = Self::parse_invoke_agent_args(&tc.function.arguments);
            let next_depth = depth + 1;
            if next_depth > agent::MAX_SUBAGENT_DEPTH {
              format!(
                "Cannot spawn sub-agent: max depth {} reached.",
                agent::MAX_SUBAGENT_DEPTH
              )
            } else {
              match subagents::registry::lookup(&subagent_type) {
                None => {
                  let listing: Vec<String> = subagents::registry::catalog()
                    .iter()
                    .map(|(name, desc)| format!("  - {}: {}", name, desc))
                    .collect();
                  format!(
                    "Unknown subagent_type '{}'. Available types:\n{}",
                    subagent_type,
                    listing.join("\n")
                  )
                }
                Some(template) => {
                  let sub_tools =
                    tools::registry::filter_by_allowed(&effective_tools, template.allowed_tools);
                  let sub_messages = vec![
                    Message::Simple {
                      role: "system".to_string(),
                      content: template.system_prompt.to_string(),
                      reasoning_content: None,
                      tool_calls: None,
                    },
                    Message::Simple {
                      role: "user".to_string(),
                      content: prompt,
                      reasoning_content: None,
                      tool_calls: None,
                    },
                  ];

                  println!(
                    "{} Spawning sub-agent '{}' (depth={}, max_iter={})...",
                    "Agent:".magenta(),
                    template.name.green(),
                    next_depth,
                    template.max_iter
                  );
                  match Box::pin(self.run_agent_loop(
                    sub_messages,
                    Some(sub_tools),
                    next_depth,
                    template.max_iter,
                    exec_span,
                  ))
                  .await
                  {
                    Ok((res, _)) => {
                      format!("Sub-agent '{}' completed. Summary:\n{}", template.name, res)
                    }
                    Err(e) => format!("Sub-agent '{}' failed: {}", template.name, e),
                  }
                }
              }
            }
          } else if tc.function.name == "load_skill" {
            if depth > 0 {
              "[ERROR] load_skill is restricted to the main agent. \
             Sub-agents cannot switch skills."
                .to_string()
            } else {
              let name = Self::parse_load_skill_args(&tc.function.arguments);
              match self.skill_manager.load_skills() {
                Err(e) => format!("Failed to enumerate skills: {}", e),
                Ok(skills) => {
                  let found = skills.iter().find(|s| s.name == name).cloned();
                  match found {
                    Some(skill) => {
                      // Same dedup as activate_skill: drop any prior skill's
                      // system message so personas don't accumulate.
                      messages.retain(|m| {
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
                      deferred_system_msgs.push(Message::Simple {
                        role: "system".to_string(),
                        content: prompt_text,
                        reasoning_content: None,
                        tool_calls: None,
                      });
                      let skill_name = skill.name.clone();
                      self.current_skill = Some(skill);
                      println!("{} Loaded skill: {}", "✦".cyan(), skill_name.green());
                      format!(
                        "Skill '{}' loaded. Its system prompt is now active. \
                       Continue the user's task in this persona.",
                        skill_name
                      )
                    }
                    None => {
                      let available: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
                      format!("Skill '{}' not found. Available: {:?}", name, available)
                    }
                  }
                }
              }
            }
          } else {
            let raw = match tool_dispatcher
              .execute(&tc.function.name, &tc.function.arguments)
              .await
            {
              Ok(res) => res,
              Err(e) => format!("Error executing tool {}: {}", tc.function.name, e),
            };
            // Context-aware Error Recovery: append an actionable hint when the
            // result looks like a failure, so the model follows a debug SOP
            // instead of blindly retrying.
            agent::recovery::augment(&tc.function.name, raw)
          };

          turn_had_failure |= Self::result_is_failure(&result_str);
          messages.push(Message::ToolResponse {
            role: "tool".to_string(),
            content: result_str,
            tool_call_id: tc.id,
          });
        }
        // Now safe to append deferred system messages (skill activations etc).
        // Order is: assistant{tool_calls} → tool{responses} → system{side-effects}.
        messages.extend(deferred_system_msgs);
      }
      self.tracer.annotate(
        exec_span,
        serde_json::json!({ "had_failure": turn_had_failure }),
      );
      self.tracer.end(exec_span);

      // System Reminder: if the main agent is repeating the same trajectory,
      // inject a high-priority user message at the point of decision to break
      // the doom loop. Sub-agents rely on their max_iter cap instead.
      if depth == 0
        && let Some(reminder) = reminder_injector.observe(&turn_tool_calls)
      {
        println!(
          "\n{}",
          "[System Reminder] doom loop detected — intervening".red()
        );
        messages.push(Message::new_user_text(reminder));
      }

      // Two-Stage ReAct micro trigger: a failed turn forces a tools-free
      // planning pass at the top of the next iteration.
      plan_next = depth == 0 && turn_had_failure;

      self.tracer.end(turn_span);
      println!("{} Returning tool results to model...", "Agent:".cyan());
    }

    if !completed {
      println!(
        "\n{}",
        format!("[Agent: reached max iterations ({})]", agent::MAX_ITER).yellow()
      );
      if final_content.is_empty() {
        final_content = format!("[Stopped at max iterations ({})]", agent::MAX_ITER);
      }
    }

    Ok((final_content, messages))
  }
}

// ---------------------------------------------------------------------------
// Rustyline command completer
// ---------------------------------------------------------------------------

/// Top-level slash commands eligible for tab completion.
const SLASH_COMMANDS: &[&str] = &[
  "/help",
  "/quit",
  "/exit",
  "/clear",
  "/history",
  "/copy",
  "/model",
  "/thinking",
  "/plan",
  "/skill",
  "/load",
];

/// Subcommands for `/skill` that aren't skill names.
const SKILL_SUBCOMMANDS: &[&str] = &["list", "proposals", "migrate", "accept", "reject"];

const MODEL_VARIANTS: &[&str] = &["flash", "pro"];
const THINKING_MODES: &[&str] = &["n", "h", "m"];

/// Rustyline `Helper` that provides command completion. Re-scans the skill
/// directory on every Tab press so newly created skills / proposals are
/// immediately discoverable without restarting.
struct CmdCompleter {
  skills_dir: PathBuf,
  proposals_dir: PathBuf,
}

impl CmdCompleter {
  fn active_skill_names(&self) -> Vec<String> {
    list_skill_names(&self.skills_dir, true)
  }

  fn proposal_names(&self) -> Vec<String> {
    list_skill_names(&self.proposals_dir, false)
  }
}

/// Enumerate skill names from `dir`, recognizing both legacy `<name>.json`
/// and the new `<name>/SKILL.md` directory format.
fn list_skill_names(dir: &std::path::Path, skip_proposals_subdir: bool) -> Vec<String> {
  let mut names = Vec::new();
  let read = match std::fs::read_dir(dir) {
    Ok(r) => r,
    Err(_) => return names,
  };
  for entry in read.flatten() {
    let path = entry.path();
    let raw_name = match path.file_name().and_then(|s| s.to_str()) {
      Some(n) => n,
      None => continue,
    };
    if skip_proposals_subdir && raw_name == "proposals" {
      continue;
    }
    if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
      if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        names.push(stem.to_string());
      }
    } else if path.is_dir() && path.join("SKILL.md").exists() {
      names.push(raw_name.to_string());
    }
  }
  names.sort();
  names.dedup();
  names
}

fn pairs<I, S>(items: I) -> Vec<Pair>
where
  I: IntoIterator<Item = S>,
  S: Into<String>,
{
  items
    .into_iter()
    .map(|s| {
      let s = s.into();
      Pair {
        display: s.clone(),
        replacement: s,
      }
    })
    .collect()
}

impl Completer for CmdCompleter {
  type Candidate = Pair;

  fn complete(
    &self,
    line: &str,
    pos: usize,
    _ctx: &rustyline::Context<'_>,
  ) -> rustyline::Result<(usize, Vec<Pair>)> {
    let prefix = &line[..pos.min(line.len())];

    // /skill <subcommand or name>
    if let Some(rest) = prefix.strip_prefix("/skill ") {
      let after_space = pos - rest.len();

      // /skill accept <proposal>
      if let Some(name_prefix) = rest.strip_prefix("accept ") {
        let start = after_space + "accept ".len();
        let matches: Vec<String> = self
          .proposal_names()
          .into_iter()
          .filter(|n| n.starts_with(name_prefix))
          .collect();
        return Ok((start, pairs(matches)));
      }

      // /skill reject <proposal>
      if let Some(name_prefix) = rest.strip_prefix("reject ") {
        let start = after_space + "reject ".len();
        let matches: Vec<String> = self
          .proposal_names()
          .into_iter()
          .filter(|n| n.starts_with(name_prefix))
          .collect();
        return Ok((start, pairs(matches)));
      }

      // /skill <single token>: either subcommand or active skill name
      let mut candidates: Vec<String> = SKILL_SUBCOMMANDS.iter().map(|s| s.to_string()).collect();
      candidates.extend(self.active_skill_names());
      candidates.sort();
      candidates.dedup();
      let matches: Vec<String> = candidates
        .into_iter()
        .filter(|c| c.starts_with(rest))
        .collect();
      return Ok((after_space, pairs(matches)));
    }

    if let Some(rest) = prefix.strip_prefix("/model ") {
      let start = pos - rest.len();
      let matches: Vec<String> = MODEL_VARIANTS
        .iter()
        .filter(|v| v.starts_with(rest))
        .map(|v| v.to_string())
        .collect();
      return Ok((start, pairs(matches)));
    }

    if let Some(rest) = prefix.strip_prefix("/thinking ") {
      let start = pos - rest.len();
      let matches: Vec<String> = THINKING_MODES
        .iter()
        .filter(|v| v.starts_with(rest))
        .map(|v| v.to_string())
        .collect();
      return Ok((start, pairs(matches)));
    }

    // Bare slash command
    if prefix.starts_with('/') && !prefix.contains(' ') {
      let matches: Vec<String> = SLASH_COMMANDS
        .iter()
        .filter(|c| c.starts_with(prefix))
        .map(|c| c.to_string())
        .collect();
      return Ok((0, pairs(matches)));
    }

    Ok((pos, Vec::new()))
  }
}

impl Hinter for CmdCompleter {
  type Hint = String;
}

impl Highlighter for CmdCompleter {}

impl Validator for CmdCompleter {}

impl rustyline::Helper for CmdCompleter {}

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
