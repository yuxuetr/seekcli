//! Context-aware Error Recovery hint injection.
//!
//! When a tool result signals a failure, returning the raw error is not
//! enough: the model tends to follow the path of least resistance — guessing
//! a new argument and blindly retrying — instead of running a proper
//! debugging SOP. This module classifies the failure by tool + error shape
//! and returns an *actionable* recovery hint that is appended to the tool
//! result, nudging the model toward the right next tool call.
//!
//! Hints are only produced for genuine failures; successful results return
//! `None` so the happy path carries no extra noise.

/// Inspect a tool result string and return an optional recovery hint.
///
/// `tool_name` is the tool that produced `result`. The returned hint, when
/// present, should be appended to the result before it is handed back to the
/// model.
pub fn hint_for(tool_name: &str, result: &str) -> Option<String> {
  // Denials already embed "do not retry" guidance in their own text and in
  // the system prompt; adding a recovery hint would contradict that.
  if result.starts_with("[USER DENIED]") || result.starts_with("[PATH DENIED]") {
    return None;
  }

  // Malformed-arguments failures apply to any tool.
  if result.starts_with("[BAD ARGS]") {
    return Some(
      "[Recovery] Re-emit this tool call with a single well-formed JSON \
       object for `arguments`. Check for unescaped quotes, trailing commas, \
       or missing braces."
        .to_string(),
    );
  }

  let lower = result.to_lowercase();

  match tool_name {
    "read_file" => {
      if lower.contains("failed to read file")
        || lower.contains("no such file")
        || lower.contains("missing 'path'")
      {
        return Some(
          "[Recovery] The file could not be read. Do NOT guess another path. \
           First call list_dir on the parent directory (or run_shell with \
           `find . -name '<file>'`) to confirm the exact path, then retry \
           read_file."
            .to_string(),
        );
      }
    }
    "list_dir" => {
      if lower.contains("failed to read directory") || lower.contains("missing 'path'") {
        return Some(
          "[Recovery] The directory could not be listed. Verify it exists with \
           run_shell `ls -la <parent>` before retrying, or list a parent \
           directory to discover the correct name."
            .to_string(),
        );
      }
    }
    "write_file" => {
      if lower.contains("missing 'path'") || lower.contains("missing 'content'") {
        return Some(
          "[Recovery] write_file requires BOTH `path` and `content`. Re-emit \
           the call with both fields populated."
            .to_string(),
        );
      }
      if lower.contains("failed to create parent")
        || lower.contains("failed to write")
        || lower.contains("permission denied")
      {
        return Some(
          "[Recovery] The write failed (permissions or invalid path). Confirm \
           the target directory with list_dir and ensure the path is inside \
           the working directory before retrying."
            .to_string(),
        );
      }
    }
    "run_shell" => {
      if lower.contains("command not found") || lower.contains("not found") {
        return Some(
          "[Recovery] The command was not found. Do NOT retry the same command. \
           Verify the tool exists first (e.g. run_shell `command -v <tool>` or \
           `which <tool>`); if missing, choose an alternative or install path."
            .to_string(),
        );
      }
      if lower.contains("command failed with exit code") {
        return Some(
          "[Recovery] The command exited non-zero. Read the STDERR above for \
           the root cause before retrying — adjust the command rather than \
           re-running it unchanged."
            .to_string(),
        );
      }
    }
    _ => {}
  }

  // Generic dispatcher-level failure (e.g. "Error executing tool ...").
  if result.starts_with("Error executing tool ") {
    return Some(
      "[Recovery] This tool call errored. Inspect the message above and gather \
       missing information (read_file / list_dir) before retrying with \
       corrected arguments."
        .to_string(),
    );
  }

  None
}

/// Append a recovery hint to `result` if the failure is recognized.
/// Returns the (possibly augmented) result.
pub fn augment(tool_name: &str, result: String) -> String {
  match hint_for(tool_name, &result) {
    Some(hint) => format!("{result}\n\n{hint}"),
    None => result,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn no_hint_on_success() {
    assert!(hint_for("read_file", "fn main() {}").is_none());
    assert!(hint_for("write_file", "Successfully wrote to foo.txt").is_none());
    assert!(hint_for("list_dir", "a.rs\nb.rs\n").is_none());
  }

  #[test]
  fn no_hint_on_denials() {
    assert!(hint_for("run_shell", "[USER DENIED] refused: rm -rf /").is_none());
    assert!(hint_for("write_file", "[PATH DENIED] outside cwd").is_none());
  }

  #[test]
  fn read_file_not_found_gets_hint() {
    let r = "Error executing tool read_file: Failed to read file: nope.rs: No such file";
    let h = hint_for("read_file", r).expect("should hint");
    assert!(h.contains("list_dir") || h.contains("find"));
  }

  #[test]
  fn shell_command_not_found_gets_hint() {
    let r = "Command failed with exit code: 127.\nSTDERR:\nsh: frobnicate: command not found\n";
    let h = hint_for("run_shell", r).expect("should hint");
    assert!(h.contains("command -v") || h.contains("which"));
  }

  #[test]
  fn bad_args_gets_hint() {
    let h = hint_for("write_file", "[BAD ARGS] not valid JSON").expect("should hint");
    assert!(h.contains("JSON"));
  }

  #[test]
  fn augment_appends_only_on_failure() {
    let ok = augment("read_file", "content".to_string());
    assert_eq!(ok, "content");
    let bad = augment("read_file", "Error executing tool read_file: x".to_string());
    assert!(bad.contains("[Recovery]"));
  }
}
