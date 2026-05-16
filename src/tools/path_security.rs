//! Workspace path containment checks for filesystem-mutating tools.
//!
//! Hooked by `tools/fs.rs::write_file`. Goal: prevent the agent from
//! overwriting files outside the current working directory (e.g. shell
//! rc files, ssh keys) through model error or prompt injection.
//!
//! Read operations are intentionally unrestricted — a malicious model
//! already has `run_shell` for exfiltration, so a read whitelist would
//! create false security without actually limiting capability.

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

/// Ensure that `path` resolves inside the current working directory subtree.
/// Errors with a descriptive message otherwise. Works for paths that do not
/// yet exist (so it can guard `write_file` to a new file).
pub fn ensure_within_cwd(path: &str) -> Result<()> {
  let cwd = std::env::current_dir().context("cannot determine current directory")?;
  let cwd_root = cwd.canonicalize().unwrap_or(cwd);

  let target = Path::new(path);
  let absolute = if target.is_absolute() {
    target.to_path_buf()
  } else {
    cwd_root.join(target)
  };

  let resolved = normalize(&absolute);

  if !resolved.starts_with(&cwd_root) {
    anyhow::bail!(
      "Path '{}' resolves outside the workspace.\n\
       Workspace root: {}\n\
       Resolved path:  {}\n\
       Writes are restricted to the current working directory subtree. \
       If you need to write outside, ask the user to cd into the target directory first.",
      path,
      cwd_root.display(),
      resolved.display()
    );
  }
  Ok(())
}

/// Collapse `..` and `.` lexically without requiring the path to exist.
/// We do NOT follow symlinks here — that would require canonicalize, which
/// fails on non-existent files. For `write_file` this is the right tradeoff.
fn normalize(path: &Path) -> PathBuf {
  let mut out = PathBuf::new();
  for component in path.components() {
    match component {
      Component::ParentDir => {
        out.pop();
      }
      Component::CurDir => {}
      c => out.push(c.as_os_str()),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn allows_relative_inside_cwd() {
    assert!(ensure_within_cwd("foo.txt").is_ok());
    assert!(ensure_within_cwd("./foo.txt").is_ok());
    assert!(ensure_within_cwd("sub/dir/foo.txt").is_ok());
  }

  #[test]
  fn rejects_parent_escape() {
    assert!(ensure_within_cwd("../foo.txt").is_err());
    assert!(ensure_within_cwd("foo/../../bar.txt").is_err());
  }

  #[test]
  fn rejects_absolute_outside() {
    assert!(ensure_within_cwd("/etc/passwd").is_err());
    assert!(ensure_within_cwd("/tmp/foo").is_err());
  }

  #[test]
  fn normalize_collapses_dots() {
    assert_eq!(
      normalize(Path::new("/a/b/../c/./d")),
      PathBuf::from("/a/c/d")
    );
  }
}
