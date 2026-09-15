//! The Harness engine: the ReAct loop and its supporting passes (Two-Stage
//! planning, System Reminders, Error Recovery, read-concurrent tool dispatch,
//! sub-agent delegation). Split out of main.rs as a separate `impl App` block;
//! as a child module it retains access to App's private fields.

use anyhow::Result;
use colored::Colorize;
use futures_util::StreamExt;
use std::sync::atomic::Ordering;

use crate::api::{self, Message, StreamItem};
use crate::session::EventPayload;
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
  /// The working set is deliberately NOT returned. Once the log is the single
  /// source of truth, handing the caller a second copy of the conversation is
  /// an invitation to persist that instead — which is exactly how the old
  /// snapshot format lost its compacted history.
  ///
  /// Durable record of what this run added to the conversation. Returned
  /// rather than written through `&mut self` so a sub-agent's trace can be
  /// discarded (or later given its own session) without the borrow checker
  /// forcing the log and the loop to share one mutable path.
  pub events: Vec<EventPayload>,
  pub status: LoopStatus,
  pub iterations: usize,
}

impl App {
  /// Route a tool call to the built-in pipeline or to an MCP server.
  ///
  /// MCP tools go through `ToolDispatcher` too, so the policy gate, the
  /// deadline and the audit log apply to foreign tools exactly as they do to
  /// ours. A capability that arrives from configuration must not be a way
  /// around `--read-only`.
  async fn dispatch(
    dispatcher: &tools::ToolDispatcher,
    mcp: &crate::mcp::McpRegistry,
    inspect: Option<&tools::inspect::Snapshot>,
    name: &str,
    arguments: &str,
  ) -> tools::result::ToolResult {
    // Self-description goes through `execute_with` like any other capability.
    // It could have been an engine branch next to `invoke_agent`, but those
    // bypass the dispatcher because they re-enter the loop or mutate engine
    // state; this one only reads, so there is no reason to give up the "exactly
    // one path" invariant (`docs/architecture/L7-observability.md` §4.5.3).
    if name == "harness_inspect" {
      return dispatcher
        .execute_with(name, arguments, Some(true), |args| async move {
          match inspect {
            Some(snap) => tools::inspect::render(&args, snap).map(tools::result::ToolOutput::from),
            // Only reachable if a caller forgot to assemble the snapshot;
            // failing loudly beats reporting an empty harness as the truth.
            None => anyhow::bail!("harness_inspect was dispatched without a snapshot"),
          }
        })
        .await;
    }
    if crate::mcp::is_mcp_tool(name) {
      // The server's own read-only annotation is the only thing that can
      // exempt a foreign tool from the mode gate.
      let declared = Some(mcp.is_read_only(name));
      return dispatcher
        .execute_with(name, arguments, declared, |args| async move {
          mcp.call(name, &args).await
        })
        .await;
    }
    dispatcher.execute(name, arguments).await
  }
}

impl App {
  /// Append a tool response to both the working set and the log.
  ///
  /// **The only place tool images are persisted.** Blobs are written before the
  /// event is recorded, so "model-visible means logged" holds even if the turn
  /// is interrupted right after. A blob that cannot be written degrades to a
  /// visible line rather than a silently image-less result — the model would
  /// otherwise be told a screenshot arrived when none did
  /// (`docs/architecture/L4-memory.md` §4.6.2).
  fn push_tool_response(
    messages: &mut Vec<Message>,
    events: &mut Vec<EventPayload>,
    call_id: String,
    mut content: String,
    images: Vec<api::ImagePart>,
  ) {
    let mut refs = Vec::new();
    let mut kept = Vec::new();
    for image in images {
      match tools::offload::persist_image(&image) {
        Ok(reference) => {
          refs.push(reference);
          kept.push(image);
        }
        Err(e) => {
          if !content.is_empty() {
            content.push('\n');
          }
          content.push_str(&format!("[an image could not be stored: {e:#}]"));
        }
      }
    }
    messages.push(Message::new_tool_response(
      call_id.clone(),
      content.clone(),
      kept,
    ));
    events.push(EventPayload::ToolResult {
      call_id,
      content,
      images: refs,
    });
  }

