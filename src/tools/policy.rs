//! The single gate every tool call passes through.
//!
//! Before this existed the boundary was inconsistent in two ways that the
//! evaluation called out as defects rather than trade-offs
//! (`docs/evaluation/2026-08-harness-gap-analysis.md` L3-1 / L3-2):
//!
//! * **No mode actually restricted anything.** The only switch that sounded
//!   like one was Plan Mode, which is really state externalisation (its prompt
//!   instructs the model to write PLAN.md / TODO.md) — so there was no way to
//!   say "investigate, change nothing". `Mode::ReadOnly` is that switch, under
//!   its own name rather than borrowed from a feature that means something
//!   else.
//! * **Writes were path-checked, shell was not.** `write_file` could not
//!   escape the workspace, but `sh -c "cat > ~/.ssh/authorized_keys"` sailed
//!   past, so the whitelist only ever stopped honest mistakes.
//!
//! The point is not that the boundary was drawn too small — a small boundary
//! is a legitimate choice, and `security-model.md` documents where it sits.
//! The point is that the drawn boundary and the enforced boundary differed,
//! and users calibrate their trust against the drawn one.
//!
//! Scope is unchanged: still no OS sandbox, still no shell AST parsing. What
//! changes is that everything claimed here is actually checked, and that the
//! checks apply to shell too.

use std::sync::Mutex;

use serde_json::Value;

use super::approval::{self, Decision};

/// What the agent is allowed to change right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
  Normal,
  /// `--read-only` / `/readonly on` — investigate and report, change nothing.
  ///
  /// Note this is *not* Plan Mode. SeekCLI's Plan Mode means "externalize
  /// state to PLAN.md / TODO.md" (stage 15) and its prompt tells the model to
  /// write those files; restricting writes there would break the feature it
  /// is named after. The dsh / Claude Code sense of plan mode — change nothing
  /// until approved — is this mode, under its own name.
  ReadOnly,
}

impl Mode {
  fn is_restricted(self) -> bool {
    matches!(self, Mode::ReadOnly)
  }

