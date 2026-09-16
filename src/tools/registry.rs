use crate::api::{FunctionDefinition, Tool};
use serde_json::{Value, json};

pub fn system_tools() -> Vec<Tool> {
  vec![
    make_tool(
      "read_file",
      "Read the content of a UTF-8 text file. Large files are offloaded to a \
       temp file and returned as a head+tail preview; use grep to locate the \
       part you need inside the original path instead of re-reading it whole.",
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
       Use glob for recursive discovery by pattern.",
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
      "glob",
      "Find files by path pattern. PREFER this over run_shell with find/ls. \
       Gitignored paths (target/, node_modules/, ...) are excluded by default. \
       Results are sorted newest-first and capped, so narrow the pattern if \
       told entries were withheld.\n\
       Pattern is a gitignore-style glob: `*.rs` matches at any depth, \
       `src/*.rs` only that directory, `**/*.test.ts` any depth explicitly.",
      json!({
        "type": "object",
        "properties": {
          "pattern": {
            "type": "string",
            "description": "Glob pattern, e.g. \"*.rs\" or \"src/**/*.toml\""
          },
          "path": {
            "type": "string",
            "description": "Directory to search under; defaults to current working directory"
          }
        },
        "required": ["pattern"]
      }),
    ),
    make_tool(
      "grep",
      "Search file contents by regex. PREFER this over run_shell with grep/rg. \
       Returns `path:line: content` rows; gitignored and binary files are \
       skipped. Results are capped, so narrow the pattern or path if told \
       matches were withheld.\n\
       The pattern is a REGEX, not a shell glob — escape . * + ? ( ) [ ] to \
       match them literally. Use the separate `glob` argument to restrict \
       which files are searched.",
      json!({
        "type": "object",
        "properties": {
          "pattern": {
            "type": "string",
            "description": "Regex to search for, e.g. \"fn main\" or \"impl \\\\w+ for\""
          },
          "path": {
            "type": "string",
            "description": "Directory to search under; defaults to current working directory"
          },
          "glob": {
            "type": "string",
            "description": "Optional file filter, e.g. \"*.rs\" to search only Rust sources"
          }
        },
        "required": ["pattern"]
      }),
    ),
    make_tool(
      "read_image",
      "Look at an image file (PNG, JPEG, GIF, WebP). Use this when a task \
       involves a screenshot, diagram, chart or photo in the workspace — you \
       can see images, so describing one to yourself second-hand is worse than \
       reading it. `read_file` is for text and will fail on an image.",
      json!({
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Path to the image (absolute or relative to cwd)"
          }
        },
        "required": ["path"]
      }),
    ),
    make_tool(
      "propose",
      "Draft a change for the user to approve: a new MCP server (`mcp`) or a \
       scheduled task (`task`). Nothing takes effect until the user accepts it. \
       Use this when a capability you need is missing — check \
       harness_inspect{what:\"mcp\"} first. For a new skill use create_skill instead.",
      json!({
        "type": "object",
        "properties": {
          "kind": {
            "type": "string",
            "enum": ["mcp", "task"],
            "description": "What the proposal would become"
          },
          "name": { "type": "string", "description": "Short identifier, e.g. filesystem" },
          "description": { "type": "string", "description": "One line: why this is worth adding" },
          "content": {
            "type": "string",
            "description": "For `mcp`: the TOML body of one [[mcp]] entry without the \
              name line, e.g. `command = \"npx\"` plus `args = [...]`. For `task`: the \
              whole TASK.md, YAML frontmatter then the prompt body."
          }
        },
        "required": ["kind", "name", "description", "content"]
      }),
    ),
    make_tool(
      "harness_inspect",
      "Inspect your own runtime: the tool surface, the policy rules actually in \
       force, skills, MCP server status, and session counters. Use it when a \
       call was refused and you need to know what IS allowed, when a capability \
       you expected is missing, or before drafting a skill. Read-only.",
      json!({
        "type": "object",
        "properties": {
          "what": {
            "type": "string",
            "enum": ["tools", "policy", "skills", "mcp", "session"],
            "description": "Which section to return. Omit for all of them."
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
          "command": { "type": "string", "description": "Shell command to execute" },
          "background": {
            "type": "boolean",
            "description": "Run detached and return immediately with a job id. \
              Use for anything long-running (builds, test suites, installs) so \
              the conversation is not blocked. Collect it later with job_output."
          }
        },
        "required": ["command"]
      }),
    ),
    make_tool(
      "job_list",
      "List background jobs with their state (running / done / failed / killed), \
       elapsed time and command.",
      json!({ "type": "object", "properties": {} }),
    ),
    make_tool(
      "job_output",
      "Read a background job's output. Returns the tail; raise `tail` to see more. \
       Output is told when it was truncated — never assume a short tail is the \
       whole story.",
      json!({
        "type": "object",
        "properties": {
          "id":   { "type": "integer", "description": "Job id from run_shell(background) or job_list" },
          "tail": { "type": "integer", "description": "How many trailing lines to return (default 40)" }
        },
        "required": ["id"]
      }),
    ),
    make_tool(
      "job_kill",
      "Stop a running background job.",
      json!({
        "type": "object",
        "properties": {
          "id": { "type": "integer", "description": "Job id to stop" }
        },
        "required": ["id"]
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
      "ask_user_question",
      "Ask the user for a decision or a missing detail instead of guessing. \
       Use it when the answer materially changes what you build and you cannot \
       infer it. Do NOT use it for things you can determine yourself by \
       reading files or running a command. In a non-interactive run this \
       returns a refusal telling you to pick a sensible default and say so — \
       do not ask twice.",
      json!({
        "type": "object",
        "properties": {
          "question": { "type": "string", "description": "The specific question to ask" },
          "options": {
            "type": "array",
            "description": "Optional choices. Put your recommendation first.",
            "items": {
              "type": "object",
              "properties": {
                "label":       { "type": "string", "description": "Short option label" },
                "description": { "type": "string", "description": "One sentence on the tradeoff" }
              },
              "required": ["label"]
            }
          }
        },
        "required": ["question"]
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

/// `name(required, [optional])` from a tool's JSON-Schema parameters.
///
/// A full schema dump is the obvious thing and the wrong one: it is long, and
/// the mistake being corrected is almost always the name or a missing
/// argument, both of which a signature shows at a glance.
pub fn signature_of(tool: &Tool) -> String {
  let params = &tool.function.parameters;
  let required: Vec<&str> = params
    .get("required")
    .and_then(Value::as_array)
    .map(|a| a.iter().filter_map(Value::as_str).collect())
    .unwrap_or_default();

  let mut shown: Vec<String> = required.iter().map(|r| (*r).to_string()).collect();
  if let Some(props) = params.get("properties").and_then(Value::as_object) {
    for key in props.keys() {
      if !required.contains(&key.as_str()) {
        shown.push(format!("[{key}]"));
      }
    }
  }
  format!("{}({})", tool.function.name, shown.join(", "))
}

/// Levenshtein distance, two rows.
///
/// Written out rather than pulled in as a dependency: one short function on an
/// error path does not justify a crate, and `cargo deny` has fewer things to
/// have an opinion about.
fn edit_distance(a: &str, b: &str) -> usize {
  let b_chars: Vec<char> = b.chars().collect();
  let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
  let mut cur = vec![0usize; b_chars.len() + 1];

  for (i, ac) in a.chars().enumerate() {
    cur[0] = i + 1;
    for (j, bc) in b_chars.iter().enumerate() {
      let substitution = prev[j] + usize::from(ac != *bc);
      cur[j + 1] = substitution.min(prev[j + 1] + 1).min(cur[j] + 1);
    }
    std::mem::swap(&mut prev, &mut cur);
  }
  prev[b_chars.len()]
}

/// Why a tool name does not resolve, naming the nearest real one.
///
/// "Unknown tool: x" alone left the model to guess again from the same schema
/// list it had already misread. A suggestion has a floor, though: one that
/// shares almost nothing is worse than none, because it sends the model down a
/// wrong path with false confidence (`docs/architecture/L1-engine.md` §4.6.6).
pub fn unknown_tool_message(name: &str) -> String {
  let tools = system_tools();
  let nearest = tools
    .iter()
    .map(|t| (edit_distance(name, &t.function.name), t))
    .min_by_key(|(d, _)| *d)
    // Accept a correction only while it is plausibly a typo of that name.
    .filter(|(d, t)| d * 3 <= t.function.name.len().max(name.len()));

  match nearest {
    Some((_, tool)) => format!(
      "Unknown tool `{}`. The closest real tool is `{}`. Call that instead; \
       do not retry this name.",
      name,
      signature_of(tool)
    ),
    None => format!(
      "Unknown tool `{}`. Available tools: {}. Call one of those; do not retry \
       this name.",
      name,
      tools
        .iter()
        .map(|t| t.function.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
    ),
  }
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
  // job_list / job_output only read registry state and a log file.
  matches!(
    tool_name,
    "read_file"
      | "read_image"
      | "list_dir"
      | "glob"
      | "grep"
      | "job_list"
      | "job_output"
      | "harness_inspect"
      // Network reads. They change nothing locally and are slow, so fanning
      // several out at once is exactly where concurrency pays.
      | "web_search"
      | "web_fetch"
  )
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

/// The out-of-network tools, offered only when `[research]` is configured.
///
/// Kept out of `system_tools()` on purpose: a tool the model can see but never
/// use costs a schema slot and a wasted turn every time it reasonably tries
/// one. Registration is the honest signal that the capability exists.
pub fn research_tools() -> Vec<Tool> {
  vec![
    make_tool(
      "web_search",
      "Find candidate sources on the web. Returns titles, URLs, publication \
       dates and engine-written snippets — NOT the pages themselves. A snippet \
       is a lead, not evidence: before resting any conclusion on a result, \
       open it with web_fetch and cite what the page actually says. Prefer \
       specific queries; for anything time-sensitive, say the date range you \
       need in the query.",
      json!({
        "type": "object",
        "properties": {
          "query": { "type": "string", "description": "What to search for" },
          "max_results": {
            "type": "integer",
            "description": "How many results to return (1-20); defaults to the configured value"
          },
          "recency_days": {
            "type": "integer",
            "description": "Only results published within this many days (1-365). \
                            Sets the publication date on every hit, which is otherwise \
                            usually unknown. Use it for anything time-sensitive — \
                            prices, news, releases — and say so in the answer."
          }
        },
        "required": ["query"]
      }),
    ),
    make_tool(
      "web_fetch",
      "Read the actual text of one web page, so a conclusion can rest on it \
       rather than on a search snippet. Returns the extracted content together \
       with the URL and the time it was fetched — cite both. If the page \
       cannot be read this FAILS rather than returning nothing: say the source \
       could not be opened, and do not substitute a snippet for it.",
      json!({
        "type": "object",
        "properties": {
          "url": {
            "type": "string",
            "description": "Absolute http:// or https:// URL"
          }
        },
        "required": ["url"]
      }),
    ),
  ]
}

/// Apply a Skill's `allowed_tools` whitelist to the effective tool surface.
///
/// Returns the narrowed set plus any declared name that matched nothing, so a
/// caller can name the typo instead of silently honouring it.
///
/// **This can only remove.** A skill listing `run_shell` does not thereby gain
/// `run_shell`: the name has to already be on the surface, and every surviving
/// tool still passes the policy gate on each call. Declaring a need is not the
/// same as being granted it — the whitelist is a filter over what the host
/// already decided to offer, never a source of authority.
pub fn narrow_to_skill(tools: Vec<Tool>, allowed: &[String]) -> (Vec<Tool>, Vec<String>) {
  let present: std::collections::HashSet<&str> =
    tools.iter().map(|t| t.function.name.as_str()).collect();
  let unknown: Vec<String> = allowed
    .iter()
    .filter(|name| !present.contains(name.as_str()))
    .cloned()
    .collect();
  let kept = tools
    .into_iter()
    .filter(|t| allowed.iter().any(|name| name == &t.function.name))
    .collect();
  (kept, unknown)
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
    // Discovery tools are pure reads and are the most common thing to fan out.
    assert!(is_parallel_readonly("glob"));
    assert!(is_parallel_readonly("grep"));
    // Writes / shell / delegation must never be parallelized.
    assert!(!is_parallel_readonly("write_file"));
    assert!(!is_parallel_readonly("run_shell"));
    assert!(!is_parallel_readonly("create_skill"));
    assert!(!is_parallel_readonly("invoke_agent"));
    assert!(!is_parallel_readonly("load_skill"));
    // Asking blocks on a human; running several at once would interleave prompts.
    assert!(!is_parallel_readonly("ask_user_question"));
  }

  #[test]
  fn a_signature_shows_required_arguments_then_optional_ones() {
    let tools = system_tools();
    let find = |n: &str| {
      tools
        .iter()
        .find(|t| t.function.name == n)
        .map(signature_of)
    };
    assert_eq!(
      find("edit_file").as_deref(),
      Some("edit_file(path, old_text, new_text)")
    );
    // `background` is optional, and saying so is the point: the model's error
    // is usually a missing required argument, not an omitted optional one.
    assert_eq!(
      find("run_shell").as_deref(),
      Some("run_shell(command, [background])")
    );
  }

  #[test]
  fn a_typo_is_corrected_to_the_nearest_real_tool() {
    let msg = unknown_tool_message("read_fil");
    assert!(msg.contains("read_file(path)"), "{msg}");
    assert!(msg.contains("do not retry"), "{msg}");
  }

  #[test]
  fn a_name_resembling_nothing_gets_the_list_rather_than_a_wrong_guess() {
    // A suggestion that shares almost nothing is worse than none: it sends the
    // model down a wrong path with false confidence.
    let msg = unknown_tool_message("xyzzy");
    assert!(!msg.contains("closest"), "{msg}");
    assert!(msg.contains("read_file"), "must list what exists: {msg}");
    assert!(msg.contains("run_shell"), "{msg}");

    // An MCP-shaped name resembles no built-in, so it must not be "corrected"
    // into one -- it reaches this path only when it was never registered.
    let mcp = unknown_tool_message("mcp__github__create_issue");
    assert!(!mcp.contains("closest"), "{mcp}");
  }

  #[test]
  fn edit_distance_is_symmetric_and_zero_on_equality() {
    assert_eq!(edit_distance("grep", "grep"), 0);
    assert_eq!(edit_distance("grep", "grp"), 1);
    assert_eq!(edit_distance("abc", "xyz"), 3);
    assert_eq!(edit_distance("", "abc"), 3);
    assert_eq!(edit_distance("abc", ""), 3);
    assert_eq!(edit_distance("glob", "grep"), edit_distance("grep", "glob"));
  }

  /// The asymmetry this closed: `filter_by_allowed` enforced a SubAgent
  /// template's whitelist from day one, while the identically-named Skill
  /// frontmatter field was parsed and thrown away. Same word, two meanings —
  /// one a permission boundary, the other decoration.
  #[test]
  fn a_skill_whitelist_removes_what_it_does_not_name() {
    let surface = vec![
      make_tool("read_file", "", json!({})),
      make_tool("run_shell", "", json!({})),
      make_tool("write_file", "", json!({})),
    ];
    let allowed = vec!["read_file".to_string(), "write_file".to_string()];
    let (kept, unknown) = narrow_to_skill(surface, &allowed);
    let names: Vec<&str> = kept.iter().map(|t| t.function.name.as_str()).collect();
    assert_eq!(names, vec!["read_file", "write_file"]);
    assert!(unknown.is_empty());
  }

  /// Declaring is not granting. A skill cannot conjure a tool the host never
  /// offered — the whitelist filters the surface, it does not add to it.
  #[test]
  fn a_skill_cannot_grant_itself_a_tool_that_is_not_offered() {
    let surface = vec![make_tool("read_file", "", json!({}))];
    let allowed = vec!["read_file".to_string(), "launch_missiles".to_string()];
    let (kept, unknown) = narrow_to_skill(surface, &allowed);
    assert_eq!(kept.len(), 1, "the surface cannot grow: {kept:?}");
    assert_eq!(
      unknown,
      vec!["launch_missiles".to_string()],
      "an unmatched name must be reported, not silently honoured"
    );
  }

  /// A whitelist that matches nothing leaves no tools. That is a legitimate
  /// (if useless) configuration, so it is honoured — but the caller is told,
  /// because it otherwise looks like a model that stopped calling tools.
  #[test]
  fn a_whitelist_matching_nothing_is_honoured_and_reported() {
    let surface = vec![make_tool("read_file", "", json!({}))];
    let allowed = vec!["nope".to_string()];
    let (kept, unknown) = narrow_to_skill(surface, &allowed);
    assert!(kept.is_empty());
    assert_eq!(unknown.len(), 1);
  }
}
