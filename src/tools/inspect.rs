//! Self-description: what the harness looks like from the inside.
//!
//! The model could previously see its tool schemas and nothing else. When a
//! call was refused it had no way to ask *what would be allowed*, so its only
//! move was to guess again — which is why the denial messages had to carry
//! their own teaching text (`docs/architecture/L1-engine.md` §4.6). This module
//! is the other half: the model can ask, instead of being told.
//!
//! Everything here is a pure function over a plain-data [`Snapshot`]. The
//! engine assembles the snapshot because only it can reach the live registries;
//! keeping the rendering pure is what makes it assertable without building an
//! `App`, the same split `observability/bench.rs` uses.

use anyhow::Result;
use serde_json::Value;

use super::policy;

/// One entry of the current tool surface.
pub struct ToolEntry {
  pub signature: String,
  /// Where the capability came from: built-in, an MCP server, or a skill.
  pub source: String,
}

/// A configured MCP server and how many tools it contributed.
pub struct ServerEntry {
  pub name: String,
  pub tools: usize,
}

/// A configured server that is unavailable, and why.
pub struct FailureEntry {
  pub server: String,
  pub reason: String,
}

/// The live facts the report is rendered from.
pub struct Snapshot {
  pub mode: &'static str,
  pub plan_mode: bool,
  pub tools: Vec<ToolEntry>,
  pub servers: Vec<ServerEntry>,
  pub failures: Vec<FailureEntry>,
  pub active_skill: Option<String>,
  pub skills: Vec<String>,
  pub proposals: Vec<String>,
  pub session_id: String,
  pub events: usize,
  pub compactions: usize,
  /// When compaction trips, and how that number was arrived at. Rendered so
  /// the model (and the user reading `/inspect`) can tell a threshold that was
  /// configured from one that was defaulted — a window set too high for the
  /// model in use is otherwise invisible until a request fails on length.
  pub memory: MemoryEntry,
  /// What this session searched up versus what it actually read.
  pub sources: Vec<super::provenance::Source>,
  /// Everything installed that extends this agent, whatever its kind.
  pub extensions: Vec<ExtensionEntry>,
  /// Sub-agent delegations made in this session, with how each ended.
  pub child_runs: Vec<ChildRunEntry>,
  /// Persistent memory scopes, and where the files live.
  pub memory_scopes: Vec<crate::memory::ScopeSummary>,
  pub memory_dir: String,
}

/// One installed extension, whatever kind it is.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionEntry {
  /// `skill` / `mcp` / `task`.
  pub kind: &'static str,
  pub name: String,
  pub version: Option<String>,
  /// Where it came from — a path, or a command for an MCP server.
  pub source: String,
  /// Whether it is doing anything right now. Means something different per
  /// kind, which is why it is a sentence and not a boolean.
  pub state: String,
}

/// One delegation, as the session section renders it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChildRunEntry {
  pub call_id: String,
  pub template: String,
  pub status: String,
  pub iterations: usize,
  pub duration_ms: u64,
}

/// The `[memory]` budget as the session section renders it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryEntry {
  pub threshold_tokens: usize,
  pub window_tokens: usize,
  pub ratio: f64,
  pub keep_tail: usize,
}

/// The sections a caller may ask for.
const SECTIONS: &[&str] = &[
  "tools",
  "policy",
  "skills",
  "mcp",
  "session",
  "sources",
  "memory",
  "extensions",
];

/// Render the requested section, or every section when `what` is absent.
pub fn render(args: &Value, snap: &Snapshot) -> Result<String> {
  let what = args.get("what").and_then(Value::as_str);
  match what {
    None => Ok(
      SECTIONS
        .iter()
        .map(|s| section(s, snap))
        .collect::<Vec<_>>()
        .join("\n"),
    ),
    Some(name) if SECTIONS.contains(&name) => Ok(section(name, snap)),
    // Same rule as every other refusal in the harness: name the valid choices
    // rather than only rejecting the invalid one.
    Some(other) => anyhow::bail!(
      "unknown section `{}`. Valid values for `what`: {}. Omit `what` for all of them.",
      other,
      SECTIONS.join(", ")
    ),
  }
}

