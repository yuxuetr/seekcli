//! Configuration loading.
//!
//! Three layers, merged in increasing priority (see
//! `docs/architecture/L6-interface.md` §4.2):
//!
//! 1. built-in defaults
//! 2. user config      `~/.seekcli/config.toml`   (generated on first run)
//! 3. project override `./.seekcli.toml`          (partial, optional)
//! 4. explicit         `$SEEKCLI_CONFIG`          (partial, highest priority)
//!
//! Merging happens at the `toml::Value` level rather than through a mirrored
//! `Option`-per-field struct: an override file only has to mention the keys it
//! changes, and adding a new config field needs no changes here.
//!
//! SeekCLI never writes to the current working directory. Older versions read
//! and created `./config.toml`, so a leftover one triggers a migration notice
//! instead of being silently ignored — see `legacy_notice`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Env var pointing at an extra, highest-priority config file.
pub const CONFIG_ENV: &str = "SEEKCLI_CONFIG";
/// Project-level override, relative to the current working directory.
pub const PROJECT_CONFIG_FILE: &str = ".seekcli.toml";
/// Pre-0.2 config location, relative to the current working directory.
const LEGACY_CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Config {
  pub brain: BrainConfig,
  /// Named endpoints `brain.provider` can point at. Absent in older config
  /// files, where `brain.provider` is the bare wire name and the built-in
  /// DeepSeek endpoint is synthesized — see `resolve_provider`.
  #[serde(default, rename = "provider")]
  pub providers: Vec<ProviderConfig>,
  /// Retry / timeout tuning. Absent means the built-in defaults.
  #[serde(default)]
  pub resilience: ResilienceConfig,
  /// MCP servers to launch. Absent means none.
  #[serde(default, rename = "mcp")]
  pub mcp_servers: Vec<McpServerConfig>,
  /// Token prices used for the cost estimate.
  #[serde(default)]
  pub pricing: PricingConfig,
  /// Optional shell-command permission policy. Absent in older config files,
  /// so it defaults to empty (built-in rules only).
  #[serde(default)]
  pub security: SecurityConfig,
  /// L8 scheduled-task state directory (reminders.md/todos.md/digest/).
  /// Absent in older config files, so it defaults to `~/.seekcli/tasks`.
  #[serde(default)]
  pub tasks: TasksConfig,
  /// Context-compression budget. Absent means the built-in defaults, which
  /// reproduce the fixed 150K threshold this section replaced.
  #[serde(default)]
  pub memory: MemoryConfig,
  /// Where `web_search` / `web_fetch` get their data. Absent means the tools
  /// are not offered at all.
  #[serde(default)]
  pub research: ResearchConfig,
  /// Ceiling on what one run may spend.
  #[serde(default)]
  pub limits: LimitsConfig,
  /// When the Two-Stage deliberation pass runs.
  #[serde(default)]
  pub planning: PlanningConfig,
  /// Decision-path tracing and how much of it to keep.
  #[serde(default)]
  pub trace: TraceConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BrainConfig {
  pub flash_model: String,
  pub pro_model: String,
  /// Which endpoint to talk to. Either the `name` of a `[[provider]]` entry,
  /// or — for configs written before `[[provider]]` existed — the bare wire
  /// name "openai" / "anthropic", which resolves to the built-in DeepSeek
  /// endpoint for that wire.
  #[serde(default = "default_provider")]
  pub provider: String,
}

/// One named endpoint.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ProviderConfig {
  pub name: String,
  /// Wire protocol: "openai" (`/chat/completions`) or "anthropic" (`/v1/messages`).
  pub wire: String,
  pub base_url: String,
  /// Where the key lives: `env:VAR` or `file:PATH`.
  ///
  /// A literal key is rejected on purpose. Config files get committed by
  /// accident, and a rejected startup is a far cheaper failure than a leaked
  /// credential. The prefix form also leaves room for `keychain:` later
  /// without another schema change.
  pub api_key: String,
}

/// One MCP server to launch as a child process.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct McpServerConfig {
  pub name: String,
  pub command: String,
  #[serde(default)]
  pub args: Vec<String>,
  /// Extra environment. Values use the same `env:VAR` / `file:PATH` /
  /// literal forms as `api_key`, so a token can be passed to a server without
  /// being written into the config file.
  #[serde(default)]
  pub env: std::collections::BTreeMap<String, String>,
  #[serde(default = "default_true")]
  pub enabled: bool,
  /// Ceiling on the handshake. A server that hangs must not stop the REPL
  /// from opening.
  #[serde(default = "default_mcp_timeout")]
  pub startup_timeout_secs: u64,
}

fn default_true() -> bool {
  true
}

fn default_mcp_timeout() -> u64 {
  10
}

