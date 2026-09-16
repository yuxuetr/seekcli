//! The `memory` tool: read and write the notes that outlive a conversation.
//!
//! Thin on purpose. Everything that decides *what may be written where* lives
//! in `crate::memory`, so the gate on `preferences` cannot be bypassed by a
//! second caller — this module only translates JSON arguments into those calls.

use anyhow::{Context, Result};
use serde_json::Value;

use crate::memory::{MemoryStore, PREFERENCES};

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
  args
    .get(key)
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|s| !s.is_empty())
}

pub async fn memory(args: &Value) -> Result<String> {
  let action = arg(args, "action").context("`action` is required: list | read | write | forget")?;
  let store = MemoryStore::new()?;

  match action {
    "list" => {
      let scopes = store.scopes();
      if scopes.is_empty() {
        return Ok("No memory recorded yet.".to_string());
      }
      let mut out = String::from("Memory scopes:\n");
      for s in scopes {
        out.push_str(&format!("  {} ({} entries)\n", s.name, s.entries));
      }
      out.push_str("\nRead one with memory{action:\"read\", scope:\"<name>\"}.");
      Ok(out)
    }
    "read" => {
      let scope = arg(args, "scope").context("`scope` is required for read")?;
      store.read(scope)
    }
    "write" => {
      let scope = arg(args, "scope").context("`scope` is required for write")?;
      let entry = arg(args, "entry").context("`entry` is required for write")?;
      let source = arg(args, "source").unwrap_or("agent");
      if scope == PREFERENCES {
        // Routed, not refused-and-forgotten: the model gets a concrete next
        // step, which is the same "teaching error" rule stage 34 established.
        let name = crate::skills::sanitize_name(&entry.chars().take(40).collect::<String>());
        let store = crate::proposals::ProposalStore::new()?;
        let saved = store.draft(crate::proposals::Kind::Memory, &name, entry)?;
        return Ok(format!(
          "`preferences` holds rules about the user that apply to every future \
           conversation, so it is not written directly. Drafted a proposal at \
           {}. Tell the user to run `/propose list` and \
           `/propose accept memory {name}` if they agree.",
          saved.display()
        ));
      }
      store.append(scope, entry, source)
    }
    "forget" => {
      let scope = arg(args, "scope").context("`scope` is required for forget")?;
      let needle =
        arg(args, "entry").context("`entry` is required for forget: the text to remove")?;
      store.forget(scope, needle)
    }
    other => anyhow::bail!("unknown action `{other}`; use list, read, write or forget"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[tokio::test]
  async fn a_missing_action_is_an_error_that_names_the_choices() {
    let err = match memory(&json!({})).await {
      Err(e) => format!("{e:#}"),
      Ok(v) => panic!("expected an error, got {v}"),
    };
    assert!(err.contains("list"), "{err}");
    assert!(err.contains("forget"), "{err}");
  }

  #[tokio::test]
  async fn an_unknown_action_names_the_valid_ones() {
    let err = match memory(&json!({"action": "obliterate"})).await {
      Err(e) => format!("{e:#}"),
      Ok(v) => panic!("expected an error, got {v}"),
    };
    assert!(err.contains("obliterate"), "{err}");
    assert!(err.contains("read"), "{err}");
  }

  #[tokio::test]
  async fn read_without_a_scope_says_which_argument_is_missing() {
    let err = match memory(&json!({"action": "read"})).await {
      Err(e) => format!("{e:#}"),
      Ok(v) => panic!("expected an error, got {v}"),
    };
    assert!(err.contains("scope"), "{err}");
  }
}
