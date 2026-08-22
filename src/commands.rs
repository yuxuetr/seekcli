//! REPL command handling: slash-command dispatch (/skill, /model, /plan, …),
//! help text, skill activation, and clipboard copy. Split out of main.rs as a
//! separate `impl App` block; as a child module it keeps access to App's
//! private fields. Only handle_command is pub(crate) (called from the REPL
//! loop); the rest are dispatched from within it.

use anyhow::Result;
use colored::Colorize;
use std::io::Write;

use crate::session::{EventPayload, PromptKind};
use crate::{App, Skill, ThinkingMode, observability};

impl App {
  fn print_help(&self) {
    println!("{}", "\nAvailable Commands:".bold().yellow());
    println!("  /model [flash|pro]      Switch DeepSeek model");
    println!("  /thinking [n|h|m]       Switch thinking intensity (None/High/Max)");
    println!("  /plan [on|off]          Toggle Plan Mode (externalize state to PLAN.md/TODO.md)");
    println!("  /readonly [on|off]      Toggle read-only mode (refuse all mutating tools)");
    println!("  /skill list             List active skills");
    println!("  /skill <name> [prompt]  Activate a skill (optional: send prompt immediately)");
    println!("  /skill proposals        List pending skill proposals from the agent");
    println!("  /skill accept <name>    Promote a proposal to active skill");
    println!("  /skill reject <name>    Discard a skill proposal");
    println!("  /skill migrate          Convert legacy <name>.json skills to <name>/SKILL.md");
    println!("  /copy [index]           Copy code block from last response");
    println!("  /clear                  Reset conversation");
    println!("  /history                List previous sessions");
    println!("  /resume <id>            Resume a previous session (alias: /load)");
    println!("  /fork <id> [n]          Fork a session at event n into a new one");
    println!("  /search <text>          Find sessions mentioning text");
    println!("  /help                   Show this help");
    println!("  /quit                   Exit\n");
  }

  fn activate_skill(&mut self, skill: Skill) {
    // Switching skills used to delete the previous skill's system message from
    // the transcript. An append-only log cannot retract an event, and should
    // not: the earlier skill genuinely was active for those turns, and erasing
    // that would make the log disagree with what the model actually saw.
    // Instead the superseding activation is appended, and the projection lets
    // the later prompt win by being closer to the end of the conversation.
    self.current_session.record(EventPayload::SkillActivated {
      name: skill.name.clone(),
    });
    self.current_session.record(EventPayload::SystemPrompt {
      kind: PromptKind::Skill,
      content: format!(
        "# Activated Skill: {}\n\n{}",
        skill.name, skill.system_prompt
      ),
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
        crate::tools::offload::set_blob_dir(self.history.blobs_dir(self.current_session.id()));
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
        // NOTE: Plan Mode deliberately does NOT restrict writes. In SeekCLI it
        // means "externalize state to PLAN.md / TODO.md" (stage 15), and its
        // prompt instructs the model to write those files. It is not the
        // dsh/Claude-Code sense of "change nothing until approved" -- that is
        // `/readonly`, a separate switch. Wiring the two together would break
        // the feature it is named after.
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
      "/readonly" => {
        let on = match parts.get(1).copied() {
          Some("on") => true,
          Some("off") => false,
          _ => crate::tools::policy::mode() != crate::tools::policy::Mode::ReadOnly,
        };
        crate::tools::policy::set_mode(if on {
          crate::tools::policy::Mode::ReadOnly
        } else {
          crate::tools::policy::Mode::Normal
        });
        if on {
          println!(
            "{} Read-only {} — write_file / edit_file / create_skill refused; \
             run_shell limited to reporting commands",
            "✦".cyan(),
            "ON".green()
          );
        } else {
          println!("{} Read-only {}", "✦".cyan(), "OFF".yellow());
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
            "- {} {} · {} events{}",
            crate::session::short_id(&s.id).yellow(),
            s.title.bold(),
            s.event_count,
            cost_note.dimmed()
          );
        }
      }
      // `/resume` is the name the docs and `-p --resume` use; `/load` stays
      // as an alias so muscle memory keeps working.
      "/load" | "/resume" => match parts.get(1) {
        None => println!("{} Usage: {} <id>", "Info:".blue(), cmd),
        Some(prefix) => match self.history.load_session(prefix) {
          Ok(session) => {
            // Restore the loaded session's cost so the bill continues from
            // where it left off rather than mixing with the prior session.
            self.cost = session.meta.cost.clone();
            println!(
              "{} Resumed: {} ({} events)",
              "✦".cyan(),
              session.meta.title,
              session.meta.event_count
            );
            crate::tools::offload::set_blob_dir(self.history.blobs_dir(session.id()));
            self.current_session = session;
          }
          Err(e) => println!("{} {}", "Error:".red(), e),
        },
      },
      "/fork" => match parts.get(1) {
        None => println!(
          "{} Usage: /fork <id> [event-count]   (omit the count to copy all)",
          "Info:".blue()
        ),
        Some(prefix) => match self.history.load_session(prefix) {
          Ok(source) => {
            let count = parts
              .get(2)
              .and_then(|n| n.parse::<usize>().ok())
              .unwrap_or(source.events.len());
            let child = source.fork(uuid::Uuid::new_v4().to_string(), count);
            self.history.save_session(&child)?;
            println!(
              "{} Forked {} at event {} -> {} ({} events)",
              "✦".cyan(),
              crate::session::short_id(&source.meta.id),
              count,
              crate::session::short_id(&child.meta.id),
              child.events.len()
            );
            self.cost = observability::cost::CostTracker::new();
            crate::tools::offload::set_blob_dir(self.history.blobs_dir(child.id()));
            self.current_session = child;
          }
          Err(e) => println!("{} {}", "Error:".red(), e),
        },
      },
      "/search" => match parts.get(1) {
        None => println!("{} Usage: /search <text>", "Info:".blue()),
        Some(_) => {
          let needle = line
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim())
            .unwrap_or("");
          let hits = self.history.search(needle, 10)?;
          if hits.is_empty() {
            println!("{} No session mentions '{}'.", "Info:".blue(), needle);
          } else {
            for (meta, excerpt) in hits {
              println!(
                "  {} {} — {}",
                crate::session::short_id(&meta.id).yellow(),
                meta.title.bold(),
                excerpt.dimmed()
              );
            }
          }
        }
      },
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