impl McpServerConfig {
  /// Environment for the child, with indirections resolved.
  ///
  /// A value that names a missing variable is dropped with a warning rather
  /// than passed through literally — handing a server the string `env:TOKEN`
  /// produces a confusing auth error instead of an obvious configuration one.
  pub fn resolved_env(&self) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, spec) in &self.env {
      let value = if let Some(var) = spec.strip_prefix("env:") {
        match std::env::var(var) {
          Ok(v) => Some(v),
          Err(_) => {
            eprintln!(
              "[MCP] server `{}`: ${} is not set; {} will be unset",
              self.name, var, key
            );
            None
          }
        }
      } else if let Some(path) = spec.strip_prefix("file:") {
        match fs::read_to_string(shellexpand_home(path)) {
          Ok(v) => Some(v.trim().to_string()),
          Err(e) => {
            eprintln!("[MCP] server `{}`: cannot read {}: {}", self.name, path, e);
            None
          }
        }
      } else {
        Some(spec.clone())
      };
      if let Some(v) = value {
        out.push((key.clone(), v));
      }
    }
    out
  }
}

/// CNY per million tokens. Published rates move and differ by model, so a
/// hard-coded figure turns into a quietly wrong bill.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct PricingConfig {
  #[serde(default = "default_cache_hit_cny")]
  pub cache_hit_cny_per_m: f64,
  #[serde(default = "default_cache_miss_cny")]
  pub cache_miss_cny_per_m: f64,
  #[serde(default = "default_output_cny")]
  pub output_cny_per_m: f64,
}

fn default_cache_hit_cny() -> f64 {
  0.5
}
fn default_cache_miss_cny() -> f64 {
  2.0
}
fn default_output_cny() -> f64 {
  3.0
}

impl Default for PricingConfig {
  fn default() -> Self {
    Self {
      cache_hit_cny_per_m: default_cache_hit_cny(),
      cache_miss_cny_per_m: default_cache_miss_cny(),
      output_cny_per_m: default_output_cny(),
    }
  }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ResilienceConfig {
  #[serde(default = "default_max_attempts")]
  pub max_attempts: u32,
  #[serde(default = "default_base_delay_ms")]
  pub base_delay_ms: u64,
  #[serde(default = "default_max_delay_secs")]
  pub max_delay_secs: u64,
  #[serde(default = "default_request_timeout_secs")]
  pub request_timeout_secs: u64,
  #[serde(default = "default_stream_idle_timeout_secs")]
  pub stream_idle_timeout_secs: u64,
}

fn default_max_attempts() -> u32 {
  4
}
fn default_base_delay_ms() -> u64 {
  500
}
fn default_max_delay_secs() -> u64 {
  30
}
fn default_request_timeout_secs() -> u64 {
  120
}
fn default_stream_idle_timeout_secs() -> u64 {
  60
}

impl Default for ResilienceConfig {
  fn default() -> Self {
    Self {
      max_attempts: default_max_attempts(),
      base_delay_ms: default_base_delay_ms(),
      max_delay_secs: default_max_delay_secs(),
      request_timeout_secs: default_request_timeout_secs(),
      stream_idle_timeout_secs: default_stream_idle_timeout_secs(),
    }
  }
}

/// Built-in DeepSeek endpoints, kept so a config that predates `[[provider]]`
/// (or one that never needed a custom endpoint) keeps working untouched.
fn builtin_provider(wire: &str) -> Option<ProviderConfig> {
  let base_url = match wire {
    "openai" => std::env::var("DEEPSEEK_API_BASE")
      .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string()),
    "anthropic" => std::env::var("DEEPSEEK_ANTHROPIC_BASE")
      .unwrap_or_else(|_| "https://api.deepseek.com/anthropic".to_string()),
    _ => return None,
  };
  Some(ProviderConfig {
    name: wire.to_string(),
    wire: wire.to_string(),
    base_url,
    api_key: "env:DEEPSEEK_API_KEY".to_string(),
  })
}

impl Config {
  /// Resolve `brain.provider` to a concrete endpoint.
  pub fn resolve_provider(&self) -> Result<ProviderConfig> {
    if let Some(p) = self
      .providers
      .iter()
      .find(|p| p.name == self.brain.provider)
    {
      if builtin_provider(&p.wire).is_none() {
        anyhow::bail!(
          "provider `{}` has unknown wire `{}`; expected \"openai\" or \"anthropic\"",
          p.name,
          p.wire
        );
      }
      return Ok(p.clone());
    }
    builtin_provider(&self.brain.provider).ok_or_else(|| {
      let known: Vec<&str> = self.providers.iter().map(|p| p.name.as_str()).collect();
      anyhow::anyhow!(
        "brain.provider = `{}` matches no [[provider]] entry {:?} and is not a \
         built-in wire name (\"openai\" / \"anthropic\")",
        self.brain.provider,
        known
      )
    })
  }
}

impl ProviderConfig {
  /// Read the key from wherever `api_key` points.
  pub fn resolve_key(&self) -> Result<String> {
    if let Some(var) = self.api_key.strip_prefix("env:") {
      return std::env::var(var)
        .with_context(|| format!("provider `{}`: ${} is not set", self.name, var));
    }
    if let Some(path) = self.api_key.strip_prefix("file:") {
      let expanded = shellexpand_home(path);
      let raw = fs::read_to_string(&expanded).with_context(|| {
        format!(
          "provider `{}`: cannot read {}",
          self.name,
          expanded.display()
        )
      })?;
      return Ok(raw.trim().to_string());
    }
    anyhow::bail!(
      "provider `{}`: api_key must be `env:VAR` or `file:PATH`, not a literal key \
       (a key in a config file gets committed by accident)",
      self.name
    )
  }
}

