use crate::api::{FunctionDefinition, Tool};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

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

    let manager = Self {
      skills_dir,
      proposals_dir,
    };
    manager.ensure_default_skills()?;
    Ok(manager)
  }

  pub fn proposals_dir(&self) -> &PathBuf {
    &self.proposals_dir
  }

  fn ensure_default_skills(&self) -> Result<()> {
    let default_skills = vec![
            Skill {
                name: "translator".to_string(),
                description: "Professional translator for multiple languages. Use this for translation tasks.".to_string(),
                system_prompt: "You are a professional translator. Translate everything into natural, idiomatic language.".to_string(),
                tools: None,
            },
            Skill {
                name: "file_helper".to_string(),
                description: "Expert at reading and writing local files.".to_string(),
                system_prompt: "You are a file system assistant. Use your tools to help users manage their local files.".to_string(),
                tools: Some(vec![
                    SkillTool {
                        name: "read_file".to_string(),
                        description: "Read the content of a file".to_string(),
                        parameters: json!({
                            "type": "object",
                            "properties": {
                                "path": { "type": "string", "description": "Path to the file" }
                            },
                            "required": ["path"]
                        }),
                    }
                ]),
            }
        ];

    for skill in default_skills {
      let path = self.skills_dir.join(format!("{}.json", skill.name));
      if !path.exists() {
        let content = serde_json::to_string_pretty(&skill)?;
        fs::write(path, content)?;
      }
    }
    Ok(())
  }

  pub fn load_skills(&self) -> Result<Vec<Skill>> {
    Self::read_skill_dir(&self.skills_dir)
  }

  pub fn list_proposals(&self) -> Result<Vec<Skill>> {
    if !self.proposals_dir.exists() {
      return Ok(Vec::new());
    }
    Self::read_skill_dir(&self.proposals_dir)
  }

  pub fn accept_proposal(&self, name: &str) -> Result<()> {
    let safe = sanitize_name(name);
    let src = self.proposals_dir.join(format!("{}.json", safe));
    let dst = self.skills_dir.join(format!("{}.json", safe));
    if !src.exists() {
      anyhow::bail!("Proposal '{}' not found", name);
    }
    if dst.exists() {
      anyhow::bail!(
        "A skill named '{}' already exists. Reject the proposal or rename it first.",
        name
      );
    }
    fs::rename(&src, &dst)
      .with_context(|| format!("Failed to promote proposal '{}' to active skill", name))?;
    Ok(())
  }

  pub fn reject_proposal(&self, name: &str) -> Result<()> {
    let safe = sanitize_name(name);
    let path = self.proposals_dir.join(format!("{}.json", safe));
    if !path.exists() {
      anyhow::bail!("Proposal '{}' not found", name);
    }
    fs::remove_file(&path).with_context(|| format!("Failed to delete proposal '{}'", name))?;
    Ok(())
  }

  /// Read every `*.json` directly under `dir` (no recursion) as a `Skill`.
  /// Files that fail to parse are silently skipped — we don't want a single
  /// malformed file to block the rest of the library.
  fn read_skill_dir(dir: &PathBuf) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();
    for entry in fs::read_dir(dir)? {
      let entry = entry?;
      let path = entry.path();
      if !path.is_file() {
        continue;
      }
      if path.extension().and_then(|s| s.to_str()) != Some("json") {
        continue;
      }
      let content = fs::read_to_string(&path)?;
      if let Ok(skill) = serde_json::from_str::<Skill>(&content) {
        skills.push(skill);
      }
    }
    Ok(skills)
  }
}

pub fn sanitize_name(name: &str) -> String {
  name.replace(['/', '\\'], "_")
}
