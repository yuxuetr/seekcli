//! The human gate every self-authored change passes through.
//!
//! The model can draft a change; only the user can land it. That split is the
//! sixth of the seven self-evolution conditions — selection pressure — and the
//! three-evaluation judged it one of the two places SeekCLI is ahead of
//! deepseek-harness, whose four persistence tiers have no promotion gate at all
//! (`docs/evaluation/2026-09-12-self-evolution-baseline.md`).
//!
//! What was missing is that only one kind of asset could pass through it. Now:
//!
//! ```text
//! ~/.seekcli/proposals/skill/<name>/SKILL.md   drafted by `create_skill`
//! ~/.seekcli/proposals/mcp/<name>.toml         drafted by `propose`
//! ~/.seekcli/proposals/task/<name>.md          drafted by `propose`
//! ```
//!
//! **Accepting a bad proposal costs far more than rejecting a good one**, so
//! every kind must be mechanically checkable before it lands — see
//! [`Pending::validate`]. Design: `docs/architecture/L5-composition.md` §4.3.

use std::fmt;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::McpServerConfig;

/// What a proposal would become once accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
  /// A skill: drafted by `create_skill`, which owns its richer schema.
  Skill,
  /// An MCP server, i.e. a new tool surface reached through configuration.
  Mcp,
  /// A declarative scheduled task.
  Task,
}

impl Kind {
  pub const ALL: &'static [Kind] = &[Kind::Skill, Kind::Mcp, Kind::Task];

  /// The kinds `propose` accepts. `Skill` is absent on purpose: flattening its
  /// structured fields into one `content` string would force the model to
  /// hand-write SKILL.md frontmatter, which `create_skill` already does better.
  pub const DRAFTABLE: &'static [Kind] = &[Kind::Mcp, Kind::Task];

  pub fn as_str(self) -> &'static str {
    match self {
      Kind::Skill => "skill",
      Kind::Mcp => "mcp",
      Kind::Task => "task",
    }
  }

  pub fn parse(s: &str) -> Option<Self> {
    Kind::ALL.iter().copied().find(|k| k.as_str() == s)
  }

  fn names(kinds: &[Kind]) -> String {
    kinds
      .iter()
      .map(|k| k.as_str())
      .collect::<Vec<_>>()
      .join(", ")
  }
}

impl fmt::Display for Kind {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.as_str())
  }
}

/// A proposal awaiting the user's decision.
pub struct Pending {
  pub kind: Kind,
  pub name: String,
  pub path: PathBuf,
}

impl Pending {
  /// Whether this proposal can land, and why not when it cannot.
  ///
  /// Runs before anything is moved or appended. A proposal that parses only
  /// halfway through landing would leave the user with a broken config and no
  /// obvious way back.
  pub fn validate(&self) -> Result<()> {
    match self.kind {
      Kind::Skill => {
        let md = self.path.join("SKILL.md");
        let text =
          fs::read_to_string(&md).with_context(|| format!("cannot read {}", md.display()))?;
        crate::skills::split_frontmatter(&text, "SKILL.md").map(|_| ())
      }
      Kind::Mcp => {
        let text = fs::read_to_string(&self.path)
          .with_context(|| format!("cannot read {}", self.path.display()))?;
        parse_mcp(&self.name, &text).map(|_| ())
      }
      Kind::Task => {
        let text = fs::read_to_string(&self.path)
          .with_context(|| format!("cannot read {}", self.path.display()))?;
        let (_, body) = crate::skills::split_frontmatter(&text, "TASK.md")?;
        if body.trim().is_empty() {
          anyhow::bail!("the body is empty, and the body is the prompt");
        }
        Ok(())
      }
    }
  }
}

/// Deserialize one `[[mcp]]` entry from `name` plus a body fragment.
///
/// Going through `McpServerConfig` rather than eyeballing the text is the whole
/// point: the check that the proposal is loadable is the same code that will
/// load it.
fn parse_mcp(name: &str, body: &str) -> Result<McpServerConfig> {
  let document = format!("name = {}\n{}", toml_string(name), body);
  toml::from_str(&document).with_context(|| format!("`{name}` is not a usable [[mcp]] entry"))
}

