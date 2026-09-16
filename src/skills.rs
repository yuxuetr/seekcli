use crate::api::{FunctionDefinition, Tool};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
  pub name: String,
  pub description: String,
  pub system_prompt: String,
  pub tools: Option<Vec<SkillTool>>,
  /// Declared `version:`, if the SKILL.md carries one.
  ///
  /// Recorded rather than acted on. Stage 49's gate can reject a skill that
  /// breaks the smoke suite, but "roll back to the version that worked" needs
  /// to know which version was running — and until that is written down,
  /// rollback is a thing you do by remembering.
  pub version: Option<String>,
  /// Where it was loaded from. An installed skill, a proposal being previewed
  /// and a hand-written test fixture are three different things, and a listing
  /// that cannot tell them apart invites the wrong one being blamed.
  pub source: Option<String>,
  /// The tool names this skill narrows its agent to. `None` means "do not narrow".
  ///
  /// This can only ever **remove** tools. A skill declaring `run_shell` does
  /// not thereby acquire `run_shell` — the tool must already be on the surface
  /// and must still pass the policy gate on every call. Declaring is not
  /// granting; that distinction is the whole reason this field is applied as a
  /// filter over the effective set rather than as a source of tools.
  pub allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillTool {
  pub name: String,
  pub description: String,
  pub parameters: serde_json::Value,
}

impl Skill {
  pub fn to_api_tools(&self) -> Option<Vec<Tool>> {
    self.tools.as_ref().map(|tools| {
      tools
        .iter()
        .map(|t| Tool {
          tool_type: "function".to_string(),
          function: FunctionDefinition {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.parameters.clone(),
          },
        })
        .collect()
    })
  }
}

pub struct SkillManager {
  skills_dir: PathBuf,
  proposals_dir: PathBuf,
}

impl SkillManager {
  pub fn new() -> Result<Self> {
    let home = std::env::var("HOME").context("Could not find HOME directory")?;
    let skills_dir = PathBuf::from(&home).join(".seekcli").join("skills");
    // Shared with every other proposable asset since stage 37; `ProposalStore`
    // owns the directory and adopts anything left in the old location.
    let proposals_dir = PathBuf::from(&home)
      .join(".seekcli")
      .join("proposals")
      .join("skill");
    if !skills_dir.exists() {
      fs::create_dir_all(&skills_dir)?;
    }
    if !proposals_dir.exists() {
      fs::create_dir_all(&proposals_dir)?;
    }

    Ok(Self {
      skills_dir,
      proposals_dir,
    })
  }

  pub fn skills_dir(&self) -> &PathBuf {
    &self.skills_dir
  }

  pub fn proposals_dir(&self) -> &PathBuf {
    &self.proposals_dir
  }

  pub fn load_skills(&self) -> Result<Vec<Skill>> {
    Self::read_skill_dir(&self.skills_dir, true)
  }

  pub fn list_proposals(&self) -> Result<Vec<Skill>> {
    if !self.proposals_dir.exists() {
      return Ok(Vec::new());
    }
    Self::read_skill_dir(&self.proposals_dir, false)
  }

  // `accept_proposal` / `reject_proposal` moved to `crate::proposals`: the
  // gate now serves every proposable asset, and two implementations over the
  // same directory would drift (`docs/architecture/L5-composition.md` §4.3).