/// Minimal `~` expansion so `file:~/.config/key` works without a new dependency.
fn shellexpand_home(path: &str) -> PathBuf {
  match path.strip_prefix("~/") {
    Some(rest) => match std::env::var("HOME") {
      Ok(home) => PathBuf::from(home).join(rest),
      Err(_) => PathBuf::from(path),
    },
    None => PathBuf::from(path),
  }
}

/// User-extensible allow/deny lists for the three-state command policy.
/// Patterns are matched case-insensitively as substrings of the command.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct SecurityConfig {
  /// Commands matching any allow pattern skip the interactive prompt even if
  /// a built-in rule would otherwise ask.
  #[serde(default)]
  pub allow: Vec<String>,
  /// Commands matching any deny pattern are blocked outright (no prompt).
  #[serde(default)]
  pub deny: Vec<String>,
}

/// Where `tasks::run_task` looks for reminders.md/todos.md/digest/. Optional
/// override so the user can point it at a synced location (e.g. iCloud
/// Drive) instead of the default `~/.seekcli/tasks`.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct TasksConfig {
  #[serde(default)]
  pub dir: Option<String>,
}

/// What one run may spend before it is stopped.
///
/// Every other ceiling in the harness bounds one dimension: `MAX_ITER` bounds
/// the main loop's turns, a sub-agent template bounds its own iterations. None
/// of them bounds the product — and one turn may emit any number of tool
/// calls, each `invoke_agent` among them costing a whole sub-agent run. The
/// worst case was roughly `MAX_ITER x N x 20` with N unbounded.
///
/// Counting total LLM calls closes every dimension at once, including the one
/// nobody was watching. It also makes "a sub-agent inherits its parent's
/// budget" true by construction rather than by plumbing: both charge the same
/// counter.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LimitsConfig {
  /// Total model calls one run may make, sub-agents included.
  ///
  /// The default is deliberately far above ordinary use — a six-turn research
  /// run with eight page fetches cost 12 — so it catches runaway loops without
  /// interrupting real work.
  #[serde(default = "default_max_llm_calls")]
  pub max_llm_calls_per_run: u64,
}

fn default_max_llm_calls() -> u64 {
  150
}

impl Default for LimitsConfig {
  fn default() -> Self {
    Self {
      max_llm_calls_per_run: default_max_llm_calls(),
    }
  }
}

/// Decision-path tracing.
///
/// On by default, which is the whole point: tracing answers questions that
/// only arise *after* something looked wrong, and it cannot be turned on
/// retroactively. Opt-in tracing is available exactly when you already knew
/// you would need it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TraceConfig {
  /// `SEEKCLI_TRACE` overrides this either way (`0` / `off` / `false` to
  /// disable, any other value to enable).
  #[serde(default = "default_trace_enabled")]
  pub enabled: bool,
  /// How many traces to keep. Measured at 7-45 KB each, so the default is a
  /// few MB — enough to still have last week's run, bounded so the directory
  /// cannot grow without limit. `0` keeps everything.
  #[serde(default = "default_trace_keep")]
  pub keep: usize,
}

fn default_trace_enabled() -> bool {
  true
}

fn default_trace_keep() -> usize {
  200
}

impl Default for TraceConfig {
  fn default() -> Self {
    Self {
      enabled: default_trace_enabled(),
      keep: default_trace_keep(),
    }
  }
}

/// When the tools-free deliberation pass runs.
///
/// It had no switch of its own: the macro trigger read `thinking_mode`, so
/// "show me the reasoning" and "deliberate before acting" were the same knob —
/// and since thinking defaults to `None`, the macro trigger could not fire at
/// all out of the box. Two behaviours on one control, one of them unreachable.
///
/// The micro trigger (plan after a failed turn) is not configurable and is not
/// meant to be: it fires on observed evidence, and turning that off is asking
/// the loop to repeat a failure it already saw.
/// `deny_unknown_fields` here and not elsewhere is deliberate: this section
/// has exactly one field, so a typo in it cannot be noticed any other way —
/// the value would simply default to `false` and the feature would be silently
/// off, which is the failure this whole section exists to end.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PlanningConfig {
  /// Deliberate once at the start of every chat turn, before the first tool.
  ///
  /// Off by default because it costs one model call per turn, and most turns
  /// of a daily-driver CLI do not need it. `/deliberate on` turns it on for
  /// the task in front of you.
  #[serde(default)]
  pub on_open: bool,
}

/// The backend behind `web_search` / `web_fetch`.
///
/// Absent or `provider = "none"` means the two tools are not registered at
/// all, rather than registered and failing on every call. A tool the model can
/// see but never use costs a slot in the schema and a wasted turn each time it
/// is tried.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ResearchConfig {
  /// `"tavily"` or `"none"`.
  #[serde(default = "default_research_provider")]
  pub provider: String,
  /// `env:VAR` or `file:PATH`, never a literal key — same rule as `api_key`,
  /// for the same reason: config files get committed by accident.
  #[serde(default)]
  pub api_key: String,
  #[serde(default = "default_max_results")]
  pub max_results: usize,
}

