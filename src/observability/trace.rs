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
  spans: Vec<Span>,
}

impl Trace {
  /// Construct a tracer; `enabled` is typically `env SEEKCLI_TRACE` being set.
  pub fn new(enabled: bool) -> Self {
    Self {
      enabled,
      run_id: String::new(),
      origin: None,
      spans: Vec::new(),
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
    self.origin.map(|o| o.elapsed().as_millis()).unwrap_or(0)
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
