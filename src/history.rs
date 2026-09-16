//! Session storage.
//!
//! One directory per session:
//!
//! ```text
//! ~/.seekcli/sessions/<id>/
//! ├── meta.json      title / model / cost / counts   <- /history reads only this
//! ├── events.jsonl   append-only log                 <- the source of truth
//! └── blobs/         offloaded tool output
//! ```
//!
//! Splitting meta from the log is what makes `/history` O(1) per session.
//! The old format parsed every message of every session just to print a list
//! of titles, so listing got linearly slower with use.
//!
//! Legacy `<id>.json` files are migrated on first sight — reversibly, backing
//! the original up rather than deleting it, the same way `/skill migrate`
//! handles skill files.

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::api::Message;
use crate::observability::cost::CostTracker;
use crate::session::{self, EventPayload, PromptKind, Session, SessionMeta};

pub struct HistoryManager {
  pub base_dir: PathBuf,
}

impl HistoryManager {
  /// A store rooted anywhere. Exists so a test `App` writes into a scratch
  /// directory instead of the user's real history — 49 of one machine's 129
  /// sessions turned out to be `cargo test` residue, because `for_test` built
  /// a real `HistoryManager` and anything that saved left a session behind.
  ///
  /// Not `#[cfg(test)]`: `for_test` lives in `main.rs`, so this has to be
  /// reachable from the binary's non-test build of that module.
  pub fn at(sessions_dir: PathBuf) -> Result<Self> {
    fs::create_dir_all(&sessions_dir)
      .with_context(|| format!("cannot create {}", sessions_dir.display()))?;
    Ok(Self {
      base_dir: sessions_dir,
    })
  }

  pub fn new() -> Result<Self> {
    let home = std::env::var("HOME").context("Could not find HOME directory")?;
    let base = PathBuf::from(home).join(".seekcli");
    let sessions_dir = base.join("sessions");
    fs::create_dir_all(&sessions_dir)
      .with_context(|| format!("cannot create {}", sessions_dir.display()))?;
    fs::create_dir_all(base.join("skills"))?;
    let manager = Self {
      base_dir: sessions_dir,
    };
    manager.migrate_legacy();
    manager.sweep_blobs();
    Ok(manager)
  }

