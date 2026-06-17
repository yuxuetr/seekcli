use anyhow::{Context, Result};
use serde_json::Value;

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
