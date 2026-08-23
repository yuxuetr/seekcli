//! MCP integration: the one switch that opens the ecosystem.
//!
//! Before this, adding a capability meant editing Rust. Nobody else could
//! extend SeekCLI at all — the evaluation called it the single largest gap
//! (L2-1 / L5-1). With MCP, web / browser / database / GitHub servers plug in
//! through configuration.
//!
//! Three properties matter more than the feature itself:
//!
//! * **One bad server must not stop startup.** External processes are
//!   unreliable by nature; a typo'd command should cost a warning, not a REPL
//!   that will not open.
//! * **Startup must stay bounded.** A server that hangs on handshake gets a
//!   deadline, because the alternative is a CLI that sometimes just does not
//!   start.
//! * **Foreign tools are untrusted by default.** They are not marked
//!   parallel-safe and they pass the same policy gate as built-ins, so
//!   `--read-only` means read-only for them too.

pub mod protocol;

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::api::{FunctionDefinition, Tool};
use crate::config::McpServerConfig;
use protocol::{McpClient, McpTool};

/// Separator between server and tool name.
///
/// Namespacing is not cosmetic: two servers may both offer `search`, and the
/// model should be able to see at a glance where a capability comes from.
const NAMESPACE: &str = "__";

pub struct McpRegistry {
  clients: BTreeMap<String, McpClient>,
  tools: Vec<RegisteredTool>,
}

struct RegisteredTool {
  /// Model-visible name, e.g. `mcp__github__create_issue`.
  qualified: String,
  server: String,
  remote_name: String,
  schema: Tool,
  read_only: bool,
}

pub fn qualify(server: &str, tool: &str) -> String {
  format!("mcp{}{}{}{}", NAMESPACE, server, NAMESPACE, tool)
}

/// Whether a tool name refers to an MCP tool.
pub fn is_mcp_tool(name: &str) -> bool {
  name.starts_with("mcp__")
}

impl McpRegistry {
  pub fn empty() -> Self {
    Self {
      clients: BTreeMap::new(),
      tools: Vec::new(),
    }
  }

  /// Start every enabled server, skipping the ones that fail.
  pub async fn connect_all(configs: &[McpServerConfig]) -> Self {
    let mut registry = Self::empty();
    for config in configs.iter().filter(|c| c.enabled) {
      let timeout = Duration::from_secs(config.startup_timeout_secs);
      let env = config.resolved_env();
      match tokio::time::timeout(
        timeout,
        McpClient::connect(&config.name, &config.command, &config.args, &env, timeout),
      )
      .await
      {
        Ok(Ok(client)) => registry.adopt(&config.name, client).await,
        // Both arms warn and continue: a broken server is the user's to fix,
        // and refusing to start the REPL over it would be a worse trade.
        Ok(Err(e)) => eprintln!("[MCP] server `{}` unavailable: {:#}", config.name, e),
        Err(_) => eprintln!(
          "[MCP] server `{}` did not finish starting within {:?}; skipped",
          config.name, timeout
        ),
      }
    }
    if !registry.tools.is_empty() {
      eprintln!(
        "[MCP] {} tool(s) from {} server(s)",
        registry.tools.len(),
        registry.clients.len()
      );
    }
    registry
  }

  async fn adopt(&mut self, name: &str, client: McpClient) {
    match client.list_tools().await {
      Ok(remote) => {
        for tool in remote {
          self.tools.push(Self::register(name, tool));
        }
        self.clients.insert(name.to_string(), client);
      }
      Err(e) => eprintln!("[MCP] server `{}` could not list tools: {:#}", name, e),
    }
  }

  fn register(server: &str, tool: McpTool) -> RegisteredTool {
    let qualified = qualify(server, &tool.name);
    let description = if tool.description.is_empty() {
      format!("Tool `{}` provided by MCP server `{}`.", tool.name, server)
    } else {
      format!("[{}] {}", server, tool.description)
    };
    RegisteredTool {
      schema: Tool {
        tool_type: "function".to_string(),
        function: FunctionDefinition {
          name: qualified.clone(),
          description,
          parameters: tool.input_schema,
        },
      },
      qualified,
      server: server.to_string(),
      remote_name: tool.name,
      read_only: tool.read_only,
    }
  }

