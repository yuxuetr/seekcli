//! Native discovery tools: `glob` (find files) and `grep` (find lines).
//!
//! Before these existed, `read_file`'s description told the model to shell out
//! to `sed` / `grep` / `head` / `tail`. That routed every search through the
//! approval policy, depended on whatever the host happened to have installed,
//! and produced output no two systems formatted alike. These two tools make
//! discovery a first-class capability instead.
//!
//! Both walk through `ignore::WalkBuilder`, which is ripgrep's engine, so
//! `.gitignore` is honoured by default — otherwise a single `glob("**/*.rs")`
//! in a Rust repo would pour `target/` into the context window.
//!
//! Both are read-only and registered as parallel-safe, so the agent loop may
//! run them concurrently with `read_file` / `list_dir`. The actual walking is
//! synchronous and IO-heavy, so it runs on `spawn_blocking` rather than
//! stalling the runtime that the concurrent batch depends on.

use anyhow::{Context, Result};
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Maximum rows either tool returns. Beyond this the model gets a count of
/// what it did not see, so it can narrow the query instead of assuming the
/// list was complete.
const MAX_RESULTS: usize = 200;

/// Longest single matched line echoed back. One minified bundle line can be
/// megabytes; truncating per line keeps a wide match from crowding out the
/// other 199 results.
const MAX_LINE_BYTES: usize = 300;

/// Walk depth guard. Deep symlink-free trees are fine, but a pathological
/// nesting should not turn a discovery call into a hang.
const MAX_DEPTH: usize = 40;

pub async fn glob(args: &Value) -> Result<String> {
  let pattern = str_arg(args, "pattern")?;
  let root = path_arg(args);
  tokio::task::spawn_blocking(move || glob_blocking(&pattern, &root))
    .await
    .context("glob task panicked")?
}

pub async fn grep(args: &Value) -> Result<String> {
  let pattern = str_arg(args, "pattern")?;
  let root = path_arg(args);
  let filter = args.get("glob").and_then(Value::as_str).map(str::to_string);
  tokio::task::spawn_blocking(move || grep_blocking(&pattern, &root, filter.as_deref()))
    .await
    .context("grep task panicked")?
}

fn glob_blocking(pattern: &str, root: &Path) -> Result<String> {
  let walker = build_walker(root, Some(pattern))?;

  // (mtime, path) so the most recently touched files come first: when a
  // search is capped, the freshest matches are the ones worth keeping.
  let mut hits: Vec<(SystemTime, PathBuf)> = Vec::new();
  let mut errors = 0usize;
  for entry in walker {
    let entry = match entry {
      Ok(e) => e,
      Err(_) => {
        errors += 1;
        continue;
      }
    };
    if !entry.file_type().is_some_and(|t| t.is_file()) {
      continue;
    }
    let mtime = entry
      .metadata()
      .and_then(|m| m.modified().map_err(Into::into))
      .unwrap_or(SystemTime::UNIX_EPOCH);
    hits.push((mtime, entry.into_path()));
  }
  hits.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));

  if hits.is_empty() {
    return Ok(format!(
      "No files match `{}` under {}.{}",
      pattern,
      root.display(),
      unreadable_suffix(errors)
    ));
  }

  let total = hits.len();
  let shown = total.min(MAX_RESULTS);
  let mut out = String::new();
  for (_, path) in hits.iter().take(shown) {
    out.push_str(&display_path(path, root));
    out.push('\n');
  }
  out.push_str(&summary("files", shown, total, errors));
  Ok(out)
}

