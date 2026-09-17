use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The third way an image can enter the conversation, and the one that fits
/// [design-principles §1.1] best: the **agent** decides to look at a file.
///
/// `/paste` is the user handing something over; MCP is an external server
/// returning one. Without this, an agent working in a repository full of
/// screenshots could not look at any of them.
///
/// Caps the size because a base64 data URI costs prompt tokens and a
/// multi-megabyte photo would blow the context on one call.
pub async fn read_image(args: &Value) -> Result<super::result::ToolOutput> {
  /// Chosen to admit ordinary screenshots while refusing camera-sized photos.
  const MAX_BYTES: usize = 4 * 1024 * 1024;

  let path = args
    .get("path")
    .and_then(|v| v.as_str())
    .context("Missing 'path' argument")?;
  let bytes = tokio::fs::read(path)
    .await
    .with_context(|| format!("Failed to read image: {path}"))?;
  if bytes.len() > MAX_BYTES {
    anyhow::bail!(
      "`{}` is {:.1} MB; the limit is {} MB. Resize it first, or describe what \
       you need from it and read a crop instead.",
      path,
      bytes.len() as f64 / (1024.0 * 1024.0),
      MAX_BYTES / (1024 * 1024)
    );
  }
  let media_type = media_type_of(&bytes)
    .with_context(|| format!("`{path}` is not a PNG, JPEG, GIF or WebP; read_file handles text"))?;

  Ok(super::result::ToolOutput {
    text: format!("[{} image, {} bytes]", media_type, bytes.len()),
    images: vec![crate::api::ImagePart {
      media_type: media_type.to_string(),
      data_base64: crate::session::base64_encode(&bytes),
    }],
  })
}

