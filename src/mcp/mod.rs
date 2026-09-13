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
  failures: Vec<McpFailure>,
}

/// A configured server that is not available this session, and why.
///
/// Kept rather than only printed. The startup warning goes to the user, but
/// the model needs this at the moment it reaches for a tool that should have
/// been there — otherwise the capability is simply absent with no explanation,
/// and the model cannot tell a typo from a broken server. Stage 35's
/// `harness_inspect` reads the same record on demand
/// (`docs/architecture/L1-engine.md` §4.6.6).
#[derive(Debug, Clone)]
pub struct McpFailure {
  pub server: String,
  pub reason: String,
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
      failures: Vec::new(),
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
        Ok(Err(e)) => registry.record_failure(&config.name, format!("{e:#}")),
        Err(_) => registry.record_failure(
          &config.name,
          format!("did not finish starting within {timeout:?}"),
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
      Err(e) => self.record_failure(name, format!("could not list its tools: {e:#}")),
    }
  }

  /// Warn the user *and* keep the reason for the model.
  ///
  /// The visible line stays: "degrade, never silently" is a standing rule
  /// (`docs/architecture/design-principles.md` §4).
  fn record_failure(&mut self, server: &str, reason: String) {
    eprintln!("[MCP] server `{server}` unavailable: {reason}");
    self.failures.push(McpFailure {
      server: server.to_string(),
      reason,
    });
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

  /// The server half of `mcp__<server>__<tool>`, if the name is shaped like one.
  fn server_of(qualified: &str) -> Option<&str> {
    qualified
      .strip_prefix("mcp__")
      .and_then(|rest| rest.split_once("__"))
      .map(|(server, _)| server)
  }

  /// Why a tool the model just tried to call does not exist.
  ///
  /// "unknown MCP tool" alone left the model unable to tell a hallucinated
  /// name from a server that failed to start, so its cheapest next move was to
  /// try the same call again. Naming the startup failure closes that loop
  /// without costing a standing prompt section.
  fn unknown_tool_error(&self, qualified: &str) -> anyhow::Error {
    if let Some(failed) = Self::server_of(qualified)
      .and_then(|server| self.failures.iter().find(|f| f.server == server))
    {
      return anyhow::anyhow!(
        "MCP server `{}` is configured but unavailable this session, so `{}` does not \
         exist: {}. Do not retry this tool — restarting it is the user's action, not \
         yours. Use the tools you do have, or tell the user this server needs fixing.",
        failed.server,
        qualified,
        failed.reason
      );
    }
    if self.tools.is_empty() {
      return anyhow::anyhow!(
        "`{}` does not exist: no MCP tools are available in this session. Do not retry \
         it; use the built-in tools instead.",
        qualified
      );
    }
    let available: Vec<&str> = self.tools.iter().map(|t| t.qualified.as_str()).collect();
    anyhow::anyhow!(
      "`{}` does not exist. Available MCP tools: {}. Do not retry this name; call one \
       of those, or a built-in tool.",
      qualified,
      available.join(", ")
    )
  }

  pub async fn call(&self, qualified: &str, args: &Value) -> anyhow::Result<String> {
    let tool = self
      .tools
      .iter()
      .find(|t| t.qualified == qualified)
      .ok_or_else(|| self.unknown_tool_error(qualified))?;
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

  /// The loop this closes: absent a reason, the model could not tell a
  /// hallucinated name from a server that failed to start, so its cheapest
  /// next move was to send the same call again.
  #[tokio::test]
  async fn a_failed_server_explains_itself_when_one_of_its_tools_is_called() {
    let mut r = McpRegistry::empty();
    r.record_failure("github", "command `ghmcp` not found".to_string());

    let err = match r.call("mcp__github__create_issue", &json!({})).await {
      Ok(_) => panic!("expected an error"),
      Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("github"), "{err}");
    assert!(err.contains("not found"), "must carry the reason: {err}");
    assert!(err.to_lowercase().contains("do not retry"), "{err}");
    // Restarting the server is the user's action; the model must not be sent
    // off to attempt it.
    assert!(err.contains("user's action"), "{err}");
  }

  #[tokio::test]
  async fn an_unknown_name_with_no_failed_server_lists_what_does_exist() {
    let mut r = McpRegistry::empty();
    r.tools
      .push(McpRegistry::register("srv", tool("reads", true)));

    let err = match r.call("mcp__srv__typo", &json!({})).await {
      Ok(_) => panic!("expected an error"),
      Err(e) => format!("{e:#}"),
    };
    assert!(
      err.contains("mcp__srv__reads"),
      "must name the real one: {err}"
    );
  }

  #[test]
  fn the_server_half_of_a_qualified_name_is_recoverable() {
    assert_eq!(
      McpRegistry::server_of("mcp__github__create_issue"),
      Some("github")
    );
    assert_eq!(McpRegistry::server_of("read_file"), None);
    assert_eq!(McpRegistry::server_of("mcp__nounderscores"), None);
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
