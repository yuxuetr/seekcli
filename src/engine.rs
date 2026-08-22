//! The Harness engine: the ReAct loop and its supporting passes (Two-Stage
//! planning, System Reminders, Error Recovery, read-concurrent tool dispatch,
//! sub-agent delegation). Split out of main.rs as a separate `impl App` block;
//! as a child module it retains access to App's private fields.

use anyhow::Result;
use colored::Colorize;
use futures_util::StreamExt;
use std::sync::atomic::Ordering;

use crate::api::{self, Message, StreamItem};
use crate::{App, Skill, ThinkingMode, agent, subagents, tools, ui};

/// How a loop run ended. Distinguished because a caller needs to act on the
/// difference: an unattended `-p` run must exit non-zero when the agent ran
/// out of iterations, which is not the same as failing and not the same as
/// succeeding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopStatus {
  Completed,
  MaxIterations,
  Interrupted,
}

pub(crate) struct LoopResult {
  pub text: String,
  pub messages: Vec<Message>,
  pub status: LoopStatus,
  pub iterations: usize,
}

/// A headless run's outcome, shaped for `--output json` and exit codes.
pub(crate) struct HeadlessOutcome {
  pub text: String,
  pub status: LoopStatus,
  pub iterations: usize,
  pub llm_calls: u64,
}

impl App {
  pub(crate) async fn chat(&mut self, content: &str) -> Result<()> {
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
    let run = self
      .run_agent_loop(messages, tools, 0, agent::MAX_ITER, run_span)
      .await?;
    self.tracer.end(run_span);
    self.current_session.messages = run.messages;

    if self.current_session.title == "New Chat" {
      self.current_session.title = content.chars().take(30).collect::<String>();
    }
    // Persist the session's running cost so it can be audited / restored later.
    self.current_session.cost = self.cost.clone();
    self.history.save_session(&self.current_session)?;

    // Print the session bill (token accounting + CNY estimate).
    if !self.cost.is_empty() {
      eprintln!("{}", self.cost.summary());
    }
    // Flush the decision-path trace (no-op unless SEEKCLI_TRACE is set).
    match self.tracer.flush() {
      Ok(Some(path)) => eprintln!("{} trace written to {}", "[Trace]".dimmed(), path.display()),
      Ok(None) => {}
      Err(e) => eprintln!("{} trace write failed: {}", "[Trace]".yellow(), e),
    }
    Ok(())
  }

