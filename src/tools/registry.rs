use crate::api::{FunctionDefinition, Tool};
use serde_json::{Value, json};

pub fn system_tools() -> Vec<Tool> {
  vec![
    make_tool(
      "read_file",
      "Read the content of a UTF-8 text file. Large files are offloaded to a \
       temp file and returned as a head+tail preview; read specific ranges of \
       the original path with run_shell (sed/grep/head/tail) when you need more.",
      json!({
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Path to the file (absolute or relative to cwd)"
          }
        },
        "required": ["path"]
      }),
    ),
    make_tool(
      "write_file",
      "Write content to a file, overwriting if it exists. Creates parent directories as needed. \
       In a later release, writes outside the current working directory will be rejected.",
      json!({
        "type": "object",
        "properties": {
          "path":    { "type": "string", "description": "Destination file path" },
          "content": { "type": "string", "description": "Full content to write" }
        },
        "required": ["path", "content"]
      }),
    ),
    make_tool(
      "edit_file",
      "Make a surgical, in-place edit: replace one occurrence of old_text with \
       new_text. PREFER this over write_file for changing existing files — it \
       does not require rewriting the whole file. old_text is matched with \
       whitespace/indentation tolerance, so copy it from read_file output; you \
       do not need to reproduce indentation perfectly. If old_text matches more \
       than one place, the edit is refused — add surrounding lines until it is \
       unique. If it matches nothing, re-read the file and copy old_text again.",
      json!({
        "type": "object",
        "properties": {
          "path":     { "type": "string", "description": "File to edit (must exist)" },
          "old_text": { "type": "string", "description": "Exact snippet to replace; include enough lines to be unique" },
          "new_text": { "type": "string", "description": "Replacement snippet" }
        },
        "required": ["path", "old_text", "new_text"]
      }),
    ),
    make_tool(
      "list_dir",
      "List entries in a directory (single level, no recursion). \
       Use run_shell with find/tree for deeper exploration.",
      json!({
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Directory path; defaults to current working directory"
          }
        }
      }),
    ),
    make_tool(
      "run_shell",
      "Execute a shell command via `sh -c`. Captures both stdout and stderr. \
       In a later release, dangerous commands (rm -rf, sudo, curl|sh, etc.) will prompt for user confirmation. \
       Failures return exit status + stderr so you can self-correct.",
      json!({
        "type": "object",
        "properties": {
          "command": { "type": "string", "description": "Shell command to execute" }
        },
        "required": ["command"]
      }),
    ),
    make_tool(
      "invoke_agent",
      "Spawn an isolated sub-agent in a fresh context to handle a focused subtask. \
       Choose subagent_type based on what the task needs:\n\
       - explore: read-only investigation (list dirs, read files, grep). \
         Fastest and safest. Use for code search, repo understanding, locating things.\n\
       - general: full read/write/shell focused subtask. Use when the sub-agent \
         needs to make small edits or run commands end-to-end.\n\
       Returns only a summary, not the full trace. Maximum nesting depth is 3.",
      json!({
        "type": "object",
        "properties": {
          "subagent_type": {
            "type": "string",
            "enum": ["explore", "general"],
            "description": "Type of sub-agent to spawn"
          },
          "prompt": {
            "type": "string",
            "description": "Self-contained instructions for the sub-agent. \
              Include all context it needs; it cannot see your conversation history."
          }
        },
        "required": ["subagent_type", "prompt"]
      }),
    ),
    make_tool(
      "create_skill",
      "Draft a reusable skill proposal. The proposal is saved to the user's review \
       queue, NOT directly activated. Use this only when the user explicitly asks \
       to remember a pattern of work. Tell the user to run `/skill proposals` \
       afterwards to review and accept.",
      json!({
        "type": "object",
        "properties": {
          "name":          { "type": "string", "description": "Short unique skill identifier (snake_case)" },
          "description":   { "type": "string", "description": "One-line summary of what the skill does" },
          "system_prompt": { "type": "string", "description": "Full system prompt that defines the skill's behavior" },
          "tools": {
            "type": "array",
            "description": "Optional tool subset this skill should expose. Omit to allow all system tools.",
            "items": { "type": "object" }
          }
        },
        "required": ["name", "description", "system_prompt"]
      }),
    ),
    make_tool(
      "load_skill",
      "Activate a previously-saved skill mid-conversation. Its system prompt is \
       appended to the conversation as a system message, taking effect immediately \
       on subsequent turns. Use this when the user's intent matches an existing \
       skill (translator, code_reviewer, etc). Only the main agent can call this; \
       sub-agents cannot switch skills.",
      json!({
        "type": "object",
        "properties": {
          "name": {
            "type": "string",
            "description": "Exact skill name from /skill list"
          }
        },
        "required": ["name"]
      }),
    ),
  ]
}

/// Whether a tool is safe to run concurrently with its siblings in the same
/// turn. Only pure read-only tools qualify.
///
/// `run_shell` is deliberately excluded: a shell command can write files or
/// have side effects, and detecting that reliably would require parsing shell
/// AST. `write_file` / `create_skill` are writes; `invoke_agent` / `load_skill`
/// mutate engine state. Per the harness "read-concurrent, write-serial" rule,
/// a turn is parallelized only when EVERY call is read-only.
pub fn is_parallel_readonly(tool_name: &str) -> bool {
  matches!(tool_name, "read_file" | "list_dir")
}

/// Filter `tools` down to those listed in `allowed`. Used to apply a
/// SubAgent template's `allowed_tools` whitelist at spawn time.
pub fn filter_by_allowed(tools: &[Tool], allowed: &[&str]) -> Vec<Tool> {
  tools
    .iter()
    .filter(|t| allowed.contains(&t.function.name.as_str()))
    .cloned()
    .collect()
}

/// Merge skill-declared tools onto the base system tools.
/// System tools take precedence on name collision so their schemas remain authoritative.
pub fn merge_with_skill(skill_tools: Option<Vec<Tool>>) -> Vec<Tool> {
  let mut merged = system_tools();
  if let Some(extra) = skill_tools {
    let known: std::collections::HashSet<String> =
      merged.iter().map(|t| t.function.name.clone()).collect();
    for t in extra {
      if !known.contains(&t.function.name) {
        merged.push(t);
      }
    }
  }
  merged
}

fn make_tool(name: &str, description: &str, parameters: Value) -> Tool {
  Tool {
    tool_type: "function".to_string(),
    function: FunctionDefinition {
      name: name.to_string(),
      description: description.to_string(),
      parameters,
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn readonly_classification() {
    assert!(is_parallel_readonly("read_file"));
    assert!(is_parallel_readonly("list_dir"));
    // Writes / shell / delegation must never be parallelized.
    assert!(!is_parallel_readonly("write_file"));
    assert!(!is_parallel_readonly("run_shell"));
    assert!(!is_parallel_readonly("create_skill"));
    assert!(!is_parallel_readonly("invoke_agent"));
    assert!(!is_parallel_readonly("load_skill"));
  }
}
