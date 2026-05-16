//! SubAgent template registry.
//!
//! A SubAgent template is a `(name, system_prompt, allowed_tools)` bundle.
//! Templates are **persistent** (defined in code); each `invoke_agent` call
//! creates a **transient instance** with fresh context, returns a summary,
//! and dies.
//!
//! Adding a new sub-agent type = adding an entry to `SUBAGENTS`. The enum
//! value automatically appears in the `invoke_agent` tool schema for the LLM
//! to choose from.

pub struct SubAgentTemplate {
  pub name: &'static str,
  pub description: &'static str,
  pub system_prompt: &'static str,
  pub allowed_tools: &'static [&'static str],
  pub max_iter: usize,
}

const EXPLORE: SubAgentTemplate = SubAgentTemplate {
  name: "explore",
  description: "Read-only exploration: list dirs, read files, grep. Fastest, safest.",
  system_prompt: "\
You are an exploration sub-agent for SeekCLI.

Your job: investigate the user's specific question and return a concise
summary with file:line citations. You CANNOT write files, modify state, or
spawn further sub-agents.

Available tools: read_file, list_dir, run_shell (read-only commands only).

Rules:
- Prefer find / grep / rg over recursive list_dir for large trees.
- Cite file:line. Be terse. The parent agent will reformat for the user.
- Stop calling tools as soon as you have enough evidence to answer.
- Do NOT propose changes; only investigate.
",
  allowed_tools: &["read_file", "list_dir", "run_shell"],
  max_iter: 15,
};

const GENERAL: SubAgentTemplate = SubAgentTemplate {
  name: "general",
  description: "Full read/write/shell focused subtask. Use for end-to-end small jobs.",
  system_prompt: "\
You are a general-purpose sub-agent for SeekCLI.

Your job: complete the user's specific subtask end-to-end and return a
concise summary. You can read, write, and run shell — same as the parent
agent — but you CANNOT spawn further sub-agents.

Available tools: read_file, write_file, list_dir, run_shell.

Rules:
- Stay focused on the subtask. Don't expand scope.
- File writes are restricted to the current working directory.
- Dangerous shell commands (rm -rf, sudo, ...) require user approval.
- Cite file:line. Be terse. Parent agent will reformat for the user.
- Stop calling tools as soon as the subtask is done.
",
  allowed_tools: &["read_file", "write_file", "list_dir", "run_shell"],
  max_iter: 20,
};

pub static SUBAGENTS: &[SubAgentTemplate] = &[EXPLORE, GENERAL];

pub fn lookup(name: &str) -> Option<&'static SubAgentTemplate> {
  SUBAGENTS.iter().find(|t| t.name == name)
}

/// `(name, description)` pairs for every registered template. Used when
/// reporting "unknown subagent_type" back to the model so it can pick a
/// valid alternative.
pub fn catalog() -> Vec<(&'static str, &'static str)> {
  SUBAGENTS.iter().map(|t| (t.name, t.description)).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn explore_excludes_write() {
    let t = lookup("explore").expect("explore template exists");
    assert!(!t.allowed_tools.contains(&"write_file"));
    assert!(!t.allowed_tools.contains(&"invoke_agent"));
    assert!(!t.allowed_tools.contains(&"create_skill"));
  }

  #[test]
  fn general_excludes_invoke() {
    let t = lookup("general").expect("general template exists");
    assert!(!t.allowed_tools.contains(&"invoke_agent"));
    assert!(!t.allowed_tools.contains(&"create_skill"));
    assert!(t.allowed_tools.contains(&"write_file"));
  }

  #[test]
  fn lookup_unknown_returns_none() {
    assert!(lookup("nonexistent").is_none());
  }

  #[test]
  fn catalog_lists_all_with_descriptions() {
    let cat = catalog();
    let names: Vec<_> = cat.iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"explore"));
    assert!(names.contains(&"general"));
    for (_, desc) in &cat {
      assert!(!desc.is_empty(), "every template must have a description");
    }
  }
}
