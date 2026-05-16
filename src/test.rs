#[cfg(test)]
mod verification_tests {
  use crate::skills::SkillManager;
  use anyhow::Result;

  #[test]
  fn skill_manager_initializes_cleanly() -> Result<()> {
    let manager = SkillManager::new()?;
    // load_skills must succeed even on a fresh install with zero skills.
    let skills = manager.load_skills()?;
    let proposals = manager.list_proposals()?;
    println!(
      "SkillManager initialized: {} active skills, {} proposals",
      skills.len(),
      proposals.len()
    );
    Ok(())
  }
}