fn grep_blocking(pattern: &str, root: &Path, filter: Option<&str>) -> Result<String> {
  // A bad regex is the model's mistake to fix, so name it precisely rather
  // than returning "no matches" and letting it conclude the code is absent.
  let matcher = match RegexMatcher::new_line_matcher(pattern) {
    Ok(m) => m,
    Err(e) => {
      return Ok(format!(
        "[BAD PATTERN] `{}` is not a valid regex: {}\n\
         Note: this is regex syntax, not shell glob. Escape regex metacharacters \
         (. * + ? ( ) [ ] {{ }} | ^ $ \\) to match them literally.",
        pattern, e
      ));
    }
  };

  let walker = build_walker(root, filter)?;
  let mut searcher: Searcher = SearcherBuilder::new().line_number(true).build();

  let mut collected: Vec<String> = Vec::new();
  let mut total = 0usize;
  let mut errors = 0usize;

  for entry in walker {
    let entry = match entry {
      Ok(e) => e,
      Err(_) => {
        errors += 1;
        continue;
      }
    };
    if !entry.file_type().is_some_and(|t| t.is_file()) {
      continue;
    }
    let path = entry.path().to_path_buf();
    let rel = display_path(&path, root);
    let mut sink = Collector {
      rel: &rel,
      out: &mut collected,
      total: &mut total,
    };
    // A single unreadable or binary file must not abort the whole search.
    if searcher.search_path(&matcher, &path, &mut sink).is_err() {
      errors += 1;
    }
  }

  if total == 0 {
    return Ok(format!(
      "No matches for `{}` under {}{}.{}",
      pattern,
      root.display(),
      filter
        .map(|g| format!(" (glob `{}`)", g))
        .unwrap_or_default(),
      unreadable_suffix(errors)
    ));
  }

  let mut out = collected.join("\n");
  out.push('\n');
  out.push_str(&summary("matches", collected.len(), total, errors));
  Ok(out)
}

/// Sink that formats each match as `path:line: content` and stops feeding the
/// buffer once the cap is reached — while still counting, so the summary can
/// report how much was withheld.
struct Collector<'a> {
  rel: &'a str,
  out: &'a mut Vec<String>,
  total: &'a mut usize,
}

impl Sink for Collector<'_> {
  type Error = std::io::Error;

  fn matched(&mut self, _searcher: &Searcher, m: &SinkMatch<'_>) -> Result<bool, Self::Error> {
    *self.total += 1;
    if self.out.len() < MAX_RESULTS {
      let line = String::from_utf8_lossy(m.bytes());
      let line = line.trim_end_matches(['\n', '\r']);
      let line = truncate_chars(line, MAX_LINE_BYTES);
      let number = m.line_number().unwrap_or(0);
      self.out.push(format!("{}:{}: {}", self.rel, number, line));
    }
    Ok(true)
  }
}

fn build_walker(root: &Path, glob: Option<&str>) -> Result<ignore::Walk> {
  if !root.exists() {
    anyhow::bail!(
      "path `{}` does not exist; use list_dir to confirm the location first",
      root.display()
    );
  }
  let mut builder = WalkBuilder::new(root);
  builder
    .hidden(false) // dotfiles are ordinary source in most repos (.github, .cargo)
    .follow_links(false) // a symlink cycle would otherwise walk forever
    .max_depth(Some(MAX_DEPTH))
    // ripgrep only applies .gitignore inside a git repo. Here the point is
    // keeping build output out of the context window, which matters just as
    // much in a plain directory that has a .gitignore.
    .require_git(false)
    // hidden(false) would otherwise walk .git itself, and one `glob("*")` in
    // a repo would return thousands of object files.
    .filter_entry(|e| e.file_name() != ".git");

  if let Some(glob) = glob {
    let mut overrides = OverrideBuilder::new(root);
    overrides
      .add(glob)
      .with_context(|| format!("invalid glob pattern `{}`", glob))?;
    let overrides = overrides
      .build()
      .with_context(|| format!("invalid glob pattern `{}`", glob))?;
    builder.overrides(overrides);
  }
  Ok(builder.build())
}

/// Paths relative to the search root when possible: the model asked about a
/// subtree, and absolute paths would eat context without adding information.
fn display_path(path: &Path, root: &Path) -> String {
  path
    .strip_prefix(root)
    .unwrap_or(path)
    .to_string_lossy()
    .into_owned()
}

