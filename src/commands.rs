//! REPL command handling: slash-command dispatch (/skill, /model, /plan, …),
//! help text, skill activation, and clipboard copy. Split out of main.rs as a
//! separate `impl App` block; as a child module it keeps access to App's
//! private fields. Only handle_command is pub(crate) (called from the REPL
//! loop); the rest are dispatched from within it.

use anyhow::Result;
use colored::Colorize;

use crate::session::{EventPayload, PromptKind};
use crate::{App, Skill, ThinkingMode, observability};

impl App {
  /// Accept a proposal, then re-read anything the acceptance changed.
  ///
  /// A landed skill is visible immediately; a landed MCP server is not, because
  /// servers are connected once at startup so the tool set cannot change under
  /// prompt caching mid-session. The note says so rather than leaving the user
  /// to wonder why the new tools are absent.
  async fn gate_accept(
    &mut self,
    kind: crate::proposals::Kind,
    name: &str,
    skip_eval: bool,
  ) -> anyhow::Result<String> {
    if let Some(refusal) = self.regression_check(kind, name, skip_eval).await? {
      anyhow::bail!("{refusal}");
    }
    crate::proposals::ProposalStore::new()?.accept(kind, name)
  }

  /// Run the smoke suite with and without a proposed skill; refuse on a loss.
  ///
  /// Returns `Some(reason)` when the proposal must not land. Only `skill`
  /// proposals are checked: an `mcp` server is not connected until restart and
  /// a `task` is a prompt for the scheduler, so evaluating either would spend
  /// the user's money measuring nothing (`docs/architecture/L7-observability.md`
  /// §4.7.1). Saying so beats skipping silently, which reads as "it passed".
  async fn regression_check(
    &mut self,
    kind: crate::proposals::Kind,
    name: &str,
    skip_eval: bool,
  ) -> anyhow::Result<Option<String>> {
    use crate::proposals::Kind;
    if kind != Kind::Skill {
      println!(
        "{} no eval gate for a {} proposal: an mcp server is not connected until restart, and a task runs on a schedule, so a suite here would measure nothing.",
        "Note:".blue(),
        kind
      );
      return Ok(None);
    }
    if skip_eval {
      // An escape hatch that hides what it skipped is not an escape hatch.
      println!(
        "{} skipping the regression check — `{}` lands unmeasured.",
        "Warning:".yellow(),
        name
      );
      return Ok(None);
    }

    let store = crate::proposals::ProposalStore::new()?;
    let skill = match store.read_skill(name) {
      Ok(s) => s,
      // Cannot read it: the accept below will fail with the real reason, and
      // guessing a verdict here would be worse than deferring to it.
      Err(_) => return Ok(None),
    };

    let suite =
      std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/benchmarks/basic.json");
    if !suite.exists() {
      println!(
        "{} smoke suite not found at {} — accepting unmeasured.",
        "Warning:".yellow(),
        suite.display()
      );
      return Ok(None);
    }

    // Stated before it runs: this costs real calls and real minutes.
    println!(
      "{} checking `{}` against the smoke suite (two runs, so ~2x the suite in LLM calls)…",
      "✦".cyan(),
      name
    );
    let before = self.score_suite(&suite, None, None).await?;
    let after = self.score_suite(&suite, None, Some(&skill)).await?;

    let verdict = observability::bench::Regression::compare(&before, &after);
    println!(
      "{} baseline {}/{} → with `{}` {}/{}",
      "✦".cyan(),
      before.passed(),
      before.total(),
      name,
      after.passed(),
      after.total()
    );
    Ok(match verdict {
      observability::bench::Regression::Clean => None,
      broke => Some(format!("`{name}` was not accepted: {}", broke.explain())),
    })
  }

  fn gate_reject(&self, kind: crate::proposals::Kind, name: &str) -> anyhow::Result<String> {
    crate::proposals::ProposalStore::new()?.reject(kind, name)
  }