/// Quote a value as a TOML basic string.
fn toml_string(value: &str) -> String {
  format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub struct ProposalStore {
  root: PathBuf,
  home: PathBuf,
}

impl ProposalStore {
  pub fn new() -> Result<Self> {
    let home = std::env::var("HOME").context("Could not find HOME directory")?;
    Self::at(PathBuf::from(home).join(".seekcli"))
  }

  pub fn at(home: PathBuf) -> Result<Self> {
    let root = home.join("proposals");
    for kind in Kind::ALL {
      fs::create_dir_all(root.join(kind.as_str()))
        .with_context(|| format!("cannot create {}", root.display()))?;
    }
    let store = Self { root, home };
    store.adopt_legacy_skill_proposals();
    Ok(store)
  }

  /// Move pre-0.3 skill proposals out of `skills/proposals/` into the shared
  /// home. Low risk: a proposal is a transient awaiting-review artefact, and
  /// this is a rename. Never silent — the user is told what moved.
  fn adopt_legacy_skill_proposals(&self) {
    let legacy = self.home.join("skills").join("proposals");
    let Ok(entries) = fs::read_dir(&legacy) else {
      return;
    };
    let mut moved = 0usize;
    for entry in entries.flatten() {
      let target = self.dir(Kind::Skill).join(entry.file_name());
      if target.exists() {
        continue;
      }
      if fs::rename(entry.path(), &target).is_ok() {
        moved += 1;
      }
    }
    if moved > 0 {
      eprintln!(
        "[Proposals] moved {} pending skill proposal(s) to {}",
        moved,
        self.dir(Kind::Skill).display()
      );
    }
  }

  pub fn dir(&self, kind: Kind) -> PathBuf {
    self.root.join(kind.as_str())
  }

  /// Draft a proposal. `content` is the whole payload for that kind.
  ///
  /// Validated on the way in as well as before landing: a draft the model can
  /// see is broken immediately is one it can correct in the same turn, instead
  /// of one the user discovers days later.
  pub fn draft(&self, kind: Kind, name: &str, content: &str) -> Result<PathBuf> {
    if !Kind::DRAFTABLE.contains(&kind) {
      anyhow::bail!(
        "`propose` does not draft {} proposals. Drafting kinds: {}. \
         Use `create_skill` for a skill.",
        kind,
        Kind::names(Kind::DRAFTABLE)
      );
    }
    let safe = crate::skills::sanitize_name(name);
    if safe.is_empty() {
      anyhow::bail!("`{name}` leaves no usable name after sanitising");
    }
    let path = self.path_for(kind, &safe);
    fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;

    let pending = Pending {
      kind,
      name: safe,
      path: path.clone(),
    };
    if let Err(e) = pending.validate() {
      // Keep the file: the model iterates on it rather than restarting, and the
      // user can still inspect what was attempted.
      return Err(e.context(format!(
        "the draft was saved to {} but will not be accepted as is",
        path.display()
      )));
    }
    Ok(path)
  }

  fn path_for(&self, kind: Kind, safe_name: &str) -> PathBuf {
    match kind {
      Kind::Skill => self.dir(kind).join(safe_name),
      Kind::Mcp => self.dir(kind).join(format!("{safe_name}.toml")),
      Kind::Task => self.dir(kind).join(format!("{safe_name}.md")),
    }
  }

  /// Everything awaiting review, every kind.
  pub fn list(&self) -> Vec<Pending> {
    let mut out = Vec::new();
    for kind in Kind::ALL {
      let Ok(entries) = fs::read_dir(self.dir(*kind)) else {
        continue;
      };
      for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
          continue;
        };
        out.push(Pending {
          kind: *kind,
          name: stem.to_string(),
          path,
        });
      }
    }
    out.sort_by(|a, b| (a.kind.as_str(), &a.name).cmp(&(b.kind.as_str(), &b.name)));
    out
  }

  fn find(&self, kind: Kind, name: &str) -> Result<Pending> {
    let safe = crate::skills::sanitize_name(name);
    let path = self.path_for(kind, &safe);
    if !path.exists() {
      let available = self.list();
      let listing: Vec<String> = available
        .iter()
        .map(|p| format!("{} {}", p.kind, p.name))
        .collect();
      anyhow::bail!(
        "no {} proposal named `{}`. Pending: {}",
        kind,
        name,
        if listing.is_empty() {
          "none".to_string()
        } else {
          listing.join("; ")
        }
      );
    }
    Ok(Pending {
      kind,
      name: safe,
      path,
    })
  }

  /// Land a proposal, after checking it can land. Returns what changed.
  pub fn accept(&self, kind: Kind, name: &str) -> Result<String> {
    let pending = self.find(kind, name)?;
    pending
      .validate()
      .with_context(|| format!("`{}` cannot be accepted as is", pending.name))?;

    match kind {
      Kind::Skill => {
        let target = self.home.join("skills").join(&pending.name);
        if target.exists() {
          anyhow::bail!(
            "a skill named `{}` already exists; reject this proposal or rename it",
            pending.name
          );
        }
        fs::rename(&pending.path, &target)
          .with_context(|| format!("cannot move {} into place", pending.path.display()))?;
        Ok(format!("skill `{}` is now active", pending.name))
      }
      Kind::Mcp => self.land_mcp(&pending),
      Kind::Task => {
        let dir = self.home.join("tasks").join(&pending.name);
        fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let target = dir.join("TASK.md");
        if target.exists() {
          anyhow::bail!("a task named `{}` already exists", pending.name);
        }
        fs::rename(&pending.path, &target)
          .with_context(|| format!("cannot move {} into place", pending.path.display()))?;
        Ok(format!(
          "task `{}` defined at {}",
          pending.name,
          target.display()
        ))
      }
    }
  }

  /// Append one `[[mcp]]` block to the user config.
  ///
  /// Appending rather than rewriting: `toml 0.8` is serde-shaped, so a
  /// round-trip would strip every comment out of the annotated config this
  /// project promises its users. TOML allows an array-of-table entry anywhere
  /// in the file, so appending is both valid and easy to review by hand
  /// (`docs/architecture/L5-composition.md` §4.3.3).
  fn land_mcp(&self, pending: &Pending) -> Result<String> {
    let body = fs::read_to_string(&pending.path)?;
    let config_path = self.home.join("config.toml");
    let existing = fs::read_to_string(&config_path).unwrap_or_default();

    if existing.contains(&format!("name = {}", toml_string(&pending.name))) {
      anyhow::bail!(
        "{} already mentions a server named `{}`",
        config_path.display(),
        pending.name
      );
    }

    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
      out.push('\n');
    }
    out.push_str(&format!(
      "\n# accepted from a proposal\n[[mcp]]\nname = {}\n{}",
      toml_string(&pending.name),
      body.trim_end()
    ));
    out.push('\n');
    fs::write(&config_path, out)
      .with_context(|| format!("cannot write {}", config_path.display()))?;
    fs::remove_file(&pending.path).ok();
    Ok(format!(
      "mcp server `{}` appended to {} — restart to connect it",
      pending.name,
      config_path.display()
    ))
  }

  pub fn reject(&self, kind: Kind, name: &str) -> Result<String> {
    let pending = self.find(kind, name)?;
    if pending.path.is_dir() {
      fs::remove_dir_all(&pending.path)
    } else {
      fs::remove_file(&pending.path)
    }
    .with_context(|| format!("cannot remove {}", pending.path.display()))?;
    Ok(format!("discarded {} proposal `{}`", kind, pending.name))
  }
}

