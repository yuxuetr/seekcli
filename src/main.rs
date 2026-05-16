use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use futures_util::StreamExt;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::env;
use std::io::{self, Write};

mod agent;
mod api;
mod config;
mod history;
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
  current_skill: Option<Skill>,
  last_code_blocks: Vec<String>,
}

impl App {
  fn new() -> Result<Self> {
    let config = Config::load()?;
    let deepseek_key = env::var("DEEPSEEK_API_KEY").context("Please set DEEPSEEK_API_KEY")?;
    let brain = ApiClient::new(deepseek_key);
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
      current_skill: None,
      last_code_blocks: Vec::new(),
    })
  }

  fn print_help(&self) {
    println!("{}", "\nAvailable Commands:".bold().yellow());
    println!("  /model [flash|pro]      Switch DeepSeek model");
    println!("  /thinking [n|h|m]       Switch thinking intensity (None/High/Max)");
    println!("  /skill list             List active skills");
    println!("  /skill <name>           Activate a skill");
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
    let mut rl = DefaultEditor::new()?;

    loop {
      let skill_label = self
        .current_skill
        .as_ref()
        .map(|s| format!("|{}", s.name))
        .unwrap_or_default();
      let prompt = format!(
        "{} ({}{}) {} ",
        self.model.blue(),
        self.thinking_mode.label().magenta(),
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
    self.current_skill = Some(skill.clone());
    if self.current_session.messages.is_empty() {
      self.current_session.messages.push(Message::Simple {
        role: "system".to_string(),
        content: skill.system_prompt,
        reasoning_content: None,
        tool_calls: None,
      });
    }
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
      "/history" => {
        let sessions = self.history.list_sessions()?;
        for s in sessions.iter().take(10) {
          println!(
            "- {} ({})",
            s.title,
            s.id.chars().take(8).collect::<String>()
          );
        }
      }
      "/load" => {
        if parts.len() > 1 {
          let prefix = parts[1];
          let sessions = self.history.list_sessions()?;
          if let Some(s) = sessions.iter().find(|s| s.id.starts_with(prefix)) {
            self.current_session = self.history.load_session(&s.id)?;
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
    self.current_session.messages.push(Message::Simple {
      role: "user".to_string(),
      content: content.to_string(),
      reasoning_content: None,
      tool_calls: None,
    });

    let tools = self.current_skill.as_ref().and_then(|s| s.to_api_tools());

    let mut messages = std::mem::take(&mut self.current_session.messages);
    Self::ensure_agent_system_prompt(&mut messages);
    let (_final_content, updated_messages) = self
      .run_agent_loop(messages, tools, 0, agent::MAX_ITER)
      .await?;
    self.current_session.messages = updated_messages;

    if self.current_session.title == "New Chat" {
      self.current_session.title = content.chars().take(30).collect::<String>();
    }
    self.history.save_session(&self.current_session)?;
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

  fn ensure_agent_system_prompt(messages: &mut Vec<Message>) {
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

    for _iter in 0..max_iter {
      // Compression only at top-level main agent; sub-agents have short focused
      // contexts and their own max_iter cap.
      if depth == 0
        && let Err(e) =
          agent::compressor::maybe_compress(&self.brain, &self.model, &mut messages).await
      {
        println!(
          "{} compression failed: {} (continuing without)",
          "[Memory]".yellow(),
          e
        );
      }

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
          }
        }
        io::stdout().flush()?;
      }

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
        break;
      }

      println!("\n{} Executing tools...", "Agent:".cyan());
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
          match tool_dispatcher
            .execute(&tc.function.name, &tc.function.arguments)
            .await
          {
            Ok(res) => res,
            Err(e) => format!("Error executing tool {}: {}", tc.function.name, e),
          }
        };

        messages.push(Message::ToolResponse {
          role: "tool".to_string(),
          content: result_str,
          tool_call_id: tc.id,
        });
      }
      // Now safe to append deferred system messages (skill activations etc).
      // Order is: assistant{tool_calls} → tool{responses} → system{side-effects}.
      messages.extend(deferred_system_msgs);
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

#[derive(Parser)]
#[command(author, version, about = "DeepSeek V4 Harness Agent for CLI", long_about = None)]
struct Cli {}

#[tokio::main]
async fn main() -> Result<()> {
  let _cli = Cli::parse();
  let mut app = App::new()?;
  app.run().await
}
