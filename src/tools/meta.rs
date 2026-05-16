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
  let skills_root = PathBuf::from(home).join(".seekcli").join("skills");
  let proposals_dir = skills_root.join("proposals");

  let safe_name = sanitize_name(&skill.name);
  let active_path = skills_root.join(format!("{}.json", safe_name));

  // Block name collision with an already-active skill. Proposals overwriting
  // earlier proposals is fine (model iterating); silently clobbering a
  // user-accepted skill is not.
  if active_path.exists() {
    return Ok(format!(
      "[NAME COLLISION] An active skill named '{}' already exists. \
       Choose a different name (e.g. '{}_v2'), or ask the user to first \
       delete the existing skill if they want to replace it.",
      skill.name, skill.name
    ));
  }

  if !proposals_dir.exists() {
    tokio::fs::create_dir_all(&proposals_dir)
      .await
      .context("Failed to create proposals directory")?;
  }

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
