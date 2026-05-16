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
    let proposals_dir = skills_dir.join("proposals");
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

  pub fn accept_proposal(&self, name: &str) -> Result<()> {
    let safe = sanitize_name(name);
    let src_dir = self.proposals_dir.join(&safe);
    let src_json = self.proposals_dir.join(format!("{}.json", safe));
    let dst_dir = self.skills_dir.join(&safe);
    let dst_json = self.skills_dir.join(format!("{}.json", safe));

    if dst_dir.exists() || dst_json.exists() {
      anyhow::bail!(
        "A skill named '{}' already exists. Reject the proposal or rename it first.",
        name
      );
    }

    if src_dir.is_dir() {
      fs::rename(&src_dir, &dst_dir).with_context(|| {
        format!(
          "Failed to promote proposal '{}' (dir) to active skill",
          name
        )
      })?;
    } else if src_json.exists() {
      fs::rename(&src_json, &dst_json).with_context(|| {
        format!(
          "Failed to promote proposal '{}' (json) to active skill",
          name
        )
      })?;
    } else {
      anyhow::bail!("Proposal '{}' not found", name);
    }
    Ok(())
  }

  pub fn reject_proposal(&self, name: &str) -> Result<()> {
    let safe = sanitize_name(name);
    let src_dir = self.proposals_dir.join(&safe);
    let src_json = self.proposals_dir.join(format!("{}.json", safe));

    if src_dir.is_dir() {
      fs::remove_dir_all(&src_dir)
        .with_context(|| format!("Failed to delete proposal directory '{}'", name))?;
    } else if src_json.exists() {
      fs::remove_file(&src_json)
        .with_context(|| format!("Failed to delete proposal file '{}'", name))?;
    } else {
      anyhow::bail!("Proposal '{}' not found", name);
    }
    Ok(())
  }

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
  if let Some(tools) = &skill.tools
    && !tools.is_empty()
  {
    out.push_str("allowed_tools:\n");
    for t in tools {
      out.push_str(&format!("  - {}\n", quote_if_needed(&t.name)));
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
/// `allowed_tools` and `version` are parsed and validated now, but not yet
/// consumed downstream — phase 12.5 will wire `allowed_tools` into a per-skill
/// tool whitelist applied at agent loop entry. Kept here so SKILL.md files
/// can be authored against the final schema starting in C1.
#[derive(Debug, Clone, Default)]
pub struct Frontmatter {
  pub name: String,
  pub description: String,
  #[allow(dead_code)] // consumed by phase 12.5 ToolDispatcher whitelist
  pub allowed_tools: Option<Vec<String>>,
  #[allow(dead_code)] // exposed via /skill info in a later UX pass
  pub version: Option<String>,
}

/// Read a SKILL.md file and produce a `Skill`. The skill's `system_prompt`
/// is the Markdown body (everything after the closing `---` frontmatter
/// delimiter).
pub fn load_skill_md(path: &Path) -> Result<Skill> {
  let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
  let (fm, body) =
    parse_skill_md(&content).with_context(|| format!("parse frontmatter of {}", path.display()))?;
  Ok(Skill {
    name: fm.name,
    description: fm.description,
    system_prompt: body,
    // `allowed_tools` in frontmatter is a name whitelist (different semantics
    // from legacy `SkillTool` which carried full schemas). Stored in tools as
    // None for now; later phases may wire the whitelist into ToolDispatcher.
    tools: None,
  })
}

/// Split a SKILL.md document into `(frontmatter, body)`. Errors if the
/// document doesn't begin with a `---` line or the frontmatter is unclosed.
pub fn parse_skill_md(content: &str) -> Result<(Frontmatter, String)> {
  let trimmed = content.trim_start_matches('\u{feff}'); // strip BOM if present
  let rest = trimmed
    .strip_prefix("---\n")
    .or_else(|| trimmed.strip_prefix("---\r\n"))
    .ok_or_else(|| anyhow::anyhow!("SKILL.md must start with a '---' frontmatter delimiter"))?;

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
    .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter not closed (expected '---' separator)"))?;

  let yaml = &rest[..end.0];
  let body = rest[end.0 + end.1.len()..].trim_start().to_string();

  let fm = parse_frontmatter(yaml)?;
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
    };
    let md = render_skill_md(&skill);
    assert!(md.contains("allowed_tools:"));
    assert!(md.contains("  - read_file"));
    assert!(md.contains("  - run_shell"));
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
}
