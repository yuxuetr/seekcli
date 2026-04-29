use crate::api::Message;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Session {
  pub id: String,
  pub title: String,
  pub messages: Vec<Message>,
  pub model: String,
  pub timestamp: DateTime<Utc>,
}

pub struct HistoryManager {
  pub base_dir: PathBuf,
}

impl HistoryManager {
  pub fn new() -> Result<Self> {
    let home = std::env::var("HOME").context("Could not find HOME directory")?;
    let base_dir = PathBuf::from(home).join(".seekcli");
    let sessions_dir = base_dir.join("sessions");
    let skills_dir = base_dir.join("skills");

    if !sessions_dir.exists() {
      fs::create_dir_all(&sessions_dir)?;
    }
    if !skills_dir.exists() {
      fs::create_dir_all(&skills_dir)?;
    }

    Ok(Self {
      base_dir: sessions_dir,
    })
  }

  pub fn save_session(&self, session: &Session) -> Result<()> {
    let path = self.base_dir.join(format!("{}.json", session.id));
    let content = serde_json::to_string_pretty(session)?;
    fs::write(path, content)?;
    Ok(())
  }

  pub fn list_sessions(&self) -> Result<Vec<Session>> {
    let mut sessions = Vec::new();
    if !self.base_dir.exists() {
      return Ok(sessions);
    }
    for entry in fs::read_dir(&self.base_dir)? {
      let entry = entry?;
      let path = entry.path();
      if path.extension().and_then(|s| s.to_str()) == Some("json") {
        let content = fs::read_to_string(path)?;
        if let Ok(session) = serde_json::from_str::<Session>(&content) {
          sessions.push(session);
        }
      }
    }
    sessions.sort_by_key(|b| std::cmp::Reverse(b.timestamp));
    Ok(sessions)
  }

  pub fn load_session(&self, id: &str) -> Result<Session> {
    let path = self.base_dir.join(format!("{}.json", id));
    if !path.exists() {
      // Try prefix match
      let entries = fs::read_dir(&self.base_dir)?;
      for entry in entries {
        let entry = entry?;
        let name = entry.file_name().into_string().unwrap_or_default();
        if name.starts_with(id) {
          let content = fs::read_to_string(entry.path())?;
          return Ok(serde_json::from_str(&content)?);
        }
      }
    }
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
  }

  pub fn create_session(&self, model: String) -> Session {
    Session {
      id: Uuid::new_v4().to_string(),
      title: "New Chat".to_string(),
      messages: Vec::new(),
      model,
      timestamp: Utc::now(),
    }
  }
}
