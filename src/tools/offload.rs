//! Tool output offloading.
//!
//! A single multi-KB tool result (a large file read, a verbose command dump)
//! bloats the context, accelerates compression/OOM, and wastes tokens — yet the
//! model usually only needs a glimpse plus a way to fetch the rest on demand.
//!
//! When a result exceeds [`OFFLOAD_THRESHOLD`], the full content is written to a
//! temp file under `~/.seekcli/tmp/` and the model receives a head+tail preview
//! plus the path, nudging it to read specific sections only when needed.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Offload results larger than this many bytes.
pub const OFFLOAD_THRESHOLD: usize = 8_192;

/// Bytes kept from the head of an offloaded result.
const HEAD_KEEP: usize = 2_000;
/// Bytes kept from the tail of an offloaded result.
const TAIL_KEEP: usize = 1_000;

/// If `content` is large, persist it and return a preview referencing the file.
/// Otherwise return `content` unchanged. `source_hint`, when given, is mentioned
/// in the preview (e.g. the original file path, which the model can re-read with
/// range tools instead of the offload copy).
///
/// Best-effort: if the temp file cannot be written, falls back to an inline
/// head+tail preview with no path reference — never errors, never drops the
/// signal the model needs.
pub async fn offload(content: String, source_hint: Option<&str>) -> String {
  if content.len() <= OFFLOAD_THRESHOLD {
    return content;
  }

  let n = content.len();
  let head = &content[..floor_boundary(&content, HEAD_KEEP)];
  let tail = &content[ceil_boundary(&content, n - TAIL_KEEP)..];

  match write_temp(&content).await {
    Ok(path) => {
      let source_note = match source_hint {
        Some(src) => format!(
          "Original source: `{src}` — prefer reading specific ranges from it \
           (run_shell with sed/grep/head/tail).\n",
        ),
        None => String::new(),
      };
      format!(
        "[output offloaded: {n} bytes; full content saved to `{path}`]\n\
         {source_note}\n\
         --- HEAD ({head_len} bytes) ---\n{head}\n\n\
         --- TAIL ({tail_len} bytes) ---\n{tail}\n\n\
         [To see more, read_file `{path}` or grep it.]",
        n = n,
        path = path.display(),
        source_note = source_note,
        head_len = head.len(),
        head = head,
        tail_len = tail.len(),
        tail = tail,
      )
    }
    Err(_) => format!(
      "[output too large: {n} bytes; could not offload to disk, showing \
       head+tail only]\n\n\
       --- HEAD ({head_len} bytes) ---\n{head}\n\n\
       --- TAIL ({tail_len} bytes) ---\n{tail}",
      n = n,
      head_len = head.len(),
      head = head,
      tail_len = tail.len(),
      tail = tail,
    ),
  }
}

/// Where offloaded output goes.
///
/// A process global, matching `approval::init_policy` and `policy::MODE`:
/// offloading happens deep inside tool execution, and threading a session id
/// through every tool signature would buy nothing over setting it once when
/// the session changes.
static BLOB_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Point offloading at the active session's `blobs/` directory.
///
/// Blobs belong to the conversation that produced them: a preview references
/// a path the model may read several turns later, so the file has to outlive
/// the turn — but not the session, or `~/.seekcli/tmp` grows without bound
/// (it previously never got cleaned at all).
pub fn set_blob_dir(dir: PathBuf) {
  if let Ok(mut guard) = BLOB_DIR.lock() {
    *guard = Some(dir);
  }
}

fn blob_dir() -> anyhow::Result<PathBuf> {
  use anyhow::Context;
  if let Ok(guard) = BLOB_DIR.lock()
    && let Some(dir) = guard.as_ref()
  {
    return Ok(dir.clone());
  }
  // Headless runs and tests never set a session; a shared scratch directory
  // keeps them working rather than failing the tool call.
  let home = std::env::var("HOME").context("Could not find HOME directory")?;
  Ok(PathBuf::from(home).join(".seekcli").join("tmp"))
}

/// Write an image into the session's blob directory, returning its reference.
///
/// Tool-returned images go through here so the event log stores a path rather
/// than hundreds of KB of base64 — the same rule `/paste` follows. The blob
/// belongs to the conversation, so the 30-day sweep reclaims it with the rest
/// (`docs/architecture/L4-memory.md` §4.6.2).
pub fn persist_image(image: &crate::api::ImagePart) -> anyhow::Result<crate::session::ImageRef> {
  let dir = blob_dir()?;
  std::fs::create_dir_all(&dir)?;
  let extension = image
    .media_type
    .rsplit('/')
    .next()
    .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
    .unwrap_or("bin");
  let path = dir.join(format!("tool-{}.{}", uuid::Uuid::new_v4(), extension));
  let bytes = crate::api::base64_decode(&image.data_base64)?;
  std::fs::write(&path, bytes)?;
  Ok(crate::session::ImageRef {
    path: path.display().to_string(),
    media_type: image.media_type.clone(),
  })
}

