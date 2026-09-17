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
  /// Whether several of these may run at the same time.
  ///
  /// The line is what the template is *for*: `general` exists to change things,
  /// so two of them editing the same file is a likely outcome rather than a
  /// remote one — and `edit_file` is read-modify-write, so interleaving them
  /// corrupts rather than conflicts. `explore` exists to look.
  ///
  /// **Declared, not derived, and the gap is worth naming**: `explore` carries
  /// `run_shell`, which can write. Its contract and its system prompt both say
  /// it must not, and `--read-only` enforces it, but in normal mode the harness
  /// does not. So this flag says "intended for concurrent use", which is a
  /// weaker claim than "cannot possibly conflict".
  pub parallel_safe: bool,
}

const EXPLORE: SubAgentTemplate = SubAgentTemplate {
  name: "explore",
  description: "Read-only exploration: list dirs, read files, grep. Fastest, safest.",
  system_prompt: "\
You are an exploration sub-agent for SeekCLI.

Your job: investigate the user's specific question and return a concise
summary with file:line citations. You CANNOT write files, modify state, or
spawn further sub-agents.

Available tools: read_file, list_dir, glob, grep, run_shell (read-only only).

Rules:
- Use glob / grep for discovery. They respect .gitignore and are far cheaper
  than shelling out or walking trees with list_dir.
- Cite file:line. Be terse. The parent agent will reformat for the user.
- Stop calling tools as soon as you have enough evidence to answer.
- Do NOT propose changes; only investigate.
- The prompt is all you get: you cannot see the conversation that produced it.
  If it refers to something you have no way to resolve (`the file above`,
  `the error we just saw`, `continue what you started`), reply immediately
  with `[PROMPT INCOMPLETE]` and say exactly what is missing. Do NOT guess.
  Guessing produces a confident summary of the wrong thing, and the parent
  has no way to tell that from a right one.
",
  allowed_tools: &["read_file", "list_dir", "glob", "grep", "run_shell"],
  max_iter: 15,
  parallel_safe: true,
};

const GENERAL: SubAgentTemplate = SubAgentTemplate {
  name: "general",
  description: "Full read/write/shell focused subtask. Use for end-to-end small jobs.",
  system_prompt: "\
You are a general-purpose sub-agent for SeekCLI.

Your job: complete the user's specific subtask end-to-end and return a
concise summary. You can read, write, and run shell — same as the parent
agent — but you CANNOT spawn further sub-agents.

Available tools: read_file, write_file, edit_file, list_dir, glob, grep,
run_shell.

Rules:
- Use glob / grep for discovery before reading files whole.
- Stay focused on the subtask. Don't expand scope.
- File writes are restricted to the current working directory.
- Dangerous shell commands (rm -rf, sudo, ...) require user approval.
- Cite file:line. Be terse. Parent agent will reformat for the user.
- Stop calling tools as soon as the subtask is done.
- The prompt is all you get: you cannot see the conversation that produced it.
  If it refers to something you have no way to resolve (`the file above`,
  `the error we just saw`, `continue what you started`), reply immediately
  with `[PROMPT INCOMPLETE]` and say exactly what is missing. Do NOT guess.
  Guessing produces a confident summary of the wrong thing, and the parent
  has no way to tell that from a right one.
",
  allowed_tools: &[
    "read_file",
    "write_file",
    "edit_file",
    "list_dir",
    "glob",
    "grep",
    "run_shell",
  ],
  max_iter: 20,
  // Two of these editing the same file is what this template is for, not an
  // edge case.
  parallel_safe: false,
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

  /// The rule that keeps concurrent delegations from corrupting a file: a
  /// template that exists to write must not be fanned out. `edit_file` is
  /// read-modify-write, so two of them on one path interleave rather than
  /// merely conflict.
  #[test]
  fn only_the_read_only_template_is_parallel_safe() {
    for t in SUBAGENTS {
      let writes = t
        .allowed_tools
        .iter()
        .any(|tool| matches!(*tool, "write_file" | "edit_file"));
      assert!(
        !(writes && t.parallel_safe),
        "`{}` can write and must not be marked parallel_safe",
        t.name
      );
    }
    // And the read-only one must actually be marked, or the feature is dead
    // code that silently never triggers.
    assert!(
      SUBAGENTS.iter().any(|t| t.parallel_safe),
      "no template is parallel_safe — fan-out can never happen"
    );
  }

  /// `[PROMPT INCOMPLETE]` is a contract between two prompts that live in
  /// different files: the child is told to emit it, the parent is told what it
  /// means. Either half alone is worse than neither — a child that refuses
  /// while the parent reads the refusal as a finding, or a parent waiting for
  /// a marker nothing produces.
  ///
  /// Nothing else can catch this. Both halves are prose in string literals, so
  /// the compiler sees two unrelated constants, and a behaviour test would
  /// need the model to actually be handed an unresolvable prompt.
  #[test]
  fn both_ends_of_the_incomplete_prompt_contract_exist() {
    const MARKER: &str = "[PROMPT INCOMPLETE]";
    for t in SUBAGENTS {
      assert!(
        t.system_prompt.contains(MARKER),
        "`{}` is never told to report an unresolvable prompt; it will guess \
         instead, and a confident summary of the wrong thing is \
         indistinguishable from a right one",
        t.name
      );
    }
    let parent = crate::agent::prompt::agent_system_prompt();
    assert!(
      parent.contains(MARKER),
      "the parent is never told what `{MARKER}` means, so it would treat a \
       refusal as a finding"
    );
  }

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