fn default_research_provider() -> String {
  "none".to_string()
}

fn default_max_results() -> usize {
  5
}

impl Default for ResearchConfig {
  fn default() -> Self {
    Self {
      provider: default_research_provider(),
      api_key: String::new(),
      max_results: default_max_results(),
    }
  }
}

impl ResearchConfig {
  /// Whether the research tools should be offered this run.
  pub fn is_enabled(&self) -> bool {
    self.provider != "none" && !self.provider.is_empty()
  }

  /// Read the key from wherever `api_key` points.
  pub fn resolve_key(&self) -> Result<String> {
    if let Some(var) = self.api_key.strip_prefix("env:") {
      return std::env::var(var).with_context(|| format!("research.api_key: ${} is not set", var));
    }
    if let Some(path) = self.api_key.strip_prefix("file:") {
      let expanded = shellexpand_home(path);
      let raw = fs::read_to_string(&expanded)
        .with_context(|| format!("research.api_key: cannot read {}", expanded.display()))?;
      return Ok(raw.trim().to_string());
    }
    anyhow::bail!(
      "research.api_key must be `env:VAR` or `file:PATH`, not a literal key \
       (a key in a config file gets committed by accident)"
    )
  }
}

/// When to compact, expressed relative to the model's context window.
///
/// This replaced a bare `const COMPRESSION_THRESHOLD_TOKENS = 150_000`. The
/// constant was wrong in a way that could not be seen: SeekCLI picks its
/// provider statically from config and speaks two wire protocols, so the
/// window behind that threshold varies per install. Configure a model whose
/// window is under 150K and compression never fires — the request just fails,
/// with nothing in the log pointing at the reason.
///
/// The defaults are chosen to reproduce the old constant exactly
/// (200_000 * 0.75 = 150_000), so upgrading changes no behaviour.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MemoryConfig {
  /// The configured model's context window, in tokens.
  #[serde(default = "default_context_window")]
  pub context_window_tokens: usize,
  /// Fraction of the window at which compaction trips. The remainder is left
  /// for the system prompts, the next tool result, and the model's own output.
  #[serde(default = "default_compact_at_ratio")]
  pub compact_at_ratio: f64,
  /// Trailing messages held out of compaction as protected working memory.
  #[serde(default = "default_keep_tail")]
  pub keep_tail_messages: usize,
}

fn default_context_window() -> usize {
  200_000
}

fn default_compact_at_ratio() -> f64 {
  0.75
}

fn default_keep_tail() -> usize {
  8
}

impl Default for MemoryConfig {
  fn default() -> Self {
    Self {
      context_window_tokens: default_context_window(),
      compact_at_ratio: default_compact_at_ratio(),
      keep_tail_messages: default_keep_tail(),
    }
  }
}

fn default_provider() -> String {
  "openai".to_string()
}

impl Default for Config {
  fn default() -> Self {
    Self {
      brain: BrainConfig {
        flash_model: "deepseek-flash".to_string(),
        pro_model: "deepseek-v4-pro".to_string(),
        provider: default_provider(),
      },
      providers: Vec::new(),
      resilience: ResilienceConfig::default(),
      mcp_servers: Vec::new(),
      pricing: PricingConfig::default(),
      security: SecurityConfig::default(),
      tasks: TasksConfig::default(),
      memory: MemoryConfig::default(),
      research: ResearchConfig::default(),
      limits: LimitsConfig::default(),
      planning: PlanningConfig::default(),
      trace: TraceConfig::default(),
    }
  }
}

/// Where each layer lives. Split out from `Config::load` so tests can point
/// every layer at a temp directory without touching `$HOME` or the real cwd.
#[derive(Debug, Clone)]
pub struct Layout {
  pub user: PathBuf,
  pub project: PathBuf,
  pub legacy: PathBuf,
  pub explicit: Option<PathBuf>,
}

impl Layout {
  pub fn from_env() -> Result<Self> {
    let home = std::env::var("HOME").context("cannot determine HOME directory")?;
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    Ok(Self {
      user: PathBuf::from(home).join(".seekcli").join("config.toml"),
      project: cwd.join(PROJECT_CONFIG_FILE),
      legacy: cwd.join(LEGACY_CONFIG_FILE),
      explicit: std::env::var(CONFIG_ENV).ok().map(PathBuf::from),
    })
  }
}

/// A loaded config plus what the user should be told about how it was loaded.
pub struct Loaded {
  pub config: Config,
  /// Files that actually contributed, lowest priority first.
  pub sources: Vec<PathBuf>,
  /// Startup notices (generated default, legacy file found). Printed to
  /// stderr by the caller so `--output json` on stdout stays parseable.
  pub notices: Vec<String>,
}

impl Config {
  pub fn load() -> Result<Loaded> {
    Self::load_from(&Layout::from_env()?)
  }