  pub fn schemas(&self) -> Vec<Tool> {
    self.tools.iter().map(|t| t.schema.clone()).collect()
  }

  /// Whether the tool may run concurrently with others.
  ///
  /// Defaults to false. We did not write these tools and cannot inspect what
  /// they do, so only an explicit `readOnlyHint` from the server earns
  /// parallelism — guessing wrong means interleaving writes.
  pub fn is_read_only(&self, qualified: &str) -> bool {
    self
      .tools
      .iter()
      .find(|t| t.qualified == qualified)
      .is_some_and(|t| t.read_only)
  }

  pub async fn call(&self, qualified: &str, args: &Value) -> anyhow::Result<String> {
    let tool = self
      .tools
      .iter()
      .find(|t| t.qualified == qualified)
      .ok_or_else(|| anyhow::anyhow!("unknown MCP tool `{}`", qualified))?;
    let client = self
      .clients
      .get(&tool.server)
      .ok_or_else(|| anyhow::anyhow!("MCP server `{}` is not connected", tool.server))?;
    client.call_tool(&tool.remote_name, args).await
  }

  /// `(qualified name, server)` pairs, for `/tools`.
  pub fn listing(&self) -> Vec<(String, String)> {
    self
      .tools
      .iter()
      .map(|t| (t.qualified.clone(), t.server.clone()))
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn tool(name: &str, read_only: bool) -> McpTool {
    McpTool {
      name: name.to_string(),
      description: "does a thing".to_string(),
      input_schema: json!({ "type": "object" }),
      read_only,
    }
  }

  #[test]
  fn names_are_namespaced_so_two_servers_can_offer_the_same_tool() {
    assert_eq!(qualify("github", "search"), "mcp__github__search");
    assert_ne!(qualify("a", "search"), qualify("b", "search"));
    assert!(is_mcp_tool("mcp__github__search"));
    assert!(!is_mcp_tool("read_file"));
  }

  #[test]
  fn a_foreign_tool_is_not_parallel_safe_unless_the_server_says_so() {
    let mut r = McpRegistry::empty();
    r.tools
      .push(McpRegistry::register("srv", tool("writes", false)));
    r.tools
      .push(McpRegistry::register("srv", tool("reads", true)));
    // We cannot inspect what these do; guessing wrong interleaves writes.
    assert!(!r.is_read_only("mcp__srv__writes"));
    assert!(r.is_read_only("mcp__srv__reads"));
    // An unknown tool is never parallel-safe.
    assert!(!r.is_read_only("mcp__srv__nonexistent"));
  }

  #[test]
  fn the_description_says_which_server_a_tool_came_from() {
    let registered = McpRegistry::register("github", tool("create_issue", false));
    assert!(
      registered
        .schema
        .function
        .description
        .starts_with("[github]"),
      "got: {}",
      registered.schema.function.description
    );
    assert_eq!(registered.schema.function.name, "mcp__github__create_issue");
    assert_eq!(registered.remote_name, "create_issue");
  }

  #[test]
  fn a_server_with_no_description_still_produces_a_usable_schema() {
    let mut bare = tool("thing", false);
    bare.description = String::new();
    let registered = McpRegistry::register("srv", bare);
    assert!(!registered.schema.function.description.is_empty());
  }

  #[tokio::test]
  async fn calling_an_unknown_tool_names_it_rather_than_panicking() {
    let err = match McpRegistry::empty().call("mcp__x__y", &json!({})).await {
      Ok(_) => panic!("expected an error"),
      Err(e) => format!("{:#}", e),
    };
    assert!(err.contains("mcp__x__y"), "got: {}", err);
  }
}
