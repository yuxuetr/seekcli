use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
  pub brain: BrainConfig,
  pub sensor: SensorConfig,
  /// Optional shell-command permission policy. Absent in older config files,
  /// so it defaults to empty (built-in rules only).
  #[serde(default)]
  pub security: SecurityConfig,
}

/// User-extensible allow/deny lists for the three-state command policy.
/// Patterns are matched case-insensitively as substrings of the command.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SecurityConfig {
  /// Commands matching any allow pattern skip the interactive prompt even if
  /// a built-in rule would otherwise ask.
  #[serde(default)]
  pub allow: Vec<String>,
  /// Commands matching any deny pattern are blocked outright (no prompt).
  #[serde(default)]
  pub deny: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrainConfig {
  pub flash_model: String,
  pub pro_model: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SensorConfig {
  pub vlm_model: String, // GLM VLM model name
}

impl Config {
  pub fn load() -> Result<Self> {
    let config_path = Path::new("config.toml");
    if config_path.exists() {
      let content = fs::read_to_string(config_path)?;
      Ok(toml::from_str(&content)?)
    } else {
      // Default values
      let default_config = Config {
        brain: BrainConfig {
          flash_model: "deepseek-v4-flash".to_string(),
          pro_model: "deepseek-v4-pro".to_string(),
        },
        sensor: SensorConfig {
          vlm_model: "step-1.5v-mini".to_string(),
        },
        security: SecurityConfig::default(),
      };
      let toml_str = toml::to_string_pretty(&default_config)?;
      fs::write(config_path, toml_str)?;
      Ok(default_config)
    }
  }
}