fn section(name: &str, snap: &Snapshot) -> String {
  match name {
    "tools" => tools(snap),
    "policy" => policy_section(snap),
    "skills" => skills(snap),
    "mcp" => mcp(snap),
    "session" => session(snap),
    "sources" => sources(snap),
    "memory" => memory(snap),
    "extensions" => extensions(snap),
    // `render` only passes values from SECTIONS.
    other => unreachable!("unlisted section `{other}` reached the renderer"),
  }
}

fn tools(snap: &Snapshot) -> String {
  let mut out = format!("# tools ({})\n", snap.tools.len());
  for t in &snap.tools {
    out.push_str(&format!("- {} [{}]\n", t.signature, t.source));
  }
  out
}

/// The policy section, rendered from `policy.rs`'s own constants.
///
/// Never a hand-written description of them: that is a second source of truth
/// for the same rules, and the model would believe it after it drifted
/// (`docs/architecture/L7-observability.md` §4.5.2).
fn policy_section(snap: &Snapshot) -> String {
  let mut out = format!("# policy\nmode: {}\n", snap.mode);
  if snap.plan_mode {
    out.push_str(
      "plan mode: on (externalize state to PLAN.md / TODO.md; not a write restriction)\n",
    );
  }
  if snap.mode == policy::Mode::ReadOnly.name() {
    out.push_str("write tools are refused; `run_shell` still runs commands that only report.\n");
    out.push_str(&format!(
      "reporting commands: {}\n",
      policy::read_only_commands().join(", ")
    ));
    for (program, subs) in policy::mutating_subcommands() {
      out.push_str(&format!(
        "`{}` sub-commands that are refused: {}\n",
        program,
        subs.join(", ")
      ));
    }
    out.push_str("any redirect (`>`) is refused whatever program precedes it.\n");
  } else {
    out.push_str("writes are allowed inside the workspace; paths outside it are refused.\n");
    out.push_str(
      "dangerous commands (sudo, rm -rf on system paths, curl|sh, ...) need the user's \
       approval, and a few (fork bomb, mkfs, raw device write) are refused outright.\n",
    );
  }
  out
}

fn skills(snap: &Snapshot) -> String {
  let mut out = String::from("# skills\n");
  out.push_str(&format!(
    "active: {}\n",
    snap.active_skill.as_deref().unwrap_or("none")
  ));
  out.push_str(&format!(
    "available: {}\n",
    if snap.skills.is_empty() {
      "none".to_string()
    } else {
      snap.skills.join(", ")
    }
  ));
  // Proposals are listed because the model writes them: seeing one already
  // pending is what stops it from drafting the same thing twice. Every kind, so
  // a pending MCP server shows up here too.
  out.push_str(&format!(
    "awaiting the user's review: {}\n",
    if snap.proposals.is_empty() {
      "none".to_string()
    } else {
      snap.proposals.join(", ")
    }
  ));
  out
}

fn mcp(snap: &Snapshot) -> String {
  let mut out = String::from("# mcp\n");
  if snap.servers.is_empty() && snap.failures.is_empty() {
    out.push_str("no servers configured.\n");
    return out;
  }
  for s in &snap.servers {
    out.push_str(&format!("- {}: connected, {} tool(s)\n", s.name, s.tools));
  }
  for f in &snap.failures {
    out.push_str(&format!("- {}: UNAVAILABLE — {}\n", f.server, f.reason));
  }
  if !snap.failures.is_empty() {
    out.push_str(
      "an unavailable server's tools do not exist this session; restarting it is the \
       user's action, not yours.\n",
    );
  }
  out
}