  fn report_gate(outcome: anyhow::Result<String>) {
    match outcome {
      Ok(note) => println!("{} {}", "Success:".green(), note),
      Err(e) => println!("{} {:#}", "Error:".red(), e),
    }
  }

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
    println!("  /propose list           Everything the agent drafted, awaiting your review");
    println!("  /propose accept <kind> <name>  Land it (kinds: skill, mcp, task)");
    println!("  /propose reject <kind> <name>  Discard it");
    println!("  /paste [说明]           把剪贴板里的图交给模型（截图后直接用，不用存文件）");
    println!("  /skill migrate          Convert legacy <name>.json skills to <name>/SKILL.md");
    println!("  /copy [index]           Copy code block from last response");
    println!("  /clear                  Reset conversation");
    println!("  /history                List previous sessions");
    println!("  /resume <id>            Resume a previous session (alias: /load)");
    println!("  /fork <id> [n]          Fork a session at event n into a new one");
    println!("  /search <text>          Find sessions mentioning text");
    println!("  /tools                  List active tools (built-in + MCP)");
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
    // Say it at activation, not only in `harness_inspect`: a skill that
    // silently removes tools looks like a model that has stopped trying.
    if let Some(allowed) = &skill.allowed_tools {
      eprintln!(
        "{} '{}' narrows tools to: {}",
        "[Skill]".cyan(),
        skill.name,
        allowed.join(", ")
      );
    }
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
        // Sources belong to the conversation that gathered them. Carrying them
        // into a new one would let a later answer claim evidence it never saw.
        crate::tools::provenance::reset();
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
        // Kept as the name muscle memory reaches for; both route to the one
        // gate in `crate::proposals` so there is no second implementation.
        Some("accept") => match parts.get(2) {
          None => println!("{} Usage: /skill accept <name>", "Info:".blue()),
          Some(name) => {
            let outcome = self
              .gate_accept(crate::proposals::Kind::Skill, name, false)
              .await;
            Self::report_gate(outcome);
          }
        },
        Some("reject") => match parts.get(2) {
          None => println!("{} Usage: /skill reject <name>", "Info:".blue()),
          Some(name) => Self::report_gate(self.gate_reject(crate::proposals::Kind::Skill, name)),
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
      "/tools" => {
        let builtin = crate::tools::registry::system_tools();
        println!("{} ({})", "Built-in".bold(), builtin.len());
        for t in &builtin {
          println!("  {}", t.function.name);
        }
        let mcp = self.mcp.listing();
        if mcp.is_empty() {
          println!(
            "{} none configured — add [[mcp]] entries to {}",
            "MCP:".dimmed(),
            "~/.seekcli/config.toml".dimmed()
          );
        } else {
          println!("{} ({})", "MCP".bold(), mcp.len());
          for (name, server) in mcp {
            println!("  {}  {}", name, format!("[{}]", server).dimmed());
          }
        }
      }
      "/paste" => {
        // Zero path management is the whole point: screenshot, Cmd+Shift+4,
        // /paste. Anything that asks the user to name a file defeats it.
        let dir = self.history.blobs_dir(self.current_session.id());
        match crate::tools::clipboard::grab_image(&dir) {
          Err(e) => println!("{} {:#}", "Error:".red(), e),
          Ok(grab) => {
            let caption = line
              .split_once(char::is_whitespace)
              .map(|(_, rest)| rest.trim().to_string())
              .unwrap_or_default();
            println!(
              "{} image attached ({:.1} KB){}",
              "✦".cyan(),
              grab.bytes as f64 / 1024.0,
              if caption.is_empty() {
                String::new()
              } else {
                format!(" — {caption}")
              }
            );
            let images = vec![crate::session::ImageRef {
              path: grab.path.display().to_string(),
              media_type: grab.media_type,
            }];
            // An empty caption is a legitimate request ("look at this"), so the
            // turn runs either way rather than demanding words.
            let prompt = if caption.is_empty() {
              "看看这张图。".to_string()
            } else {
              caption
            };
            self.chat_with_images(&prompt, images).await?;
          }
        }
      }
      "/propose" => {
        use crate::proposals::Kind;
        match (
          parts.get(1).copied(),
          parts.get(2).copied(),
          parts.get(3).copied(),
        ) {
          (None, ..) | (Some("list"), ..) => {
            let store = crate::proposals::ProposalStore::new()?;
            let pending = store.list();
            if pending.is_empty() {
              println!("{} Nothing awaiting review.", "Info:".blue());
            } else {
              println!(
                "Awaiting your review (run {} or {}):",
                "/propose accept <kind> <name>".green(),
                "/propose reject <kind> <name>".yellow()
              );
              for p in pending {
                let note = match p.validate() {
                  Ok(()) => "ok".green(),
                  // Surfaced here rather than at accept time: a proposal that
                  // cannot land is worth knowing about while reviewing.
                  Err(e) => format!("cannot land: {e}").red(),
                };
                println!(
                  "- {} {} — {}",
                  p.kind.to_string().cyan(),
                  p.name.bold(),
                  note
                );
              }
            }
          }
          (Some(verb @ ("accept" | "reject")), Some(kind), Some(name)) => match Kind::parse(kind) {
            None => println!(
              "{} unknown kind '{}'. Valid kinds: {}.",
              "Error:".red(),
              kind,
              crate::proposals::kind_names()
            ),
            Some(kind) if verb == "accept" => {
              // A hatch that hides what it skipped is not a hatch.
              let skip = parts.contains(&"--skip-eval");
              let outcome = self.gate_accept(kind, name, skip).await;
              Self::report_gate(outcome);
            }
            Some(kind) => Self::report_gate(self.gate_reject(kind, name)),
          },
          _ => println!(
            "{} Usage: /propose list | /propose accept <kind> <name> | /propose reject <kind> <name>   (kinds: {})",
            "Info:".blue(),
            crate::proposals::kind_names()
          ),
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
            // Same rule as /clear: this is a different conversation, and the
            // previous one's sources are not evidence for it.
            crate::tools::provenance::reset();
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
            let mut child = source.fork(uuid::Uuid::new_v4().to_string(), count);
            self.history.save_session(&mut child)?;
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
      // Scoped to this block: `write_all` is the only use, and at module level
      // the import is dead on every other platform -- which `-D warnings`
      // turns into a build failure nobody sees until CI runs on Linux.
      use std::io::Write;

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
