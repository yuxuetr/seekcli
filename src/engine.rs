//! The Harness engine: the ReAct loop and its supporting passes (Two-Stage
//! planning, System Reminders, Error Recovery, read-concurrent tool dispatch,
//! sub-agent delegation). Split out of main.rs as a separate `impl App` block;
//! as a child module it retains access to App's private fields.

use anyhow::Result;
use colored::Colorize;
use futures_util::StreamExt;
use std::sync::atomic::Ordering;

use crate::api::{self, Message, StreamItem};
use crate::session::{EventPayload, PromptKind};
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
  /// The run hit its total model-call ceiling. Distinct from `MaxIterations`:
  /// that one means the main loop ran out of turns, this means the run — its
  /// sub-agents included — ran out of budget, which a turn cap cannot express
  /// because one turn may spawn any number of sub-agent runs.
  BudgetExhausted,
}

/// Whether a run writes its events to the session log as it goes.
///
/// Only the interactive turn does. `run_headless` is documented as saving no
/// session (the benchmark runner would otherwise leave one behind per eval
/// task), and a sub-agent's events never belong to the parent log — its
/// summary comes back as a tool result instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Journaling {
  /// Append to `current_session` at each ordering point, and fsync.
  Session,
  /// Buffer in `LoopResult::events` and write nothing.
  Buffered,
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
  /// Every installed extension, in one list.
  ///
  /// The three kinds are genuinely different — a skill is a method, an MCP
  /// server is a process, a task is a schedule — and "enabled" means something
  /// different for each, which is why `state` is a sentence rather than a
  /// boolean. Flattening them into a uniform lifecycle would need that
  /// difference to be untrue.
  fn installed_extensions(&self) -> Vec<tools::inspect::ExtensionEntry> {
    let mut out = Vec::new();

    for skill in self.skill_manager.load_skills().unwrap_or_default() {
      let active = self
        .current_skill
        .as_ref()
        .is_some_and(|s| s.name == skill.name);
      out.push(tools::inspect::ExtensionEntry {
        kind: "skill",
        name: skill.name.clone(),
        version: skill.version.clone(),
        source: skill.source.clone().unwrap_or_else(|| "-".to_string()),
        // A skill does nothing until activated, so "installed" is the honest
        // resting state — not "enabled".
        state: if active {
          "active in this conversation".to_string()
        } else {
          "installed, not activated".to_string()
        },
      });
    }

    let connected = self.mcp.server_tool_counts();
    for server in &self.config.mcp_servers {
      let tools = connected
        .iter()
        .find(|(name, _)| name == &server.name)
        .map(|(_, n)| *n);
      out.push(tools::inspect::ExtensionEntry {
        kind: "mcp",
        name: server.name.clone(),
        version: None,
        source: server.command.clone(),
        state: match (server.enabled, tools) {
          (false, _) => "disabled in config".to_string(),
          (true, Some(n)) => format!("connected, {n} tool(s)"),
          // Configured and enabled but absent from the registry: it failed to
          // start. Saying "enabled" here would describe the config rather than
          // the world.
          (true, None) => "enabled but not connected".to_string(),
        },
      });
    }

    for task in crate::tasks::installed(&self.config) {
      out.push(tools::inspect::ExtensionEntry {
        kind: "task",
        name: task.0,
        version: None,
        source: task.1,
        // Whether launchd actually runs it is outside this process, so
        // claiming a schedule here would be claiming something unverified.
        state: "defined; run by the scheduler".to_string(),
      });
    }

    out
  }

  /// What this run's model is allowed to see.
  ///
  /// Assembly order is load-bearing: built-ins, then the skill's own schemas,
  /// then MCP, then the skill's `allowed_tools` as a filter over all of it. The
  /// narrowing has to come last so it narrows the *effective* surface rather
  /// than only the built-ins — and it is a filter precisely so it can never
  /// widen. A skill declaring `run_shell` does not thereby acquire it.
  ///
  /// Sub-agents take their template's list verbatim: that list was written
  /// without knowledge of whatever MCP servers the user happens to have
  /// configured, so silently widening it would undo the narrowing that makes a
  /// sub-agent cheap and safe.
  fn effective_tool_surface(&self, tools: Option<Vec<api::Tool>>, depth: usize) -> Vec<api::Tool> {
    if depth > 0 {
      return tools.unwrap_or_default();
    }

    let mut merged = tools::registry::merge_with_skill(tools);
    merged.push(tools::registry::memory_tool());
    merged.extend(self.mcp.schemas());
    if self.research_enabled {
      merged.extend(tools::registry::research_tools());
    }

    let Some(allowed) = self
      .current_skill
      .as_ref()
      .and_then(|s| s.allowed_tools.as_ref())
    else {
      return merged;
    };

    let (kept, unknown) = tools::registry::narrow_to_skill(merged, allowed);
    if !unknown.is_empty() {
      // A name matching nothing is a typo, or a tool this host does not offer.
      // Honouring it silently would leave the author believing their skill is
      // narrower than it is.
      eprintln!(
        "{} skill declares tool(s) that are not on the surface: {}",
        "[Skill]".yellow(),
        unknown.join(", ")
      );
    }
    if kept.is_empty() {
      eprintln!(
        "{} `allowed_tools` matched nothing — this turn runs with no tools at all",
        "[Skill]".yellow()
      );
    }
    kept
  }

  fn inspect_snapshot(&self, effective: &[api::Tool]) -> tools::inspect::Snapshot {
    let builtin: std::collections::HashSet<String> = tools::registry::system_tools()
      .into_iter()
      .chain(tools::registry::research_tools())
      .chain(std::iter::once(tools::registry::memory_tool()))
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

    let memory_store = self.memory.as_ref();
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
      sources: tools::provenance::snapshot(),
      extensions: self.installed_extensions(),
      child_runs: self
        .current_session
        .events
        .iter()
        .filter_map(|e| match &e.payload {
          EventPayload::ChildRun {
            call_id,
            template,
            status,
            iterations,
            duration_ms,
          } => Some(tools::inspect::ChildRunEntry {
            call_id: call_id.clone(),
            template: template.clone(),
            status: status.clone(),
            iterations: *iterations,
            duration_ms: *duration_ms,
          }),
          _ => None,
        })
        .collect(),
      memory_scopes: memory_store.map(|m| m.scopes()).unwrap_or_default(),
      memory_dir: memory_store
        .map(|m| m.dir().display().to_string())
        .unwrap_or_else(|| "(unavailable)".to_string()),
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
  /// Returns the summary the parent sees, plus the record of the delegation.
  ///
  /// The event is returned rather than recorded here so it joins the same
  /// journal buffer as everything else in the turn and lands in the right
  /// order. `None` when the delegation never started — a bad `subagent_type`
  /// or a depth refusal is a rejected tool call, not a child run.
  async fn delegate_to_subagent(
    &mut self,
    call_id: &str,
    arguments: &str,
    available: &[api::Tool],
    depth: usize,
    parent_span: Option<usize>,
  ) -> (String, Option<EventPayload>) {
    let (subagent_type, prompt) = Self::parse_invoke_agent_args(arguments);
    let next_depth = depth + 1;
    if next_depth > agent::MAX_SUBAGENT_DEPTH {
      return (
        format!(
          "Cannot spawn sub-agent: max depth {} reached.",
          agent::MAX_SUBAGENT_DEPTH
        ),
        None,
      );
    }
    let Some(template) = subagents::registry::lookup(&subagent_type) else {
      let listing: Vec<String> = subagents::registry::catalog()
        .iter()
        .map(|(name, desc)| format!("  - {}: {}", name, desc))
        .collect();
      return (
        format!(
          "Unknown subagent_type '{}'. Available types:\n{}",
          subagent_type,
          listing.join("\n")
        ),
        None,
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
    let started = std::time::Instant::now();
    match Box::pin(self.run_agent_loop(
      sub_messages,
      Some(sub_tools),
      next_depth,
      template.max_iter,
      parent_span,
      // A sub-agent's turns are not the parent's conversation; only its
      // summary crosses back, as a tool result the parent then journals.
      Journaling::Buffered,
    ))
    .await
    {
      // An interrupted or exhausted sub-agent has not "completed". Saying so
      // would hand the parent a false success to reason from — the same
      // confusion between "the model stopped" and "the task passed" that
      // `LoopStatus` exists to keep apart.
      Ok(run) => {
        let (verdict, status) = match run.status {
          LoopStatus::Completed => ("completed", "completed"),
          LoopStatus::BudgetExhausted => (
            "stopped because the run's model-call budget was spent",
            "budget_exhausted",
          ),
          LoopStatus::Interrupted => (
            "was interrupted by the user before finishing",
            "interrupted",
          ),
          LoopStatus::MaxIterations => ("hit its iteration cap before finishing", "max_iterations"),
        };
        (
          format!(
            "Sub-agent '{}' {}. Summary:\n{}",
            template.name, verdict, run.text
          ),
          Some(EventPayload::ChildRun {
            call_id: call_id.to_string(),
            template: template.name.to_string(),
            status: status.to_string(),
            iterations: run.iterations,
            duration_ms: started.elapsed().as_millis() as u64,
          }),
        )
      }
      Err(e) => (
        format!("Sub-agent '{}' failed: {}", template.name, e),
        Some(EventPayload::ChildRun {
          call_id: call_id.to_string(),
          template: template.name.to_string(),
          status: "failed".to_string(),
          iterations: 0,
          duration_ms: started.elapsed().as_millis() as u64,
        }),
      ),
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
/// Get everything recorded so far onto disk, and say whether it worked.
///
/// The ordering this exists to enforce:
///
/// ```text
///   record the intent (and its approval)  ->  fsync  ->  run the tool
///   ->  record the result  ->  fsync  ->  hand it back to the model
/// ```
///
/// Before this, nothing was written until the turn ended, so a crash mid-turn
/// lost every tool call it had already made — the files were changed, the log
/// said nothing. Writing the intent first cannot make a crash impossible; it
/// makes the crash *legible*, by leaving a call with no result, which the
/// projection turns into an explicit "unknown result" rather than a hole.
///
/// Returns the number of events written, or the error that stopped it.
impl App {
  fn flush_journal(&mut self, pending: &mut Vec<EventPayload>) -> Result<usize> {
    if pending.is_empty() {
      return Ok(0);
    }
    let n = pending.len();
    self.current_session.extend(pending.drain(..));
    self.history.save_session(&mut self.current_session)?;
    Ok(n)
  }
}

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
    let memory_note = self
      .memory
      .as_ref()
      .and_then(|m| agent::prompt::memory_rules(&m.preferences(), &m.scopes()));
    let injected = Self::ensure_agent_system_prompt(
      &mut messages,
      self.plan_mode,
      self.research_enabled,
      memory_note,
    );
    // Logged before the request goes out, for the same reason tool intent is:
    // a record written afterwards is a record that a crash can lose.
    self.record_injected_context(&injected);
    self.run_call_baseline = self.cost.api_calls;
    let run_span = self.tracer.start_run();
    // The join key between the two records. Before this the trace and the
    // event log were separate identity spaces — spans numbered per run, events
    // numbered per session — so "which turn produced this span" could only be
    // guessed at from timestamps. `first_seq` is where this run starts in the
    // log; a span's `seq` says which event it belongs to.
    self.tracer.annotate(
      run_span,
      serde_json::json!({
        "session": self.current_session.id(),
        "first_seq": self.current_session.events.len(),
      }),
    );
    let run = self
      .run_agent_loop(
        messages,
        tools,
        0,
        agent::MAX_ITER,
        run_span,
        Journaling::Session,
      )
      .await?;
    self.tracer.end(run_span);
    self.current_session.extend(run.events);

    if self.current_session.meta.title == crate::session::UNTITLED {
      self.current_session.meta.title = title_from(content);
    }
    // Persist the session's running cost so it can be audited / restored later.
    self.current_session.meta.cost = self.cost.clone();
    self.history.save_session(&mut self.current_session)?;

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
    let memory_note = self
      .memory
      .as_ref()
      .and_then(|m| agent::prompt::memory_rules(&m.preferences(), &m.scopes()));
    // Headless keeps no session, so there is nothing to record into — the
    // composed prompt is returned and dropped. `--bench` and `--run-task`
    // deliberately leave no conversation behind.
    let _injected = Self::ensure_agent_system_prompt(
      &mut messages,
      self.plan_mode,
      self.research_enabled,
      memory_note,
    );
    let tools = skill.and_then(|s| s.to_api_tools());
    // Headless runs used to produce no trace at all: this path never opened a
    // run span and never flushed. That left tracing broken in exactly the mode
    // where nobody is watching the terminal — `-p`, `--bench`, `--run-task`.
    self.run_call_baseline = self.cost.api_calls;
    let run_span = self.tracer.start_run();
    let run = self
      .run_agent_loop(
        messages,
        tools,
        0,
        self.max_iter,
        run_span,
        Journaling::Buffered,
      )
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

  /// Compose the system messages a request needs, and report the dynamic ones
  /// back so the caller can log them.
  ///
  /// The kernel is not reported: it is a compile-time constant, so "what did
  /// the model see" is answerable for it without a record. Everything else
  /// depends on state outside the log — the workspace's `AGENTS.md`, the user's
  /// memory directory, the config — and `design-principles.md` §3 says model
  /// visible means logged.
  fn ensure_agent_system_prompt(
    messages: &mut Vec<Message>,
    plan_mode: bool,
    research: bool,
    memory: Option<String>,
  ) -> Vec<(PromptKind, String)> {
    let mut injected: Vec<(PromptKind, String)> = Vec::new();
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
      injected.push((PromptKind::Workspace, rules.clone()));
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

    // The citation contract, on the same terms as workspace rules: a separate
    // message after the kernel, so the cache prefix is untouched and it is
    // absent entirely when the web tools are not on the surface.
    if research {
      let rules = agent::prompt::research_rules();
      injected.push((PromptKind::Research, rules.clone()));
      let present = messages.iter().any(|m| {
        matches!(
          m,
          Message::Simple { role, content: t, .. }
            if role == "system" && t == &rules
        )
      });
      if !present {
        let head_end = messages
          .iter()
          .take_while(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
          .count();
        messages.insert(
          head_end,
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

    // Persistent memory, on the same terms: a separate message after the
    // kernel, absent entirely when nothing has been recorded.
    if let Some(mem) = memory {
      injected.push((PromptKind::Memory, mem.clone()));
      let present = messages.iter().any(|m| {
        matches!(
          m,
          Message::Simple { role, content: t, .. }
            if role == "system" && t == &mem
        )
      });
      if !present {
        let head_end = messages
          .iter()
          .take_while(|m| matches!(m, Message::Simple { role, .. } if role == "system"))
          .count();
        messages.insert(
          head_end,
          Message::Simple {
            images: Vec::new(),
            role: "system".to_string(),
            content: mem,
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
      injected.push((PromptKind::Skill, plan_msg.clone()));
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
    injected
  }

  /// Log what the harness injected into this request, once per distinct
  /// content.
  ///
  /// Deduplicated by digest against the last record for the same kind: the
  /// memory note and the workspace rules are usually identical turn after
  /// turn, and one event each per turn would bury the conversation in
  /// bookkeeping while telling a reader nothing new. A digest that *has*
  /// changed is exactly the interesting case, and it gets a fresh blob.
  fn record_injected_context(&mut self, injected: &[(PromptKind, String)]) {
    for (kind, content) in injected {
      let digest = crate::tools::audit::digest(content);
      let unchanged = self
        .current_session
        .events
        .iter()
        .rev()
        .find_map(|e| match &e.payload {
          EventPayload::ContextInjected {
            kind: k, digest: d, ..
          } if k == kind => Some(d.clone()),
          _ => None,
        })
        .is_some_and(|last| last == digest);
      if unchanged {
        continue;
      }
      let blob = crate::tools::offload::persist_text(&digest, content).ok();
      if blob.is_none() {
        // Visible, not silent: without the blob the record says *that* the
        // context changed but not to what, and a reader should know which.
        eprintln!(
          "{} could not store the injected {:?} context; logging its digest only",
          "[Session]".yellow(),
          kind
        );
      }
      self.current_session.record(EventPayload::ContextInjected {
        kind: *kind,
        digest,
        blob: blob.map(|p| p.display().to_string()),
      });
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
    journaling: Journaling,
  ) -> Result<LoopResult> {
    if depth > agent::MAX_SUBAGENT_DEPTH {
      anyhow::bail!(
        "Max sub-agent depth ({}) exceeded",
        agent::MAX_SUBAGENT_DEPTH
      );
    }

    let tool_dispatcher = tools::ToolDispatcher::new();
    let effective_tools = self.effective_tool_surface(tools, depth);

    let mut events: Vec<EventPayload> = Vec::new();
    let mut final_content = String::new();
    let mut completed = false;
    let mut interrupted = false;
    let mut budget_exhausted = false;
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
      // Where this turn sits in the session log. Only meaningful for the run
      // that journals — a sub-agent's events never reach the session.
      if journaling == Journaling::Session {
        self.tracer.annotate(
          turn_span,
          serde_json::json!({ "seq": self.current_session.events.len() + events.len() }),
        );
      }

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

      // The one ceiling that bounds a whole run rather than one dimension of
      // it. Checked at every depth, against a baseline taken when the run
      // started — so a sub-agent charges the same counter as its parent, and
      // "inherits the budget" is true by construction rather than by plumbing.
      let spent = self.cost.api_calls.saturating_sub(self.run_call_baseline);
      if spent >= self.max_llm_calls {
        eprintln!(
          "\n{} run budget spent: {} model call(s). Stopping. Raise \
           `[limits] max_llm_calls_per_run` if this run was legitimate.",
          "[Budget]".yellow(),
          spent
        );
        budget_exhausted = true;
        final_content = format!(
          "[Stopped: run budget of {} model calls spent]",
          self.max_llm_calls
        );
        completed = true;
        self.tracer.end(turn_span);
        break;
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

      // Ordering point 1: the intent to call these tools is on disk before
      // any of them runs. A crash from here on leaves a call with no result,
      // which the projection renders as an explicit "unknown result".
      if journaling == Journaling::Session
        && let Err(e) = self.flush_journal(&mut events)
      {
        // Whether this is fatal depends on what is about to happen. Read-only
        // calls can be replayed freely, so an unwritable journal costs
        // traceability and nothing else. Anything that writes must not run:
        // continuing would change the world while the record of why is
        // already known to be lost.
        let side_effecting = tool_calls.iter().any(|tc| {
          if crate::mcp::is_mcp_tool(&tc.function.name) {
            !self.mcp.is_read_only(&tc.function.name)
          } else {
            !tools::registry::is_parallel_readonly(&tc.function.name)
          }
        });
        if side_effecting {
          self.tracer.end(turn_span);
          anyhow::bail!(
            "refusing to run tools with side effects: the session journal could not be \
             written ({e}). Nothing was executed. Fix the problem with \
             ~/.seekcli/sessions and retry."
          );
        }
        eprintln!(
          "{} journal write failed: {} — continuing because this turn only reads",
          "[Session]".yellow(),
          e
        );
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
            let (summary, record) = self
              .delegate_to_subagent(
                &tc.id,
                &tc.function.arguments,
                &effective_tools,
                depth,
                exec_span,
              )
              .await;
            // Into the same buffer as the rest of the turn, so it lands between
            // the assistant message that asked for the delegation and the tool
            // result that reports it.
            if let Some(event) = record {
              events.push(event);
            }
            summary
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

      // Ordering point 2: results are on disk before they go back to the
      // model. A degradation here is not fatal — the side effects already
      // happened, and stopping now would lose the results as well as the
      // record of them — but it is never silent.
      if journaling == Journaling::Session
        && let Err(e) = self.flush_journal(&mut events)
      {
        eprintln!(
          "{} tool results could not be journaled: {} — this turn is no longer \
           fully recoverable",
          "[Session]".yellow(),
          e
        );
      }

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
    } else if budget_exhausted {
      LoopStatus::BudgetExhausted
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

  /// The contract is absent when the tools are — describing a capability that
  /// is not on the surface costs a turn every time the model tries it.
  #[test]
  fn the_citation_contract_appears_only_with_the_web_tools() {
    let mut without = vec![Message::new_user_text("hi".to_string())];
    App::ensure_agent_system_prompt(&mut without, false, false, None);
    assert!(
      !without.iter().any(|m| matches!(
        m,
        Message::Simple { content, .. } if content.contains("Research and citation")
      )),
      "must not describe tools that are not offered"
    );

    let mut with = vec![Message::new_user_text("hi".to_string())];
    App::ensure_agent_system_prompt(&mut with, false, true, None);
    assert!(
      with.iter().any(|m| matches!(
        m,
        Message::Simple { content, .. } if content.contains("Research and citation")
      )),
      "{with:?}"
    );
  }

  /// The kernel at index 0 must stay byte-identical whatever else is injected,
  /// or every turn pays a full prompt-cache miss.
  #[test]
  fn injecting_the_contract_does_not_disturb_the_cache_prefix() {
    let kernel = agent::prompt::agent_system_prompt();
    let mut messages = vec![Message::new_user_text("hi".to_string())];
    App::ensure_agent_system_prompt(&mut messages, false, true, None);
    match messages.first() {
      Some(Message::Simple { role, content, .. }) => {
        assert_eq!(role, "system");
        assert_eq!(content, &kernel, "the cache prefix moved");
      }
      other => panic!("expected the kernel at index 0, got {other:?}"),
    }
  }

  /// Injection must be idempotent: `ensure_` runs every turn, and a contract
  /// appended once per turn would grow the prompt without bound.
  #[test]
  fn the_contract_is_injected_once_not_once_per_turn() {
    let mut messages = vec![Message::new_user_text("hi".to_string())];
    for _ in 0..3 {
      App::ensure_agent_system_prompt(&mut messages, false, true, None);
    }
    let count = messages
      .iter()
      .filter(|m| {
        matches!(m, Message::Simple { content, .. } if content.contains("Research and citation"))
      })
      .count();
    assert_eq!(count, 1, "{messages:?}");
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
  /// The ordering the journal exists to guarantee: what the loop recorded is
  /// on disk *before* the tools run, not after the whole turn ends. Until this
  /// existed, a crash mid-turn lost every call the turn had already made — the
  /// files were changed and the log said nothing about why.
  #[test]
  fn flushing_puts_the_record_on_disk_mid_turn() {
    let _guard = crate::testsync::lock();
    let mut app = match App::for_test(Box::new(crate::api::record::Replaying::new(
      std::path::PathBuf::from("tests/fixtures/nonexistent"),
    ))) {
      Ok(a) => a,
      Err(e) => panic!("cannot build test App: {e}"),
    };

    let mut pending = vec![EventPayload::UserMessage {
      images: Vec::new(),
      content: "before the tools run".to_string(),
    }];
    match app.flush_journal(&mut pending) {
      Ok(n) => assert_eq!(n, 1),
      Err(e) => panic!("flush failed: {e}"),
    }
    assert!(pending.is_empty(), "flushed events must leave the buffer");

    // On disk, not merely in the in-memory session.
    let id = app.current_session.id().to_string();
    let reloaded = match app.history.load_session(&id) {
      Ok(s) => s,
      Err(e) => panic!("cannot reload: {e}"),
    };
    assert_eq!(reloaded.events.len(), 1, "the event never reached the file");

    // And a second flush with nothing pending is a no-op, not a rewrite.
    let mut empty: Vec<EventPayload> = Vec::new();
    match app.flush_journal(&mut empty) {
      Ok(n) => assert_eq!(n, 0),
      Err(e) => panic!("no-op flush failed: {e}"),
    }
  }

  /// Assembly order is the property worth testing, and it could not be tested
  /// while it lived inside the 470-line loop body: narrowing must apply to the
  /// *whole* surface, so it has to run after MCP and the research tools, not
  /// over the built-ins alone.
  #[test]
  fn a_skills_whitelist_narrows_the_whole_surface_not_just_the_builtins() {
    let mut app = test_app();
    app.research_enabled = true;
    app.current_skill = Some(crate::Skill {
      name: "narrow".into(),
      description: "d".into(),
      system_prompt: "p".into(),
      tools: None,
      version: None,
      source: None,
      allowed_tools: Some(vec!["read_file".into(), "web_search".into()]),
    });

    let surface = app.effective_tool_surface(None, 0);
    let names: Vec<&str> = surface.iter().map(|t| t.function.name.as_str()).collect();
    assert!(names.contains(&"read_file"), "{names:?}");
    // A research tool added after the built-ins must still be subject to the
    // whitelist — which is what "narrow the effective surface" means.
    assert!(names.contains(&"web_search"), "{names:?}");
    assert!(!names.contains(&"run_shell"), "narrowing failed: {names:?}");
    assert!(!names.contains(&"web_fetch"), "narrowing failed: {names:?}");
    assert!(!names.contains(&"memory"), "narrowing failed: {names:?}");
  }

  /// A sub-agent takes its template's list verbatim. Widening it with whatever
  /// MCP servers the host happens to have configured would undo the narrowing
  /// that makes a sub-agent cheap and safe.
  #[test]
  fn a_sub_agent_surface_is_not_widened_by_host_configuration() {
    let mut app = test_app();
    app.research_enabled = true;
    let template_tools = tools::registry::system_tools()
      .into_iter()
      .filter(|t| t.function.name == "read_file")
      .collect::<Vec<_>>();

    let surface = app.effective_tool_surface(Some(template_tools), 1);
    let names: Vec<&str> = surface.iter().map(|t| t.function.name.as_str()).collect();
    assert_eq!(names, vec!["read_file"], "{names:?}");
  }

  /// Every other ceiling bounds one dimension; none bounds the product. The
  /// measurement that produced this: both sub-agent templates exclude
  /// `invoke_agent`, so `MAX_SUBAGENT_DEPTH = 3` guards a path that cannot be
  /// taken — while tool calls per turn, and therefore sub-agent runs per turn,
  /// had no ceiling at all.
  #[test]
  fn the_run_budget_is_measured_from_the_runs_own_baseline() {
    let mut app = test_app();
    app.max_llm_calls = 10;
    // A session that has already spent calls must not eat into this run's
    // budget — the ceiling is per run, the counter is per session.
    app.cost.api_calls = 500;
    app.run_call_baseline = app.cost.api_calls;
    let spent = app.cost.api_calls.saturating_sub(app.run_call_baseline);
    assert_eq!(spent, 0, "a fresh run starts with nothing spent");

    app.cost.api_calls += 10;
    let spent = app.cost.api_calls.saturating_sub(app.run_call_baseline);
    assert!(spent >= app.max_llm_calls, "the ceiling must be reachable");
  }

  /// The property that makes "a sub-agent inherits its parent's budget" true
  /// without threading a budget object through the call stack: both charge the
  /// same `cost` counter, so the parent's remaining budget shrinks as the
  /// child spends.
  #[test]
  fn a_sub_agent_spends_the_same_budget_as_its_parent() {
    let mut app = test_app();
    app.max_llm_calls = 100;
    app.run_call_baseline = app.cost.api_calls;
    // Parent makes 3 calls, then delegates; the child makes 20.
    app.cost.api_calls += 3;
    let after_parent = app.cost.api_calls.saturating_sub(app.run_call_baseline);
    app.cost.api_calls += 20;
    let after_child = app.cost.api_calls.saturating_sub(app.run_call_baseline);
    assert_eq!(after_parent, 3);
    assert_eq!(
      after_child, 23,
      "the child's calls must come out of the same budget"
    );
  }

  /// `BudgetExhausted` has to be its own status: "the main loop ran out of
  /// turns" and "the run ran out of budget" are different facts, and one turn
  /// can spawn any number of sub-agent runs, so a turn cap cannot express the
  /// second.
  #[test]
  fn budget_exhaustion_is_not_reported_as_max_iterations() {
    assert_ne!(LoopStatus::BudgetExhausted, LoopStatus::MaxIterations);
    assert_ne!(LoopStatus::BudgetExhausted, LoopStatus::Completed);
  }

  /// A recorded run must be reproducible from the recording alone.
  ///
  /// The memory note goes into the prompt, so a test `App` that read the real
  /// `~/.seekcli/memory` would replay differently depending on what the user
  /// happened to remember last week. That is not hypothetical — it is how this
  /// was found: five replay fixtures went red the moment the developer's own
  /// memory directory had anything in it. On a clean machine they would have
  /// stayed green, so the invariant needs its own assertion rather than relying
  /// on the fixtures to notice.
  fn test_app() -> App {
    match App::for_test(Box::new(crate::api::record::Replaying::new(
      std::path::PathBuf::from("tests/fixtures/nonexistent"),
    ))) {
      Ok(a) => a,
      Err(e) => panic!("cannot build test App: {e}"),
    }
  }

  /// The trap this design avoids. Recording an injected prompt as a
  /// `SystemPrompt` would make it re-project, and since the memory note changes
  /// as the user's notes do — and an append-only log cannot retract the old one
  /// — the prompt would accumulate stale snapshots of itself.
  #[test]
  fn an_injected_context_is_logged_without_entering_the_projection() {
    let mut app = test_app();
    let before = app.current_session.messages().len();
    app.record_injected_context(&[(PromptKind::Memory, "notes v1".to_string())]);
    assert_eq!(
      app.current_session.messages().len(),
      before,
      "a bookkeeping event must not become a message"
    );
    assert_eq!(
      app.current_session.events.len(),
      1,
      "but it must be in the log — that is the whole point"
    );
  }

  /// The memory note and the workspace rules are usually identical turn after
  /// turn. One event each per turn would bury the conversation in bookkeeping
  /// while telling a reader nothing new.
  #[test]
  fn unchanged_context_is_not_re_recorded_every_turn() {
    let mut app = test_app();
    for _ in 0..5 {
      app.record_injected_context(&[(PromptKind::Memory, "notes v1".to_string())]);
    }
    assert_eq!(
      app.current_session.events.len(),
      1,
      "five turns, one record"
    );
  }

  /// A digest that *has* changed is the interesting case, and it must land.
  #[test]
  fn changed_context_is_recorded_again() {
    let mut app = test_app();
    app.record_injected_context(&[(PromptKind::Memory, "notes v1".to_string())]);
    app.record_injected_context(&[(PromptKind::Memory, "notes v2".to_string())]);
    assert_eq!(app.current_session.events.len(), 2);
    let digests: Vec<String> = app
      .current_session
      .events
      .iter()
      .filter_map(|e| match &e.payload {
        EventPayload::ContextInjected { digest, .. } => Some(digest.clone()),
        _ => None,
      })
      .collect();
    assert_eq!(digests.len(), 2);
    assert_ne!(digests[0], digests[1], "the change must be visible");
  }

  /// Two kinds are tracked independently: memory changing must not suppress a
  /// workspace-rules record, or a reader would see one and conclude the other
  /// was unchanged.
  #[test]
  fn each_kind_is_deduplicated_against_its_own_history() {
    let mut app = test_app();
    app.record_injected_context(&[
      (PromptKind::Memory, "m1".to_string()),
      (PromptKind::Workspace, "w1".to_string()),
    ]);
    app.record_injected_context(&[
      (PromptKind::Memory, "m2".to_string()),
      (PromptKind::Workspace, "w1".to_string()),
    ]);
    assert_eq!(
      app.current_session.events.len(),
      3,
      "m1, w1, m2 — w1 unchanged so not repeated"
    );
  }

  #[test]
  fn a_test_app_reads_no_real_memory() {
    let app = test_app();
    assert!(
      app.memory.is_none(),
      "a replayed prompt must not depend on the developer's memory directory"
    );
  }

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
