//! Decision-path tracing.
//!
//! A full agent run is naturally a tree: a Run contains Turns, each Turn
//! contains leaf operations (LLM generate, tool execute, planning, compaction).
//! Recording that tree with timings lets you replay *why* a run went the way it
//! did — the agent equivalent of distributed tracing.
//!
//! Opt-in via the `SEEKCLI_TRACE=1` environment variable. When disabled,
//! `begin`/`end`/`flush` are cheap no-ops so normal runs pay nothing and no
//! files are written. When enabled, each run flushes a JSON span tree to
//! `~/.seekcli/traces/<run_id>.json` — alongside the other SeekCLI products
//! (sessions, skills, offload tmp).
//!
//! Instrumentation lives at the engine boundary (the agent loop), keeping the
//! tracer out of the tools and provider code.

use std::time::Instant;

use serde_json::{Value, json};

/// One node in the trace tree.
struct Span {
  id: usize,
  parent: Option<usize>,
  kind: String,
  name: String,
  start_ms: u128,
  dur_ms: u128,
  meta: Value,
}

/// Per-run span recorder. Held on `App`; `start_run` resets it for each turn.
pub struct Trace {
  enabled: bool,
  run_id: String,
  origin: Option<Instant>,
  /// Added to every timestamp this recorder produces. Non-zero only for a
  /// detached child recorder, so that its spans land at the right place on the
  /// parent's timeline once grafted in.
  offset_ms: u128,
  spans: Vec<Span>,
}

impl Trace {
  /// Construct a tracer; `enabled` is typically `env SEEKCLI_TRACE` being set.
  pub fn new(enabled: bool) -> Self {
    Self {
      enabled,
      run_id: String::new(),
      origin: None,
      offset_ms: 0,
      spans: Vec::new(),
    }
  }

  /// A detached recorder for work that cannot borrow this one mutably.
  ///
  /// `begin` needs `&mut self`, and a sub-agent runs behind `&self` precisely
  /// so several can run at once — so it cannot record into the parent while it
  /// works. It records into one of these instead and the parent grafts the
  /// result in with [`absorb`](Self::absorb), the same shape as
  /// `CostTracker::absorb`: the child keeps its own books and hands them over
  /// when it is done. No lock, and nothing to contend on.
  ///
  /// The clock is pinned here rather than in the child so its spans stay on
  /// *this* recorder's timeline; concurrent children therefore overlap in the
  /// tree, which is the truth about them.
  pub fn child(&self) -> Self {
    Self {
      enabled: self.enabled,
      run_id: String::new(),
      origin: Some(Instant::now()),
      offset_ms: self.elapsed_ms(),
      spans: Vec::new(),
    }
  }

  /// Graft a detached recorder's spans in under `parent`.
  ///
  /// Ids are dense indices assigned by `begin`, so remapping is a single
  /// shift; a child's root spans (`parent == None`) re-parent to `parent`.
  pub fn absorb(&mut self, child: Self, parent: Option<usize>) {
    if !self.enabled {
      return;
    }
    let base = self.spans.len();
    for span in child.spans {
      self.spans.push(Span {
        id: base + span.id,
        parent: span.parent.map(|p| base + p).or(parent),
        ..span
      });
    }
  }

  /// Read the `SEEKCLI_TRACE` env var to decide whether tracing is on.
  pub fn from_env() -> Self {
    Self::new(std::env::var("SEEKCLI_TRACE").is_ok())
  }

  /// Begin a new run: assign a fresh id, clear prior spans, start the clock.
  /// Returns the root span id (or `None` when disabled).
  pub fn start_run(&mut self) -> Option<usize> {
    if !self.enabled {
      return None;
    }
    self.run_id = uuid::Uuid::new_v4().to_string();
    self.origin = Some(Instant::now());
    self.spans.clear();
    self.begin("run", "chat", None)
  }

  /// Open a span of `kind`/`name` under `parent`. Returns the span id, or
  /// `None` when disabled (so callers can pass it straight back to `end`).
  pub fn begin(&mut self, kind: &str, name: &str, parent: Option<usize>) -> Option<usize> {
    if !self.enabled {
      return None;
    }
    let id = self.spans.len();
    let start_ms = self.elapsed_ms();
    self.spans.push(Span {
      id,
      parent,
      kind: kind.to_string(),
      name: name.to_string(),
      start_ms,
      dur_ms: 0,
      meta: Value::Null,
    });
    Some(id)
  }

