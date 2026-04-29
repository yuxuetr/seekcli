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
}

impl SkillManager {
  pub fn new() -> Result<Self> {
    let home = std::env::var("HOME").context("Could not find HOME directory")?;
    let skills_dir = PathBuf::from(home).join(".seekcli").join("skills");
    if !skills_dir.exists() {
      fs::create_dir_all(&skills_dir)?;
    }

    let manager = Self { skills_dir };
    manager.ensure_default_skills()?;
    Ok(manager)
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
    let mut skills = Vec::new();
    for entry in fs::read_dir(&self.skills_dir)? {
      let entry = entry?;
      let path = entry.path();
      if path.extension().and_then(|s| s.to_str()) == Some("json") {
        let content = fs::read_to_string(path)?;
        if let Ok(skill) = serde_json::from_str::<Skill>(&content) {
          skills.push(skill);
        }
      }
    }
    Ok(skills)
  }
}