  fn label(self) -> &'static str {
    match self {
      Mode::Normal => "normal",
      Mode::ReadOnly => "read-only",
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
  Allow,
  Ask(String),
  Deny(String),
}

static MODE: Mutex<Mode> = Mutex::new(Mode::Normal);

pub fn set_mode(mode: Mode) {
  if let Ok(mut guard) = MODE.lock() {
    *guard = mode;
  }
}

pub fn mode() -> Mode {
  match MODE.lock() {
    Ok(g) => *g,
    // A poisoned lock must not silently unlock writes.
    Err(_) => Mode::ReadOnly,
  }
}

/// Tools that can change something outside the process.
///
/// `run_shell` is here because a command's effects cannot be known without
/// parsing it. In a restricted mode it is not refused outright — see
/// `read_only_obstacle` — but it is never waved through.
/// Whether a tool can change something outside the process.
///
/// Takes the foreign tool's own declaration rather than offering a
/// convenience overload without it: a no-argument version is exactly what
/// would get reached for by accident, and it is the unsafe default.
///
/// MCP tools arrive from configuration with arbitrary names — `mcp__fs__
/// write_file` is not in any list we maintain, so name matching alone let one
/// straight through `--read-only` (found by the stage 28 end-to-end check,
/// which created the file it was told not to). The rule for anything we did
/// not author is therefore inverted: **assume it writes** unless the server
/// explicitly annotated it `readOnlyHint`.
pub fn is_mutating_with(tool: &str, declared_read_only: Option<bool>) -> bool {
  if crate::mcp::is_mcp_tool(tool) {
    return !declared_read_only.unwrap_or(false);
  }
  matches!(
    tool,
    "write_file" | "edit_file" | "run_shell" | "create_skill"
  )
}

/// Commands whose whole purpose is to report. Anything not on this list is
/// refused in a restricted mode, because an allowlist that guesses is worse
/// than one that is small.
const READ_ONLY_COMMANDS: &[&str] = &[
  "ls", "cat", "head", "tail", "wc", "find", "grep", "rg", "fd", "pwd", "echo", "date", "which",
  "file", "stat", "du", "df", "ps", "env", "uname", "tree", "diff", "cmp", "sort", "uniq", "cut",
  "awk", "sed", "jq", "basename", "dirname", "realpath", "readlink", "git", "cargo", "true",
];

/// Sub-commands of otherwise-read-only tools that do mutate.
const MUTATING_SUBCOMMANDS: &[(&str, &[&str])] = &[
  (
    "git",
    &[
      "commit",
      "push",
      "merge",
      "rebase",
      "reset",
      "checkout",
      "clean",
      "rm",
      "mv",
      "apply",
      "cherry-pick",
      "revert",
      "stash",
      "tag",
      "branch",
      "add",
      "init",
      "clone",
      "fetch",
      "pull",
    ],
  ),
  (
    "cargo",
    &[
      "build", "run", "install", "publish", "clean", "fix", "add", "remove", "update",
    ],
  ),
];

/// Redirects write regardless of which command precedes them, so their
/// presence disqualifies a command from being called read-only.
fn has_redirect(cmd: &str) -> bool {
  cmd.contains('>')
}

/// Why a command is not a pure report.
///
/// `shell_is_read_only` used to answer only yes/no, and the refusal message
/// could therefore say nothing more than "not available". Naming the obstacle
/// is what lets the model switch to a reporting command instead of concluding
/// the whole tool is gone (`docs/architecture/L1-engine.md` §4.6.3).
#[derive(Debug, Clone, PartialEq)]
enum NotReadOnly {
  /// A redirect writes whatever program precedes it.
  Redirect,
  NoCommand,
  /// Unparsable quoting, or no program word to judge.
  Unjudgeable(String),
  UnknownProgram {
    part: String,
    program: String,
  },
  MutatingSubcommand {
    program: String,
    sub: String,
  },
}

impl NotReadOnly {
  /// One sentence naming what disqualified the command.
  fn explain(&self) -> String {
    match self {
      Self::Redirect => {
        "it redirects output (`>`), which writes whatever program precedes it".to_string()
      }
      Self::NoCommand => "no command was given".to_string(),
      Self::Unjudgeable(part) => {
        format!("`{part}` cannot be judged (unparsable quoting, or no program word)")
      }
      Self::UnknownProgram { part, program } => format!(
        "`{program}` is not a known reporting command, so `{part}` cannot be assumed read-only"
      ),
      Self::MutatingSubcommand { program, sub } => {
        format!("`{program} {sub}` changes state, unlike other `{program}` sub-commands")
      }
    }
  }
}

/// The first reason `cmd` is not a pure report, or `None` when it is one.
///
/// Evaluation order is identical to the predicate it backs, so the verdict is
/// unchanged — only the explanation is new.
fn read_only_obstacle(cmd: &str) -> Option<NotReadOnly> {
  if has_redirect(cmd) {
    return Some(NotReadOnly::Redirect);
  }
  let parts = split_subcommands(cmd);
  if parts.is_empty() {
    return Some(NotReadOnly::NoCommand);
  }
  for part in &parts {
    let argv = match shlex::split(part) {
      Some(v) => v,
      // Unparsable quoting: refuse rather than guess.
      None => return Some(NotReadOnly::Unjudgeable(part.clone())),
    };
    let Some(program) = argv.first() else {
      return Some(NotReadOnly::Unjudgeable(part.clone()));
    };
    let program = program.rsplit('/').next().unwrap_or(program);
    if !READ_ONLY_COMMANDS.contains(&program) {
      return Some(NotReadOnly::UnknownProgram {
        part: part.clone(),
        program: program.to_string(),
      });
    }
    // `git status` reports; `git push` does not.
    if let Some((_, subs)) = MUTATING_SUBCOMMANDS.iter().find(|(p, _)| *p == program) {
      let first_arg = argv.iter().skip(1).find(|a| !a.starts_with('-'));
      if let Some(sub) = first_arg
        && subs.contains(&sub.as_str())
      {
        return Some(NotReadOnly::MutatingSubcommand {
          program: program.to_string(),
          sub: sub.clone(),
        });
      }
    }
  }
  None
}

/// Split a command line into the individual commands it runs.
///
/// `ls; rm -rf /tmp/x` used to be classified as one string, so the harmless
/// prefix decided the verdict for the destructive suffix. Separators handled:
/// `;` `&&` `||` `|` `&`, plus the bodies of `$(...)` and backticks, which are
/// commands in their own right.
pub fn split_subcommands(cmd: &str) -> Vec<String> {
  let mut parts: Vec<String> = Vec::new();
  let mut current = String::new();
  let mut nested: Vec<String> = Vec::new();
  let chars: Vec<char> = cmd.chars().collect();
  let mut i = 0usize;
  let mut depth = 0usize;
  let mut in_backtick = false;

  while i < chars.len() {
    let c = chars[i];
    let next = chars.get(i + 1).copied();

    if c == '$' && next == Some('(') {
      depth += 1;
      nested.push(String::new());
      i += 2;
      continue;
    }
    if c == '`' {
      if in_backtick {
        if let Some(inner) = nested.pop() {
          parts.extend(split_subcommands(&inner));
        }
        in_backtick = false;
      } else {
        nested.push(String::new());
        in_backtick = true;
      }
      i += 1;
      continue;
    }
    if c == ')' && depth > 0 {
      depth -= 1;
      if let Some(inner) = nested.pop() {
        parts.extend(split_subcommands(&inner));
      }
      i += 1;
      continue;
    }

    let sink = if depth > 0 || in_backtick {
      match nested.last_mut() {
        Some(s) => s,
        None => &mut current,
      }
    } else {
      &mut current
    };

    if depth == 0 && !in_backtick && matches!(c, ';' | '|' | '&') {
      // `&&` / `||` are two chars; consume both.
      if next == Some(c) {
        i += 1;
      }
      let done = std::mem::take(&mut current);
      if !done.trim().is_empty() {
        parts.push(done.trim().to_string());
      }
      i += 1;
      continue;
    }

    sink.push(c);
    i += 1;
  }

  // Anything left inside an unterminated nesting still counts as a command.
  for leftover in nested {
    parts.extend(split_subcommands(&leftover));
  }
  if !current.trim().is_empty() {
    parts.push(current.trim().to_string());
  }
  parts
}

/// Classify a shell command by evaluating every sub-command and keeping the
/// strictest verdict. Deny beats Ask beats Allow.
pub fn classify_command(cmd: &str) -> Decision {
  let parts = split_subcommands(cmd);
  if parts.is_empty() {
    return approval::classify(cmd);
  }
  let mut strictest = Decision::Allow;
  for part in &parts {
    match approval::classify(part) {
      Decision::Deny(r) => return Decision::Deny(attribute(&r, part, parts.len())),
      Decision::Ask(r) => {
        if matches!(strictest, Decision::Allow) {
          strictest = Decision::Ask(attribute(&r, part, parts.len()));
        }
      }
      Decision::Allow => {}
    }
  }
  strictest
}

/// Verbs that write. Used to decide whether an out-of-workspace path in a
/// shell command deserves a prompt.
const WRITE_VERBS: &[&str] = &[
  "tee", "cp", "mv", "rm", "install", "touch", "mkdir", "rmdir", "dd", "chmod", "chown", "ln",
  "truncate", "shred", "rsync",
];

/// Paths a command would write to that lie outside the workspace.
///
/// Deliberately shallow: it looks for a write verb or a redirect, then for an
/// absolute or `~` path. Full shell AST parsing stays out of scope
/// (`security-model.md` §2) — the goal is to raise the cost of walking out of
/// the workspace, not to make it impossible.
pub fn escaping_write_targets(cmd: &str) -> Vec<String> {
  let mut out = Vec::new();
  for part in split_subcommands(cmd) {
    let argv = shlex::split(&part).unwrap_or_default();
    let program = argv
      .first()
      .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
      .unwrap_or_default();
    let writes = has_redirect(&part) || WRITE_VERBS.contains(&program.as_str());
    if !writes {
      continue;
    }
    // Redirect targets are not argv words once shlex splits `>`, so scan the
    // raw text for candidates too.
    let candidates = argv.iter().skip(1).cloned().chain(
      part
        .split(['>', ' ', '\t'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()),
    );
    for token in candidates {
      if token.starts_with('-') {
        continue;
      }
      if !(token.starts_with('/') || token.starts_with("~/") || token == "~") {
        continue;
      }
      if super::path_security::ensure_within_cwd(&expand_home(&token)).is_err()
        && !out.contains(&token)
      {
        out.push(token);
      }
    }
  }
  out
}

fn expand_home(path: &str) -> String {
  match path.strip_prefix("~/") {
    Some(rest) => match std::env::var("HOME") {
      Ok(home) => format!("{}/{}", home, rest),
      Err(_) => path.to_string(),
    },
    None => path.to_string(),
  }
}

/// Why a tool is wholly unavailable, plus what remains available.
///
/// Naming the alternatives matters as much as the refusal: a model told only
/// "not available" tends to stop investigating, while one told which tools
/// still work continues with them.
fn unavailable_tool_reason(tool: &str, mode: Mode) -> String {
  format!(
    "`{}` is not available in {} mode, and no approval can enable it. \
     Still available: read_file, list_dir, glob, grep, and `run_shell` for \
     reporting commands. Investigate and report instead of changing anything; \
     do not retry this call.",
    tool,
    mode.label()
  )
}

/// Why one shell command was refused, when the tool itself still works.
///
/// The message this replaced said `run_shell` was "not available", which is
/// false — reporting commands pass. The model therefore abandoned shell
/// entirely instead of narrowing to a command that would have run
/// (`docs/architecture/L1-engine.md` §4.6.3).
fn restricted_shell_reason(mode: Mode, obstacle: &NotReadOnly) -> String {
  format!(
    "`run_shell` works in {} mode, but only for commands that just report. \
     This one was refused because {}. Retry with a reporting command instead \
     (ls, cat, head, grep, find, git status, git diff, ...); do not retry this \
     command unchanged.",
    mode.label(),
    obstacle.explain()
  )
}

/// Name the sub-command that decided the verdict.
///
/// Only when the line has more than one: for a single command the reason
/// already refers to the whole thing, and repeating it is noise. Without this
/// the model saw `ls; rm -rf /` refused for "recursive delete" with no way to
/// tell which half to drop, so its cheapest next move was to re-send the
/// whole line.
fn attribute(reason: &str, part: &str, count: usize) -> String {
  if count < 2 {
    return reason.to_string();
  }
  format!("{reason} — triggered by `{part}`")
}

/// The gate. Every tool call goes through here before it executes.
/// The gate. `declared_read_only` carries a foreign tool's own annotation;
/// `None` means "we did not author this and it made no claim", which is
/// treated as writing.
pub fn check_with(tool: &str, args: &Value, declared_read_only: Option<bool>) -> Verdict {
  let current = mode();

  // 1. Mode gate.
  if current.is_restricted() && is_mutating_with(tool, declared_read_only) {
    if tool == "run_shell" {
      // An absent `command` is still a refusal, exactly as before: an empty
      // string has no sub-commands, so it is not a report either.
      let cmd = args.get("command").and_then(Value::as_str).unwrap_or("");
      if let Some(obstacle) = read_only_obstacle(cmd) {
        return Verdict::Deny(restricted_shell_reason(current, &obstacle));
      }
    } else {
      return Verdict::Deny(unavailable_tool_reason(tool, current));
    }
  }

  // 2. Path gate.
  if let Some(path) = args.get("path").and_then(Value::as_str)
    && is_mutating_with(tool, declared_read_only)
    && let Err(e) = super::path_security::ensure_within_cwd(path)
  {
    return Verdict::Deny(format!("{e}"));
  }

  // 3. Command gate.
  if tool == "run_shell"
    && let Some(cmd) = args.get("command").and_then(Value::as_str)
  {
    match classify_command(cmd) {
      Decision::Deny(reason) => return Verdict::Deny(reason),
      Decision::Ask(reason) => return Verdict::Ask(reason),
      Decision::Allow => {}
    }
    let escaping = escaping_write_targets(cmd);
    if !escaping.is_empty() {
      return Verdict::Ask(format!(
        "writes outside the workspace: {}",
        escaping.join(", ")
      ));
    }
  }

  Verdict::Allow
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  /// The yes/no view of `read_only_obstacle`, which is what the mode gate
  /// ultimately asks. Kept test-local: the shipped gate needs the obstacle's
  /// payload to explain itself, so a callerless predicate in the binary would
  /// be dead weight.
  fn shell_is_read_only(cmd: &str) -> bool {
    read_only_obstacle(cmd).is_none()
  }

  #[test]
  fn subcommands_are_split_on_every_separator() {
    assert_eq!(
      split_subcommands("ls; rm -rf /tmp/x"),
      vec!["ls", "rm -rf /tmp/x"]
    );
    assert_eq!(split_subcommands("a && b || c"), vec!["a", "b", "c"]);
    assert_eq!(split_subcommands("cat f | grep x"), vec!["cat f", "grep x"]);
    // Command substitution is a command in its own right.
    assert!(
      split_subcommands("echo $(rm -rf /tmp/x)")
        .iter()
        .any(|p| p.contains("rm -rf"))
    );
    assert!(
      split_subcommands("echo `sudo id`")
        .iter()
        .any(|p| p.contains("sudo"))
    );
  }

  #[test]
  fn a_harmless_prefix_no_longer_decides_the_verdict() {
    // The bug this fixes: `classify` saw one string, and `ls` at the front
    // made the whole line look benign.
    match classify_command("ls; rm -rf /") {
      Decision::Deny(_) | Decision::Ask(_) => {}
      Decision::Allow => panic!("the rm must decide, not the ls"),
    }
    match classify_command("echo hi && sudo reboot") {
      Decision::Ask(_) | Decision::Deny(_) => {}
      Decision::Allow => panic!("sudo after && must still be caught"),
    }
  }

  #[test]
  fn strictest_verdict_wins_across_subcommands() {
    assert_eq!(classify_command("ls -la"), Decision::Allow);
    assert!(matches!(classify_command("ls && pwd"), Decision::Allow));
  }

  #[test]
  fn read_only_shell_accepts_reporting_commands_only() {
    assert!(shell_is_read_only("ls -la"));
    assert!(shell_is_read_only("git status"));
    assert!(shell_is_read_only("cat a | grep b"));
    assert!(shell_is_read_only("/usr/bin/wc -l f"));

    // Mutating sub-commands of otherwise-read-only programs.
    assert!(!shell_is_read_only("git push"));
    assert!(!shell_is_read_only("cargo build"));
    // Any redirect writes, whatever precedes it.
    assert!(!shell_is_read_only("echo hi > f"));
    assert!(!shell_is_read_only("cat a >> b"));
    // Unknown program: refuse rather than guess.
    assert!(!shell_is_read_only("mystery-tool --go"));
    // One bad sub-command taints the line.
    assert!(!shell_is_read_only("ls; rm f"));
  }

  /// The defect this stage fixes: the refusal claimed `run_shell` was
  /// unavailable while the very next assertion below shows it is not, so the
  /// model abandoned shell instead of narrowing to a reporting command.
  #[test]
  fn a_restricted_shell_refusal_names_the_obstacle_not_a_missing_tool() {
    let _guard = crate::testsync::lock();
    set_mode(Mode::ReadOnly);

    let Verdict::Deny(reason) = check_with("run_shell", &json!({"command": "git push"}), None)
    else {
      set_mode(Mode::Normal);
      panic!("git push must be denied in read-only mode");
    };
    set_mode(Mode::Normal);

    // It must not claim the tool is gone -- it demonstrably is not.
    assert!(
      !reason.contains("not available"),
      "must not claim run_shell is unavailable: {reason}"
    );
    // It must name what disqualified this command,
    assert!(
      reason.contains("git push"),
      "must name the obstacle: {reason}"
    );
    // and what to do instead.
    assert!(
      reason.contains("git status"),
      "must offer a reporting alternative: {reason}"
    );
  }

  #[test]
  fn a_wholly_unavailable_tool_says_what_still_works() {
    let _guard = crate::testsync::lock();
    set_mode(Mode::ReadOnly);
    let verdict = check_with("write_file", &json!({"path": "x"}), None);
    set_mode(Mode::Normal);

    let Verdict::Deny(reason) = verdict else {
      panic!("write_file must be denied in read-only mode");
    };
    // Unlike run_shell, this one really is unavailable -- and approval cannot
    // change that, so the model should not go looking for a prompt.
    assert!(reason.contains("no approval can enable it"), "{reason}");
    assert!(
      reason.contains("read_file"),
      "must name what remains: {reason}"
    );
  }

  #[test]
  fn a_refused_line_names_which_subcommand_decided_it() {
    // Without attribution the model's cheapest next move is to re-send the
    // whole line, bringing the offending half back with it.
    let reason = match classify_command("ls; sudo reboot") {
      Decision::Ask(r) | Decision::Deny(r) => r,
      Decision::Allow => panic!("sudo must decide the verdict"),
    };
    assert!(reason.contains("sudo reboot"), "{reason}");

    // A single command needs no attribution: the reason already refers to it.
    let single = match classify_command("sudo reboot") {
      Decision::Ask(r) | Decision::Deny(r) => r,
      Decision::Allow => panic!("sudo must decide the verdict"),
    };
    assert!(!single.contains("triggered by"), "{single}");
  }

  #[test]
  fn each_obstacle_explains_itself_distinctly() {
    // A redirect is disqualifying whatever precedes it, and says so rather
    // than blaming the program.
    let redirect = read_only_obstacle("echo hi > f").map(|o| o.explain());
    assert!(
      redirect.as_deref().is_some_and(|e| e.contains("redirect")),
      "{redirect:?}"
    );

    // A mutating sub-command blames the sub-command, not `git` itself --
    // otherwise the model concludes git is off-limits.
    let sub = read_only_obstacle("git commit -m x").map(|o| o.explain());
    assert!(
      sub.as_deref().is_some_and(|e| e.contains("git commit")),
      "{sub:?}"
    );

    let unknown = read_only_obstacle("mystery-tool --go").map(|o| o.explain());
    assert!(
      unknown
        .as_deref()
        .is_some_and(|e| e.contains("mystery-tool")),
      "{unknown:?}"
    );

    assert_eq!(read_only_obstacle("git status"), None);
  }

  #[test]
  fn out_of_workspace_writes_are_detected_but_in_workspace_ones_are_not() {
    // In-workspace writes are the normal case and must not prompt.
    assert!(escaping_write_targets("echo hi > ./local.txt").is_empty());
    assert!(!escaping_write_targets("echo hi > /etc/passwd").is_empty());
    assert!(!escaping_write_targets("cp a ~/.ssh/authorized_keys").is_empty());
    assert!(!escaping_write_targets("rm -rf /var/tmp/x").is_empty());
    // Reading outside the workspace is fine -- run_shell can exfiltrate
    // anyway, so a read restriction would cost usability and buy nothing.
    assert!(escaping_write_targets("cat /etc/passwd").is_empty());
    assert!(escaping_write_targets("ls /usr/bin").is_empty());
  }

  #[test]
  fn read_only_mode_denies_mutating_tools_and_allows_reads() {
    let _guard = crate::testsync::lock();
    set_mode(Mode::ReadOnly);
    for tool in ["write_file", "edit_file", "create_skill"] {
      assert!(
        matches!(
          check_with(tool, &json!({"path": "x"}), None),
          Verdict::Deny(_)
        ),
        "{} must be denied",
        tool
      );
    }
    for tool in ["read_file", "list_dir", "glob", "grep"] {
      assert_eq!(
        check_with(tool, &json!({"path": "."}), None),
        Verdict::Allow,
        "{}",
        tool
      );
    }
    // Shell degrades rather than disappearing: reporting still works.
    assert_eq!(
      check_with("run_shell", &json!({"command": "git status"}), None),
      Verdict::Allow
    );
    assert!(matches!(
      check_with("run_shell", &json!({"command": "rm f"}), None),
      Verdict::Deny(_)
    ));
    set_mode(Mode::Normal);
  }

  #[test]
  fn normal_mode_still_gates_paths_and_commands() {
    let _guard = crate::testsync::lock();
    set_mode(Mode::Normal);
    assert!(matches!(
      check_with("write_file", &json!({"path": "/etc/hosts"}), None),
      Verdict::Deny(_)
    ));
    assert!(matches!(
      check_with("run_shell", &json!({"command": "sudo ls"}), None),
      Verdict::Ask(_)
    ));
    // The gap this stage closes: shell writing outside the workspace used to
    // be waved straight through.
    assert!(matches!(
      check_with("run_shell", &json!({"command": "echo x > ~/.zshrc"}), None),
      Verdict::Ask(_)
    ));
    assert_eq!(
      check_with("run_shell", &json!({"command": "ls -la"}), None),
      Verdict::Allow
    );
  }

  /// Regression guard for a real hole: `--read-only` let `mcp__fs__write_file`
  /// through because the name was not on any built-in list, and the end-to-end
  /// check created the file it had been told not to.
  #[test]
  fn a_foreign_tool_cannot_walk_through_read_only_mode() {
    let _guard = crate::testsync::lock();
    set_mode(Mode::ReadOnly);

    // No declaration: assume it writes.
    assert!(matches!(
      check_with("mcp__fs__write_file", &json!({}), None),
      Verdict::Deny(_)
    ));
    // Declared as a writer: denied.
    assert!(matches!(
      check_with("mcp__fs__write_file", &json!({}), Some(false)),
      Verdict::Deny(_)
    ));
    // Declared read-only by the server: allowed, so read-only mode stays
    // useful rather than blocking every configured capability.
    assert_eq!(
      check_with("mcp__fs__read_text_file", &json!({}), Some(true)),
      Verdict::Allow
    );

    set_mode(Mode::Normal);
    // Outside restricted mode a foreign tool runs normally.
    assert_eq!(
      check_with("mcp__fs__write_file", &json!({}), None),
      Verdict::Allow
    );
  }

  #[test]
  fn plan_mode_is_not_a_write_restriction() {
    let _guard = crate::testsync::lock();
    // Regression guard. Plan Mode means "externalize state to PLAN.md /
    // TODO.md" and its prompt tells the model to write them; wiring it to the
    // restricted mode would break the feature it is named after.
    set_mode(Mode::Normal);
    assert_eq!(
      check_with("write_file", &json!({"path": "PLAN.md"}), None),
      Verdict::Allow
    );
  }
}