  /// Convert every legacy `<name>.json` skill in `skills_dir` to a
  /// `<name>/SKILL.md` directory. The original `.json` is renamed to
  /// `.json.bak` so the migration is reversible by hand.
  ///
  /// Returns `(migrated_count, skipped_count, errors)` where each entry of
  /// `errors` describes one skill that could not be migrated.
  pub fn migrate_legacy(&self) -> Result<MigrateReport> {
    let mut report = MigrateReport::default();
    for entry in fs::read_dir(&self.skills_dir)? {
      let entry = entry?;
      let path = entry.path();
      let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

      if name == "proposals" {
        continue;
      }
      if !path.is_file() {
        continue;
      }
      if path.extension().and_then(|s| s.to_str()) != Some("json") {
        continue;
      }

      let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
          report.errors.push(format!("{}: read failed: {}", name, e));
          continue;
        }
      };
      let skill: Skill = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
          report.errors.push(format!("{}: invalid JSON: {}", name, e));
          continue;
        }
      };

      let safe = sanitize_name(&skill.name);
      let dir = self.skills_dir.join(&safe);
      if dir.exists() {
        report
          .skipped
          .push(format!("{}: directory already exists", skill.name));
        continue;
      }

      if let Err(e) = fs::create_dir(&dir) {
        report
          .errors
          .push(format!("{}: mkdir failed: {}", skill.name, e));
        continue;
      }

      let md = render_skill_md(&skill);
      if let Err(e) = fs::write(dir.join("SKILL.md"), md) {
        report
          .errors
          .push(format!("{}: write SKILL.md failed: {}", skill.name, e));
        // Roll back the dir so a retry can succeed
        let _ = fs::remove_dir_all(&dir);
        continue;
      }

      let bak = path.with_extension("json.bak");
      if let Err(e) = fs::rename(&path, &bak) {
        report
          .errors
          .push(format!("{}: backup rename failed: {}", skill.name, e));
        continue;
      }

      report.migrated.push(skill.name);
    }
    Ok(report)
  }

  /// Scan `dir` for skills in either format:
  /// - `<name>.json` (legacy)
  /// - `<name>/SKILL.md` (new agentskills.io-compatible directory format)
  ///
  /// If `skip_proposals_subdir` is true, ignore an entry literally named
  /// "proposals" (relevant when scanning the active skills root).
  ///
  /// Skills that fail to parse are skipped with a warning so a single
  /// malformed entry doesn't block the rest of the library.
  fn read_skill_dir(dir: &PathBuf, skip_proposals_subdir: bool) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();
    for entry in fs::read_dir(dir)? {
      let entry = entry?;
      let path = entry.path();
      let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

      if skip_proposals_subdir && name == "proposals" {
        continue;
      }

      if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
        // Legacy <name>.json format
        let content = fs::read_to_string(&path)?;
        if let Ok(skill) = serde_json::from_str::<Skill>(&content) {
          skills.push(skill);
        }
        continue;
      }

      if path.is_dir() {
        // New <name>/SKILL.md format
        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
          continue;
        }
        match load_skill_md(&skill_md) {
          Ok(skill) => skills.push(skill),
          Err(e) => eprintln!("[skills] Failed to load {:?}: {}", skill_md.display(), e),
        }
      }
    }
    Ok(skills)
  }
}

pub fn sanitize_name(name: &str) -> String {
  name.replace(['/', '\\'], "_")
}

#[derive(Debug, Default)]
pub struct MigrateReport {
  pub migrated: Vec<String>,
  pub skipped: Vec<String>,
  pub errors: Vec<String>,
}

/// Render a `Skill` into the `<name>/SKILL.md` body string. Used by both
/// the `migrate_legacy` tool and `create_skill` when writing new proposals.
pub fn render_skill_md(skill: &Skill) -> String {
  let mut out = String::new();
  out.push_str("---\n");
  out.push_str(&format!("name: {}\n", quote_if_needed(&skill.name)));
  out.push_str(&format!(
    "description: {}\n",
    quote_if_needed(&skill.description)
  ));
  // The declared whitelist wins. Deriving it from `tools` is the legacy path,
  // kept for skills that carry full schemas instead of a name list — before
  // stage 42 that derivation was the *only* source, so a rendered skill named
  // tools it did not actually narrow to.
  let names: Vec<String> = match &skill.allowed_tools {
    Some(list) => list.clone(),
    None => skill
      .tools
      .as_ref()
      .map(|tools| tools.iter().map(|t| t.name.clone()).collect())
      .unwrap_or_default(),
  };
  if !names.is_empty() {
    out.push_str("allowed_tools:\n");
    for name in &names {
      out.push_str(&format!("  - {}\n", quote_if_needed(name)));
    }
  }
  out.push_str("---\n\n");
  out.push_str(&skill.system_prompt);
  if !skill.system_prompt.ends_with('\n') {
    out.push('\n');
  }
  out
}

/// Quote a scalar value for safe YAML emission. Conservative: quotes
/// anything containing `:` or `#`, leading/trailing whitespace, or YAML
/// indicator characters at the start.
fn quote_if_needed(s: &str) -> String {
  let needs_quotes = s.is_empty()
    || s != s.trim()
    || s.contains(':')
    || s.contains('#')
    || s.contains('\n')
    || s.starts_with('-')
    || s.starts_with('[')
    || s.starts_with('{')
    || s.starts_with('"')
    || s.starts_with('\'')
    || s.starts_with('&')
    || s.starts_with('*')
    || s.starts_with('!')
    || s.starts_with('|')
    || s.starts_with('>')
    || s.starts_with('%')
    || s.starts_with('@');
  if needs_quotes {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
  } else {
    s.to_string()
  }
}