  /// Drop offloaded output nobody has touched in a month, including the
  /// pre-0.2 shared `~/.seekcli/tmp` directory, which was never cleaned at all.
  fn sweep_blobs(&self) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);
    let mut removed = crate::tools::offload::sweep(&self.base_dir, MAX_AGE);
    if let Some(root) = self.base_dir.parent() {
      removed += crate::tools::offload::sweep(&root.join("tmp"), MAX_AGE);
    }
    // Background job logs grow the same way and are equally disposable once
    // the conversation that produced them is long gone.
    removed += crate::tools::jobs::sweep_logs(MAX_AGE);
    if removed > 0 {
      eprintln!(
        "[Session] removed {} stale offload / job file(s) older than 30 days",
        removed
      );
    }
  }

  pub fn session_dir(&self, id: &str) -> PathBuf {
    self.base_dir.join(id)
  }

  pub fn blobs_dir(&self, id: &str) -> PathBuf {
    self.session_dir(id).join("blobs")
  }

  pub fn create_session(&self, model: String) -> Session {
    Session::new(Uuid::new_v4().to_string(), model)
  }

  /// Append everything new, then refresh `meta.json`.
  ///
  /// The order matters and is the opposite of the obvious one. `meta.json` is
  /// a summary of the log; writing it first would, on a crash in between,
  /// leave a session claiming events its log does not contain. Appending
  /// first can only ever leave the summary *behind* the log, which the next
  /// load corrects for free.
  ///
  /// This used to rewrite `events.jsonl` wholesale on every call — an O(n)
  /// write per turn on a file the module header calls append-only, and a
  /// window in which a crash could destroy the whole session rather than the
  /// last record.
  pub fn save_session(&self, session: &mut Session) -> Result<()> {
    let dir = self.session_dir(session.id());
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;

    let pending = session.unpersisted();
    if !pending.is_empty() {
      let body = session::to_jsonl(pending)?;
      let path = dir.join("events.jsonl");
      let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot open {} for append", path.display()))?;
      file
        .write_all(body.as_bytes())
        .with_context(|| format!("cannot append to {}", path.display()))?;
      // Without this the events are in the page cache but not necessarily on
      // the disk, and "recorded before the tool ran" would be a claim the
      // ordering cannot actually back after a power loss.
      file
        .sync_data()
        .with_context(|| format!("cannot flush {}", path.display()))?;
      session.mark_persisted();
    }

    fs::write(
      dir.join("meta.json"),
      serde_json::to_string_pretty(&session.meta)?,
    )?;
    Ok(())
  }

  /// The session touched most recently, if any.
  ///
  /// "Pick up where I left off" is the common case, and making the user run
  /// `/history`, read a hex id and type it back is friction for the thing they
  /// do most. Reads `meta.json` only, like `list_sessions`.
  pub fn latest(&self) -> Option<SessionMeta> {
    self.list_sessions().ok()?.into_iter().next()
  }

  /// Session summaries, newest first. Reads `meta.json` only.
  pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
    let mut out = Vec::new();
    if !self.base_dir.exists() {
      return Ok(out);
    }
    for entry in fs::read_dir(&self.base_dir)? {
      let entry = entry?;
      let meta_path = entry.path().join("meta.json");
      if !meta_path.exists() {
        continue;
      }
      let Ok(text) = fs::read_to_string(&meta_path) else {
        continue;
      };
      // A session whose meta cannot be parsed is skipped, not fatal: one bad
      // directory must not make `/history` unusable.
      if let Ok(meta) = serde_json::from_str::<SessionMeta>(&text) {
        out.push(meta);
      }
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.updated));
    Ok(out)
  }

  /// Load by full id or unique id prefix.
  pub fn load_session(&self, id: &str) -> Result<Session> {
    let dir = self.resolve_id(id)?;
    let meta: SessionMeta = serde_json::from_str(
      &fs::read_to_string(dir.join("meta.json"))
        .with_context(|| format!("cannot read {}", dir.join("meta.json").display()))?,
    )?;
    let events = session::from_jsonl(&fs::read_to_string(dir.join("events.jsonl"))?);
    // `meta.event_count` can legitimately lag the log: `save_session` appends
    // events before refreshing the summary, so a crash in between leaves the
    // log ahead. The log is the source of truth, so re-derive from it.
    let mut meta = meta;
    meta.event_count = events.len();
    Ok(Session::restored(meta, events))
  }

  fn resolve_id(&self, id: &str) -> Result<PathBuf> {
    let exact = self.session_dir(id);
    if exact.join("meta.json").exists() {
      return Ok(exact);
    }
    let mut hits: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&self.base_dir)? {
      let entry = entry?;
      let name = entry.file_name().to_string_lossy().to_string();
      if name.starts_with(id) && entry.path().join("meta.json").exists() {
        hits.push(entry.path());
      }
    }
    match hits.len() {
      1 => Ok(hits.remove(0)),
      0 => anyhow::bail!("no session matches `{}`", id),
      // Silently picking one would load a conversation the user did not ask
      // for, which is worse than making them type two more characters.
      n => anyhow::bail!("`{}` matches {} sessions; use a longer prefix", id, n),
    }
  }

  /// Full-text search across logs. Returns (meta, matching line) pairs.
  ///
  /// A plain scan rather than an index: at single-machine scale it is fast
  /// enough, and an index would mean a database, a schema and a migration
  /// story for a feature that answers "where did I discuss X".
  pub fn search(&self, needle: &str, limit: usize) -> Result<Vec<(SessionMeta, String)>> {
    let needle_lower = needle.to_lowercase();
    let mut out = Vec::new();
    for meta in self.list_sessions()? {
      let path = self.session_dir(&meta.id).join("events.jsonl");
      let Ok(text) = fs::read_to_string(&path) else {
        continue;
      };
      // Match on the raw line (cheap), but show the message text. Echoing
      // the JSON record back at the user would technically answer "which
      // session" while being unreadable.
      if let Some(line) = text
        .lines()
        .find(|l| l.to_lowercase().contains(&needle_lower))
      {
        out.push((meta, excerpt_of(line)));
        if out.len() >= limit {
          break;
        }
      }
    }
    Ok(out)
  }

  /// Convert any pre-0.2 `<id>.json` snapshots into directories.
  ///
  /// Best-effort and reversible: the original is renamed to `.json.bak`
  /// rather than removed, so a failed migration loses nothing.
  fn migrate_legacy(&self) {
    let Ok(entries) = fs::read_dir(&self.base_dir) else {
      return;
    };
    let mut migrated = 0usize;
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().and_then(|s| s.to_str()) != Some("json") {
        continue;
      }
      match migrate_one(&path, &self.base_dir) {
        Ok(true) => migrated += 1,
        Ok(false) => {}
        Err(e) => eprintln!("[Session] could not migrate {}: {}", path.display(), e),
      }
    }
    if migrated > 0 {
      eprintln!(
        "[Session] migrated {} session(s) to the event-log format (originals kept as .json.bak)",
        migrated
      );
    }
  }
}