  /// Run the agent headlessly on a single prompt in the current working
  /// directory, optionally with a Skill activated (its system prompt and
  /// tools merged in exactly as `commands::activate_skill` does for the
  /// interactive path), returning the final assistant text plus the LLM-call
  /// count consumed (proxy for turns). Used by the benchmark runner (`skill:
  /// None`, text discarded) and by `tasks::run_task` (L8); no session save,
  /// no REPL state.
  pub(crate) async fn run_headless(
    &mut self,
    prompt: &str,
    skill: Option<&Skill>,
  ) -> Result<HeadlessOutcome> {
    let calls_before = self.cost.api_calls;
    let mut messages = Vec::new();
    if let Some(skill) = skill {
      messages.push(Message::Simple {
        role: "system".to_string(),
        content: format!(
          "# Activated Skill: {}\n\n{}",
          skill.name, skill.system_prompt
        ),
        reasoning_content: None,
        tool_calls: None,
      });
    }
    messages.push(Message::new_user_text(prompt.to_string()));
    Self::ensure_agent_system_prompt(&mut messages, self.plan_mode);
    let tools = skill.and_then(|s| s.to_api_tools());
    let run = self
      .run_agent_loop(messages, tools, 0, self.max_iter, None)
      .await?;
    Ok(HeadlessOutcome {
      text: run.text,
      status: run.status,
      iterations: run.iterations,
      llm_calls: self.cost.api_calls - calls_before,
    })
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
  pub(crate) fn result_is_failure(result: &str) -> bool {
    result.contains("[Recovery]")
      || result.contains("[BAD ARGS]")
      || result.starts_with("[ERROR]")
      || result.starts_with("Error executing")
      || result.contains("' failed:")
  }

  /// Append the Two-Stage plan as an assistant message, followed by a
  /// synthetic user "go" message.
  ///
  /// Without the bridge, `messages` would end in role `assistant` right
  /// before the next (tool-enabled) completion request — every other turn in
  /// this loop ends in `tool` or `user`. That shape is out-of-distribution
  /// for chat-tuned models: two consecutive assistant turns with no
  /// intervening user/tool message reads as "continue your own utterance",
  /// not "your turn to decide", and empirically DeepSeek sometimes responds
  /// by narrating a fake tool call as plain text instead of emitting a real
  /// structured `tool_calls` — silently dropped (dispatcher sees zero calls),
  /// no error, model reports false success. Restoring the normal
  /// assistant→user→assistant alternation (same idiom as the doom-loop
  /// reminder in `agent::reminders`) fixes it.
  fn append_plan_with_bridge(messages: &mut Vec<Message>, plan: String) {
    if plan.trim().is_empty() {
      return;
    }
    messages.push(Message::Simple {
      role: "assistant".to_string(),
      content: plan,
      reasoning_content: None,
      tool_calls: None,
    });
    messages.push(Message::new_user_text(
      "[System] Proceed: call the tool(s) needed to execute the plan above now.".to_string(),
    ));
  }

  /// System directive scoped to a single tools-withheld planning call —
  /// pushed onto an ephemeral clone of `messages` for that one request only,
  /// never into the shared history (that would bloat every future request
  /// and defeat the prompt-cache-stable static kernel).
  ///
  /// Necessary because `agent_system_prompt` tells the model "don't narrate
  /// 'I will now call X' — just call it", which is correct advice when tools
  /// are available but actively counterproductive here, where they
  /// deliberately are not. Without this override, the model sometimes "just
  /// calls it" anyway by writing tool-call-shaped pseudo-syntax as plain
  /// content — which then reads back on the *next* turn as an
  /// already-completed action, so the model that actually has tools just
  /// confirms success without ever calling anything for real.
  fn planning_only_directive() -> Message {
    Message::Simple {
      role: "system".to_string(),
      content: "[Two-Stage ReAct planning pass] Tools are deliberately withheld for \
                this one completion only — you cannot actually invoke anything right \
                now, so do not write tool-call syntax, function invocations, or any \
                tool-call-shaped text; doing so will be mistaken for a completed \
                action on the next turn. Just think in plain prose: what's the \
                situation, what should happen next, and why. The very next turn has \
                tools available again and will act on this plan."
        .to_string(),
      reasoning_content: None,
      tool_calls: None,
    }
  }

  /// Truncate suspected fake tool-call syntax out of a tools-withheld
  /// planning pass's output.
  ///
  /// `planning_only_directive` asks the model not to do this, but that's a
  /// probabilistic mitigation — DeepSeek sometimes ignores it and emits its
  /// internal function-calling grammar as literal text anyway (observed as
  /// `<｜｜...｜｜tool_calls>`-shaped pseudo-tags built from the full-width
  /// vertical line U+FF5C, which is not otherwise going to appear in normal
  /// prose). Left in the appended plan message, that text reads on the next
  /// turn as an already-completed action, so the model that actually has
  /// tools just confirms success without calling anything for real. This is
  /// a deterministic backstop: if the marker shows up, cut the plan off right
  /// before it rather than trust the instruction alone.
  fn strip_fake_tool_syntax(plan: &str) -> String {
    const MARKER: &str = "\u{FF5C}\u{FF5C}"; // "｜｜"
    match plan.find(MARKER) {
      Some(idx) => plan[..idx].trim_end().to_string(),
      None => plan.to_string(),
    }
  }

  /// Two-Stage ReAct planning pass: a tools-free completion that forces the
  /// model to deliberate before acting. The plan text is appended to
  /// `messages` (plus a bridge message, see `append_plan_with_bridge`) so the
  /// subsequent action call sees it. Called for the main agent only.
  async fn planning_phase(&self, messages: &mut Vec<Message>) -> Result<()> {
    eprintln!("\n{}", "[Plan] deliberating (tools withheld)...".dimmed());
    let mut planning_request = messages.clone();
    planning_request.push(Self::planning_only_directive());
    let mut stream = self
      .brain
      .call_api_with_params(
        &self.model,
        planning_request,
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
            ui::content(&format!("\n{}", "Thinking: ".italic().bright_black()));
            is_reasoning = true;
          }
          ui::content(&format!("{}", r.italic().bright_black()));
        }
        StreamItem::Content(c) => {
          if is_reasoning {
            eprintln!();
            is_reasoning = false;
          }
          ui::content(&format!("{}", c.dimmed()));
          plan.push_str(&c);
        }
        _ => {}
      }
      ui::flush_content()?;
    }
    eprintln!();

    let sanitized = Self::strip_fake_tool_syntax(&plan);
    if sanitized.len() != plan.len() {
      eprintln!(
        "{}",
        "[Plan] discarded suspected fake tool-call syntax from plan text".yellow()
      );
    }
    Self::append_plan_with_bridge(messages, sanitized);
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
  ) -> Result<LoopResult> {
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
    let mut interrupted = false;
    let mut iterations = 0usize;
    // Doom-loop detector — main agent only. Persists across iterations of this
    // chat turn so it can spot repeated tool-call trajectories.
    let mut reminder_injector = agent::reminders::ReminderInjector::new();
    // Two-Stage ReAct: set when the previous turn failed, forcing a tools-free
    // planning pass before the next action (micro trigger).
    let mut plan_next = false;

    for iter in 0..max_iter {
      iterations = iter + 1;
      // Top-of-iteration interrupt check (Ctrl-C between turns).
      if depth == 0 && self.interrupt.swap(false, Ordering::SeqCst) {
        eprintln!("\n{}", "[Agent] interrupted by user".yellow());
        final_content = "[Interrupted by user]".to_string();
        completed = true;
        interrupted = true;
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
          agent::compressor::maybe_compress(self.brain.as_ref(), &self.model, &mut messages).await
        {
          eprintln!(
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
            eprintln!(
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
          eprintln!("\n{}", "[Agent] interrupted by user (mid-stream)".yellow());
          break;
        }
        match item? {
          StreamItem::Reasoning(r) => {
            if !is_reasoning {
              ui::content(&format!("\n{}", "Thinking: ".italic().bright_black()));
              is_reasoning = true;
            }
            ui::content(&format!("{}", r.italic().bright_black()));
            assistant_reasoning.push_str(&r);
          }
          StreamItem::Content(c) => {
            if is_reasoning {
              eprintln!();
              is_reasoning = false;
            }
            ui::content(&c);
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
            eprintln!(
              "\n{} Called: {} {}",
              "Agent:".cyan(),
              tc.function.name.yellow(),
              preview.bright_black()
            );
            tool_calls.push(tc);
          }
          StreamItem::Finish(reason) => {
            eprintln!();
            if let Some(r) = reason
              && r == "length"
            {
              eprintln!("\n{}", "[Note: Max output limit reached.]".yellow());
            }
          }
          StreamItem::Usage(info) => {
            let pct = info
              .prompt_cache_hit_tokens
              .checked_mul(100)
              .and_then(|n| n.checked_div(info.prompt_tokens))
              .unwrap_or(0);
            eprintln!(
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
        ui::flush_content()?;
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
      eprintln!("\n{} Executing tools...", "Agent:".cyan());
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
        eprintln!(
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

                  eprintln!(
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
                    Ok(run) => {
                      format!(
                        "Sub-agent '{}' completed. Summary:\n{}",
                        template.name, run.text
                      )
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
                      eprintln!("{} Loaded skill: {}", "✦".cyan(), skill_name.green());
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
        eprintln!(
          "\n{}",
          "[System Reminder] doom loop detected — intervening".red()
        );
        messages.push(Message::new_user_text(reminder));
      }

      // Two-Stage ReAct micro trigger: a failed turn forces a tools-free
      // planning pass at the top of the next iteration.
      plan_next = depth == 0 && turn_had_failure;

      self.tracer.end(turn_span);
      eprintln!("{} Returning tool results to model...", "Agent:".cyan());
    }

    if !completed {
      eprintln!(
        "\n{}",
        format!("[Agent: reached max iterations ({})]", max_iter).yellow()
      );
      if final_content.is_empty() {
        final_content = format!("[Stopped at max iterations ({})]", max_iter);
      }
    }

    let status = if interrupted {
      LoopStatus::Interrupted
    } else if completed {
      LoopStatus::Completed
    } else {
      LoopStatus::MaxIterations
    };
    Ok(LoopResult {
      text: final_content,
      messages,
      status,
      iterations,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn strip_fake_tool_syntax_truncates_at_marker() {
    let plan = "两个文件都不存在，现在按需创建它们。\n\n\u{FF5C}\u{FF5C}tool_calls>\n\u{FF5C}\u{FF5C}invoke name=\"write_file\">...";
    let out = App::strip_fake_tool_syntax(plan);
    assert_eq!(out, "两个文件都不存在，现在按需创建它们。");
  }

  #[test]
  fn strip_fake_tool_syntax_leaves_clean_prose_untouched() {
    let plan = "The file doesn't exist yet; I should create it with write_file next.";
    assert_eq!(App::strip_fake_tool_syntax(plan), plan);
  }

  #[test]
  fn planning_only_directive_is_system_and_forbids_tool_syntax() {
    let msg = App::planning_only_directive();
    match msg {
      Message::Simple {
        role,
        content,
        tool_calls,
        ..
      } => {
        assert_eq!(role, "system");
        assert!(tool_calls.is_none());
        assert!(content.contains("do not write tool-call syntax"));
      }
      _ => panic!("expected Simple system message"),
    }
  }

  #[test]
  fn plan_bridge_appends_assistant_then_user() {
    let mut messages = vec![Message::ToolResponse {
      role: "tool".to_string(),
      content: "some result".to_string(),
      tool_call_id: "t1".to_string(),
    }];
    App::append_plan_with_bridge(&mut messages, "I should read the file next.".to_string());

    assert_eq!(messages.len(), 3);
    match &messages[1] {
      Message::Simple {
        role,
        content,
        tool_calls,
        ..
      } => {
        assert_eq!(role, "assistant");
        assert_eq!(content, "I should read the file next.");
        assert!(tool_calls.is_none());
      }
      _ => panic!("expected Simple assistant message"),
    }
    match &messages[2] {
      Message::Simple { role, content, .. } => {
        assert_eq!(role, "user");
        assert!(content.contains("Proceed"));
      }
      _ => panic!("expected Simple user bridge message"),
    }
  }

  #[test]
  fn plan_bridge_skips_empty_plan() {
    let mut messages = vec![Message::new_user_text("hi".to_string())];
    App::append_plan_with_bridge(&mut messages, "   ".to_string());
    assert_eq!(messages.len(), 1, "blank plan must not append anything");
  }
}
