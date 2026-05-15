use crate::skills::Skill;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

pub async fn create_skill(args: &Value) -> Result<String> {
  // Attempt to parse the arguments into our Skill struct
  let skill: Skill = serde_json::from_value(args.clone())
        .context("Failed to parse arguments into a valid Skill format. Ensure 'name', 'description', and 'system_prompt' are provided.")?;

  let home = std::env::var("HOME").context("Could not find HOME directory")?;
  let skills_dir = PathBuf::from(home).join(".seekcli").join("skills");

  if !skills_dir.exists() {
    tokio::fs::create_dir_all(&skills_dir)
      .await
      .context("Failed to create skills directory")?;
  }

  // Sanitize skill name to prevent directory traversal
  let safe_name = skill.name.replace("/", "_").replace("\\", "_");
  let file_path = skills_dir.join(format!("{}.json", safe_name));

  let content = serde_json::to_string_pretty(&skill)?;
  tokio::fs::write(&file_path, content)
    .await
    .context("Failed to write skill file")?;

  Ok(format!(
    "Successfully created new skill '{}' at {:?}",
    skill.name, file_path
  ))
}