// ---------------------------------------------------------------------------
// SKILL.md parsing
// ---------------------------------------------------------------------------

/// YAML frontmatter for a SKILL.md file. Only `name` and `description` are
/// required; the rest are optional. We deliberately hand-parse a tiny subset
/// of YAML to avoid pulling in a full YAML crate.
///
/// `allowed_tools` is consumed since stage 42: it narrows the effective tool
/// surface at agent-loop entry. `version` is still parsed for validation only,
/// awaiting the `/skill info` UX pass.
#[derive(Debug, Clone, Default)]
pub struct Frontmatter {
  pub name: String,
  pub description: String,
  pub allowed_tools: Option<Vec<String>>,
  #[allow(dead_code)] // exposed via /skill info in a later UX pass
  pub version: Option<String>,
}

/// Read a SKILL.md file and produce a `Skill`. The skill's `system_prompt`
/// is the Markdown body (everything after the closing `---` frontmatter
/// delimiter), optionally appended with a `## Skill Assets` section that
/// enumerates `scripts/` and `references/` files under the same directory.
pub fn load_skill_md(path: &Path) -> Result<Skill> {
  let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
  let (fm, body) =
    parse_skill_md(&content).with_context(|| format!("parse frontmatter of {}", path.display()))?;

  let skill_dir = path.parent().unwrap_or(Path::new("."));
  let assets = enumerate_skill_assets(skill_dir);

  let system_prompt = if assets.is_empty() {
    body
  } else {
    format!("{}\n\n{}", body.trim_end(), assets)
  };

  Ok(Skill {
    name: fm.name,
    description: fm.description,
    system_prompt,
    // `allowed_tools` is a name whitelist, semantically different from the
    // legacy `SkillTool` list which carried full schemas — so it travels in
    // its own field and is applied as a filter, never as a source of tools.
    tools: None,
    version: fm.version,
    source: Some(path.display().to_string()),
    allowed_tools: fm.allowed_tools,
  })
}

/// Scan a skill directory for `scripts/` and `references/` subfolders and
/// emit a Markdown section listing the available assets with one-line
/// descriptions extracted from each file's leading comment or heading.
/// Returns an empty string when no assets exist.
fn enumerate_skill_assets(skill_dir: &Path) -> String {
  let mut out = String::new();

  let scripts_dir = skill_dir.join("scripts");
  if scripts_dir.is_dir() {
    let entries = list_asset_entries(&scripts_dir, &["sh", "py", "js", "ts", "rb"]);
    if !entries.is_empty() {
      out.push_str("## Skill Scripts\n");
      out.push_str(&format!(
        "Located in `{}`. Invoke via `run_shell` with the absolute path.\n\n",
        scripts_dir.display()
      ));
      for (name, desc) in &entries {
        if desc.is_empty() {
          out.push_str(&format!("- `{}`\n", name));
        } else {
          out.push_str(&format!("- `{}` — {}\n", name, desc));
        }
      }
      out.push('\n');
    }
  }

  let refs_dir = skill_dir.join("references");
  if refs_dir.is_dir() {
    let entries = list_asset_entries(&refs_dir, &["md", "txt", "json", "yaml", "yml"]);
    if !entries.is_empty() {
      out.push_str("## Skill References\n");
      out.push_str(&format!(
        "Located in `{}`. Read with `read_file` when relevant.\n\n",
        refs_dir.display()
      ));
      for (name, desc) in &entries {
        if desc.is_empty() {
          out.push_str(&format!("- `{}`\n", name));
        } else {
          out.push_str(&format!("- `{}` — {}\n", name, desc));
        }
      }
    }
  }

  out
}

