//! Minimal MCP client over stdio.
//!
//! Hand-written rather than pulled from an SDK. The surface actually needed is
//! four messages — `initialize`, `notifications/initialized`, `tools/list`,
//! `tools/call` — and owning them means the failure modes are ours to handle:
//! a server that never answers, that answers out of order, or that writes
//! diagnostics to stdout and corrupts the frame stream. An SDK would hide
//! exactly those.
//!
//! Transport is stdio only. SSE and HTTP exist in the spec, but a local CLI
//! launching a local process covers the overwhelming majority of servers, and
//! every additional transport is another set of failure modes to get right.

use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// Protocol version this client speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// A running MCP server process plus its framed pipes.
pub struct McpClient {
  pub name: String,
  child: Child,
  stdin: Mutex<ChildStdin>,
  stdout: Mutex<BufReader<ChildStdout>>,
  next_id: AtomicI64,
  timeout: Duration,
}

impl McpClient {
  /// Spawn `command args...` and complete the MCP handshake.
  pub async fn connect(
    name: &str,
    command: &str,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
  ) -> Result<Self> {
    let mut cmd = tokio::process::Command::new(command);
    cmd
      .args(args)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      // Servers routinely log to stderr. Inheriting it would interleave their
      // noise with ours; discarding it loses diagnostics. Piping and dropping
      // keeps our output clean without blocking the child on a full pipe.
      .stderr(Stdio::null());
    for (k, v) in env {
      cmd.env(k, v);
    }

    let mut child = cmd
      .spawn()
      .with_context(|| format!("cannot start MCP server `{}` ({})", name, command))?;
    let stdin = child.stdin.take().context("no stdin pipe")?;
    let stdout = child.stdout.take().context("no stdout pipe")?;

    let client = Self {
      name: name.to_string(),
      child,
      stdin: Mutex::new(stdin),
      stdout: Mutex::new(BufReader::new(stdout)),
      next_id: AtomicI64::new(1),
      timeout,
    };

    client
      .request(
        "initialize",
        json!({
          "protocolVersion": PROTOCOL_VERSION,
          "capabilities": {},
          "clientInfo": { "name": "seekcli", "version": env!("CARGO_PKG_VERSION") }
        }),
      )
      .await
      .with_context(|| format!("MCP server `{}` failed to initialize", name))?;

    client
      .notify("notifications/initialized", json!({}))
      .await?;
    Ok(client)
  }