/// Describe a path as a proposal kind, for error messages that name choices.
pub fn kind_names() -> String {
  Kind::names(Kind::ALL)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn store(tag: &str) -> (ProposalStore, PathBuf) {
    let home = std::env::temp_dir().join(format!("seekcli_prop_{}_{}", tag, uuid::Uuid::new_v4()));
    let s = match ProposalStore::at(home.clone()) {
      Ok(s) => s,
      Err(e) => panic!("cannot build store: {e}"),
    };
    (s, home)
  }

  #[test]
  fn an_mcp_draft_is_validated_by_the_loader_that_will_read_it() {
    let (s, home) = store("mcp");
    let ok = s.draft(
      Kind::Mcp,
      "files",
      "command = \"npx\"\nargs = [\"-y\", \"srv\"]\n",
    );
    assert!(ok.is_ok(), "{:?}", ok.err());

    // Not prose-checked: the same deserialization that loads a server decides.
    let bad = s.draft(Kind::Mcp, "broken", "args = [\"-y\"]\n");
    let err = match bad {
      Ok(p) => panic!(
        "a server with no command must not validate: {}",
        p.display()
      ),
      Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("broken"), "{err}");
    // The draft is kept so the model can correct it instead of starting over.
    assert!(s.dir(Kind::Mcp).join("broken.toml").exists());
    std::fs::remove_dir_all(home).ok();
  }

  #[test]
  fn accepting_an_mcp_proposal_appends_and_keeps_the_comments() {
    let (s, home) = store("append");
    let config = home.join("config.toml");
    let original = "# my notes\n[brain]\nflash_model = \"x\"\n";
    let _ = std::fs::write(&config, original);

    if let Err(e) = s.draft(Kind::Mcp, "files", "command = \"npx\"\n") {
      panic!("draft: {e:#}");
    }
    match s.accept(Kind::Mcp, "files") {
      Ok(note) => assert!(note.contains("files"), "{note}"),
      Err(e) => panic!("accept: {e:#}"),
    }

    let after = std::fs::read_to_string(&config).unwrap_or_default();
    // The whole reason for appending rather than rewriting.
    assert!(after.starts_with(original), "comments were lost:\n{after}");
    assert!(after.contains("[[mcp]]"), "{after}");
    // And it must still parse as one document.
    let parsed: Result<toml::Value, _> = toml::from_str(&after);
    assert!(parsed.is_ok(), "{:?}", parsed.err());
    std::fs::remove_dir_all(home).ok();
  }

  #[test]
  fn a_duplicate_server_name_is_refused_rather_than_appended_twice() {
    let (s, home) = store("dup");
    let config = home.join("config.toml");
    let _ = std::fs::write(&config, "[[mcp]]\nname = \"files\"\ncommand = \"npx\"\n");
    if let Err(e) = s.draft(Kind::Mcp, "files", "command = \"other\"\n") {
      panic!("draft: {e:#}");
    }
    assert!(s.accept(Kind::Mcp, "files").is_err());
    std::fs::remove_dir_all(home).ok();
  }

  #[test]
  fn a_task_proposal_needs_a_body_because_the_body_is_the_prompt() {
    let (s, home) = store("task");
    let header = "---\ninterval_hint: 1d\n---\n";
    assert!(s.draft(Kind::Task, "empty", header).is_err());
    assert!(
      s.draft(
        Kind::Task,
        "real",
        &format!("{header}Summarise today's commits.\n")
      )
      .is_ok()
    );

    match s.accept(Kind::Task, "real") {
      Ok(note) => assert!(note.contains("TASK.md"), "{note}"),
      Err(e) => panic!("accept: {e:#}"),
    }
    assert!(home.join("tasks").join("real").join("TASK.md").exists());
    std::fs::remove_dir_all(home).ok();
  }

  #[test]
  fn propose_refuses_skills_and_says_what_to_use_instead() {
    let (s, home) = store("skill");
    let err = match s.draft(Kind::Skill, "x", "whatever") {
      Ok(_) => panic!("skills are drafted by create_skill"),
      Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("create_skill"), "{err}");
    std::fs::remove_dir_all(home).ok();
  }

  #[test]
  fn an_unknown_name_lists_what_is_pending() {
    let (s, home) = store("unknown");
    if let Err(e) = s.draft(Kind::Mcp, "files", "command = \"npx\"\n") {
      panic!("draft: {e:#}");
    }
    let err = match s.accept(Kind::Mcp, "nope") {
      Ok(_) => panic!("expected an error"),
      Err(e) => format!("{e:#}"),
    };
    assert!(
      err.contains("mcp files"),
      "must list what is pending: {err}"
    );
    std::fs::remove_dir_all(home).ok();
  }

  #[test]
  fn pending_proposals_are_listed_across_every_kind() {
    let (s, home) = store("list");
    let _ = s.draft(Kind::Mcp, "a", "command = \"x\"\n");
    let _ = s.draft(Kind::Task, "b", "---\n---\nbody\n");
    let listed: Vec<String> = s
      .list()
      .iter()
      .map(|p| format!("{} {}", p.kind, p.name))
      .collect();
    assert!(listed.contains(&"mcp a".to_string()), "{listed:?}");
    assert!(listed.contains(&"task b".to_string()), "{listed:?}");
    std::fs::remove_dir_all(home).ok();
  }

  #[test]
  fn legacy_skill_proposals_are_adopted_into_the_shared_home() {
    let home = std::env::temp_dir().join(format!("seekcli_prop_legacy_{}", uuid::Uuid::new_v4()));
    let legacy = home.join("skills").join("proposals").join("old_one");
    let _ = std::fs::create_dir_all(&legacy);
    let _ = std::fs::write(legacy.join("SKILL.md"), "---\nname: old_one\n---\nbody\n");

    let s = match ProposalStore::at(home.clone()) {
      Ok(s) => s,
      Err(e) => panic!("{e}"),
    };
    assert!(s.dir(Kind::Skill).join("old_one").exists(), "not adopted");
    std::fs::remove_dir_all(home).ok();
  }
}