/// Never silently truncate: a capped list that looks complete is worse than
/// no list, because the model will conclude the missing entries do not exist.
fn summary(unit: &str, shown: usize, total: usize, errors: usize) -> String {
  if total > shown {
    format!(
      "[{} of {} {} shown — {} more not listed. Narrow the pattern or path to see them.]{}",
      shown,
      total,
      unit,
      total - shown,
      unreadable_suffix(errors)
    )
  } else {
    format!("[{} {}]{}", total, unit, unreadable_suffix(errors))
  }
}

fn unreadable_suffix(errors: usize) -> String {
  if errors == 0 {
    String::new()
  } else {
    format!(" ({} path(s) skipped: unreadable or binary)", errors)
  }
}

fn truncate_chars(s: &str, max: usize) -> String {
  if s.len() <= max {
    return s.to_string();
  }
  let mut cut = max;
  while cut > 0 && !s.is_char_boundary(cut) {
    cut -= 1;
  }
  format!("{}… [line truncated]", &s[..cut])
}

fn str_arg(args: &Value, key: &str) -> Result<String> {
  args
    .get(key)
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
    .ok_or_else(|| anyhow::anyhow!("missing required argument `{}`", key))
}

fn path_arg(args: &Value) -> PathBuf {
  PathBuf::from(
    args
      .get("path")
      .and_then(Value::as_str)
      .filter(|s| !s.is_empty())
      .unwrap_or("."),
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;
  use std::fs;

  /// A throwaway tree with a .gitignore, so the gitignore-awareness that
  /// motivates using `ignore` over a plain walker is actually exercised.
  fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("seekcli-search-test-{}", name));
    let _ = fs::remove_dir_all(&root);
    let _ = fs::create_dir_all(root.join("src"));
    let _ = fs::create_dir_all(root.join("target/debug"));
    let _ = fs::write(root.join(".gitignore"), "/target\n");
    let _ = fs::write(
      root.join("src/main.rs"),
      "fn main() {\n  eprintln!(\"hi\");\n}\n",
    );
    let _ = fs::write(root.join("src/lib.rs"), "pub fn helper() {}\n");
    let _ = fs::write(root.join("README.md"), "# demo\nfn main is in src\n");
    let _ = fs::write(
      root.join("target/debug/main.rs"),
      "fn main() { /* build artifact */ }\n",
    );
    root
  }

  #[tokio::test]
  async fn glob_respects_gitignore() {
    let root = fixture("glob-gitignore");
    let out = match glob(&json!({ "pattern": "*.rs", "path": root.to_string_lossy() })).await {
      Ok(v) => v,
      Err(e) => panic!("glob failed: {}", e),
    };
    assert!(out.contains("src/main.rs"), "got: {}", out);
    assert!(out.contains("src/lib.rs"), "got: {}", out);
    assert!(
      !out.contains("target"),
      "gitignored build artifacts must not leak into context: {}",
      out
    );
  }

  #[tokio::test]
  async fn glob_scopes_pattern_to_a_directory() {
    let root = fixture("glob-scoped");
    let out = match glob(&json!({ "pattern": "src/*.rs", "path": root.to_string_lossy() })).await {
      Ok(v) => v,
      Err(e) => panic!("glob failed: {}", e),
    };
    assert!(out.contains("src/main.rs"), "got: {}", out);
    assert!(!out.contains("README.md"), "got: {}", out);
  }

  #[tokio::test]
  async fn glob_reports_no_matches_instead_of_empty_output() {
    let root = fixture("glob-empty");
    let out = match glob(&json!({ "pattern": "*.py", "path": root.to_string_lossy() })).await {
      Ok(v) => v,
      Err(e) => panic!("glob failed: {}", e),
    };
    assert!(out.contains("No files match"), "got: {}", out);
  }

  #[tokio::test]
  async fn grep_returns_path_line_content_and_honours_gitignore() {
    let root = fixture("grep-basic");
    let out = match grep(&json!({ "pattern": "fn main", "path": root.to_string_lossy() })).await {
      Ok(v) => v,
      Err(e) => panic!("grep failed: {}", e),
    };
    assert!(out.contains("src/main.rs:1: fn main() {"), "got: {}", out);
    assert!(!out.contains("target"), "got: {}", out);
  }

  #[tokio::test]
  async fn grep_glob_filter_restricts_searched_files() {
    let root = fixture("grep-filter");
    let out = match grep(&json!({
      "pattern": "fn main",
      "path": root.to_string_lossy(),
      "glob": "*.md"
    }))
    .await
    {
      Ok(v) => v,
      Err(e) => panic!("grep failed: {}", e),
    };
    assert!(out.contains("README.md"), "got: {}", out);
    assert!(!out.contains("src/main.rs"), "got: {}", out);
  }

  #[tokio::test]
  async fn grep_names_an_invalid_regex_rather_than_reporting_no_matches() {
    let root = fixture("grep-badre");
    let out = match grep(&json!({ "pattern": "fn main(", "path": root.to_string_lossy() })).await {
      Ok(v) => v,
      Err(e) => panic!("grep should report, not fail: {}", e),
    };
    // Reporting "no matches" here would let the model conclude the code is absent.
    assert!(out.contains("[BAD PATTERN]"), "got: {}", out);
    assert!(!out.contains("No matches"), "got: {}", out);
  }

  #[tokio::test]
  async fn missing_path_is_an_error_with_a_recovery_hint() {
    let out = grep(&json!({ "pattern": "x", "path": "/nonexistent/seekcli/xyz" })).await;
    let err = match out {
      Ok(v) => panic!("expected an error, got: {}", v),
      Err(e) => format!("{:#}", e),
    };
    assert!(err.contains("list_dir"), "unhelpful error: {}", err);
  }

  /// Runs against the real repository rather than a fixture: the failure mode
  /// this tool exists to prevent (build output flooding the context) only
  /// shows up at real-repo scale.
  #[tokio::test]
  async fn smoke_against_this_repository() {
    let repo = env!("CARGO_MANIFEST_DIR");
    let out = match glob(&json!({ "pattern": "src/**/*.rs", "path": repo })).await {
      Ok(v) => v,
      Err(e) => panic!("glob failed: {}", e),
    };
    assert!(out.contains("src/engine.rs"), "got: {}", out);
    assert!(!out.contains("/target/"), "target/ leaked: {}", out);
    assert!(!out.contains(".git/"), ".git leaked: {}", out);

    let out =
      match grep(&json!({ "pattern": "fn run_agent_loop", "glob": "*.rs", "path": repo })).await {
        Ok(v) => v,
        Err(e) => panic!("grep failed: {}", e),
      };
    assert!(out.contains("src/engine.rs:"), "got: {}", out);
  }

  #[test]
  fn summary_states_how_many_rows_were_withheld() {
    let s = summary("matches", 200, 517, 0);
    assert!(s.contains("317 more not listed"), "got: {}", s);
    let s = summary("files", 3, 3, 0);
    assert!(s.contains("[3 files]"), "got: {}", s);
  }

  #[test]
  fn long_lines_are_truncated_with_a_marker() {
    let long = "x".repeat(MAX_LINE_BYTES + 50);
    let out = truncate_chars(&long, MAX_LINE_BYTES);
    assert!(out.len() < long.len());
    assert!(out.ends_with("[line truncated]"), "got: {}", out);
  }

  #[test]
  fn truncation_never_splits_a_utf8_character() {
    // A cut landing mid-character would panic on slicing; CJK makes that likely.
    let cjk = "中".repeat(MAX_LINE_BYTES);
    let out = truncate_chars(&cjk, MAX_LINE_BYTES);
    assert!(out.starts_with('中'));
  }
}