  /// Close a span, recording its duration.
  pub fn end(&mut self, span: Option<usize>) {
    if let Some(id) = span
      && let Some(start) = self.spans.get(id).map(|s| s.start_ms)
    {
      let dur = self.elapsed_ms().saturating_sub(start);
      if let Some(s) = self.spans.get_mut(id) {
        s.dur_ms = dur;
      }
    }
  }

  /// Attach structured metadata to a span (e.g. tool names, token counts).
  pub fn annotate(&mut self, span: Option<usize>, meta: Value) {
    if let Some(id) = span
      && let Some(s) = self.spans.get_mut(id)
    {
      s.meta = meta;
    }
  }

  fn elapsed_ms(&self) -> u128 {
    self.offset_ms + self.origin.map(|o| o.elapsed().as_millis()).unwrap_or(0)
  }

  /// Write the span tree to `~/.seekcli/traces/<run_id>.json`. Best-effort: a
  /// write failure is reported but never propagated. No-op when disabled or
  /// empty. The originating workspace is recorded in the doc, since traces from
  /// every project share one global directory.
  pub fn flush(&self) -> std::io::Result<Option<std::path::PathBuf>> {
    if !self.enabled || self.spans.is_empty() {
      return Ok(None);
    }
    let home = std::env::var("HOME")
      .map_err(|_| std::io::Error::other("HOME not set; cannot locate ~/.seekcli/traces"))?;
    let dir = std::path::PathBuf::from(home)
      .join(".seekcli")
      .join("traces");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", self.run_id));
    let cwd = std::env::current_dir()
      .map(|p| p.display().to_string())
      .unwrap_or_default();
    let doc = json!({
      "run_id": self.run_id,
      "workspace": cwd,
      "total_ms": self.spans.first().map(|s| s.dur_ms).unwrap_or(0),
      "tree": self.tree_for(None),
    });
    std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
    Ok(Some(path))
  }

  /// Locate a written trace: the newest, or the one whose run id starts with
  /// `prefix`.
  ///
  /// Newest is by **modification time**, not by name — run ids are v4 UUIDs,
  /// so sorting them lexically returns an arbitrary run while looking
  /// deliberate.
  fn locate(dir: &std::path::Path, prefix: Option<&str>) -> Option<std::path::PathBuf> {
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
      let path = entry.path();
      if path.extension().and_then(|e| e.to_str()) != Some("json") {
        continue;
      }
      let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
      if let Some(p) = prefix
        && !stem.starts_with(p)
      {
        continue;
      }
      let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
      if best.as_ref().is_none_or(|(t, _)| modified > *t) {
        best = Some((modified, path));
      }
    }
    best.map(|(_, p)| p)
  }

  /// Render a written trace as an indented tree.
  ///
  /// Exists because reading the raw JSON does not work: answering "was that
  /// capped sub-agent a doom loop" means comparing fifteen `execute` spans'
  /// tool lists at a glance, and in pretty-printed JSON those are hundreds of
  /// lines apart. This was hand-written as a throwaway script three times in
  /// one day before it earned a place here.
  pub fn show(prefix: Option<&str>) -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let dir = std::path::PathBuf::from(home)
      .join(".seekcli")
      .join("traces");
    let Some(path) = Self::locate(&dir, prefix) else {
      return Err(match prefix {
        Some(p) => format!(
          "no trace whose run id starts with `{p}` in {}",
          dir.display()
        ),
        // The likeliest reason by far, so say what to do rather than just
        // reporting an empty directory.
        None => format!(
          "no traces in {}. Tracing is opt-in and cannot be turned on after \
           the fact — re-run with `SEEKCLI_TRACE=1`.",
          dir.display()
        ),
      });
    };
    let text =
      std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc: Value = serde_json::from_str(&text)
      .map_err(|e| format!("{} is not valid trace JSON: {e}", path.display()))?;
    Ok(Self::render(&doc))
  }

  /// Pure renderer, so the layout is testable without touching the filesystem.
  pub fn render(doc: &Value) -> String {
    let mut out = format!(
      "run {}  {}ms  {}\n",
      doc["run_id"].as_str().unwrap_or("?"),
      doc["total_ms"].as_u64().unwrap_or(0),
      doc["workspace"].as_str().unwrap_or("")
    );
    render_nodes(&doc["tree"], 0, &mut out);
    out
  }

  /// Recursively build the JSON subtree for the given parent.
  fn tree_for(&self, parent: Option<usize>) -> Value {
    let children: Vec<Value> = self
      .spans
      .iter()
      .filter(|s| s.parent == parent)
      .map(|s| {
        json!({
          "kind": s.kind,
          "name": s.name,
          "start_ms": s.start_ms,
          "dur_ms": s.dur_ms,
          "meta": s.meta,
          "children": self.tree_for(Some(s.id)),
        })
      })
      .collect();
    Value::Array(children)
  }
}