  pub fn load_from(layout: &Layout) -> Result<Loaded> {
    let mut notices = Vec::new();
    let user_existed = layout.user.exists();

    // Check for the pre-0.2 `./config.toml` BEFORE generating the user file,
    // otherwise the freshly created default would suppress the notice.
    if let Some(notice) = legacy_notice(layout, user_existed) {
      notices.push(notice);
    }

    if !user_existed && layout.explicit.is_none() {
      match write_default_config(&layout.user) {
        Ok(()) => notices.push(format!(
          "[Config] generated default config at {}",
          layout.user.display()
        )),
        // A missing config is recoverable — built-in defaults still apply.
        // Refusing to start because we could not write a convenience file
        // would be worse than running with defaults.
        Err(e) => notices.push(format!(
          "[Config] could not create {} ({}); continuing with built-in defaults",
          layout.user.display(),
          e
        )),
      }
    }

    let mut merged = toml::Value::try_from(Config::default())
      .context("built-in default config is not serializable")?;
    let mut sources = Vec::new();

    for path in layout.layers() {
      if !path.exists() {
        continue;
      }
      let text = fs::read_to_string(&path)
        .with_context(|| format!("cannot read config file {}", path.display()))?;
      let value: toml::Value =
        toml::from_str(&text).with_context(|| format!("invalid TOML in {}", path.display()))?;
      merge(&mut merged, value);
      sources.push(path);
    }

    // An explicitly requested file that does not exist is a user error, not a
    // fallback: they asked for a specific config and did not get it.
    if let Some(explicit) = &layout.explicit
      && !explicit.exists()
    {
      anyhow::bail!(
        "{} points at {}, which does not exist",
        CONFIG_ENV,
        explicit.display()
      );
    }

    let config: Config = merged
      .try_into()
      .context("config does not match the expected schema")?;

    Ok(Loaded {
      config,
      sources,
      notices,
    })
  }
}

impl Layout {
  /// Layer paths in increasing priority.
  fn layers(&self) -> Vec<PathBuf> {
    let mut out = vec![self.user.clone(), self.project.clone()];
    if let Some(explicit) = &self.explicit {
      out.push(explicit.clone());
    }
    out
  }
}

/// Tell the user about a pre-0.2 `./config.toml` that is no longer read.
///
/// Deliberately a notice and not an automatic move: relocating a file the
/// user put somewhere is not ours to do silently, and the settings inside may
/// have been meant for this directory only.
fn legacy_notice(layout: &Layout, user_existed: bool) -> Option<String> {
  if user_existed || !layout.legacy.exists() {
    return None;
  }
  Some(format!(
    "[Config] found a legacy {} in this directory; it is no longer read.\n\
     \x20         SeekCLI now loads {} (plus an optional ./{} override).\n\
     \x20         To carry those settings over (overwrites the generated default):\n\
     \x20           cp {} {}",
    LEGACY_CONFIG_FILE,
    layout.user.display(),
    PROJECT_CONFIG_FILE,
    layout.legacy.display(),
    layout.user.display()
  ))
}

/// Deep-merge `overlay` onto `base`. Tables merge key by key; every other
/// value (including arrays) replaces wholesale, so an override that sets
/// `security.deny` means exactly that list rather than an append.
fn merge(base: &mut toml::Value, overlay: toml::Value) {
  match (base, overlay) {
    (toml::Value::Table(base), toml::Value::Table(overlay)) => {
      for (key, value) in overlay {
        match base.get_mut(&key) {
          Some(existing) => merge(existing, value),
          None => {
            base.insert(key, value);
          }
        }
      }
    }
    (base, overlay) => *base = overlay,
  }
}

