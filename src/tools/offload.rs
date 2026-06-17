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

/// Write `content` to `~/.seekcli/tmp/<hash>.txt`, returning the path.
async fn write_temp(content: &str) -> anyhow::Result<PathBuf> {
  use anyhow::Context;
  let home = std::env::var("HOME").context("Could not find HOME directory")?;
  let dir = PathBuf::from(home).join(".seekcli").join("tmp");
  tokio::fs::create_dir_all(&dir)
    .await
    .context("create tmp dir")?;

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
