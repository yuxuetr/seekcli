use anyhow::{Context, Result};
use serde_json::Value;

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

  tokio::fs::write(path, content)
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

  let content = tokio::fs::read_to_string(path)
    .await
    .context(format!("Failed to read file: {}", path))?;

  match super::edit::apply_edit(&content, old_text, new_text) {
    super::edit::EditOutcome::Replaced {
      level,
      content: new,
    } => {
      tokio::fs::write(path, &new)
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