/// Hand-written so the generated file carries comments; `toml::to_string`
/// would emit valid but undocumented TOML.
fn write_default_config(path: &Path) -> Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
  }
  let d = Config::default();
  let body = format!(
    "# SeekCLI configuration.\n\
     #\n\
     # Layers, in increasing priority:\n\
     #   1. built-in defaults\n\
     #   2. this file\n\
     #   3. ./{project} in the working directory (partial override)\n\
     #   4. ${env} (partial override, highest priority)\n\
     #\n\
     # An override file only needs the keys it changes.\n\
     \n\
     [brain]\n\
     # Either a [[provider]] name below, or the bare wire name\n\
     # \"openai\" / \"anthropic\" for the built-in DeepSeek endpoints.\n\
     provider = \"{provider}\"\n\
     flash_model = \"{flash}\"\n\
     pro_model = \"{pro}\"\n\
     \n\
     # Named endpoints. Omit this entirely to use the built-in DeepSeek ones.\n\
     # api_key must be `env:VAR` or `file:PATH` -- a literal key is refused,\n\
     # because config files get committed by accident.\n\
     #\n\
     # [[provider]]\n\
     # name     = \"local\"\n\
     # wire     = \"openai\"            # openai | anthropic\n\
     # base_url = \"http://127.0.0.1:8000/v1\"\n\
     # api_key  = \"env:LOCAL_API_KEY\"\n\
     \n\
     # MCP servers. Each is launched as a child process on startup; a server\n\
     # that fails or hangs is skipped with a warning, never blocking startup.\n\
     # Tools appear as `mcp__<server>__<tool>` and pass the same policy gate\n\
     # as built-ins, so --read-only applies to them too.\n\
     #\n\
     # [[mcp]]\n\
     # name    = \"filesystem\"\n\
     # command = \"npx\"\n\
     # args    = [\"-y\", \"@modelcontextprotocol/server-filesystem\", \".\"]\n\
     # env     = {{ }}                 # values accept env: / file: like api_key\n\
     # enabled = true\n\
     # startup_timeout_secs = 10\n\
     \n\
     # Retry / timeout. Shown with the built-in defaults.\n\
     # Only the initial request is retried; once the stream is flowing a\n\
     # failure goes to L1 error recovery instead, so a partially-applied\n\
     # tool call is never replayed.\n\
     [resilience]\n\
     max_attempts = {attempts}\n\
     base_delay_ms = {base_delay}\n\
     max_delay_secs = {max_delay}\n\
     request_timeout_secs = {req_timeout}\n\
     stream_idle_timeout_secs = {idle_timeout}\n\
     \n\
     # Shell-command permission policy. Patterns are matched case-insensitively\n\
     # as substrings. Priority: deny > built-in deny > allow > built-in ask.\n\
     [security]\n\
     allow = []\n\
     deny = []\n\
     \n\
     [tasks]\n\
     # Where --run-task keeps reminders.md / todos.md / digest/.\n\
     # Defaults to ~/.seekcli/tasks when unset.\n\
     # dir = \"~/Library/Mobile Documents/com~apple~CloudDocs/seekcli-tasks\"\n\
     \n\
     # When to compact the conversation. The threshold is derived, not fixed:\n\
     #   compact_at = context_window_tokens * compact_at_ratio\n\
     # Set context_window_tokens to YOUR model's window. Leaving it too high\n\
     # means compaction never fires and requests fail on length instead.\n\
     [memory]\n\
     context_window_tokens = {window}\n\
     compact_at_ratio = {ratio}\n\
     keep_tail_messages = {keep_tail}\n\
     \n\
     # web_search / web_fetch. Left as \"none\" the two tools are not offered\n\
     # at all -- a tool the model can see but never use costs a schema slot\n\
     # and a wasted turn every time it is tried.\n\
     #\n\
     # api_key must be `env:VAR` or `file:PATH`, same rule as above.\n\
     [research]\n\
     provider = \"{research_provider}\"       # tavily | none\n\
     # api_key = \"env:TAVILY_API_KEY\"\n\
     max_results = {max_results}\n\
     \n\
     # Ceiling on one run, sub-agents included. Every other limit bounds a\n\
     # single dimension (main-loop turns, a sub-agent's iterations); none\n\
     # bounds the product, and one turn may emit any number of tool calls.\n\
     # Counting model calls closes all of them at once.\n\
     [limits]\n\
     max_llm_calls_per_run = {max_calls}\n\
     \n\
     # Deliberate once before acting, with tools withheld, at the start of\n\
     # every chat turn. Costs one model call per turn, so it is off by\n\
     # default; `/deliberate on` enables it for the task in front of you.\n\
     # Planning after a *failed* turn always happens and is not configurable.\n\
     [planning]\n\
     on_open = {plan_on_open}\n\
     \n\
     # Decision-path tracing: one JSON span tree per run under\n\
     # ~/.seekcli/traces, viewable with `/trace`. On by default because it\n\
     # answers questions that only arise after something looked wrong, and\n\
     # it cannot be turned on retroactively. 7-45 KB per run; `keep` bounds\n\
     # the directory (0 = keep everything). SEEKCLI_TRACE overrides `enabled`.\n\
     [trace]\n\
     enabled = {trace_enabled}\n\
     keep = {trace_keep}\n",
    project = PROJECT_CONFIG_FILE,
    env = CONFIG_ENV,
    provider = d.brain.provider,
    flash = d.brain.flash_model,
    pro = d.brain.pro_model,
    attempts = d.resilience.max_attempts,
    base_delay = d.resilience.base_delay_ms,
    max_delay = d.resilience.max_delay_secs,
    req_timeout = d.resilience.request_timeout_secs,
    idle_timeout = d.resilience.stream_idle_timeout_secs,
    window = d.memory.context_window_tokens,
    ratio = d.memory.compact_at_ratio,
    keep_tail = d.memory.keep_tail_messages,
    research_provider = d.research.provider,
    max_results = d.research.max_results,
    max_calls = d.limits.max_llm_calls_per_run,
    plan_on_open = d.planning.on_open,
    trace_enabled = d.trace.enabled,
    trace_keep = d.trace.keep,
  );
  fs::write(path, body).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Layout with every layer inside `root`, and no layer present yet.
  fn layout(root: &Path) -> Layout {
    Layout {
      user: root.join("home/.seekcli/config.toml"),
      project: root.join("work/.seekcli.toml"),
      legacy: root.join("work/config.toml"),
      explicit: None,
    }
  }

  fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("seekcli-config-test-{}", name));
    let _ = fs::remove_dir_all(&root);
    let _ = fs::create_dir_all(root.join("work"));
    root
  }

  fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
      let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, body);
  }

  /// A misspelled key in `[planning]` must be an error, not a silent `false`.
  ///
  /// This section has one field, so there is no other way to notice: the
  /// feature would simply stay off and the config file would look like it was
  /// on. That is why `deny_unknown_fields` is here and nowhere else.
  #[test]
  fn a_typo_in_the_planning_section_is_refused() {
    let good: Config = match toml::from_str(
      "[brain]\nprovider = \"openai\"\nflash_model = \"f\"\npro_model = \"p\"\n[planning]\non_open = true\n",
    ) {
      Ok(c) => c,
      Err(e) => panic!("valid config rejected: {e}"),
    };
    assert!(good.planning.on_open, "on_open = true must be read");

    let typo = toml::from_str::<Config>(
      "[brain]\nprovider = \"openai\"\nflash_model = \"f\"\npro_model = \"p\"\n[planning]\non_opne = true\n",
    );
    assert!(
      typo.is_err(),
      "a misspelled key silently left deliberation off"
    );
  }

  /// The generated template must parse back into the very defaults it was
  /// rendered from. Nothing checked this before, and `[memory]` introduced the
  /// first *float* into the template — a rendering that TOML would not accept
  /// (or that rounds) breaks first run for everyone, silently, on a path no
  /// existing test walked.
  #[test]
  fn the_generated_template_parses_back_into_its_own_defaults() {
    let root = temp_root("template-roundtrip");
    let path = root.join("generated.toml");
    if let Err(e) = write_default_config(&path) {
      panic!("cannot write template: {e}");
    }
    let text = match fs::read_to_string(&path) {
      Ok(t) => t,
      Err(e) => panic!("cannot read back: {e}"),
    };
    let parsed: Config = match toml::from_str(&text) {
      Ok(c) => c,
      Err(e) => panic!("generated template is not valid TOML / schema: {e}\n{text}"),
    };
    assert_eq!(
      parsed,
      Config::default(),
      "the template must round-trip to the defaults it claims to show"
    );
  }

  #[test]
  fn generates_user_config_on_first_run_and_never_touches_cwd() {
    let root = temp_root("first-run");
    let l = layout(&root);

    let loaded = match Config::load_from(&l) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };

    assert!(l.user.exists(), "user config should be generated");
    assert!(
      !l.project.exists(),
      "must not write into the working directory"
    );
    assert!(
      !l.legacy.exists(),
      "must not write into the working directory"
    );
    assert_eq!(loaded.config, Config::default());
    assert!(
      loaded
        .notices
        .iter()
        .any(|n| n.contains("generated default"))
    );
  }

  #[test]
  fn project_layer_overrides_user_layer_per_key() {
    let root = temp_root("project-override");
    let l = layout(&root);
    write(
      &l.user,
      "[brain]\nprovider = \"openai\"\nflash_model = \"user-flash\"\npro_model = \"user-pro\"\n",
    );
    // Mentions one key only; the rest must survive from the layer below.
    write(&l.project, "[brain]\nflash_model = \"project-flash\"\n");

    let loaded = match Config::load_from(&l) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert_eq!(loaded.config.brain.flash_model, "project-flash");
    assert_eq!(loaded.config.brain.pro_model, "user-pro");
    assert_eq!(loaded.config.brain.provider, "openai");
    assert_eq!(loaded.sources.len(), 2);
  }

  #[test]
  fn explicit_env_layer_wins_over_project() {
    let root = temp_root("explicit");
    let mut l = layout(&root);
    write(
      &l.user,
      "[brain]\nflash_model = \"user\"\npro_model = \"p\"\n",
    );
    write(&l.project, "[brain]\nflash_model = \"project\"\n");
    let explicit = root.join("explicit.toml");
    write(&explicit, "[brain]\nflash_model = \"explicit\"\n");
    l.explicit = Some(explicit);

    let loaded = match Config::load_from(&l) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert_eq!(loaded.config.brain.flash_model, "explicit");
  }

  #[test]
  fn missing_explicit_file_is_an_error_not_a_fallback() {
    let root = temp_root("explicit-missing");
    let mut l = layout(&root);
    l.explicit = Some(root.join("nope.toml"));
    assert!(Config::load_from(&l).is_err());
  }

  #[test]
  fn partial_layer_keeps_defaults_for_absent_sections() {
    let root = temp_root("partial");
    let l = layout(&root);
    write(&l.user, "[security]\ndeny = [\"shutdown\"]\n");

    let loaded = match Config::load_from(&l) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert_eq!(loaded.config.security.deny, vec!["shutdown".to_string()]);
    assert_eq!(loaded.config.brain, Config::default().brain);
  }

  #[test]
  fn legacy_config_in_cwd_produces_a_migration_notice() {
    let root = temp_root("legacy");
    let l = layout(&root);
    write(&l.legacy, "[brain]\nflash_model = \"legacy\"\n");

    let loaded = match Config::load_from(&l) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert!(
      loaded.notices.iter().any(|n| n.contains("legacy")),
      "expected a migration notice, got {:?}",
      loaded.notices
    );
    // Notice only — the legacy file must not be read.
    assert_eq!(
      loaded.config.brain.flash_model,
      Config::default().brain.flash_model
    );
  }

  #[test]
  fn no_legacy_notice_once_the_user_config_exists() {
    let root = temp_root("legacy-settled");
    let l = layout(&root);
    write(&l.legacy, "[brain]\nflash_model = \"legacy\"\n");
    write(
      &l.user,
      "[brain]\nflash_model = \"user\"\npro_model = \"p\"\n",
    );

    let loaded = match Config::load_from(&l) {
      Ok(v) => v,
      Err(e) => panic!("load failed: {}", e),
    };
    assert!(loaded.notices.is_empty(), "got {:?}", loaded.notices);
  }

  #[test]
  fn invalid_toml_names_the_offending_file() {
    let root = temp_root("invalid");
    let l = layout(&root);
    write(&l.user, "[brain\nflash_model =");

    let err = match Config::load_from(&l) {
      Ok(_) => panic!("expected a parse error"),
      Err(e) => format!("{:#}", e),
    };
    assert!(err.contains("config.toml"), "unhelpful error: {}", err);
  }
}