/// Sorted `(filename, description)` list for files in `dir` matching any of
/// `allowed_exts`. Returns empty Vec on any IO failure (assets are best-effort
/// enrichment, not load-blocking).
fn list_asset_entries(dir: &Path, allowed_exts: &[&str]) -> Vec<(String, String)> {
  let read = match fs::read_dir(dir) {
    Ok(r) => r,
    Err(_) => return Vec::new(),
  };
  let mut out = Vec::new();
  for entry in read.flatten() {
    let path = entry.path();
    if !path.is_file() {
      continue;
    }
    let ext = path
      .extension()
      .and_then(|s| s.to_str())
      .unwrap_or("")
      .to_lowercase();
    if !allowed_exts.iter().any(|e| *e == ext) {
      continue;
    }
    let name = match path.file_name().and_then(|s| s.to_str()) {
      Some(n) => n.to_string(),
      None => continue,
    };
    let desc = extract_asset_description(&path).unwrap_or_default();
    out.push((name, desc));
  }
  out.sort_by(|a, b| a.0.cmp(&b.0));
  out
}

/// Pull a one-line description from the first meaningful line of a file.
/// Skips shebangs and common comment markers (`#`, `//`, `/*`, `*`, `;`).
/// Capped at ~120 chars.
fn extract_asset_description(path: &Path) -> Result<String> {
  let content = fs::read_to_string(path)?;
  for line in content.lines().take(20) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("#!") {
      continue;
    }
    let stripped = trimmed
      .trim_start_matches('#')
      .trim_start_matches("//")
      .trim_start_matches("/*")
      .trim_start_matches('*')
      .trim_start_matches(';')
      .trim();
    if stripped.is_empty() {
      continue;
    }
    let truncated: String = stripped.chars().take(120).collect();
    let suffix = if stripped.chars().count() > 120 {
      "…"
    } else {
      ""
    };
    return Ok(format!("{}{}", truncated, suffix));
  }
  Ok(String::new())
}

/// Split a `---`-delimited document into `(frontmatter yaml, body)`.
///
/// Shared with L8's `TASK.md` (`tasks.rs`): both are "YAML header plus a
/// Markdown body that is really a prompt", and having them drift apart on
/// BOMs or CRLF would be a pointless class of bug. `label` only shapes the
/// error message.
pub fn split_frontmatter(content: &str, label: &str) -> Result<(String, String)> {
  let trimmed = content.trim_start_matches('\u{feff}'); // strip BOM if present
  let rest = trimmed
    .strip_prefix("---\n")
    .or_else(|| trimmed.strip_prefix("---\r\n"))
    .ok_or_else(|| anyhow::anyhow!("{} must start with a '---' frontmatter delimiter", label))?;

  let end = rest
    .find("\n---\n")
    .map(|i| (i, "\n---\n"))
    .or_else(|| rest.find("\n---\r\n").map(|i| (i, "\n---\r\n")))
    .or_else(|| {
      // Handle the case where ---\n comes at end-of-file with no trailing newline
      if rest.ends_with("\n---") {
        Some((rest.len() - 4, "\n---"))
      } else {
        None
      }
    })
    .ok_or_else(|| {
      anyhow::anyhow!(
        "{} frontmatter not closed (expected '---' separator)",
        label
      )
    })?;

  Ok((
    rest[..end.0].to_string(),
    rest[end.0 + end.1.len()..].trim_start().to_string(),
  ))
}

/// Read one scalar key out of frontmatter, ignoring everything else.
///
/// Enough for `TASK.md`, which has no lists. Returns `None` for a missing key
/// and for the explicit YAML nulls, so `skill: null` reads the same as
/// omitting the line.
pub fn frontmatter_scalar(yaml: &str, key: &str) -> Option<String> {
  for line in yaml.lines() {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }
    let Some((k, v)) = line.split_once(':') else {
      continue;
    };
    if k.trim() != key {
      continue;
    }
    let value = v.trim().trim_matches(['"', '\'']).trim();
    if value.is_empty() || value == "null" || value == "~" {
      return None;
    }
    return Some(value.to_string());
  }
  None
}

/// Split a SKILL.md document into `(frontmatter, body)`. Errors if the
/// document doesn't begin with a `---` line or the frontmatter is unclosed.
pub fn parse_skill_md(content: &str) -> Result<(Frontmatter, String)> {
  let (yaml, body) = split_frontmatter(content, "SKILL.md")?;
  let fm = parse_frontmatter(&yaml)?;
  Ok((fm, body))
}

