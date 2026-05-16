use crate::skills::{Skill, render_skill_md, sanitize_name};
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

  // Block name collision with any active skill (both legacy .json and new
  // <name>/ directory forms). Proposals overwriting earlier proposals is
  // fine (model iterating); silently clobbering a user-accepted skill is not.
  let active_dir = skills_root.join(&safe_name);
  let active_json = skills_root.join(format!("{}.json", safe_name));
  if active_dir.exists() || active_json.exists() {
    return Ok(format!(
      "[NAME COLLISION] An active skill named '{}' already exists. \
       Choose a different name (e.g. '{}_v2'), or ask the user to first \
       delete the existing skill if they want to replace it.",
      skill.name, skill.name
    ));
  }

  let proposal_dir = proposals_dir.join(&safe_name);

  // If an earlier iteration of the same proposal exists, replace it.
  if proposal_dir.exists() {
    tokio::fs::remove_dir_all(&proposal_dir)
      .await
      .context("Failed to clear previous proposal directory")?;
  }
  // Also clean up any legacy `<name>.json` proposal so accept_proposal
  // doesn't pick up a stale copy.
  let legacy_json_proposal = proposals_dir.join(format!("{}.json", safe_name));
  if legacy_json_proposal.exists() {
    tokio::fs::remove_file(&legacy_json_proposal).await.ok();
  }

  tokio::fs::create_dir_all(&proposal_dir)
    .await
    .context("Failed to create proposal directory")?;

  let md_content = render_skill_md(&skill);
  let md_path = proposal_dir.join("SKILL.md");
  tokio::fs::write(&md_path, md_content)
    .await
    .context("Failed to write SKILL.md")?;

  Ok(format!(
    "Skill proposal '{}' saved to {:?}.\n\
     The proposal is NOT active yet. The user must review and accept it via:\n\
       /skill proposals          (list pending)\n\
       /skill accept {}   (promote to active)\n\
       /skill reject {}   (discard)\n\
     Do not assume the skill is loaded; tell the user a proposal awaits review.",
    skill.name, proposal_dir, skill.name, skill.name
  ))
}