#[cfg(test)]
mod provider_tests {
  use super::*;

  fn base() -> Config {
    Config::default()
  }

  #[test]
  fn legacy_bare_wire_name_still_resolves_to_the_builtin_endpoint() {
    // Configs written before [[provider]] existed say provider = "openai".
    let mut c = base();
    c.brain.provider = "openai".into();
    let p = match c.resolve_provider() {
      Ok(p) => p,
      Err(e) => panic!("legacy config must keep working: {}", e),
    };
    assert_eq!(p.wire, "openai");
    assert!(p.base_url.contains("deepseek"), "got {}", p.base_url);
    assert_eq!(p.api_key, "env:DEEPSEEK_API_KEY");

    c.brain.provider = "anthropic".into();
    match c.resolve_provider() {
      Ok(p) => assert_eq!(p.wire, "anthropic"),
      Err(e) => panic!("legacy anthropic must keep working: {}", e),
    }
  }

  #[test]
  fn named_entry_takes_precedence_and_can_point_anywhere() {
    let mut c = base();
    c.brain.provider = "local".into();
    c.providers.push(ProviderConfig {
      name: "local".into(),
      wire: "openai".into(),
      base_url: "http://127.0.0.1:8000/v1".into(),
      api_key: "env:LOCAL_KEY".into(),
    });
    let p = match c.resolve_provider() {
      Ok(p) => p,
      Err(e) => panic!("resolve failed: {}", e),
    };
    assert_eq!(p.base_url, "http://127.0.0.1:8000/v1");
  }