/// Store a piece of text under a content-derived name, returning its path.
///
/// Used for the system prompts the harness composes: the session log records
/// that a request carried them, and the content lives here rather than in the
/// log, so a turn stays readable and an unchanged context costs nothing to
/// re-record.
///
/// Content-addressed by the caller's digest, so identical content is written
/// once however many sessions inject it.
pub fn persist_text(digest: &str, content: &str) -> anyhow::Result<PathBuf> {
  let dir = blob_dir()?;
  std::fs::create_dir_all(&dir)?;
  let safe: String = digest
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
    .collect();
  let path = dir.join(format!("ctx-{safe}.txt"));
  if !path.exists() {
    std::fs::write(&path, content)?;
  }
  Ok(path)
}

/// Delete blobs untouched for longer than `max_age`.
///
/// Content-addressed names mean identical output is written once, so the only
/// growth is genuinely new output; age is the right axis to trim on.
pub fn sweep(root: &std::path::Path, max_age: std::time::Duration) -> usize {
  let mut removed = 0usize;
  let Ok(entries) = std::fs::read_dir(root) else {
    return 0;
  };
  let now = std::time::SystemTime::now();
  for entry in entries.flatten() {
    let path = entry.path();
    if path.is_dir() {
      removed += sweep(&path, max_age);
      continue;
    }
    let stale = entry
      .metadata()
      .and_then(|m| m.modified())
      .ok()
      .and_then(|t| now.duration_since(t).ok())
      .is_some_and(|age| age > max_age);
    if stale && std::fs::remove_file(&path).is_ok() {
      removed += 1;
    }
  }
  removed
}

/// Write `content` to `<blob dir>/<hash>.txt`, returning the path.
async fn write_temp(content: &str) -> anyhow::Result<PathBuf> {
  use anyhow::Context;
  let dir = blob_dir()?;
  tokio::fs::create_dir_all(&dir)
    .await
    .context("create blob dir")?;

  let mut hasher = DefaultHasher::new();
  content.hash(&mut hasher);
  let path = dir.join(format!("{:016x}.txt", hasher.finish()));
  tokio::fs::write(&path, content)
    .await
    .context("write offload file")?;
  Ok(path)
}

fn floor_boundary(s: &str, idx: usize) -> usize {
  let mut i = idx.min(s.len());
  while i > 0 && !s.is_char_boundary(i) {
    i -= 1;
  }
  i
}

fn ceil_boundary(s: &str, idx: usize) -> usize {
  let mut i = idx.min(s.len());
  while i < s.len() && !s.is_char_boundary(i) {
    i += 1;
  }
  i
}

#[cfg(test)]
mod lifecycle_tests {
  use super::*;
  use std::time::Duration;

  #[test]
  fn sweep_removes_stale_blobs_and_keeps_fresh_ones() {
    let root = std::env::temp_dir().join("seekcli-blob-sweep");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(root.join("nested"));
    let fresh = root.join("fresh.txt");
    let nested = root.join("nested/also-fresh.txt");
    let _ = std::fs::write(&fresh, "x");
    let _ = std::fs::write(&nested, "y");

    // Nothing is older than a year, so nothing should go.
    assert_eq!(sweep(&root, Duration::from_secs(365 * 24 * 3600)), 0);
    assert!(fresh.exists());
    assert!(nested.exists());

    // With a zero-age cutoff everything qualifies, including nested files --
    // that is what proves the walk recurses into per-session directories.
    assert_eq!(sweep(&root, Duration::from_secs(0)), 2);
    assert!(!fresh.exists());
    assert!(!nested.exists());
  }

  #[test]
  fn sweeping_a_missing_directory_is_not_an_error() {
    assert_eq!(
      sweep(
        &std::env::temp_dir().join("seekcli-does-not-exist"),
        Duration::from_secs(0)
      ),
      0
    );
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn small_output_unchanged() {
    let s = "small".to_string();
    assert_eq!(offload(s.clone(), None).await, s);
  }

  #[tokio::test]
  async fn large_output_offloaded_with_preview() {
    let big = format!("HEADMARK{}TAILMARK", "x".repeat(20_000));
    let out = offload(big, Some("orig.txt")).await;
    assert!(out.contains("offloaded"));
    assert!(out.contains("HEADMARK"));
    assert!(out.contains("TAILMARK"));
    assert!(out.contains("orig.txt"));
    // The huge middle must be gone.
    assert!(out.len() < 20_000);
  }
}
