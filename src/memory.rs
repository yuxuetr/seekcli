//! Persistent notes that outlive a conversation.
//!
//! `design-principles.md` §2 excluded "cross-session semantic memory", and for
//! a coding CLI that was right — the event log plus a workspace `PLAN.md`
//! covers a task that starts and ends in an afternoon. Studying for IELTS,
//! working through CQF, or tracking a position runs for months, and `PLAN.md`
//! lives in a working directory those conversations do not have. The exclusion
//! was written against a premise the user's actual scenarios changed (stage 45;
//! see `TODOs.md`).
//!
//! What it is deliberately not: a vector store. Everything here is plain
//! Markdown the user can open and edit, one file per scope. Retrieval is "read
//! the file for the topic you are on", which for a personal tool is both
//! sufficient and inspectable. Embeddings would buy fuzzy recall at the cost of
//! a corpus nobody can read, review, or correct by hand.
//!
//! # Two tiers, and why the split is structural
//!
//! | Tier | Where | Written by |
//! |---|---|---|
//! | Domain state | `memory/<scope>.md` | the agent, directly |
//! | Personal preferences | `memory/preferences.md` | **only through the proposal gate** |
//!
//! Domain state is factual and narrow: "target band 7.0", "weak on Task 2
//! argument development". Wrong entries are correctable and affect one topic.
//!
//! Preferences are rules *about the user* that shape every future
//! conversation. A model that has just failed at something is exactly the model
//! most likely to write "the user does not understand conditional
//! expectation", and that sentence would then arrive in every session
//! afterwards. So the requirement "a reflection written after one failure must
//! not auto-promote into a permanent rule" is enforced by routing, not
//! requested in a prompt: `write` to `preferences` produces a proposal, and
//! only `/propose accept memory <name>` lands it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The scope whose entries are rules about the user rather than facts about a
/// topic. Gated; see the module docs.
pub const PREFERENCES: &str = "preferences";

/// Header written into a new scope file.
///
/// It states the fourth thing every persistent memory needs, alongside source,
/// scope and time: **how to undo it**. A note the user cannot see how to remove
/// is not a note, it is a rule they did not agree to.
fn header(scope: &str) -> String {
  format!(
    "# {scope}\n\n\
     <!-- SeekCLI memory. Plain Markdown: edit or delete any line by hand.\n\
     \x20    Each entry carries its source and the date it was written.\n\
     \x20    To forget one: delete its line here, or ask the agent to. -->\n\n"
  )
}

pub struct MemoryStore {
  dir: PathBuf,
}

/// One scope and how much is in it — the cheap index the prompt carries, so
/// the model knows what exists without every file's contents being injected.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeSummary {
  pub name: String,
  pub entries: usize,
}

impl MemoryStore {
  pub fn new() -> Result<Self> {
    let home = std::env::var("HOME").context("Could not find HOME directory")?;
    Self::at(PathBuf::from(home).join(".seekcli").join("memory"))
  }

  pub fn at(dir: PathBuf) -> Result<Self> {
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    Ok(Self { dir })
  }