/// Human-readable one-liner for a matched event.
fn excerpt_of(line: &str) -> String {
  let text = match serde_json::from_str::<crate::session::SessionEvent>(line) {
    Ok(event) => match event.payload {
      EventPayload::UserMessage { content, .. } => content,
      EventPayload::AssistantMessage { content, .. } => content,
      EventPayload::ToolResult { content, .. } => content,
      EventPayload::SystemPrompt { content, .. } => content,
      EventPayload::Compaction { summary, .. } => summary,
      other => format!("{:?}", other),
    },
    // Unparsable line: fall back to the raw text rather than hiding the hit.
    Err(_) => line.to_string(),
  };
  let flattened: String = text
    .chars()
    .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
    .collect();
  flattened
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .chars()
    .take(100)
    .collect()
}

/// The pre-0.2 on-disk shape, kept only so old files can be read once.
#[derive(serde::Deserialize)]
struct LegacySession {
  id: String,
  #[serde(default)]
  title: String,
  #[serde(default)]
  messages: Vec<Message>,
  #[serde(default)]
  model: String,
  timestamp: chrono::DateTime<chrono::Utc>,
  #[serde(default)]
  cost: CostTracker,
}

fn migrate_one(path: &Path, base: &Path) -> Result<bool> {
  let text = fs::read_to_string(path)?;
  let legacy: LegacySession = match serde_json::from_str(&text) {
    Ok(v) => v,
    // Not a session file — leave it alone rather than guessing.
    Err(_) => return Ok(false),
  };
  let dir = base.join(&legacy.id);
  if dir.join("meta.json").exists() {
    return Ok(false);
  }

  let mut session = Session::new(legacy.id.clone(), legacy.model.clone());
  for message in &legacy.messages {
    if let Some(payload) = session::event_for(message) {
      // A leading system message in the old format is the prompt kernel.
      let payload = match payload {
        EventPayload::SystemPrompt { content, .. } if session.events.is_empty() => {
          EventPayload::SystemPrompt {
            kind: PromptKind::Kernel,
            content,
          }
        }
        other => other,
      };
      session.record(payload);
    }
  }
  session.meta.title = if legacy.title.is_empty() {
    session::UNTITLED.to_string()
  } else {
    legacy.title
  };
  session.meta.created = legacy.timestamp;
  session.meta.updated = legacy.timestamp;
  session.meta.cost = legacy.cost;

  fs::create_dir_all(&dir)?;
  fs::write(
    dir.join("meta.json"),
    serde_json::to_string_pretty(&session.meta)?,
  )?;
  fs::write(
    dir.join("events.jsonl"),
    session::to_jsonl(&session.events)?,
  )?;
  fs::rename(path, path.with_extension("json.bak"))?;
  Ok(true)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::Message;

  fn store(name: &str) -> HistoryManager {
    let base = std::env::temp_dir()
      .join(format!("seekcli-history-{}", name))
      .join("sessions");
    let _ = fs::remove_dir_all(base.parent().unwrap_or(&base));
    let _ = fs::create_dir_all(&base);
    HistoryManager { base_dir: base }
  }

  #[test]
  fn save_and_load_round_trips_through_the_event_log() {
    let h = store("roundtrip");
    let mut s = h.create_session("m".into());
    s.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "hello".into(),
    });
    s.meta.title = "greeting".into();
    if let Err(e) = h.save_session(&mut s) {
      panic!("save failed: {}", e);
    }

    let back = match h.load_session(s.id()) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert_eq!(back.meta.title, "greeting");
    assert_eq!(back.events.len(), 1);
    assert_eq!(back.messages().len(), 1);
  }

  /// The whole point of the watermark: saving twice must append the new
  /// events, not re-append the old ones. Rewriting the file hid this class of
  /// bug entirely — a wholesale write is idempotent by construction, so
  /// nothing ever had to be right about *which* events got written.
  #[test]
  fn saving_twice_appends_rather_than_duplicating() {
    let h = store("append-twice");
    let mut s = h.create_session("m".into());
    s.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "first".into(),
    });
    let _ = h.save_session(&mut s);
    s.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "second".into(),
    });
    let _ = h.save_session(&mut s);

    let back = match h.load_session(s.id()) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert_eq!(back.events.len(), 2, "events: {:?}", back.events);
    let text = fs::read_to_string(h.session_dir(s.id()).join("events.jsonl")).unwrap_or_default();
    assert_eq!(
      text.lines().filter(|l| l.contains("first")).count(),
      1,
      "the first event must be written exactly once:\n{text}"
    );
  }

  /// A no-op save must not touch the log at all. Otherwise every turn that
  /// records nothing would still rewrite, which is what this replaced.
  #[test]
  fn saving_with_nothing_new_writes_no_events() {
    let h = store("append-noop");
    let mut s = h.create_session("m".into());
    s.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "only".into(),
    });
    let _ = h.save_session(&mut s);
    let path = h.session_dir(s.id()).join("events.jsonl");
    let before = fs::read_to_string(&path).unwrap_or_default();
    let _ = h.save_session(&mut s);
    let after = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(before, after, "a second save appended something");
  }

  /// `save_session` appends events before refreshing `meta.json`, so a crash
  /// in between leaves the summary behind the log. The log is the source of
  /// truth, so the next load must believe the log, not the summary.
  #[test]
  fn a_meta_lagging_behind_the_log_is_corrected_on_load() {
    let h = store("meta-lag");
    let mut s = h.create_session("m".into());
    s.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "a".into(),
    });
    s.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "b".into(),
    });
    let _ = h.save_session(&mut s);

    // Simulate the crash window: meta claims one event, the log holds two.
    let meta_path = h.session_dir(s.id()).join("meta.json");
    let mut stale = s.meta.clone();
    stale.event_count = 1;
    if let Ok(text) = serde_json::to_string_pretty(&stale) {
      let _ = fs::write(&meta_path, text);
    }

    let back = match h.load_session(s.id()) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert_eq!(back.events.len(), 2);
    assert_eq!(
      back.meta.event_count, 2,
      "the summary must be re-derived from the log"
    );
  }

  /// A fork writes a new file, so everything it inherited is unpersisted —
  /// carrying the parent's watermark across would silently produce an empty
  /// log for the child.
  #[test]
  fn a_fork_persists_every_event_it_inherited() {
    let h = store("fork-persist");
    let mut parent = h.create_session("m".into());
    for text in ["one", "two"] {
      parent.record(EventPayload::UserMessage {
        images: Vec::new(),
        content: text.into(),
      });
    }
    let _ = h.save_session(&mut parent);

    let mut child = parent.fork(uuid::Uuid::new_v4().to_string(), 2);
    let _ = h.save_session(&mut child);
    let back = match h.load_session(child.id()) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert_eq!(back.events.len(), 2, "fork wrote an empty log");
  }

  /// `--continue` and a bare `/resume` both mean "the one I was just in", so
  /// this has to agree with what `/history` shows at the top.
  #[test]
  fn latest_is_the_most_recently_updated_session() {
    let h = store("latest");
    for (n, title) in ["older", "newer"].iter().enumerate() {
      let mut s = h.create_session("m".into());
      s.meta.title = (*title).into();
      s.meta.updated = chrono::Utc::now() + chrono::Duration::seconds(n as i64);
      let _ = h.save_session(&mut s);
    }
    assert_eq!(
      h.latest().map(|m| m.title),
      Some("newer".to_string()),
      "latest must match the top of /history"
    );
  }

  /// A fresh install has nothing to continue, and the caller needs to say so
  /// rather than open a blank session that looks resumed.
  #[test]
  fn latest_is_none_when_nothing_is_stored() {
    let h = store("latest-empty");
    assert!(h.latest().is_none());
  }

  #[test]
  fn listing_reads_meta_only_and_sorts_newest_first() {
    let h = store("listing");
    for (n, title) in ["first", "second"].iter().enumerate() {
      let mut s = h.create_session("m".into());
      s.meta.title = (*title).into();
      s.meta.updated = chrono::Utc::now() + chrono::Duration::seconds(n as i64);
      let _ = h.save_session(&mut s);
    }
    let list = match h.list_sessions() {
      Ok(v) => v,
      Err(e) => panic!("list failed: {}", e),
    };
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].title, "second", "newest first");
  }

  #[test]
  fn an_ambiguous_prefix_is_refused_rather_than_guessed() {
    let h = store("ambiguous");
    for suffix in ["aa", "ab"] {
      let mut s = Session::new(format!("dup-{}", suffix), "m".into());
      s.record(EventPayload::UserMessage {
        images: Vec::new(),
        content: "x".into(),
      });
      let _ = h.save_session(&mut s);
    }
    let err = match h.load_session("dup-") {
      Ok(_) => panic!("an ambiguous prefix must not silently pick one"),
      Err(e) => format!("{:#}", e),
    };
    assert!(err.contains("longer prefix"), "got: {}", err);
  }

  #[test]
  fn legacy_snapshots_migrate_reversibly() {
    let h = store("migrate");
    let legacy = serde_json::json!({
      "id": "legacy-one",
      "title": "old chat",
      "model": "m",
      "timestamp": "2026-01-01T00:00:00Z",
      "messages": [
        { "role": "user", "content": "hi" },
        { "role": "assistant", "content": "hello" }
      ]
    });
    let path = h.base_dir.join("legacy-one.json");
    let _ = fs::write(&path, legacy.to_string());

    h.migrate_legacy();

    let migrated = match h.load_session("legacy-one") {
      Ok(v) => v,
      Err(e) => panic!("migration did not produce a loadable session: {}", e),
    };
    assert_eq!(migrated.meta.title, "old chat");
    assert_eq!(migrated.messages().len(), 2);
    // Reversible: the original is kept, not deleted.
    assert!(path.with_extension("json.bak").exists());
    assert!(!path.exists());
  }

  #[test]
  fn migration_is_idempotent_and_ignores_unrelated_json() {
    let h = store("migrate-twice");
    let _ = fs::write(h.base_dir.join("notes.json"), r#"{"unrelated": true}"#);
    h.migrate_legacy();
    h.migrate_legacy();
    // Unrelated files are left exactly where they were.
    assert!(h.base_dir.join("notes.json").exists());
  }

  #[test]
  fn search_finds_the_session_containing_a_phrase() {
    let h = store("search");
    let mut a = Session::new("aaa".into(), "m".into());
    a.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "how do I configure ripgrep".into(),
    });
    let _ = h.save_session(&mut a);
    let mut b = Session::new("bbb".into(), "m".into());
    b.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: "unrelated chatter".into(),
    });
    let _ = h.save_session(&mut b);

    let hits = match h.search("RIPGREP", 10) {
      Ok(v) => v,
      Err(e) => panic!("search failed: {}", e),
    };
    assert_eq!(hits.len(), 1, "case-insensitive, one match");
    assert_eq!(hits[0].0.id, "aaa");
  }

  #[test]
  fn search_excerpts_show_the_message_not_the_json_record() {
    let event = crate::session::SessionEvent {
      seq: 0,
      ts: chrono::Utc::now(),
      payload: EventPayload::UserMessage {
        images: Vec::new(),
        content: "how do I\n  configure   ripgrep".into(),
      },
    };
    let line = match serde_json::to_string(&event) {
      Ok(l) => l,
      Err(e) => panic!("serialise failed: {}", e),
    };
    let excerpt = excerpt_of(&line);
    assert_eq!(excerpt, "how do I configure ripgrep");
    assert!(!excerpt.contains("payload"), "raw JSON leaked: {}", excerpt);
  }

  #[test]
  fn message_conversion_covers_every_role_the_log_stores() {
    let msgs = vec![
      Message::new_user_text("u".into()),
      Message::Simple {
        images: Vec::new(),
        role: "assistant".into(),
        content: "a".into(),
        reasoning_content: None,
        tool_calls: None,
      },
      Message::ToolResponse {
        images: Vec::new(),
        role: "tool".into(),
        content: "t".into(),
        tool_call_id: "c1".into(),
      },
    ];
    for m in &msgs {
      assert!(session::event_for(m).is_some(), "unconverted message");
    }
  }
}
