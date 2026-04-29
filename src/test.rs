#[cfg(test)]
mod verification_tests {
  use crate::skills::SkillManager;
  use anyhow::Result;

  #[test]
  fn test_load_all_skills() -> Result<()> {
    let manager = SkillManager::new()?;
    let skills = manager.load_skills()?;

    println!("\n--- Skill Verification Report ---");
    println!("Total skills found: {}", skills.len());

    for skill in &skills {
      println!("- Skill ID: {}", skill.name);
      println!("  Desc: {}", skill.description);
    }
    println!("--------------------------------\n");

    assert!(skills.iter().any(|s| s.name == "ielts_writing"));
    assert!(skills.iter().any(|s| s.name == "ielts_speaking"));
    assert!(skills.iter().any(|s| s.name == "rust_backend"));
    assert!(skills.iter().any(|s| s.name == "dioxus_frontend"));
    Ok(())
  }
}