  /// Assemble the live facts `harness_inspect` reports.
  ///
  /// Here rather than in `tools::inspect` because only the engine can reach the
  /// registries; the rendering stays a pure function over this struct so it can
  /// be asserted without constructing an `App`.
  fn inspect_snapshot(&self, effective: &[api::Tool]) -> tools::inspect::Snapshot {
    let builtin: std::collections::HashSet<String> = tools::registry::system_tools()
      .into_iter()
      .map(|t| t.function.name)
      .collect();

    let tool_entries = effective
      .iter()
      .map(|t| {
        let name = &t.function.name;
        let source = if let Some(server) = name
          .strip_prefix("mcp__")
          .and_then(|rest| rest.split_once("__"))
          .map(|(server, _)| server)
        {
          format!("mcp:{server}")
        } else if builtin.contains(name) {
          "built-in".to_string()
        } else {
          // Anything else reached the surface through the active skill.
          "skill".to_string()
        };
        tools::inspect::ToolEntry {
          signature: tools::registry::signature_of(t),
          source,
        }
      })
      .collect();

    let skills = self
      .skill_manager
      .load_skills()
      .map(|s| s.into_iter().map(|k| k.name).collect())
      .unwrap_or_default();
    // Every kind, not just skills: the model needs to see that the MCP server
    // it proposed last turn is still awaiting review, or it proposes it again.
    let proposals = crate::proposals::ProposalStore::new()
      .map(|store| {
        store
          .list()
          .iter()
          .map(|p| format!("{} {}", p.kind, p.name))
          .collect()
      })
      .unwrap_or_default();

    let compactions = self
      .current_session
      .events
      .iter()
      .filter(|e| matches!(e.payload, crate::session::EventPayload::Compaction { .. }))
      .count();

    tools::inspect::Snapshot {
      mode: tools::policy::mode().name(),
      plan_mode: self.plan_mode,
      tools: tool_entries,
      servers: self
        .mcp
        .server_tool_counts()
        .into_iter()
        .map(|(name, tools)| tools::inspect::ServerEntry { name, tools })
        .collect(),
      failures: self
        .mcp
        .failures()
        .iter()
        .map(|f| tools::inspect::FailureEntry {
          server: f.server.clone(),
          reason: f.reason.clone(),
        })
        .collect(),
      active_skill: self.current_skill.as_ref().map(|s| s.name.clone()),
      skills,
      proposals,
      session_id: self.current_session.id().to_string(),
      events: self.current_session.events.len(),
      compactions,
      memory: tools::inspect::MemoryEntry {
        threshold_tokens: self.memory_budget.threshold_tokens,
        window_tokens: self.memory_budget.window_tokens,
        ratio: self.memory_budget.ratio,
        keep_tail: self.memory_budget.keep_tail,
      },
    }
  }