/// Identify an image by its magic bytes rather than its extension.
///
/// A `.png` that is really a JPEG would otherwise be announced to the model
/// with the wrong type, and the provider would reject or misread it.
fn media_type_of(bytes: &[u8]) -> Option<&'static str> {
  match bytes {
    [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some("image/png"),
    [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
    [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
    [
      b'R',
      b'I',
      b'F',
      b'F',
      _,
      _,
      _,
      _,
      b'W',
      b'E',
      b'B',
      b'P',
      ..,
    ] => Some("image/webp"),
    _ => None,
  }
}

pub async fn read_file(args: &Value) -> Result<String> {
  let path = args
    .get("path")
    .and_then(|v| v.as_str())
    .context("Missing 'path' argument")?;
  let content = tokio::fs::read_to_string(path)
    .await
    .context(format!("Failed to read file: {}", path))?;

  // Offload oversized reads to a temp file, returning a head+tail preview that
  // points back at the original path (the model can re-read specific ranges).
  Ok(super::offload::offload(content, Some(path)).await)
}

/// Locks held while a path is being read-modified-written.
///
/// Only `edit_file` needs this: it reads, computes a replacement, and writes
/// back, and two of those interleaving on one path loses an update silently.
/// `write_file` does not read first, so `atomic_replace` alone is enough for it.
///
/// Keyed by path, so unrelated files never wait on each other. The map only
/// grows with distinct paths touched in one process lifetime, which for a CLI
/// session is small; reclaiming entries would need refcounting for no
/// measurable gain.
static EDIT_LOCKS: Mutex<Option<BTreeMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = Mutex::new(None);

fn edit_lock(path: &Path) -> Arc<tokio::sync::Mutex<()>> {
  let key = path.to_path_buf();
  let mut guard = match EDIT_LOCKS.lock() {
    Ok(g) => g,
    Err(poisoned) => poisoned.into_inner(),
  };
  guard
    .get_or_insert_with(BTreeMap::new)
    .entry(key)
    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
    .clone()
}

/// Replace a file's contents so no reader can ever see a half-written one.
///
/// `fs::write` truncates and then writes. Anything reading at that moment —
/// another tool, a background `run_shell` job, the user's editor — sees a
/// truncated or partial file, and nothing in the result says so. Writing a
/// sibling temp file and renaming over the target makes the swap atomic:
/// `rename(2)` within one filesystem is all-or-nothing, so a reader gets the
/// old contents or the new ones and never a mixture.
///
/// The temp file is a sibling rather than in `/tmp` on purpose: `rename` is
/// only atomic within a filesystem, and a cross-device rename silently
/// degrades to copy-then-delete, which is exactly the torn window this avoids.
pub async fn atomic_replace(path: &Path, content: &str) -> Result<()> {
  let dir = path.parent().unwrap_or(Path::new("."));
  let tmp = dir.join(format!(
    ".{}.seekcli-{}.tmp",
    path.file_name().and_then(|n| n.to_str()).unwrap_or("out"),
    uuid::Uuid::new_v4()
  ));
  tokio::fs::write(&tmp, content)
    .await
    .with_context(|| format!("cannot stage {}", tmp.display()))?;
  match tokio::fs::rename(&tmp, path).await {
    Ok(()) => Ok(()),
    Err(e) => {
      // Leaving a stray dotfile behind would be a second failure on top of the
      // first, and the user would have no idea where it came from.
      let _ = tokio::fs::remove_file(&tmp).await;
      Err(e).with_context(|| format!("cannot replace {}", path.display()))
    }
  }
}

pub async fn write_file(args: &Value) -> Result<String> {
  let path = args
    .get("path")
    .and_then(|v| v.as_str())
    .context("Missing 'path' argument")?;
  let content = args
    .get("content")
    .and_then(|v| v.as_str())
    .context("Missing 'content' argument")?;

  if let Err(e) = super::path_security::ensure_within_cwd(path) {
    return Ok(format!("[PATH DENIED] {e}"));
  }

  // Ensure parent dir exists
  if let Some(parent) = std::path::Path::new(path).parent() {
    tokio::fs::create_dir_all(parent)
      .await
      .context("Failed to create parent directories")?;
  }

  // Atomic: a reader never sees a half-written file. See `atomic_replace`.
  atomic_replace(std::path::Path::new(path), content)
    .await
    .context(format!("Failed to write to file: {}", path))?;
  Ok(format!("Successfully wrote to {}", path))
}

pub async fn edit_file(args: &Value) -> Result<String> {
  let path = args
    .get("path")
    .and_then(|v| v.as_str())
    .context("Missing 'path' argument")?;
  let old_text = args
    .get("old_text")
    .and_then(|v| v.as_str())
    .context("Missing 'old_text' argument")?;
  let new_text = args
    .get("new_text")
    .and_then(|v| v.as_str())
    .context("Missing 'new_text' argument")?;

  if let Err(e) = super::path_security::ensure_within_cwd(path) {
    return Ok(format!("[PATH DENIED] {e}"));
  }

  // Held across read → apply → write. Two edits interleaving on one path would
  // otherwise lose an update with nothing to show for it: both read the same
  // contents, both compute a replacement from it, and the second write erases
  // the first. Serialised, the second edit instead finds its `old_text` gone
  // and says so — a loud, correct refusal rather than silent loss.
  let lock = edit_lock(std::path::Path::new(path));
  let _serialised = lock.lock().await;

  let content = tokio::fs::read_to_string(path)
    .await
    .context(format!("Failed to read file: {}", path))?;

  match super::edit::apply_edit(&content, old_text, new_text) {
    super::edit::EditOutcome::Replaced {
      level,
      content: new,
    } => {
      atomic_replace(std::path::Path::new(path), &new)
        .await
        .context(format!("Failed to write to file: {}", path))?;
      let note = if level == 1 {
        String::new()
      } else {
        format!(" (matched via fuzzy level {level})")
      };
      Ok(format!("Successfully edited {}{}", path, note))
    }
    super::edit::EditOutcome::NotFound => Ok(format!(
      "old_text not found in {}. The text may have different whitespace, or you \
       may be looking at a stale version. Re-read the file with read_file and \
       copy old_text exactly, then retry.",
      path
    )),
    super::edit::EditOutcome::Ambiguous { level, count } => Ok(format!(
      "old_text matched {count} places in {} (at fuzzy level {level}). Refusing \
       to edit ambiguously. Add more surrounding lines to old_text so it \
       uniquely identifies the single region you mean.",
      path
    )),
  }
}

pub async fn list_dir(args: &Value) -> Result<String> {
  let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

  let mut entries = tokio::fs::read_dir(path)
    .await
    .context(format!("Failed to read directory: {}", path))?;
  let mut result = String::new();

  while let Some(entry) = entries.next_entry().await? {
    let name = entry.file_name().to_string_lossy().to_string();
    let file_type = entry.file_type().await?;
    let marker = if file_type.is_dir() { "/" } else { "" };
    result.push_str(&format!("{}{}\n", name, marker));
  }

  if result.is_empty() {
    Ok(format!("Directory '{}' is empty.", path))
  } else {
    Ok(result)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  /// The window `fs::write` leaves open: it truncates, then writes. Anything
  /// reading at that moment — another tool, a background `run_shell` job, the
  /// user's editor — sees a truncated file and nothing says so.
  #[tokio::test]
  async fn a_replaced_file_is_never_observed_half_written() {
    let dir = std::env::temp_dir().join("seekcli-atomic-write");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("big.txt");
    let old = "o".repeat(200_000);
    let new = "n".repeat(200_000);
    let _ = std::fs::write(&path, &old);

    let reader = {
      let path = path.clone();
      tokio::spawn(async move {
        let mut seen_partial = false;
        for _ in 0..400 {
          if let Ok(text) = std::fs::read_to_string(&path) {
            // Every observation must be one whole version or the other.
            let whole = text.len() == 200_000
              && (text.bytes().all(|b| b == b'o') || text.bytes().all(|b| b == b'n'));
            if !whole {
              seen_partial = true;
              break;
            }
          }
          tokio::task::yield_now().await;
        }
        seen_partial
      })
    };
    for _ in 0..40 {
      let _ = atomic_replace(&path, &new).await;
      let _ = atomic_replace(&path, &old).await;
    }
    let torn = reader.await.unwrap_or(false);
    assert!(!torn, "a reader observed a partially written file");
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// Replacing must not leave its staging file behind — a stray dotfile next to
  /// the user's source is litter they cannot trace back to anything.
  #[tokio::test]
  async fn replacing_leaves_no_temporary_behind() {
    let dir = std::env::temp_dir().join("seekcli-atomic-clean");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("f.txt");
    let _ = atomic_replace(&path, "hello").await;
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
      .map(|rd| {
        rd.flatten()
          .map(|e| e.file_name().to_string_lossy().into_owned())
          .filter(|n| n != "f.txt")
          .collect()
      })
      .unwrap_or_default();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// The lost-update race, run for real: two edits to one file at once.
  /// Without the per-path lock both read the same contents and the second
  /// write erases the first.
  ///
  /// Runs inside the scratch directory, not merely pointing at it: `edit_file`
  /// is path-gated to the workspace, so an absolute temp path comes back as
  /// `[PATH DENIED]` — inside an `Ok`, which is how the first version of this
  /// test passed its `is_ok()` check while editing nothing at all.
  /// The guard is held across the awaits deliberately: the whole point is that
  /// the process cwd stays put for the duration. Safe because only tests take
  /// this lock, and the test runtime cannot deadlock on it.
  #[expect(
    clippy::await_holding_lock,
    reason = "the test lock is held across awaits on purpose; see the fn doc"
  )]
  #[tokio::test]
  async fn concurrent_edits_to_one_file_cannot_lose_an_update() {
    let _guard = crate::testsync::lock();
    let original = std::env::current_dir().unwrap_or_default();
    let dir = std::env::temp_dir().join("seekcli-edit-race");
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
      panic!("cannot create scratch: {e}");
    }
    if let Err(e) = std::env::set_current_dir(&dir) {
      panic!("cannot enter scratch: {e}");
    }
    let _ = std::fs::write("shared.txt", "alpha\nbeta\n");

    let args_a = serde_json::json!({
      "path": "shared.txt", "old_text": "alpha", "new_text": "ALPHA"
    });
    let args_b = serde_json::json!({
      "path": "shared.txt", "old_text": "beta", "new_text": "BETA"
    });
    let (ra, rb) = tokio::join!(edit_file(&args_a), edit_file(&args_b));
    let (ra, rb) = (ra.unwrap_or_default(), rb.unwrap_or_default());
    assert!(!ra.contains("DENIED"), "{ra}");
    assert!(!rb.contains("DENIED"), "{rb}");

    // Disjoint edits: serialising means BOTH land, because the second still
    // finds its own `old_text` in the file the first wrote. Interleaved, one
    // would have been silently erased.
    let text = std::fs::read_to_string("shared.txt").unwrap_or_default();
    let _ = std::env::set_current_dir(&original);
    assert!(text.contains("ALPHA"), "first edit lost: {text:?}");
    assert!(text.contains("BETA"), "second edit lost: {text:?}");
    let _ = std::fs::remove_dir_all(&dir);
  }

  fn png_bytes() -> Vec<u8> {
    // 1x1 PNG: enough to exercise the magic-byte path without a fixture file.
    vec![
      0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R',
    ]
  }

  /// The type comes from the bytes, not the name: a `.png` that is really a
  /// JPEG would otherwise be announced wrong and the provider would reject it.
  #[test]
  fn the_media_type_comes_from_the_magic_bytes() {
    assert_eq!(media_type_of(&png_bytes()), Some("image/png"));
    assert_eq!(media_type_of(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
    assert_eq!(media_type_of(b"GIF89a...."), Some("image/gif"));
    assert_eq!(media_type_of(b"RIFF____WEBPVP8 "), Some("image/webp"));
    assert_eq!(media_type_of(b"not an image at all"), None);
  }

  #[tokio::test]
  async fn reading_an_image_returns_it_as_an_image_not_as_text() {
    let dir = std::env::temp_dir().join(format!("seekcli_ri_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("shot.png");
    let _ = std::fs::write(&path, png_bytes());

    let out = match read_image(&json!({ "path": path.display().to_string() })).await {
      Ok(o) => o,
      Err(e) => panic!("{e:#}"),
    };
    assert_eq!(out.images.len(), 1);
    assert_eq!(out.images[0].media_type, "image/png");
    // The text is a placeholder, not a description: the image itself is what
    // the model reads.
    assert!(out.text.contains("image/png"), "{}", out.text);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[tokio::test]
  async fn a_text_file_is_refused_with_a_pointer_to_read_file() {
    let dir = std::env::temp_dir().join(format!("seekcli_ri_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("notes.txt");
    let _ = std::fs::write(&path, b"just words");

    let err = match read_image(&json!({ "path": path.display().to_string() })).await {
      Ok(_) => panic!("a text file is not an image"),
      Err(e) => format!("{e:#}"),
    };
    assert!(
      err.contains("read_file"),
      "must point at the right tool: {err}"
    );
    std::fs::remove_dir_all(&dir).ok();
  }
}