  /// Reject anything that could escape the memory directory or collide with a
  /// neighbouring file. Scope names come from the model, so they are input.
  fn path_of(&self, scope: &str) -> Result<PathBuf> {
    let clean = scope.trim();
    if clean.is_empty() {
      anyhow::bail!("scope must not be empty");
    }
    if !clean
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
      anyhow::bail!(
        "scope `{clean}` must be ASCII letters, digits, `-` or `_` \
         (it becomes a filename)"
      );
    }
    Ok(self.dir.join(format!("{clean}.md")))
  }

  /// Every scope that exists, with its entry count. Cheap: counts bullets,
  /// reads no more than it must.
  pub fn scopes(&self) -> Vec<ScopeSummary> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&self.dir) else {
      return out;
    };
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().and_then(|e| e.to_str()) != Some("md") {
        continue;
      }
      let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
        continue;
      };
      let body = fs::read_to_string(&path).unwrap_or_default();
      out.push(ScopeSummary {
        name: name.to_string(),
        entries: count_entries(&body),
      });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
  }

  pub fn read(&self, scope: &str) -> Result<String> {
    let path = self.path_of(scope)?;
    match fs::read_to_string(&path) {
      Ok(text) => Ok(text),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
        // Not an error: "nothing remembered yet" is a real and useful answer,
        // and failing here would push the model into inventing a reason.
        Ok(format!(
          "(no memory for scope `{scope}` yet — nothing has been recorded)"
        ))
      }
      Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
  }

  /// Append one entry to a domain scope.
  ///
  /// Refuses `preferences` outright rather than silently redirecting: the
  /// caller needs to know its write became a proposal, or it will report the
  /// preference as saved.
  pub fn append(&self, scope: &str, entry: &str, source: &str) -> Result<String> {
    if scope.trim() == PREFERENCES {
      anyhow::bail!(
        "`{PREFERENCES}` is gated: it holds rules about the user that apply to \
         every future conversation. Propose it instead, and the user decides."
      );
    }
    let entry = entry.trim();
    if entry.is_empty() {
      anyhow::bail!("entry must not be empty");
    }
    let path = self.path_of(scope)?;
    let existing = fs::read_to_string(&path).unwrap_or_else(|_| header(scope));
    if existing.contains(entry) {
      return Ok(format!("`{scope}` already records that; nothing appended"));
    }
    let line = format!(
      "- {entry} <!-- {} · {} -->\n",
      if source.trim().is_empty() {
        "agent"
      } else {
        source.trim()
      },
      today()
    );
    let mut out = existing;
    if !out.ends_with('\n') {
      out.push('\n');
    }
    out.push_str(&line);
    fs::write(&path, out).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(format!("recorded in `{scope}`: {entry}"))
  }

  /// Remove entries containing `needle`.
  ///
  /// Ambiguity is refused rather than resolved, the same rule `edit_file`
  /// follows: deleting the wrong memory is silent and the user finds out much
  /// later, when an answer quietly stops accounting for something.
  pub fn forget(&self, scope: &str, needle: &str) -> Result<String> {
    let needle = needle.trim();
    if needle.is_empty() {
      anyhow::bail!("give the text to forget");
    }
    let path = self.path_of(scope)?;
    let text = fs::read_to_string(&path)
      .with_context(|| format!("no memory for scope `{scope}` to forget from"))?;
    let hits: Vec<&str> = text
      .lines()
      .filter(|l| l.trim_start().starts_with("- ") && l.contains(needle))
      .collect();
    match hits.len() {
      0 => anyhow::bail!("nothing in `{scope}` contains `{needle}`"),
      1 => {}
      n => anyhow::bail!(
        "`{needle}` matches {n} entries in `{scope}`; be more specific:\n{}",
        hits.join("\n")
      ),
    }
    let removed = hits.join("\n");
    let kept: Vec<&str> = text
      .lines()
      .filter(|l| !(l.trim_start().starts_with("- ") && l.contains(needle)))
      .collect();
    let mut out = kept.join("\n");
    out.push('\n');
    fs::write(&path, out).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(format!("forgot from `{scope}`:\n{removed}"))
  }

  /// The preferences body, for injection. Empty when there are none.
  pub fn preferences(&self) -> String {
    self
      .path_of(PREFERENCES)
      .ok()
      .and_then(|p| fs::read_to_string(p).ok())
      .map(|t| entry_lines(&t).join("\n"))
      .unwrap_or_default()
  }

  /// Land an accepted preference proposal.
  pub fn accept_preference(&self, entry: &str) -> Result<String> {
    let path = self.path_of(PREFERENCES)?;
    let existing = fs::read_to_string(&path).unwrap_or_else(|_| header(PREFERENCES));
    let mut out = existing;
    if !out.ends_with('\n') {
      out.push('\n');
    }
    out.push_str(&format!(
      "- {} <!-- accepted by the user · {} -->\n",
      entry.trim(),
      today()
    ));
    fs::write(&path, out).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(format!("preference recorded: {}", entry.trim()))
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }
}

