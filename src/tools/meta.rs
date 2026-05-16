use crate::skills::{Skill, sanitize_name};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

pub async fn create_skill(args: &Value) -> Result<String> {
  let skill: Skill = serde_json::from_value(args.clone()).context(
    "Failed to parse arguments into a valid Skill format. \
     Ensure 'name', 'description', and 'system_prompt' are provided.",
  )?;

  let home = std::env::var("HOME").context("Could not find HOME directory")?;
  let proposals_dir = PathBuf::from(home)
    .join(".seekcli")
    .join("skills")
    .join("proposals");

  if !proposals_dir.exists() {
    tokio::fs::create_dir_all(&proposals_dir)
      .await
      .context("Failed to create proposals directory")?;
  }

  let safe_name = sanitize_name(&skill.name);
  let file_path = proposals_dir.join(format!("{}.json", safe_name));

  let content = serde_json::to_string_pretty(&skill)?;
  tokio::fs::write(&file_path, content)
    .await
    .context("Failed to write skill proposal")?;

  Ok(format!(
    "Skill proposal '{}' saved to {:?}.\n\
     The proposal is NOT active yet. The user must review and accept it via:\n\
       /skill proposals          (list pending)\n\
       /skill accept {}   (promote to active)\n\
       /skill reject {}   (discard)\n\
     Do not assume the skill is loaded; tell the user a proposal awaits review.",
    skill.name, file_path, skill.name, skill.name
  ))
}