  /// `invoke_agent`: run a typed sub-agent and bring back only its summary.
  ///
  /// Handled in the engine rather than the dispatcher because it re-enters the
  /// loop — the dispatcher deliberately knows nothing about the loop, and
  /// giving it a recursive escape hatch would undo that.
  async fn delegate_to_subagent(
    &mut self,
    arguments: &str,
    available: &[api::Tool],
    depth: usize,
    parent_span: Option<usize>,
  ) -> String {
    let (subagent_type, prompt) = Self::parse_invoke_agent_args(arguments);
    let next_depth = depth + 1;
    if next_depth > agent::MAX_SUBAGENT_DEPTH {
      return format!(
        "Cannot spawn sub-agent: max depth {} reached.",
        agent::MAX_SUBAGENT_DEPTH
      );
    }
    let Some(template) = subagents::registry::lookup(&subagent_type) else {
      let listing: Vec<String> = subagents::registry::catalog()
        .iter()
        .map(|(name, desc)| format!("  - {}: {}", name, desc))
        .collect();
      return format!(
        "Unknown subagent_type '{}'. Available types:\n{}",
        subagent_type,
        listing.join("\n")
      );
    };

    let sub_tools = tools::registry::filter_by_allowed(available, template.allowed_tools);
    let sub_messages = vec![
      Message::Simple {
        images: Vec::new(),
        role: "system".to_string(),
        content: template.system_prompt.to_string(),
        reasoning_content: None,
        tool_calls: None,
      },
      Message::new_user_text(prompt),
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
      parent_span,
    ))
    .await
    {
      // An interrupted or exhausted sub-agent has not "completed". Saying so
      // would hand the parent a false success to reason from — the same
      // confusion between "the model stopped" and "the task passed" that
      // `LoopStatus` exists to keep apart.
      Ok(run) => {
        let verdict = match run.status {
          LoopStatus::Completed => "completed",
          LoopStatus::Interrupted => "was interrupted by the user before finishing",
          LoopStatus::MaxIterations => "hit its iteration cap before finishing",
        };
        format!(
          "Sub-agent '{}' {}. Summary:\n{}",
          template.name, verdict, run.text
        )
      }
      Err(e) => format!("Sub-agent '{}' failed: {}", template.name, e),
    }
  }

  /// `load_skill`: swap the active persona mid-conversation.
  ///
  /// Main agent only. A sub-agent switching skills would mutate state its
  /// parent owns and outlive the delegation that created it.
  fn activate_skill_by_name(
    &mut self,
    arguments: &str,
    depth: usize,
    messages: &mut Vec<Message>,
    deferred: &mut Vec<Message>,
  ) -> String {
    if depth > 0 {
      return "[ERROR] load_skill is restricted to the main agent. \
              Sub-agents cannot switch skills."
        .to_string();
    }
    let name = Self::parse_load_skill_args(arguments);
    let skills = match self.skill_manager.load_skills() {
      Ok(s) => s,
      Err(e) => return format!("Failed to enumerate skills: {}", e),
    };
    let Some(skill) = skills.iter().find(|s| s.name == name).cloned() else {
      let available: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
      return format!("Skill '{}' not found. Available: {:?}", name, available);
    };

    // Same dedup as activate_skill: drop any prior skill's system message from
    // the *working set* so personas don't accumulate. The log keeps both --
    // the earlier skill really was active for those turns.
    messages.retain(|m| {
      !matches!(
        m,
        Message::Simple { role, content, .. }
          if role == "system" && content.starts_with("# Activated Skill: ")
      )
    });
    deferred.push(Message::Simple {
      images: Vec::new(),
      role: "system".to_string(),
      content: format!(
        "# Activated Skill: {}\n\n{}",
        skill.name, skill.system_prompt
      ),
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
}

/// One model response, assembled from the stream.
struct Response {
  content: String,
  reasoning: String,
  tool_calls: Vec<api::ToolCall>,
}

impl App {
  /// Issue one request and consume its stream.
  ///
  /// Split out of the loop body because it is the one part with no control
  /// flow of its own: it turns a stream of deltas into a single response, and
  /// everything it touches (rendering, usage accounting, the mid-stream
  /// interrupt check) belongs to that job rather than to the loop's.
  async fn request_step(&mut self, messages: &[Message], tools: &[api::Tool]) -> Result<Response> {
    let mut stream = self
      .brain
      .call_api_with_params(
        &self.model,
        messages.to_vec(),
        self.thinking_mode.as_str(),
        Some(tools.to_vec()),
      )
      .await?;

    let mut out = Response {
      content: String::new(),
      reasoning: String::new(),
      tool_calls: Vec::new(),
    };
    let mut is_reasoning = false;

    while let Some(item) = stream.next().await {
      // Mid-stream interrupt check, at every depth. Don't reset the flag here
      // — let the outer loop see it and exit cleanly.
      if self.interrupt.load(Ordering::SeqCst) {
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
          out.reasoning.push_str(&r);
        }
        StreamItem::Content(c) => {
          if is_reasoning {
            eprintln!();
            is_reasoning = false;
          }
          ui::content(&c);
          out.content.push_str(&c);
        }
        StreamItem::ToolCall(tc) => {
          eprintln!(
            "\n{} Called: {} {}",
            "Agent:".cyan(),
            tc.function.name.yellow(),
            Self::preview_args(&tc.function.arguments).bright_black()
          );
          out.tool_calls.push(tc);
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
    Ok(out)
  }

  /// Truncate tool arguments for the console line, on a char boundary.
  fn preview_args(args: &str) -> String {
    const LIMIT: usize = 160;
    if args.len() <= LIMIT {
      return args.to_string();
    }
    let mut cut = LIMIT;
    while cut > 0 && !args.is_char_boundary(cut) {
      cut -= 1;
    }
    format!("{}…", &args[..cut])
  }
}

/// Append a message to the working set **and** the durable log in one step.
///
/// The two must never be written separately: the moment a site appends to one
/// and forgets the other, "model-visible means logged" quietly stops being
/// true, and nothing would fail loudly to say so.
/// Whether this loop level should stop for a user interrupt.
///
/// Every depth *observes* the flag, but only the top level *consumes* it. A
/// sub-agent that cleared it would stop itself and leave its parent running —
/// the user asked for everything to stop, not just the innermost thing. So the
/// sub-agent breaks out, its `[Interrupted by user]` summary goes back as the
/// tool result, and the parent's own top-of-iteration check sees the flag still
/// set and unwinds in turn.
fn take_interrupt(flag: &std::sync::atomic::AtomicBool, depth: usize) -> bool {
  if depth == 0 {
    flag.swap(false, Ordering::SeqCst)
  } else {
    flag.load(Ordering::SeqCst)
  }
}

fn log_push(messages: &mut Vec<Message>, events: &mut Vec<EventPayload>, msg: Message) {
  if let Some(payload) = crate::session::event_for(&msg) {
    events.push(payload);
  }
  messages.push(msg);
}

/// First line of the prompt, trimmed — good enough as a title until the model
/// is asked for a better one.
fn title_from(prompt: &str) -> String {
  let line = prompt.lines().next().unwrap_or(prompt).trim();
  let title: String = line.chars().take(48).collect();
  if title.is_empty() {
    crate::session::UNTITLED.to_string()
  } else {
    title
  }
}

/// A headless run's outcome, shaped for `--output json` and exit codes.
pub(crate) struct HeadlessOutcome {
  pub text: String,
  pub status: LoopStatus,
  pub iterations: usize,
  pub llm_calls: u64,
  /// What the run appended to the conversation.
  ///
  /// Returned rather than only stashed for tests: trajectory export needs it,
  /// and it was already being captured — behind `#[cfg(test)]`, which is the
  /// only reason nothing outside could read it
  /// (`docs/architecture/L7-observability.md` §4.6.1).
  pub events: Vec<EventPayload>,
}

impl App {
  pub(crate) async fn chat(&mut self, content: &str) -> Result<()> {
    self.chat_with_images(content, Vec::new()).await
  }

  /// A turn whose user message carries images.
  ///
  /// The images arrive as `ImageRef`s — blobs already on disk — because the log
  /// is written before the request is built, and "model-visible means logged"
  /// has to hold even if the turn is interrupted mid-flight
  /// (`docs/architecture/L4-memory.md` §4.6.2).
  pub(crate) async fn chat_with_images(
    &mut self,
    content: &str,
    images: Vec<crate::session::ImageRef>,
  ) -> Result<()> {
    // Clear any stale interrupt flag from a previous turn (e.g. Ctrl-C
    // pressed at the readline prompt also fires the global watcher).
    self.interrupt.store(false, Ordering::SeqCst);

    self.current_session.record(EventPayload::UserMessage {
      images,
      content: content.to_string(),
    });

    let tools = self.current_skill.as_ref().and_then(|s| s.to_api_tools());

    // Compact at the turn boundary, before the working set is built, so the
    // summary lands in the log and carries forward. `maybe_compress` inside
    // the loop stays as the in-turn safety net for a single ballooning turn.
    if let Err(e) = agent::compressor::maybe_compact_session(
      self.brain.as_ref(),
      &self.model,
      &mut self.current_session,
      self.memory_budget,
    )
    .await
    {
      eprintln!(
        "{} session compaction failed: {} (continuing uncompacted)",
        "[Memory]".yellow(),
        e
      );
    }

    // The working set is *projected* from the log, never held alongside it.
    // One source of truth is what keeps "model-visible means logged" true
    // instead of aspirational.
    let mut messages = self.current_session.messages();
    Self::ensure_agent_system_prompt(&mut messages, self.plan_mode);
    let run_span = self.tracer.start_run();
    let run = self
      .run_agent_loop(messages, tools, 0, agent::MAX_ITER, run_span)
      .await?;
    self.tracer.end(run_span);
    self.current_session.extend(run.events);

    if self.current_session.meta.title == crate::session::UNTITLED {
      self.current_session.meta.title = title_from(content);
    }
    // Persist the session's running cost so it can be audited / restored later.
    self.current_session.meta.cost = self.cost.clone();
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
        images: Vec::new(),
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
    // Headless runs used to produce no trace at all: this path never opened a
    // run span and never flushed. That left tracing broken in exactly the mode
    // where nobody is watching the terminal — `-p`, `--bench`, `--run-task`.
    let run_span = self.tracer.start_run();
    let run = self
      .run_agent_loop(messages, tools, 0, self.max_iter, run_span)
      .await?;
    self.tracer.end(run_span);
    match self.tracer.flush() {
      Ok(Some(path)) => eprintln!("{} trace written to {}", "[Trace]".dimmed(), path.display()),
      Ok(None) => {}
      Err(e) => eprintln!("{} trace write failed: {}", "[Trace]".yellow(), e),
    }
    Ok(HeadlessOutcome {
      text: run.text,
      status: run.status,
      iterations: run.iterations,
      llm_calls: self.cost.api_calls - calls_before,
      events: run.events,
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
      images: Vec::new(),
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
      images: Vec::new(),
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
  /// Takes `&mut self` solely to bill the planning request. It used to take
  /// `&self`, which made the `Usage` item unrecordable, so every Two-Stage
  /// pass spent real tokens that never appeared in the cost summary — and a
  /// failing turn triggers one of these, so the under-count grew exactly when
  /// a run was going badly. Found by the stage 25 replay tests: the recorded
  /// trajectory made four requests but only three were billed.
  async fn planning_phase(&mut self, messages: &mut Vec<Message>) -> Result<()> {
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
        StreamItem::Usage(u) => self.cost.record(&u),
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
          images: Vec::new(),
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
            images: Vec::new(),
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
          images: Vec::new(),
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
      // MCP tools go to the main agent only. A sub-agent runs under a template
      // whose allowed_tools list was written without knowledge of whatever the
      // user happens to have configured, so silently widening it would break
      // the tool-narrowing that makes sub-agents cheap and safe.
      let mut merged = tools::registry::merge_with_skill(tools);
      merged.extend(self.mcp.schemas());
      merged
    } else {
      tools.unwrap_or_default()
    };

    let mut events: Vec<EventPayload> = Vec::new();
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
      // Top-of-iteration interrupt check (Ctrl-C between turns), at every
      // depth — see `take_interrupt` for why only depth 0 clears the flag.
      if take_interrupt(&self.interrupt, depth) {
        eprintln!("\n{}", "[Agent] interrupted by user".yellow());
        final_content = "[Interrupted by user]".to_string();
        completed = true;
        interrupted = true;
        events.push(EventPayload::Interrupted);
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
        if let Err(e) = agent::compressor::maybe_compress(
          self.brain.as_ref(),
          &self.model,
          &mut messages,
          self.memory_budget,
        )
        .await
        {
          eprintln!(
            "{} compression failed: {} (continuing without)",
            "[Memory]".yellow(),
            e
          );
        }
        self.tracer.end(cspan);
      }

      // A finished background job is reported at the top of a step, never
      // mid-step: a build completing should not derail whatever the model is
      // in the middle of doing.
      if depth == 0
        && let Some(note) = tools::jobs::drain_completions()
      {
        eprintln!("{} {}", "[Jobs]".magenta(), note);
        log_push(&mut messages, &mut events, Message::new_user_text(note));
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
      let Response {
        content: assistant_content,
        reasoning: assistant_reasoning,
        tool_calls,
      } = self.request_step(&messages, &effective_tools).await?;

      // `verdict` makes the stage 19 failure mode visible at a glance: a turn
      // that produced prose but zero tool calls, while the model claimed to
      // have acted. That was found by reading traces by eye; naming it means
      // the next occurrence is greppable.
      let verdict = if !tool_calls.is_empty() {
        "acted"
      } else if assistant_content.trim().is_empty() {
        "empty"
      } else {
        "answered"
      };
      self.tracer.annotate(
        gen_span,
        serde_json::json!({
          "tool_calls": tool_calls.len(),
          "verdict": verdict,
          "content_bytes": assistant_content.len(),
          "reasoning_bytes": assistant_reasoning.len(),
        }),
      );
      self.tracer.end(gen_span);

      self.last_code_blocks = Self::extract_code_blocks(&assistant_content);

      log_push(
        &mut messages,
        &mut events,
        Message::Simple {
          images: Vec::new(),
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
        },
      );

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
      // Assembled once per turn and only when asked for: it walks the skill
      // directory and the event log, which is wasted work on the turns -- most
      // of them -- that never inspect anything.
      let inspect_snapshot = tool_calls
        .iter()
        .any(|tc| tc.function.name == "harness_inspect")
        .then(|| self.inspect_snapshot(&effective_tools));

      let parallelizable = tool_calls.len() > 1
        && tool_calls.iter().all(|tc| {
          if crate::mcp::is_mcp_tool(&tc.function.name) {
            self.mcp.is_read_only(&tc.function.name)
          } else {
            tools::registry::is_parallel_readonly(&tc.function.name)
          }
        });

      if parallelizable {
        eprintln!(
          "{} {} read-only tools — running concurrently",
          "Agent:".cyan(),
          tool_calls.len()
        );
        let futs = tool_calls.iter().map(|tc| {
          let disp = &tool_dispatcher;
          let mcp = &self.mcp;
          let inspect = inspect_snapshot.as_ref();
          let name = tc.function.name.clone();
          let args = tc.function.arguments.clone();
          let id = tc.id.clone();
          async move {
            let outcome = Self::dispatch(disp, mcp, inspect, &name, &args).await;
            let failed = outcome.kind.is_failure();
            let text = if failed {
              agent::recovery::augment(&name, outcome.render())
            } else {
              outcome.render()
            };
            (id, text, failed, outcome.images)
          }
        });
        let results = futures_util::future::join_all(futs).await;
        for (id, content, failed, images) in results {
          // Classified by the pipeline, not re-derived from the text. A
          // refusal is deliberately not a failure -- see ToolKind::is_failure.
          turn_had_failure |= failed;
          Self::push_tool_response(&mut messages, &mut events, id, content, images);
        }
      } else {
        // Side-effect system messages (e.g. from load_skill) must be appended
        // AFTER all ToolResponse messages for this turn — DeepSeek's validator
        // requires assistant{tool_calls} to be immediately followed by its
        // matching tool messages, with no system message interleaved.
        let mut deferred_system_msgs: Vec<Message> = Vec::new();
        for tc in tool_calls {
          let mut dispatched_failure = false;
          let mut dispatched_images = Vec::new();
          let result_str = if tc.function.name == "invoke_agent" {
            self
              .delegate_to_subagent(&tc.function.arguments, &effective_tools, depth, exec_span)
              .await
          } else if tc.function.name == "load_skill" {
            self.activate_skill_by_name(
              &tc.function.arguments,
              depth,
              &mut messages,
              &mut deferred_system_msgs,
            )
          } else {
            let outcome = Self::dispatch(
              &tool_dispatcher,
              &self.mcp,
              inspect_snapshot.as_ref(),
              &tc.function.name,
              &tc.function.arguments,
            )
            .await;
            dispatched_failure = outcome.kind.is_failure();
            dispatched_images = outcome.images.clone();
            // Context-aware Error Recovery: append an actionable hint on a
            // real failure, so the model follows a debug SOP instead of
            // blindly retrying. A denial gets no hint -- there is nothing to
            // recover from, and suggesting one invites working around policy.
            if dispatched_failure {
              agent::recovery::augment(&tc.function.name, outcome.render())
            } else {
              outcome.render()
            }
          };

          // Delegation paths (sub-agent, load_skill) still classify by text:
          // they are engine-level, never reach the dispatcher, and their
          // failure markers are produced right here.
          turn_had_failure |= dispatched_failure || Self::result_is_failure(&result_str);
          Self::push_tool_response(
            &mut messages,
            &mut events,
            tc.id,
            result_str,
            dispatched_images,
          );
        }
        // Now safe to append deferred system messages (skill activations etc).
        // Order is: assistant{tool_calls} → tool{responses} → system{side-effects}.
        for msg in deferred_system_msgs {
          log_push(&mut messages, &mut events, msg);
        }
      }
      self.tracer.annotate(
        exec_span,
        serde_json::json!({
          "had_failure": turn_had_failure,
          "tools": turn_tool_calls.iter().map(|t| t.function.name.clone()).collect::<Vec<_>>(),
        }),
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
        log_push(&mut messages, &mut events, Message::new_user_text(reminder));
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
      events,
      status,
      iterations,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The whole path, not just the renderer: a real `App` builds the snapshot
  /// from its live registries, and the call goes through `execute_with` so the
  /// gate, the deadline and the audit apply. Only the model's decision to call
  /// it is absent, and that needs a recorded fixture.
  ///
  /// The guard is held across the awaits on purpose: the snapshot describes the
  /// policy mode, so it must not change under the call. Safe because only tests
  /// take this lock and the test runtime cannot deadlock on it.
  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn harness_inspect_runs_through_the_one_guarded_path() {
    let _guard = crate::testsync::lock();
    let app = match App::for_test(Box::new(crate::api::record::Replaying::new(
      std::path::PathBuf::from("tests/fixtures/nonexistent"),
    ))) {
      Ok(a) => a,
      Err(e) => panic!("cannot build test App: {e}"),
    };

    let surface = tools::registry::system_tools();
    let snap = app.inspect_snapshot(&surface);
    let dispatcher = tools::ToolDispatcher::new();

    let out = App::dispatch(
      &dispatcher,
      &app.mcp,
      Some(&snap),
      "harness_inspect",
      r#"{"what":"tools"}"#,
    )
    .await;
    assert_eq!(out.kind, tools::result::ToolKind::Ok, "{}", out.render());
    // The surface describes itself, including the tool doing the describing.
    assert!(out.render().contains("harness_inspect"), "{}", out.render());
    assert!(out.render().contains("built-in"), "{}", out.render());

    // A bad section is a failure with the valid names, not a panic.
    let bad = App::dispatch(
      &dispatcher,
      &app.mcp,
      Some(&snap),
      "harness_inspect",
      r#"{"what":"nope"}"#,
    )
    .await;
    assert_eq!(bad.kind, tools::result::ToolKind::Failed);
    assert!(bad.render().contains("policy"), "{}", bad.render());
  }

  /// Dispatching it without a snapshot must fail loudly rather than report an
  /// empty harness as the truth.
  #[tokio::test]
  async fn harness_inspect_without_a_snapshot_fails_loudly() {
    let dispatcher = tools::ToolDispatcher::new();
    let out = App::dispatch(
      &dispatcher,
      &crate::mcp::McpRegistry::empty(),
      None,
      "harness_inspect",
      "{}",
    )
    .await;
    assert_eq!(out.kind, tools::result::ToolKind::Failed);
  }

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
      images: Vec::new(),
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

  /// The bug this replaced: both interrupt checks were gated on `depth == 0`,
  /// so Ctrl-C during a sub-agent run was invisible to that sub-agent's loop.
  /// `run_shell` died (it has its own copy of the flag) while the loop kept
  /// iterating — up to `max_iter` 15 or 20 more times.
  #[test]
  fn a_sub_agent_sees_the_interrupt() {
    let flag = std::sync::atomic::AtomicBool::new(true);
    assert!(
      take_interrupt(&flag, 1),
      "a sub-agent must observe the flag"
    );
    assert!(take_interrupt(&flag, 3), "so must a nested one");
  }

  /// And the trap in fixing it: if the sub-agent *consumed* the flag, it would
  /// stop itself and leave the parent running. The user asked for everything
  /// to stop, so only depth 0 clears it.
  #[test]
  fn only_the_top_level_consumes_the_interrupt() {
    let flag = std::sync::atomic::AtomicBool::new(true);
    assert!(take_interrupt(&flag, 2));
    assert!(
      flag.load(Ordering::SeqCst),
      "a sub-agent must leave the flag set for its parent"
    );
    assert!(take_interrupt(&flag, 0), "the parent then sees it");
    assert!(
      !flag.load(Ordering::SeqCst),
      "and the top level is what clears it, so the next turn starts clean"
    );
  }

  #[test]
  fn an_unset_interrupt_stops_nobody() {
    let flag = std::sync::atomic::AtomicBool::new(false);
    assert!(!take_interrupt(&flag, 0));
    assert!(!take_interrupt(&flag, 1));
  }
}