/// Minimal hand-rolled YAML parser for our constrained frontmatter shape.
/// Supports:
/// - `key: scalar`
/// - `key:` followed by `  - item` indented lines (list values)
/// - quoted scalars (single or double quotes, stripped)
/// - blank lines and `# comment` lines are skipped
///
/// Unknown keys are silently ignored so future spec additions don't break
/// older clients.
fn parse_frontmatter(yaml: &str) -> Result<Frontmatter> {
  let mut name: Option<String> = None;
  let mut description: Option<String> = None;
  let mut allowed_tools: Option<Vec<String>> = None;
  let mut version: Option<String> = None;

  enum ListContext {
    None,
    AllowedTools,
    OtherIgnored,
  }
  let mut list_ctx = ListContext::None;

  for raw_line in yaml.lines() {
    let line = raw_line.trim_end();
    if line.trim_start().is_empty() || line.trim_start().starts_with('#') {
      continue;
    }

    // List item line: starts with optional whitespace + `- `
    let leading_ws_len = line.len() - line.trim_start().len();
    if leading_ws_len > 0 {
      let trimmed = line.trim_start();
      if let Some(item) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("-"))
      {
        let value = strip_quotes(item.trim()).to_string();
        match list_ctx {
          ListContext::AllowedTools => {
            allowed_tools.get_or_insert_with(Vec::new).push(value);
          }
          ListContext::None | ListContext::OtherIgnored => {}
        }
        continue;
      }
    }

    // key: value (or key: with list to follow)
    let (key, value) = match line.split_once(':') {
      Some(pair) => pair,
      None => continue,
    };
    let key = key.trim();
    let value = value.trim();

    if value.is_empty() {
      // A list is about to follow.
      list_ctx = match key {
        "allowed_tools" => ListContext::AllowedTools,
        _ => ListContext::OtherIgnored,
      };
      continue;
    }

    list_ctx = ListContext::None;
    let cleaned = strip_quotes(value).to_string();
    match key {
      "name" => name = Some(cleaned),
      "description" => description = Some(cleaned),
      "version" => version = Some(cleaned),
      _ => {} // unknown scalar keys silently ignored
    }
  }

  Ok(Frontmatter {
    name: name.ok_or_else(|| anyhow::anyhow!("frontmatter missing required field 'name'"))?,
    description: description
      .ok_or_else(|| anyhow::anyhow!("frontmatter missing required field 'description'"))?,
    allowed_tools,
    version,
  })
}