fn today() -> String {
  chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn entry_lines(text: &str) -> Vec<&str> {
  text
    .lines()
    .map(str::trim_end)
    .filter(|l| l.trim_start().starts_with("- "))
    .collect()
}

fn count_entries(text: &str) -> usize {
  entry_lines(text).len()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn store(name: &str) -> MemoryStore {
    let dir = std::env::temp_dir().join(format!("seekcli-memory-{name}"));
    let _ = fs::remove_dir_all(&dir);
    match MemoryStore::at(dir) {
      Ok(s) => s,
      Err(e) => panic!("cannot build store: {e}"),
    }
  }

  /// The structural half of "a reflection must not become a permanent rule".
  /// Stated in a prompt it is a request; refused here it is a fact.
  #[test]
  fn preferences_cannot_be_written_directly() {
    let s = store("gated");
    let err = match s.append(PREFERENCES, "the user is bad at maths", "agent") {
      Err(e) => format!("{e:#}"),
      Ok(v) => panic!("a direct preference write must be refused, got {v}"),
    };
    assert!(err.contains("gated"), "{err}");
    assert!(
      err.contains("Propose"),
      "the refusal must name the way through: {err}"
    );
  }

  /// And domain state must stay freely writable, or the agent cannot record
  /// the ordinary progress this whole module exists for.
  #[test]
  fn domain_state_is_written_directly() {
    let s = store("domain");
    match s.append("ielts", "target band 7.0, exam 2026-12", "user") {
      Ok(msg) => assert!(msg.contains("ielts"), "{msg}"),
      Err(e) => panic!("domain write failed: {e:#}"),
    }
    let body = s.read("ielts").unwrap_or_default();
    assert!(body.contains("target band 7.0"), "{body}");
    // Source and date travel with the entry.
    assert!(body.contains("user ·"), "{body}");
    // And the file says how to undo it.
    assert!(body.contains("delete its line"), "{body}");
  }

  #[test]
  fn an_unknown_scope_reads_as_empty_rather_than_failing() {
    let s = store("empty");
    let out = s.read("nothing").unwrap_or_default();
    assert!(out.contains("nothing has been recorded"), "{out}");
  }

  /// Scope names come from the model, so they are input, not identifiers.
  #[test]
  fn a_scope_name_cannot_escape_the_memory_directory() {
    let s = store("escape");
    for hostile in ["../../etc/passwd", "a/b", "..", "with space"] {
      assert!(
        s.append(hostile, "x", "t").is_err(),
        "`{hostile}` must be refused"
      );
    }
  }

  /// Deleting the wrong memory is silent, and the user only finds out much
  /// later when an answer quietly stops accounting for something.
  #[test]
  fn an_ambiguous_forget_is_refused_rather_than_guessed() {
    let s = store("ambiguous");
    let _ = s.append("cqf", "weak on stochastic calculus", "t");
    let _ = s.append("cqf", "weak on numerical methods", "t");
    let err = match s.forget("cqf", "weak on") {
      Err(e) => format!("{e:#}"),
      Ok(v) => panic!("expected a refusal, got {v}"),
    };
    assert!(err.contains("matches 2"), "{err}");
    // Nothing was removed.
    let body = s.read("cqf").unwrap_or_default();
    assert!(body.contains("stochastic"), "{body}");
    assert!(body.contains("numerical"), "{body}");
  }

  #[test]
  fn a_unique_forget_removes_exactly_one_entry() {
    let s = store("forget-one");
    let _ = s.append("ielts", "weak on Task 2", "t");
    let _ = s.append("ielts", "target band 7.0", "t");
    match s.forget("ielts", "Task 2") {
      Ok(msg) => assert!(msg.contains("Task 2"), "{msg}"),
      Err(e) => panic!("forget failed: {e:#}"),
    }
    let body = s.read("ielts").unwrap_or_default();
    assert!(!body.contains("Task 2"), "{body}");
    assert!(
      body.contains("target band 7.0"),
      "the rest must survive: {body}"
    );
  }

  #[test]
  fn the_index_counts_entries_not_lines() {
    let s = store("index");
    let _ = s.append("finance", "watching NVDA", "user");
    let _ = s.append("finance", "watching TSM", "user");
    let _ = s.append("ielts", "target band 7.0", "user");
    let scopes = s.scopes();
    assert_eq!(scopes.len(), 2, "{scopes:?}");
    // Header lines and blank lines must not inflate the count.
    assert_eq!(scopes[0].name, "finance");
    assert_eq!(scopes[0].entries, 2, "{scopes:?}");
    assert_eq!(scopes[1].entries, 1, "{scopes:?}");
  }

  #[test]
  fn recording_the_same_thing_twice_does_not_duplicate_it() {
    let s = store("dedupe");
    let _ = s.append("ielts", "target band 7.0", "user");
    let msg = s
      .append("ielts", "target band 7.0", "user")
      .unwrap_or_default();
    assert!(msg.contains("already"), "{msg}");
    assert_eq!(s.scopes().first().map(|x| x.entries), Some(1));
  }

  #[test]
  fn an_accepted_preference_lands_marked_as_the_users_decision() {
    let s = store("accepted");
    match s.accept_preference("answer in Chinese unless asked otherwise") {
      Ok(msg) => assert!(msg.contains("Chinese"), "{msg}"),
      Err(e) => panic!("accept failed: {e:#}"),
    }
    let body = s.read(PREFERENCES).unwrap_or_default();
    assert!(body.contains("accepted by the user"), "{body}");
    assert!(s.preferences().contains("answer in Chinese"), "{body}");
  }
}
