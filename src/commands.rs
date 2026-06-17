//! REPL command handling: slash-command dispatch (/skill, /model, /plan, …),
//! help text, skill activation, and clipboard copy. Split out of main.rs as a
//! separate `impl App` block; as a child module it keeps access to App's
//! private fields. Only handle_command is pub(crate) (called from the REPL
//! loop); the rest are dispatched from within it.

use anyhow::Result;
use colored::Colorize;
use std::io::Write;

use crate::{App, Message, Skill, ThinkingMode, observability};

impl App {
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

  pub(crate) async fn handle_command(&mut self, line: &str) -> Result<bool> {
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
