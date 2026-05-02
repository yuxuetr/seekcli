use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
  pub brain: BrainConfig,
  pub sensor: SensorConfig,
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
      };
      let toml_str = toml::to_string_pretty(&default_config)?;
      fs::write(config_path, toml_str)?;
      Ok(default_config)
    }
  }
}
