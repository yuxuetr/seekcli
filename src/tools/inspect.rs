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
const SECTIONS: &[&str] = &["tools", "policy", "skills", "mcp", "session", "sources"];

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
  format!(
    "# session\nid: {}\nevents: {}\ncompactions: {}\n\
     compact_at: {} tokens (window {} x {})\nkeep_tail: {} messages\n",
    snap.session_id,
    snap.events,
    snap.compactions,
    snap.memory.threshold_tokens,
    snap.memory.window_tokens,
    snap.memory.ratio,
    snap.memory.keep_tail
  )
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
