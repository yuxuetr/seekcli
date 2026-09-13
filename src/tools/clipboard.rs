//! Reading an image out of the system clipboard.
//!
//! This is **user input, not a capability** — the model cannot reach the user's
//! clipboard on its own, so `/paste` is the same kind of thing as typing a
//! line. That distinction is what keeps it clear of
//! [design-principles §1.1](../../docs/architecture/design-principles.md), which
//! excludes the client fetching capabilities *on the model's behalf*.
//!
//! The whole point is zero path management: screenshot, `Cmd+Shift+4`,
//! `/paste`. Anything that makes the user name a file defeats it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// What came off the clipboard.
pub struct Grab {
  pub path: PathBuf,
  pub media_type: String,
  pub bytes: usize,
}

/// Read the clipboard image into `dir`, returning where it landed.
///
/// macOS only for now; elsewhere this reports that rather than pretending the
/// clipboard was empty, so the user can tell "no image" from "not supported".
pub fn grab_image(dir: &Path) -> Result<Grab> {
  if !cfg!(target_os = "macos") {
    anyhow::bail!(
      "reading the clipboard is implemented for macOS only; on this platform, \
       attach the image through a tool instead"
    );
  }
  std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
  let path = dir.join(format!("paste-{}.png", uuid::Uuid::new_v4()));

  // `«class PNGf»` is the pasteboard type a screenshot lands in. Writing from
  // AppleScript rather than piping keeps the bytes binary-safe.
  let script = format!(
    r#"try
  set theData to the clipboard as «class PNGf»
  set theFile to (POSIX file "{}")
  set theOpenFile to open for access theFile with write permission
  set eof theOpenFile to 0
  write theData to theOpenFile
  close access theOpenFile
  return "ok"
on error errMsg
  return "ERROR:" & errMsg
end try"#,
    path.display()
  );

  let out = std::process::Command::new("osascript")
    .arg("-e")
    .arg(&script)
    .output()
    .context("cannot run osascript")?;
  let reply = String::from_utf8_lossy(&out.stdout);

  if let Some(detail) = reply.trim().strip_prefix("ERROR:") {
    // Almost always "the clipboard does not contain image data", which is a
    // normal thing for a user to do by accident.
    let _ = std::fs::remove_file(&path);
    anyhow::bail!("the clipboard has no image ({})", detail.trim());
  }

  let bytes = std::fs::metadata(&path)
    .map(|m| m.len() as usize)
    .unwrap_or(0);
  if bytes == 0 {
    // A zero-byte file is worse than an error: it would be base64'd into an
    // empty image the model silently cannot read.
    let _ = std::fs::remove_file(&path);
    anyhow::bail!("the clipboard image came back empty");
  }

  Ok(Grab {
    path,
    media_type: "image/png".to_string(),
    bytes,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Whatever the clipboard holds, a failure must not leave a zero-byte file
  /// behind for a later turn to pick up as a valid image.
  #[test]
  fn a_failed_grab_leaves_no_file_behind() {
    let dir = std::env::temp_dir().join(format!("seekcli_clip_{}", uuid::Uuid::new_v4()));
    // Either it succeeds (this machine has an image in the clipboard) or it
    // fails; both are fine, but a failure must clean up after itself.
    match grab_image(&dir) {
      Ok(g) => assert!(g.bytes > 0, "a successful grab must have bytes"),
      Err(_) => {
        let leftovers = std::fs::read_dir(&dir)
          .map(|d| d.flatten().count())
          .unwrap_or(0);
        assert_eq!(leftovers, 0, "a failed grab left a file behind");
      }
    }
    std::fs::remove_dir_all(&dir).ok();
  }
}
