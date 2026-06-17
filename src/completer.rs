//! Rustyline tab-completion for REPL slash commands. Re-scans the skill
//! directory on every Tab press so newly created skills / proposals are
//! immediately discoverable without restarting.

use std::path::{Path, PathBuf};

use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;

/// Top-level slash commands eligible for tab completion.
const SLASH_COMMANDS: &[&str] = &[
  "/help",
  "/quit",
  "/exit",
  "/clear",
  "/history",
  "/copy",
  "/model",
  "/thinking",
  "/plan",
  "/skill",
  "/load",
];

/// Subcommands for `/skill` that aren't skill names.
const SKILL_SUBCOMMANDS: &[&str] = &["list", "proposals", "migrate", "accept", "reject"];

const MODEL_VARIANTS: &[&str] = &["flash", "pro"];
const THINKING_MODES: &[&str] = &["n", "h", "m"];

/// Rustyline `Helper` that provides command completion.
pub(crate) struct CmdCompleter {
  pub(crate) skills_dir: PathBuf,
  pub(crate) proposals_dir: PathBuf,
}

impl CmdCompleter {
  fn active_skill_names(&self) -> Vec<String> {
    list_skill_names(&self.skills_dir, true)
  }

  fn proposal_names(&self) -> Vec<String> {
    list_skill_names(&self.proposals_dir, false)
  }
}

/// Enumerate skill names from `dir`, recognizing both legacy `<name>.json`
/// and the new `<name>/SKILL.md` directory format.
fn list_skill_names(dir: &Path, skip_proposals_subdir: bool) -> Vec<String> {
  let mut names = Vec::new();
  let read = match std::fs::read_dir(dir) {
    Ok(r) => r,
    Err(_) => return names,
  };
  for entry in read.flatten() {
    let path = entry.path();
    let raw_name = match path.file_name().and_then(|s| s.to_str()) {
      Some(n) => n,
      None => continue,
    };
    if skip_proposals_subdir && raw_name == "proposals" {
      continue;
    }
    if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
      if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        names.push(stem.to_string());
      }
    } else if path.is_dir() && path.join("SKILL.md").exists() {
      names.push(raw_name.to_string());
    }
  }
  names.sort();
  names.dedup();
  names
}

fn pairs<I, S>(items: I) -> Vec<Pair>
where
  I: IntoIterator<Item = S>,
  S: Into<String>,
{
  items
    .into_iter()
    .map(|s| {
      let s = s.into();
      Pair {
        display: s.clone(),
        replacement: s,
      }
    })
    .collect()
}

impl Completer for CmdCompleter {
  type Candidate = Pair;

  fn complete(
    &self,
    line: &str,
    pos: usize,
    _ctx: &rustyline::Context<'_>,
  ) -> rustyline::Result<(usize, Vec<Pair>)> {
    let prefix = &line[..pos.min(line.len())];

    // /skill <subcommand or name>
    if let Some(rest) = prefix.strip_prefix("/skill ") {
      let after_space = pos - rest.len();

      // /skill accept <proposal>
      if let Some(name_prefix) = rest.strip_prefix("accept ") {
        let start = after_space + "accept ".len();
        let matches: Vec<String> = self
          .proposal_names()
          .into_iter()
          .filter(|n| n.starts_with(name_prefix))
          .collect();
        return Ok((start, pairs(matches)));
      }

      // /skill reject <proposal>
      if let Some(name_prefix) = rest.strip_prefix("reject ") {
        let start = after_space + "reject ".len();
        let matches: Vec<String> = self
          .proposal_names()
          .into_iter()
          .filter(|n| n.starts_with(name_prefix))
          .collect();
        return Ok((start, pairs(matches)));
      }

      // /skill <single token>: either subcommand or active skill name
      let mut candidates: Vec<String> = SKILL_SUBCOMMANDS.iter().map(|s| s.to_string()).collect();
      candidates.extend(self.active_skill_names());
      candidates.sort();
      candidates.dedup();
      let matches: Vec<String> = candidates
        .into_iter()
        .filter(|c| c.starts_with(rest))
        .collect();
      return Ok((after_space, pairs(matches)));
    }

    if let Some(rest) = prefix.strip_prefix("/model ") {
      let start = pos - rest.len();
      let matches: Vec<String> = MODEL_VARIANTS
        .iter()
        .filter(|v| v.starts_with(rest))
        .map(|v| v.to_string())
        .collect();
      return Ok((start, pairs(matches)));
    }

    if let Some(rest) = prefix.strip_prefix("/thinking ") {
      let start = pos - rest.len();
      let matches: Vec<String> = THINKING_MODES
        .iter()
        .filter(|v| v.starts_with(rest))
        .map(|v| v.to_string())
        .collect();
      return Ok((start, pairs(matches)));
    }

    // Bare slash command
    if prefix.starts_with('/') && !prefix.contains(' ') {
      let matches: Vec<String> = SLASH_COMMANDS
        .iter()
        .filter(|c| c.starts_with(prefix))
        .map(|c| c.to_string())
        .collect();
      return Ok((0, pairs(matches)));
    }

    Ok((pos, Vec::new()))
  }
}

impl Hinter for CmdCompleter {
  type Hint = String;
}

impl Highlighter for CmdCompleter {}

impl Validator for CmdCompleter {}

impl rustyline::Helper for CmdCompleter {}
