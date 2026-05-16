use crate::api::{FunctionDefinition, Tool};
use serde_json::{Value, json};

pub fn system_tools() -> Vec<Tool> {
  vec![
    make_tool(
      "read_file",
      "Read the content of a UTF-8 text file. \
       Content is truncated at 50KB; for larger files use run_shell with grep/head/tail.",
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
       The sub-agent runs its own ReAct loop and returns only a summary, not the full trace. \
       Use this for exploration, large-file scanning, or any work that would bloat your context. \
       Maximum nesting depth is 3.",
      json!({
        "type": "object",
        "properties": {
          "prompt": {
            "type": "string",
            "description": "Self-contained instructions for the sub-agent. \
              Include all context it needs; it cannot see your conversation history."
          }
        },
        "required": ["prompt"]
      }),
    ),
    make_tool(
      "create_skill",
      "Persist a reusable skill (system_prompt + tool subset) to the user's skill library. \
       Use this only when the user explicitly asks to remember a pattern of work.",
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
  ]
}

/// Tools that must not be inherited by sub-agents.
/// `invoke_agent` would allow unbounded recursion; `create_skill` is a user-facing
/// meta-action that should not be triggered from inside an isolated subtask.
fn restricted_for_subagents() -> &'static [&'static str] {
  &["invoke_agent", "create_skill"]
}

pub fn filter_for_subagent(tools: &[Tool]) -> Vec<Tool> {
  let restricted = restricted_for_subagents();
  tools
    .iter()
    .filter(|t| !restricted.contains(&t.function.name.as_str()))
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
