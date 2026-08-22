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
  /// Optional shell-command permission policy. Absent in older config files,
  /// so it defaults to empty (built-in rules only).
  #[serde(default)]
  pub security: SecurityConfig,
  /// L8 scheduled-task state directory (reminders.md/todos.md/digest/).
  /// Absent in older config files, so it defaults to `~/.seekcli/tasks`.
  #[serde(default)]
  pub tasks: TasksConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BrainConfig {
  pub flash_model: String,
  pub pro_model: String,
  /// LLM wire protocol: "openai" (DeepSeek /chat/completions) or "anthropic"
  /// (DeepSeek /anthropic). Defaults to openai for older config files.
  #[serde(default = "default_provider")]
  pub provider: String,
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

fn default_provider() -> String {
  "openai".to_string()
}

impl Default for Config {
  fn default() -> Self {
    Self {
      brain: BrainConfig {
        flash_model: "deepseek-v4-flash".to_string(),
        pro_model: "deepseek-v4-pro".to_string(),
        provider: default_provider(),
      },
      security: SecurityConfig::default(),
      tasks: TasksConfig::default(),
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
     # Wire protocol: \"openai\" (DeepSeek /chat/completions)\n\
     #             or \"anthropic\" (DeepSeek /anthropic)\n\
     provider = \"{provider}\"\n\
     flash_model = \"{flash}\"\n\
     pro_model = \"{pro}\"\n\
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
     # dir = \"~/Library/Mobile Documents/com~apple~CloudDocs/seekcli-tasks\"\n",
    project = PROJECT_CONFIG_FILE,
    env = CONFIG_ENV,
    provider = d.brain.provider,
    flash = d.brain.flash_model,
    pro = d.brain.pro_model,
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