fn session(snap: &Snapshot) -> String {
  let mut out = format!(
    "# session\nid: {}\nevents: {}\ncompactions: {}\n\
     compact_at: {} tokens (window {} x {})\nkeep_tail: {} messages\n",
    snap.session_id,
    snap.events,
    snap.compactions,
    snap.memory.threshold_tokens,
    snap.memory.window_tokens,
    snap.memory.ratio,
    snap.memory.keep_tail
  );
  // A delegation that quietly hit its iteration cap used to be
  // indistinguishable from one that finished, because only the summary text
  // came back. Showing how each ended, and how long it took, is the point of
  // recording them at all.
  if !snap.child_runs.is_empty() {
    out.push_str("sub-agent runs:\n");
    for c in &snap.child_runs {
      out.push_str(&format!(
        "  {} — {} after {} iteration(s), {}ms (call {})\n",
        c.template, c.status, c.iterations, c.duration_ms, c.call_id
      ));
    }
  }
  out
}

/// One listing for everything installed that extends the agent.
///
/// Skills and MCP servers already had sections, and tasks had none at all —
/// but spread across them, "what is extending this agent right now, and where
/// did it come from" had no single answer. That question is the whole of what
/// a plugin system would be *for*; the registry, the lifecycle and the
/// hot-swapping are machinery in service of it, and on a single-user CLI the
/// machinery costs more than the answer is worth.
///
/// So: the answer, without the machinery. `docs/architecture/L5-composition.md`
/// §4.6 records what is deliberately absent and what would have to change for
/// that to be worth revisiting.
fn extensions(snap: &Snapshot) -> String {
  if snap.extensions.is_empty() {
    return "# extensions\n(nothing installed)\n".to_string();
  }
  let mut out = String::from("# extensions\n");
  for e in &snap.extensions {
    // `v-` for an absent version reads as a version called "-". A bare dash
    // reads as "there isn't one", which is what it means.
    let version = match &e.version {
      Some(v) => format!("v{v}"),
      None => "-".to_string(),
    };
    out.push_str(&format!(
      "  {:<5} {:<22} {:<8} {:<26} {}\n",
      e.kind, e.name, version, e.state, e.source
    ));
  }
  out.push_str(
    "Versions are recorded, not enforced: nothing here is hot-swapped \
     mid-run, and a change lands on the next run.\n",
  );
  out
}

/// Where the persistent notes are and what is in them.
///
/// Naming the directory is the point, not decoration: a memory the user cannot
/// see how to remove is a rule they did not agree to. The path is the answer to
/// "how do I undo this".
fn memory(snap: &Snapshot) -> String {
  let mut out = format!("# memory\ndir: {}\n", snap.memory_dir);
  if snap.memory_scopes.is_empty() {
    out.push_str("(nothing recorded yet)\n");
    return out;
  }
  for s in &snap.memory_scopes {
    let note = if s.name == crate::memory::PREFERENCES {
      "  <- rules about the user; only the proposal gate writes here"
    } else {
      ""
    };
    out.push_str(&format!("  {} ({} entries){}\n", s.name, s.entries, note));
  }
  out.push_str("Every file is plain Markdown: edit or delete any line by hand.\n");
  out
}

