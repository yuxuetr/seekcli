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

  use crate::App;

  #[test]
  fn result_is_failure_classification() {
    // Failures (drive the Two-Stage ReAct micro trigger).
    assert!(App::result_is_failure(
      "Error executing tool read_file: no such file"
    ));
    assert!(App::result_is_failure("foo\n\n[Recovery] do X"));
    assert!(App::result_is_failure("[BAD ARGS] not json"));
    assert!(App::result_is_failure(
      "[ERROR] load_skill is restricted to the main agent."
    ));
    assert!(App::result_is_failure("Sub-agent 'explore' failed: boom"));
    // Successes must NOT trigger replanning.
    assert!(!App::result_is_failure("fn main() {}"));
    assert!(!App::result_is_failure("Successfully wrote to foo.txt"));
    assert!(!App::result_is_failure("a.rs\nb.rs\n"));
  }
}