  /// Tool descriptors the server offers.
  pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
    let result = self.request("tools/list", json!({})).await?;
    let items = result
      .get("tools")
      .and_then(Value::as_array)
      .cloned()
      .unwrap_or_default();
    Ok(
      items
        .into_iter()
        .filter_map(|t| {
          Some(McpTool {
            name: t.get("name")?.as_str()?.to_string(),
            description: t
              .get("description")
              .and_then(Value::as_str)
              .unwrap_or("")
              .to_string(),
            // Servers vary on the key; accept both rather than silently
            // registering a tool the model cannot call.
            input_schema: t
              .get("inputSchema")
              .or_else(|| t.get("input_schema"))
              .cloned()
              .unwrap_or_else(|| json!({ "type": "object" })),
            read_only: t
              .get("annotations")
              .and_then(|a| a.get("readOnlyHint"))
              .and_then(Value::as_bool)
              .unwrap_or(false),
          })
        })
        .collect(),
    )
  }

  /// Invoke a tool and flatten the content blocks into text.
  pub async fn call_tool(&self, tool: &str, args: &Value) -> Result<String> {
    let result = self
      .request("tools/call", json!({ "name": tool, "arguments": args }))
      .await?;

    // A server signals a tool-level failure with isError, not a JSON-RPC
    // error. Conflating the two would report "the server broke" when in fact
    // the tool ran and said no.
    let is_error = result
      .get("isError")
      .and_then(Value::as_bool)
      .unwrap_or(false);
    let text = flatten_content(&result);
    if is_error {
      anyhow::bail!("{}", text);
    }
    Ok(text)
  }

  async fn request(&self, method: &str, params: Value) -> Result<Value> {
    let id = self.next_id.fetch_add(1, Ordering::SeqCst);
    let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    self.write_frame(&payload).await?;

    // Read until the response with our id. Notifications and log messages may
    // arrive in between; discarding anything else would drop them on the floor
    // and then block forever waiting for a reply that already went past.
    let deadline = tokio::time::Instant::now() + self.timeout;
    loop {
      let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
      if remaining.is_zero() {
        anyhow::bail!("`{}` timed out waiting for {}", self.name, method);
      }
      let line = match tokio::time::timeout(remaining, self.read_line()).await {
        Ok(Ok(Some(line))) => line,
        Ok(Ok(None)) => anyhow::bail!("`{}` closed its output during {}", self.name, method),
        Ok(Err(e)) => return Err(e),
        Err(_) => anyhow::bail!("`{}` timed out waiting for {}", self.name, method),
      };
      let Ok(message) = serde_json::from_str::<Value>(&line) else {
        // Servers sometimes print banners to stdout. Skipping unparsable
        // lines keeps one stray println from killing the session.
        continue;
      };
      if message.get("id").and_then(Value::as_i64) != Some(id) {
        continue;
      }
      if let Some(error) = message.get("error") {
        let msg = error
          .get("message")
          .and_then(Value::as_str)
          .unwrap_or("unknown error");
        anyhow::bail!("`{}` rejected {}: {}", self.name, method, msg);
      }
      return Ok(message.get("result").cloned().unwrap_or(Value::Null));
    }
  }

  async fn notify(&self, method: &str, params: Value) -> Result<()> {
    self
      .write_frame(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
      .await
  }

  async fn write_frame(&self, payload: &Value) -> Result<()> {
    let mut line = serde_json::to_string(payload)?;
    line.push('\n');
    let mut stdin = self.stdin.lock().await;
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
  }

  async fn read_line(&self) -> Result<Option<String>> {
    let mut stdout = self.stdout.lock().await;
    let mut buf = String::new();
    let n = stdout.read_line(&mut buf).await?;
    Ok(if n == 0 { None } else { Some(buf) })
  }
}

impl Drop for McpClient {
  fn drop(&mut self) {
    // Servers are children of this process; leaving them running after the
    // REPL exits would leak a process per configured server per run.
    let _ = self.child.start_kill();
  }
}

#[derive(Debug, Clone)]
pub struct McpTool {
  pub name: String,
  pub description: String,
  pub input_schema: Value,
  /// Only true when the server explicitly says so. Absence means "assume it
  /// writes" — the safe default for something we did not author.
  pub read_only: bool,
}

/// MCP returns a list of typed content blocks; the model wants text.
pub fn flatten_content(result: &Value) -> String {
  let Some(blocks) = result.get("content").and_then(Value::as_array) else {
    return String::new();
  };
  let mut parts = Vec::new();
  for block in blocks {
    match block.get("type").and_then(Value::as_str) {
      Some("text") => {
        if let Some(t) = block.get("text").and_then(Value::as_str) {
          parts.push(t.to_string());
        }
      }
      // Images and embedded resources cannot be shown to a text-only model.
      // Naming them beats dropping them silently, which would look like the
      // tool returned nothing.
      Some(other) => parts.push(format!("[{} content omitted]", other)),
      None => {}
    }
  }
  parts.join("\n")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn text_blocks_are_joined() {
    let result = json!({ "content": [
      { "type": "text", "text": "line one" },
      { "type": "text", "text": "line two" }
    ]});
    assert_eq!(flatten_content(&result), "line one\nline two");
  }

  #[test]
  fn non_text_blocks_are_named_not_dropped() {
    // Dropping them silently would look like the tool returned nothing.
    let result = json!({ "content": [
      { "type": "text", "text": "here" },
      { "type": "image", "data": "..." }
    ]});
    let out = flatten_content(&result);
    assert!(out.contains("here"));
    assert!(out.contains("[image content omitted]"), "got: {}", out);
  }

  #[test]
  fn a_result_with_no_content_is_empty_not_an_error() {
    assert_eq!(flatten_content(&json!({})), "");
    assert_eq!(flatten_content(&json!({ "content": [] })), "");
  }
}