/// Searched versus read, kept apart.
///
/// A conclusion supported only by search snippets rests on a page nobody
/// opened. Rendering the two groups separately makes that visible without
/// having to take anyone's word for it.
fn sources(snap: &Snapshot) -> String {
  if snap.sources.is_empty() {
    return "# sources\n(nothing searched or fetched this session)\n".to_string();
  }
  let mut out = String::from("# sources\n");
  let (read, seen): (Vec<_>, Vec<_>) = snap.sources.iter().partition(|s| s.fetched_at.is_some());
  out.push_str(&format!("read ({}):\n", read.len()));
  for s in &read {
    out.push_str(&format!(
      "  {} (fetched {})\n",
      s.url,
      s.fetched_at.as_deref().unwrap_or("?")
    ));
  }
  out.push_str(&format!(
    "search hits not opened ({}) -- snippets only, not evidence:\n",
    seen.len()
  ));
  for s in &seen {
    out.push_str(&format!("  {}\n", s.url));
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn snap() -> Snapshot {
    Snapshot {
      mode: "normal",
      plan_mode: false,
      tools: vec![ToolEntry {
        signature: "read_file(path)".into(),
        source: "built-in".into(),
      }],
      servers: vec![ServerEntry {
        name: "github".into(),
        tools: 3,
      }],
      failures: vec![FailureEntry {
        server: "db".into(),
        reason: "command `dbmcp` not found".into(),
      }],
      active_skill: None,
      skills: vec!["rust-review".into()],
      proposals: vec!["draft-helper".into()],
      session_id: "abc123".into(),
      events: 42,
      compactions: 1,
      sources: Vec::new(),
      extensions: Vec::new(),
      child_runs: Vec::new(),
      memory_scopes: Vec::new(),
      memory_dir: "/tmp/seekcli-test-memory".into(),
      memory: MemoryEntry {
        threshold_tokens: 150_000,
        window_tokens: 200_000,
        ratio: 0.75,
        keep_tail: 8,
      },
    }
  }

  #[test]
  fn omitting_what_returns_every_section() {
    let out = match render(&json!({}), &snap()) {
      Ok(o) => o,
      Err(e) => panic!("{e}"),
    };
    for s in SECTIONS {
      assert!(out.contains(&format!("# {s}")), "missing {s} in:\n{out}");
    }
  }

  /// A window set too high for the model in use is invisible until a request
  /// fails on length. Rendering the derivation makes it inspectable before then.
  #[test]
  fn the_session_section_shows_where_the_threshold_came_from() {
    let out = section("session", &snap());
    assert!(out.contains("compact_at: 150000"), "{out}");
    assert!(out.contains("window 200000"), "{out}");
    assert!(out.contains("keep_tail: 8"), "{out}");
  }

  /// The distinction the section exists for: a conclusion supported only by
  /// snippets rests on a page nobody opened, and that must be visible.
  #[test]
  fn the_sources_section_separates_what_was_read_from_what_was_merely_found() {
    let mut s = snap();
    s.sources = vec![
      super::super::provenance::Source {
        url: "https://opened.test".into(),
        fetched_at: Some("2026-09-16T00:00:00Z".into()),
      },
      super::super::provenance::Source {
        url: "https://only-a-snippet.test".into(),
        fetched_at: None,
      },
    ];
    let out = section("sources", &s);
    assert!(out.contains("read (1)"), "{out}");
    assert!(out.contains("https://opened.test"), "{out}");
    assert!(out.contains("search hits not opened (1)"), "{out}");
    assert!(out.contains("not evidence"), "{out}");
  }

  #[test]
  fn an_empty_sources_section_says_so_rather_than_rendering_nothing() {
    let out = section("sources", &snap());
    assert!(out.contains("nothing searched or fetched"), "{out}");
  }

  /// The one question a plugin system would exist to answer: what is extending
  /// this agent right now, and where did each piece come from. Answering it
  /// needed a listing, not a lifecycle.
  #[test]
  fn the_extensions_section_answers_what_is_installed_and_from_where() {
    let mut s = snap();
    s.extensions = vec![
      ExtensionEntry {
        kind: "skill",
        name: "ielts_writing".into(),
        version: Some("2".into()),
        source: "/home/u/.seekcli/skills/ielts_writing/SKILL.md".into(),
        state: "installed, not activated".into(),
      },
      ExtensionEntry {
        kind: "mcp",
        name: "github".into(),
        version: None,
        source: "npx".into(),
        state: "enabled but not connected".into(),
      },
    ];
    let out = section("extensions", &s);
    assert!(out.contains("ielts_writing"), "{out}");
    assert!(out.contains("v2"), "a recorded version must show: {out}");
    assert!(out.contains("SKILL.md"), "and where it came from: {out}");
    // The distinction that matters for MCP: configured is not connected.
    assert!(out.contains("enabled but not connected"), "{out}");
    assert!(
      out.contains("hot-swapped"),
      "the listing must say what it does NOT do: {out}"
    );
  }

  #[test]
  fn an_empty_extensions_section_says_so() {
    let out = section("extensions", &snap());
    assert!(out.contains("nothing installed"), "{out}");
  }

  /// A delegation that quietly hit its iteration cap used to be
  /// indistinguishable, in the record, from one that finished — only the
  /// summary text came back.
  #[test]
  fn the_session_section_shows_how_each_delegation_ended() {
    let mut s = snap();
    s.child_runs = vec![
      ChildRunEntry {
        call_id: "call_1".into(),
        template: "explore".into(),
        status: "completed".into(),
        iterations: 3,
        duration_ms: 1200,
      },
      ChildRunEntry {
        call_id: "call_2".into(),
        template: "general".into(),
        status: "max_iterations".into(),
        iterations: 20,
        duration_ms: 45000,
      },
    ];
    let out = section("session", &s);
    assert!(
      out.contains("explore — completed after 3 iteration(s)"),
      "{out}"
    );
    assert!(
      out.contains("general — max_iterations after 20 iteration(s)"),
      "an exhausted delegation must not read as a finished one: {out}"
    );
    assert!(out.contains("45000ms"), "{out}");
    assert!(
      out.contains("call_2"),
      "it must join back to the call: {out}"
    );
  }

  /// A session with no delegations must not grow an empty heading.
  #[test]
  fn the_session_section_omits_delegations_when_there_were_none() {
    let out = section("session", &snap());
    assert!(!out.contains("sub-agent runs"), "{out}");
  }

  /// A memory the user cannot see how to remove is a rule they did not agree
  /// to. The section names the directory for exactly that reason.
  #[test]
  fn the_memory_section_says_where_the_files_are_and_which_scope_is_gated() {
    let mut s = snap();
    s.memory_scopes = vec![
      crate::memory::ScopeSummary {
        name: "ielts".into(),
        entries: 4,
      },
      crate::memory::ScopeSummary {
        name: crate::memory::PREFERENCES.into(),
        entries: 1,
      },
    ];
    let out = section("memory", &s);
    assert!(out.contains("/tmp/seekcli-test-memory"), "{out}");
    assert!(out.contains("ielts (4 entries)"), "{out}");
    assert!(
      out.contains("only the proposal gate writes here"),
      "the gated scope must be marked: {out}"
    );
    assert!(out.contains("edit or delete any line by hand"), "{out}");
  }

  #[test]
  fn an_unknown_section_names_the_valid_ones() {
    // Same rule as the rest of the harness: a refusal has to be actionable.
    let err = match render(&json!({"what": "everything"}), &snap()) {
      Ok(o) => panic!("expected an error, got {o}"),
      Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("policy"), "{err}");
    assert!(err.contains("session"), "{err}");
  }

  /// The whole point of §4.5.2: this section must be the same data the gate
  /// enforces, so editing the constant changes the report.
  #[test]
  fn the_policy_section_is_rendered_from_the_gates_own_constants() {
    let mut s = snap();
    s.mode = policy::Mode::ReadOnly.name();
    let out = match render(&json!({"what": "policy"}), &s) {
      Ok(o) => o,
      Err(e) => panic!("{e}"),
    };
    for cmd in policy::read_only_commands() {
      assert!(out.contains(cmd), "`{cmd}` missing from:\n{out}");
    }
    // And the sub-command carve-outs, which are the non-obvious half.
    assert!(out.contains("push"), "{out}");
  }

  #[test]
  fn an_unavailable_server_is_reported_with_its_reason() {
    let out = match render(&json!({"what": "mcp"}), &snap()) {
      Ok(o) => o,
      Err(e) => panic!("{e}"),
    };
    assert!(out.contains("db"), "{out}");
    assert!(out.contains("dbmcp"), "must carry the reason: {out}");
    assert!(out.contains("github"), "{out}");
  }

  #[test]
  fn a_pending_proposal_is_visible_so_it_is_not_drafted_twice() {
    let out = match render(&json!({"what": "skills"}), &snap()) {
      Ok(o) => o,
      Err(e) => panic!("{e}"),
    };
    assert!(out.contains("draft-helper"), "{out}");
    assert!(
      out.contains("none"),
      "active skill must read as none: {out}"
    );
  }
}