fn strip_quotes(s: &str) -> &str {
  let stripped = s
    .strip_prefix('"')
    .and_then(|s| s.strip_suffix('"'))
    .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')));
  stripped.unwrap_or(s)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_minimal_skill_md() {
    let content = "---\nname: translator\ndescription: translate stuff\n---\n# Body\n\nHello.";
    let (fm, body) = parse_skill_md(content).expect("parse ok");
    assert_eq!(fm.name, "translator");
    assert_eq!(fm.description, "translate stuff");
    assert!(fm.allowed_tools.is_none());
    assert!(body.starts_with("# Body"));
  }

  #[test]
  fn parses_allowed_tools_list() {
    let content = "---\nname: explorer\ndescription: read-only\nallowed_tools:\n  - read_file\n  - list_dir\n  - run_shell\n---\nbody";
    let (fm, _) = parse_skill_md(content).expect("parse ok");
    let tools = fm.allowed_tools.expect("has tools");
    assert_eq!(tools, vec!["read_file", "list_dir", "run_shell"]);
  }

  #[test]
  fn parses_quoted_values() {
    let content = "---\nname: \"my-skill\"\ndescription: 'has spaces'\nversion: \"1.2\"\n---\nbody";
    let (fm, _) = parse_skill_md(content).expect("parse ok");
    assert_eq!(fm.name, "my-skill");
    assert_eq!(fm.description, "has spaces");
    assert_eq!(fm.version.as_deref(), Some("1.2"));
  }

  #[test]
  fn ignores_unknown_keys() {
    let content =
      "---\nname: x\ndescription: y\nfuture_field: ignored\nweird:\n  - a\n  - b\n---\nbody";
    let (fm, _) = parse_skill_md(content).expect("parse ok");
    assert_eq!(fm.name, "x");
    assert!(fm.allowed_tools.is_none());
  }

  #[test]
  fn errors_on_missing_frontmatter() {
    let content = "no delimiter here\nname: x\n---\nbody";
    assert!(parse_skill_md(content).is_err());
  }

  #[test]
  fn errors_on_unclosed_frontmatter() {
    let content = "---\nname: x\ndescription: y\nbody never gets out";
    assert!(parse_skill_md(content).is_err());
  }

  #[test]
  fn errors_on_missing_required_field() {
    let content = "---\ndescription: only-desc\n---\nbody";
    assert!(parse_skill_md(content).is_err());
  }

  #[test]
  fn body_is_preserved_verbatim() {
    let content = "---\nname: x\ndescription: y\n---\n\n# Title\n\n```rust\nlet x = 1;\n```\n";
    let (_, body) = parse_skill_md(content).expect("parse ok");
    assert!(body.contains("```rust"));
    assert!(body.contains("let x = 1;"));
  }

  #[test]
  fn render_skill_md_minimal() {
    let skill = Skill {
      name: "tester".to_string(),
      description: "a test skill".to_string(),
      system_prompt: "do the thing".to_string(),
      tools: None,
      version: None,
      source: None,
      allowed_tools: None,
    };
    let md = render_skill_md(&skill);
    assert!(md.starts_with("---\n"));
    assert!(md.contains("name: tester"));
    assert!(md.contains("description: a test skill"));
    assert!(md.ends_with("do the thing\n"));
  }

  #[test]
  fn render_then_parse_roundtrips() {
    let original = Skill {
      name: "rt-test".to_string(),
      description: "round: trip with # special chars".to_string(),
      system_prompt: "Body line 1.\nBody line 2.".to_string(),
      tools: None,
      version: None,
      source: None,
      allowed_tools: None,
    };
    let md = render_skill_md(&original);
    let (fm, body) = parse_skill_md(&md).expect("roundtrip parses");
    assert_eq!(fm.name, original.name);
    assert_eq!(fm.description, original.description);
    assert!(body.contains("Body line 1."));
    assert!(body.contains("Body line 2."));
  }

  #[test]
  fn render_quotes_problematic_values() {
    let skill = Skill {
      name: "x".to_string(),
      description: "has: colon and #hash".to_string(),
      system_prompt: "body".to_string(),
      tools: None,
      version: None,
      source: None,
      allowed_tools: None,
    };
    let md = render_skill_md(&skill);
    // description must be quoted since it contains both ':' and '#'
    assert!(md.contains("description: \"has: colon and #hash\""));
  }

  #[test]
  fn render_includes_allowed_tools() {
    let skill = Skill {
      name: "x".to_string(),
      description: "y".to_string(),
      system_prompt: "body".to_string(),
      tools: Some(vec![
        SkillTool {
          name: "read_file".to_string(),
          description: "".to_string(),
          parameters: serde_json::json!({}),
        },
        SkillTool {
          name: "run_shell".to_string(),
          description: "".to_string(),
          parameters: serde_json::json!({}),
        },
      ]),
      version: None,
      source: None,
      allowed_tools: None,
    };
    let md = render_skill_md(&skill);
    assert!(md.contains("allowed_tools:"));
    assert!(md.contains("  - read_file"));
    assert!(md.contains("  - run_shell"));
  }

  /// The gap this closed ran from the parser to the loop: `allowed_tools` was
  /// read off disk into `Frontmatter` and then dropped on the floor, because
  /// `load_skill_md` hard-coded `tools: None` and had nowhere else to put it.
  #[test]
  fn a_loaded_skill_carries_its_whitelist_to_the_loop() {
    let tmp = std::env::temp_dir().join("seekcli_test_skill_allowed.md");
    std::fs::write(
      &tmp,
      "---\nname: narrow\ndescription: read-only\nallowed_tools:\n  - read_file\n  - grep\n---\nbody\n",
    )
    .expect("write tmp");
    let skill = load_skill_md(&tmp).expect("load ok");
    assert_eq!(
      skill.allowed_tools,
      Some(vec!["read_file".to_string(), "grep".to_string()]),
      "the whitelist must survive the trip from disk to Skill"
    );
    let _ = std::fs::remove_file(&tmp);
  }

  /// A skill with no `allowed_tools` must not narrow anything. `Some(vec![])`
  /// and `None` mean different things and the loop branches on it.
  #[test]
  fn a_skill_without_a_whitelist_does_not_narrow() {
    let tmp = std::env::temp_dir().join("seekcli_test_skill_nonarrow.md");
    std::fs::write(&tmp, "---\nname: wide\ndescription: d\n---\nbody\n").expect("write tmp");
    let skill = load_skill_md(&tmp).expect("load ok");
    assert_eq!(skill.allowed_tools, None);
    let _ = std::fs::remove_file(&tmp);
  }

  /// Render must emit the declared whitelist, not re-derive it from `tools`.
  /// Before stage 42 the derivation was the only source, so a rendered skill
  /// could name tools it did not actually narrow to.
  #[test]
  fn render_emits_the_declared_whitelist_over_the_derived_one() {
    let skill = Skill {
      name: "x".to_string(),
      description: "y".to_string(),
      system_prompt: "body".to_string(),
      tools: Some(vec![SkillTool {
        name: "run_shell".to_string(),
        description: String::new(),
        parameters: serde_json::json!({}),
      }]),
      version: None,
      source: None,
      allowed_tools: Some(vec!["read_file".to_string()]),
    };
    let md = render_skill_md(&skill);
    assert!(md.contains("  - read_file"), "{md}");
    assert!(
      !md.contains("  - run_shell"),
      "derived list must not win: {md}"
    );
  }

  #[test]
  fn load_skill_md_from_disk() {
    let tmp = std::env::temp_dir().join("seekcli_test_skill.md");
    std::fs::write(
      &tmp,
      "---\nname: roundtrip\ndescription: from-disk\n---\nbody-content\n",
    )
    .expect("write tmp");
    let skill = load_skill_md(&tmp).expect("load ok");
    assert_eq!(skill.name, "roundtrip");
    assert_eq!(skill.description, "from-disk");
    assert!(skill.system_prompt.contains("body-content"));
    std::fs::remove_file(&tmp).ok();
  }

  #[test]
  fn assets_enumeration_handles_missing_dirs() {
    let tmp = std::env::temp_dir().join(format!("seekcli_assets_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).expect("mkdir");
    // No scripts/ or references/ — should produce empty enrichment
    let out = enumerate_skill_assets(&tmp);
    assert!(out.is_empty());
    std::fs::remove_dir_all(&tmp).ok();
  }

  #[test]
  fn assets_enumeration_lists_scripts_and_references() {
    let tmp = std::env::temp_dir().join(format!("seekcli_assets_{}", uuid::Uuid::new_v4()));
    let scripts = tmp.join("scripts");
    let refs = tmp.join("references");
    std::fs::create_dir_all(&scripts).expect("scripts dir");
    std::fs::create_dir_all(&refs).expect("refs dir");

    std::fs::write(
      scripts.join("format.sh"),
      "#!/bin/bash\n# Format the output as JSON\necho hi\n",
    )
    .unwrap();
    std::fs::write(
      scripts.join("lookup.py"),
      "#!/usr/bin/env python3\n# Lookup glossary term\nprint('x')\n",
    )
    .unwrap();
    std::fs::write(
      refs.join("examples.md"),
      "# Translation examples\n\nSome content.\n",
    )
    .unwrap();
    std::fs::write(refs.join("glossary.txt"), "API terms in three languages\n").unwrap();

    let out = enumerate_skill_assets(&tmp);
    assert!(out.contains("## Skill Scripts"));
    assert!(out.contains("`format.sh`"));
    assert!(out.contains("Format the output as JSON"));
    assert!(out.contains("`lookup.py`"));
    assert!(out.contains("Lookup glossary term"));
    assert!(out.contains("## Skill References"));
    assert!(out.contains("`examples.md`"));
    assert!(out.contains("Translation examples"));
    assert!(out.contains("`glossary.txt`"));
    assert!(out.contains("API terms in three languages"));

    std::fs::remove_dir_all(&tmp).ok();
  }

  #[test]
  fn load_skill_md_with_assets() {
    let tmp = std::env::temp_dir().join(format!("seekcli_skill_{}", uuid::Uuid::new_v4()));
    let scripts = tmp.join("scripts");
    std::fs::create_dir_all(&scripts).expect("mkdir");
    std::fs::write(
      scripts.join("hello.sh"),
      "#!/bin/bash\n# Say hello in style\necho hi\n",
    )
    .unwrap();
    std::fs::write(
      tmp.join("SKILL.md"),
      "---\nname: with-assets\ndescription: has scripts\n---\nDo the thing.\n",
    )
    .unwrap();

    let skill = load_skill_md(&tmp.join("SKILL.md")).expect("load ok");
    assert!(skill.system_prompt.contains("Do the thing."));
    assert!(skill.system_prompt.contains("## Skill Scripts"));
    assert!(skill.system_prompt.contains("hello.sh"));
    assert!(skill.system_prompt.contains("Say hello in style"));

    std::fs::remove_dir_all(&tmp).ok();
  }
}