/// One line per span: `kind`, name, `start+duration`, and the few meta fields
/// that carry the answer. Meta is rendered selectively rather than dumped —
/// the whole reason this exists is that the full JSON hides the signal.
fn render_nodes(nodes: &Value, depth: usize, out: &mut String) {
  let Some(items) = nodes.as_array() else {
    return;
  };
  for n in items {
    let meta = &n["meta"];
    let note = if let Some(tools) = meta["tools"].as_array() {
      let names: Vec<&str> = tools.iter().filter_map(|t| t.as_str()).collect();
      format!("[{}]", names.join(", "))
    } else if let Some(status) = meta["status"].as_str() {
      format!(
        "{status} after {} iteration(s)",
        meta["iterations"].as_u64().unwrap_or(0)
      )
    } else if let Some(calls) = meta["tool_calls"].as_u64() {
      format!("{calls} call(s)")
    } else {
      String::new()
    };
    out.push_str(&format!(
      "{:indent$}{:<9} {:<26} {:>6}+{:<6} {}\n",
      "",
      n["kind"].as_str().unwrap_or("?"),
      n["name"].as_str().unwrap_or(""),
      n["start_ms"].as_u64().unwrap_or(0),
      n["dur_ms"].as_u64().unwrap_or(0),
      note,
      indent = depth * 2,
    ));
    render_nodes(&n["children"], depth + 1, out);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn disabled_is_noop() {
    let mut t = Trace::new(false);
    assert!(t.start_run().is_none());
    let s = t.begin("turn", "x", None);
    assert!(s.is_none());
    t.end(s);
    assert!(matches!(t.flush(), Ok(None)));
  }

  #[test]
  fn builds_nested_tree() {
    let mut t = Trace::new(true);
    let run = t.start_run();
    let turn = t.begin("turn", "iter 0", run);
    let generate = t.begin("generate", "llm", turn);
    t.end(generate);
    t.end(turn);
    t.end(run);

    let tree = t.tree_for(None);
    // One run at the root.
    let arr = tree.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["kind"], "run");
    // Run has one turn child; turn has one generate child.
    let turns = arr[0]["children"].as_array().expect("turns");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["kind"], "turn");
    let leaves = turns[0]["children"].as_array().expect("leaves");
    assert_eq!(leaves[0]["kind"], "generate");
  }

  /// A detached recorder's spans must keep their own nesting *and* land under
  /// the node the parent grafts them at. Getting the id remap wrong here is
  /// silent: the tree still renders, just with children hanging off the wrong
  /// parent — or off the root, which reads as "the sub-agent was the run".
  #[test]
  fn an_absorbed_child_keeps_its_shape_under_the_graft_point() {
    let mut parent = Trace::new(true);
    let run = parent.start_run();
    let execute = parent.begin("execute", "1 tool(s)", run);

    let mut child = parent.child();
    let sub = child.begin("subagent", "explore", None);
    let turn = child.begin("turn", "iter 0", sub);
    child.end(turn);
    child.end(sub);

    parent.absorb(child, execute);
    parent.end(execute);
    parent.end(run);

    let roots = parent.tree_for(None);
    let roots = roots.as_array().expect("array");
    assert_eq!(
      roots.len(),
      1,
      "the child must not surface as a second root"
    );
    let executes = roots[0]["children"].as_array().expect("executes");
    let subs = executes[0]["children"].as_array().expect("subagents");
    assert_eq!(subs[0]["kind"], "subagent");
    assert_eq!(subs[0]["name"], "explore");
    let turns = subs[0]["children"].as_array().expect("turns");
    assert_eq!(turns.len(), 1, "the child's own nesting must survive");
    assert_eq!(turns[0]["kind"], "turn");
  }

  /// Two children grafted in must not collide. Ids are dense indices, so the
  /// second absorb has to shift past the first — otherwise its spans re-parent
  /// onto the first child's.
  #[test]
  fn two_absorbed_children_stay_separate() {
    let mut parent = Trace::new(true);
    let run = parent.start_run();
    for name in ["a", "b"] {
      let mut child = parent.child();
      let sub = child.begin("subagent", name, None);
      let turn = child.begin("turn", "iter 0", sub);
      child.end(turn);
      child.end(sub);
      parent.absorb(child, run);
    }
    let roots = parent.tree_for(None);
    let subs = roots.as_array().expect("array")[0]["children"]
      .as_array()
      .expect("subagents")
      .clone();
    assert_eq!(subs.len(), 2);
    for (span, expected) in subs.iter().zip(["a", "b"]) {
      assert_eq!(span["name"], expected);
      assert_eq!(
        span["children"].as_array().map(Vec::len),
        Some(1),
        "`{expected}` lost or gained a turn"
      );
    }
  }

  /// Tracing off means a child records nothing and absorbing it is free — the
  /// path a normal run takes, where paying for observability would be a bug.
  #[test]
  fn a_disabled_child_absorbs_to_nothing() {
    let mut parent = Trace::new(false);
    let mut child = parent.child();
    assert!(child.begin("subagent", "explore", None).is_none());
    parent.absorb(child, None);
    assert!(parent.spans.is_empty());
  }

  /// The renderer must surface the thing the raw JSON buries: which tools each
  /// turn called, on adjacent lines, so a repeated trajectory is visible by
  /// eye. That is the whole reason it exists.
  #[test]
  fn rendering_puts_each_turns_tools_on_its_own_line() {
    let mut t = Trace::new(true);
    let run = t.start_run();
    let execute = t.begin("execute", "1 tool(s)", run);
    let mut child = t.child();
    let sub = child.begin("subagent", "explore", None);
    for i in 0..2 {
      let turn = child.begin("turn", &format!("iter {i}"), sub);
      let ex = child.begin("execute", "1 tool(s)", turn);
      child.annotate(ex, json!({ "tools": ["grep"] }));
      child.end(ex);
      child.end(turn);
    }
    child.annotate(sub, json!({ "status": "max_iterations", "iterations": 2 }));
    child.end(sub);
    t.absorb(child, execute);
    t.end(execute);
    t.end(run);

    let doc = json!({
      "run_id": "abc", "workspace": "/w",
      "total_ms": 1, "tree": t.tree_for(None),
    });
    let text = Trace::render(&doc);

    assert!(text.starts_with("run abc"), "header missing:\n{text}");
    assert!(
      text.contains("max_iterations after 2 iteration(s)"),
      "a sub-agent's outcome must be on its own line:\n{text}"
    );
    assert_eq!(
      text.matches("[grep]").count(),
      2,
      "each turn's tools must be rendered separately, so a repeat is visible \
       by eye:\n{text}"
    );
    // Nesting has to survive into the text, or "which agent did this" is lost.
    let sub_line = text
      .lines()
      .find(|l| l.contains("subagent"))
      .unwrap_or_default();
    let turn_line = text
      .lines()
      .find(|l| l.contains("iter 0"))
      .unwrap_or_default();
    assert!(
      turn_line.len() - turn_line.trim_start().len() > sub_line.len() - sub_line.trim_start().len(),
      "a turn must be indented under its sub-agent:\n{text}"
    );
  }

  #[test]
  fn annotate_records_meta() {
    let mut t = Trace::new(true);
    let run = t.start_run();
    let ex = t.begin("execute", "tools", run);
    t.annotate(ex, json!({ "tools": ["read_file"] }));
    let tree = t.tree_for(Some(run.unwrap()));
    let arr = tree.as_array().expect("array");
    assert_eq!(arr[0]["meta"]["tools"][0], "read_file");
  }
}