  #[test]
  fn unknown_provider_name_lists_what_was_available() {
    let mut c = base();
    c.brain.provider = "typo".into();
    c.providers.push(ProviderConfig {
      name: "local".into(),
      wire: "openai".into(),
      base_url: "http://x".into(),
      api_key: "env:K".into(),
    });
    let err = match c.resolve_provider() {
      Ok(_) => panic!("expected an error"),
      Err(e) => format!("{:#}", e),
    };
    assert!(
      err.contains("local"),
      "error should name known providers: {}",
      err
    );
  }

  #[test]
  fn unknown_wire_is_rejected_at_resolve_time_not_at_first_request() {
    let mut c = base();
    c.brain.provider = "weird".into();
    c.providers.push(ProviderConfig {
      name: "weird".into(),
      wire: "grpc".into(),
      base_url: "http://x".into(),
      api_key: "env:K".into(),
    });
    assert!(c.resolve_provider().is_err());
  }

  #[test]
  fn literal_api_key_is_refused() {
    let p = ProviderConfig {
      name: "oops".into(),
      wire: "openai".into(),
      base_url: "http://x".into(),
      api_key: "sk-realkeyinconfigfile".into(),
    };
    let err = match p.resolve_key() {
      Ok(_) => panic!("a literal key must not be accepted"),
      Err(e) => format!("{:#}", e),
    };
    assert!(
      err.contains("env:"),
      "error should show the accepted forms: {}",
      err
    );
  }

  #[test]
  fn file_backed_key_is_read_and_trimmed() {
    let dir = std::env::temp_dir().join("seekcli-key-test");
    let _ = fs::create_dir_all(&dir);
    let key_path = dir.join("key");
    let _ = fs::write(&key_path, "  sk-from-file\n");
    let p = ProviderConfig {
      name: "f".into(),
      wire: "openai".into(),
      base_url: "http://x".into(),
      api_key: format!("file:{}", key_path.display()),
    };
    match p.resolve_key() {
      Ok(k) => assert_eq!(k, "sk-from-file"),
      Err(e) => panic!("resolve_key failed: {}", e),
    }
  }

  #[test]
  fn missing_env_var_names_the_provider_and_the_variable() {
    let p = ProviderConfig {
      name: "myprov".into(),
      wire: "openai".into(),
      base_url: "http://x".into(),
      api_key: "env:SEEKCLI_DEFINITELY_UNSET_VAR".into(),
    };
    let err = match p.resolve_key() {
      Ok(_) => panic!("expected an error"),
      Err(e) => format!("{:#}", e),
    };
    assert!(err.contains("myprov"), "got: {}", err);
    assert!(err.contains("SEEKCLI_DEFINITELY_UNSET_VAR"), "got: {}", err);
  }
}
