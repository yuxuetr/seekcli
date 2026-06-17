use std::path::Path;

use super::{MAX_ITER, MAX_SUBAGENT_DEPTH};

/// Workspace-rules files probed (in order) for the dynamic Prompt Composer.
/// First match wins. `AGENTS.md` is the agentskills.io / Hermes convention;
/// `CLAUDE.md` is supported for Claude Code interop.
const WORKSPACE_RULE_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

/// Hard cap on injected workspace-rules bytes, so a huge AGENTS.md cannot
/// blow up every request's prompt. Tail is truncated with a marker.
const WORKSPACE_RULES_CAP: usize = 8_192;

/// Dynamic Prompt Composer: read the workspace rules file (`AGENTS.md` /
/// `CLAUDE.md`) from `workspace` if present, returning a formatted system
/// message body. Returns `None` when no rules file exists (zero overhead).
///
/// This is injected as a SEPARATE system message AFTER the static kernel so
/// the cache prefix (`agent_system_prompt`) stays byte-identical and the
/// prompt cache keeps hitting.
pub fn workspace_rules(workspace: &Path) -> Option<String> {
  for name in WORKSPACE_RULE_FILES {
    let path = workspace.join(name);
    let raw = match std::fs::read_to_string(&path) {
      Ok(s) => s,
      Err(_) => continue,
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
      continue;
    }
    let body = if trimmed.len() > WORKSPACE_RULES_CAP {
      let mut cut = WORKSPACE_RULES_CAP;
      while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
      }
      format!(
        "{}\n\n[...workspace rules truncated at {} bytes...]",
        &trimmed[..cut],
        WORKSPACE_RULES_CAP
      )
    } else {
      trimmed.to_string()
    };
    return Some(format!(
      "# Workspace Rules ({})\n\n\
       The following project-specific conventions were loaded from `{}` in the \
       working directory. Treat them as authoritative for this project.\n\n{}",
      name, name, body
    ));
  }
  None
}

/// Base system prompt that turns DeepSeek into a SeekCLI Harness Agent.
/// Kept stable across sessions to maximize prompt cache hit rate.
pub fn agent_system_prompt() -> String {
  format!(
    r#"You are SeekCLI, a terminal-based Harness Agent powered by DeepSeek V4.

You operate inside a ReAct loop: think -> call tools -> observe results -> think again.
Each turn you may either:
  1. Emit one or more tool_calls to gather information or take action, OR
  2. Emit your final textual answer (which stops the loop).

# Tools available
- read_file / write_file / list_dir : filesystem operations (single level for list_dir)
- run_shell : execute shell commands; captures stdout and stderr
- invoke_agent : delegate to a typed sub-agent. Pass subagent_type:
    - explore : read-only investigation (list dirs, read files, grep). Fastest, safest.
    - general : full read/write/shell for end-to-end focused subtasks.
- load_skill : activate a previously-saved skill (e.g. translator) mid-conversation.
- create_skill : draft a NEW skill proposal as `<name>/SKILL.md` (Markdown body
                 with YAML frontmatter). The proposal goes to the user's review
                 queue; it is NOT auto-activated. After creating, instruct the
                 user to run `/skill proposals` and `/skill accept <name>`.
                 Tip: write `system_prompt` as readable Markdown (headings,
                 lists, code fences welcome) — it will be rendered verbatim
                 into the body. If a tool result starts with `[NAME COLLISION]`,
                 the chosen name conflicts with an existing skill — pick a
                 different name.

# How to choose
- For broad exploration / multi-file scans -> invoke_agent("explore", ...) (avoids context bloat)
- For end-to-end small jobs in isolation -> invoke_agent("general", ...)
- For one-off operations -> call the matching tool directly
- For long edits -> read_file -> reason -> write_file
- Stop calling tools as soon as you have enough information to answer.

# Output discipline
- Be terse. Reference code as `file:line` so the user can jump there.
- Do not narrate "I will now call X" -- just call it.
- When you decide no more tools are needed, output the final answer as plain text.

# Safety
- You are operating on the user's local machine.
- Destructive shell commands (rm -rf, sudo, curl|sh, etc.) are intercepted and
  require the user's interactive y/N approval.
- Filesystem writes outside the current working directory are rejected.
- If a tool result starts with `[USER DENIED]` or `[PATH DENIED]`, **do not retry**
  the same call. Either propose a safer alternative or ask the user how to proceed.
- A `[Recovery]` line appended to a failed tool result is a debugging SOP from the
  harness. Follow it before retrying — do not ignore it and re-issue the same call.
- A `[BAD ARGS]` result means your tool arguments were not valid JSON; re-emit the
  call with a single well-formed JSON object.

# Stopping conditions
- Maximum {max_iter} iterations per chat turn.
- Maximum sub-agent nesting depth: {max_depth}.

# Sub-agent context
- A sub-agent receives only the prompt you give it. It has no access to your conversation.
- Pass all required context (file paths, prior findings, goals) in the prompt.
- Sub-agents cannot spawn further sub-agents indefinitely; depth is limited.
"#,
    max_iter = MAX_ITER,
    max_depth = MAX_SUBAGENT_DEPTH,
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Write;

  #[test]
  fn workspace_rules_none_when_absent() {
    let dir = std::env::temp_dir().join("seekcli_ws_test_absent");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::remove_file(dir.join("AGENTS.md"));
    let _ = std::fs::remove_file(dir.join("CLAUDE.md"));
    assert!(workspace_rules(&dir).is_none());
  }

  #[test]
  fn workspace_rules_reads_agents_md() {
    let dir = std::env::temp_dir().join("seekcli_ws_test_present");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("AGENTS.md");
    let mut f = std::fs::File::create(&path).expect("create AGENTS.md");
    write!(f, "Use 2-space indentation.").expect("write");
    let out = workspace_rules(&dir).expect("should find AGENTS.md");
    assert!(out.contains("Use 2-space indentation."));
    assert!(out.contains("Workspace Rules"));
    let _ = std::fs::remove_file(&path);
  }

  #[test]
  fn workspace_rules_truncates_oversized() {
    let dir = std::env::temp_dir().join("seekcli_ws_test_big");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("AGENTS.md");
    let big = "x".repeat(WORKSPACE_RULES_CAP + 5_000);
    std::fs::write(&path, &big).expect("write big");
    let out = workspace_rules(&dir).expect("should find AGENTS.md");
    assert!(out.contains("truncated"));
    let _ = std::fs::remove_file(&path);
  }
}
