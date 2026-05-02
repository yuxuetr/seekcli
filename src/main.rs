use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use base64::Engine;
use colored::*;
use futures_util::StreamExt;
use regex::Regex;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::env;
use std::io::{self, Write};
use termimad::MadSkin;

mod api;
mod config;
mod history;
mod skills;

#[cfg(test)]
mod test;

pub use api::{ApiClient, Message, StreamItem, ToolCall};
pub use config::Config;
pub use history::{HistoryManager, Session};
pub use skills::{Skill, SkillManager};

#[derive(Debug, PartialEq, Clone, Copy)]
enum ThinkingMode {
  None,
  High,
  Max,
}

impl ThinkingMode {
  fn label(&self) -> &str {
    match self {
      ThinkingMode::None => "None",
      ThinkingMode::High => "High",
      ThinkingMode::Max => "Max",
    }
  }
  fn as_str(&self) -> &str {
    match self {
      ThinkingMode::None => "none",
      ThinkingMode::High => "high",
      ThinkingMode::Max => "max",
    }
  }
}

struct App {
  brain: ApiClient,
  #[allow(dead_code)]
  vlm_sensor: Option<ApiClient>,
  doc_sensor: Option<ApiClient>,
  config: Config,
  history: HistoryManager,
  skill_manager: SkillManager,
  current_session: Session,
  model: String,
  thinking_mode: ThinkingMode,
  current_skill: Option<Skill>,
  auto_route: bool,
  last_code_blocks: Vec<String>,
}

impl App {
  fn new() -> Result<Self> {
    let config = Config::load()?;

    let deepseek_key = env::var("DEEPSEEK_API_KEY")
      .or_else(|_| env::var("DASHSCOPE_API_KEY"))
      .context("Please set DEEPSEEK_API_KEY or DASHSCOPE_API_KEY")?;

    let step_key = env::var("STEP_API_KEY").ok();
    let mineru_key = env::var("MINERU_API_KEY").ok();
    let jina_key = env::var("JINA_API_KEY").ok();
    let tavily_key = env::var("TAVILY_API_KEY").ok();

    let brain = ApiClient::new(
      deepseek_key,
      api::Provider::DeepSeek,
      jina_key.clone(),
      tavily_key.clone(),
    );
    let vlm_sensor = step_key.map(|key| {
      ApiClient::new(
        key,
        api::Provider::StepFun,
        jina_key.clone(),
        tavily_key.clone(),
      )
    });
    let doc_sensor =
      mineru_key.map(|key| ApiClient::new(key, api::Provider::MinerU, jina_key, tavily_key));

    let history = HistoryManager::new()?;
    let skill_manager = SkillManager::new()?;
    let model = config.brain.flash_model.clone();
    let current_session = history.create_session(model.clone());

    Ok(Self {
      brain,
      vlm_sensor,
      doc_sensor,
      config,
      history,
      skill_manager,
      current_session,
      model,
      thinking_mode: ThinkingMode::None,
      current_skill: None,
      auto_route: true,
      last_code_blocks: Vec::new(),
    })
  }

  fn print_help(&self) {
    println!("{}", "\nAvailable Commands:".bold().yellow());
    println!("  /model [flash|pro]  Switch between deepseek-v4 models (1M Context)");
    println!("  /thinking [n|h|m]   Switch thinking intensity");
    println!("  /skill list         List all skills");
    println!("  /skill auto [on|off] Toggle auto-routing");
    println!("  /image              VLM analyze image from clipboard (StepFun powered)");
    println!("  /file <path>        MinerU parse file to markdown context");
    println!("  /web <url>          Fetch and parse web page content");
    println!("  /search <query>     GLM Web Search");
    println!("  /tavily <query>     Tavily AI Search");
    println!("  /copy [index]       Copy code block from last response");
    println!("  /clear              Reset context (start a new 1M session)");
    println!("  /history            List sessions");
    println!("  /load [id]          Load and continue a session");
    println!("  /quit               Exit\n");
  }

  async fn route_skill(&mut self, input: &str) -> Result<Option<Skill>> {
    let skills = self.skill_manager.load_skills()?;
    if skills.is_empty() {
      return Ok(None);
    }

    let mut skill_desc = String::new();
    for s in &skills {
      skill_desc.push_str(&format!("- {}: {}\n", s.name, s.description));
    }

    let route_prompt = format!(
      "You are an expert intent classifier for DeepSeek V4 (1M Context). \n\
            Assign the input to the most relevant skill.\n\
            - Domains: Rust, IELTS, Dioxus, etc.\n\
            - Return ONLY the skill name or 'none'.\n\n\
            Skills:\n{}\n\nInput: {}",
      skill_desc, input
    );

    let messages = vec![Message::Simple {
      role: "user".to_string(),
      content: api::MessageContent::Text(route_prompt),
      reasoning_content: None,
      tool_calls: None,
    }];

    let mut stream = self
      .brain
      .call_api_with_params(&self.config.brain.flash_model, messages, "none", None)
      .await?;
    let mut selected_name = String::new();
    while let Some(item) = stream.next().await {
      if let Ok(StreamItem::Content(c)) = item {
        selected_name.push_str(&c);
      }
    }

    let name_lower = selected_name.to_lowercase();
    if name_lower.contains("none") {
      return Ok(None);
    }

    Ok(skills.into_iter().find(|s| {
      let s_name = s.name.to_lowercase();
      name_lower.contains(&s_name) || s_name.contains(name_lower.trim())
    }))
  }

  async fn run(&mut self) -> Result<()> {
    println!(
      "{}",
      format!("Welcome to SeekCLI (DeepSeek {} Powered)", self.model)
        .bold()
        .green()
    );
    let mut rl = DefaultEditor::new()?;

    loop {
      let skill_label = self
        .current_skill
        .as_ref()
        .map(|s| format!("|{}", s.name))
        .unwrap_or_default();
      let prompt = format!(
        "{} ({}{}{}) {} ",
        self.model.blue(),
        self.thinking_mode.label().magenta(),
        skill_label.yellow(),
        if self.auto_route { "|auto" } else { "" }.dimmed(),
        "❯".green()
      );

      let readline = rl.readline(&prompt);
      match readline {
        Ok(line) => {
          let line = line.trim();
          if line.is_empty() {
            continue;
          }
          rl.add_history_entry(line)?;

          if line.starts_with('/') {
            if self.handle_command(line).await? {
              break;
            }
          } else {
            if self.auto_route
              && let Ok(Some(skill)) = self.route_skill(line).await
              && self.current_skill.as_ref().map(|s| &s.name) != Some(&skill.name)
            {
              println!(
                "{} Switching skill -> {}",
                "Auto-Route:".blue(),
                skill.name.green()
              );
              self.activate_skill(skill);
            }

            if let Some(ref skill) = self.current_skill {
              println!(
                "{} Active Skill: {}",
                "✦".yellow(),
                skill.name.bold().green()
              );
            }

            self.chat(line).await?;
          }
        }
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
        Err(err) => {
          println!("Error: {:?}", err);
          break;
        }
      }
    }
    Ok(())
  }

  fn activate_skill(&mut self, skill: Skill) {
    self.current_skill = Some(skill.clone());
    if self.current_session.messages.is_empty() {
      self.current_session.messages.push(Message::Simple {
        role: "system".to_string(),
        content: api::MessageContent::Text(skill.system_prompt),
        reasoning_content: None,
        tool_calls: None,
      });
    }
  }

  async fn analyze_complex_file(&mut self, path: std::path::PathBuf) -> Result<String> {
    let ext_lower = path
      .extension()
      .and_then(|e| e.to_str())
      .unwrap_or("")
      .to_lowercase();
    let is_doc = [
      "pdf", "docx", "pptx", "xlsx", "png", "jpg", "jpeg", "webp", "bmp",
    ]
    .contains(&ext_lower.as_str());

    if is_doc && let Some(ref doc_sensor) = self.doc_sensor {
      println!(
        "{} [MinerU] Extracting content from: {:?}...",
        "✦".cyan(),
        path
      );
      match doc_sensor.mineru_extract(&path).await {
        Ok(task_id) => {
          let mut attempts = 0;
          print!("{} [MinerU] Processing: ", "✦".cyan());
          loop {
            attempts += 1;
            match doc_sensor.mineru_get_result(&task_id).await {
              Ok(res) if res.is_done() => {
                if let Some(url) = res.markdown_url {
                  let md = doc_sensor.fetch_url_content(&url).await?;
                  println!(
                    "\n{} High-fidelity extraction successful.",
                    "Success:".green()
                  );
                  println!(
                    "\n{}",
                    "┏━━━━━━━━━━━━━━━━━━━━━ MinerU 解析预览 ━━━━━━━━━━━━━━━━━━━━━┓".cyan()
                  );
                  let skin = MadSkin::default();
                  let preview = if md.len() > 1000 {
                    format!("{}...", &md[..1000])
                  } else {
                    md.clone()
                  };
                  skin.print_text(&preview);
                  println!(
                    "{}",
                    "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛".cyan()
                  );
                  return Ok(md);
                } else {
                  anyhow::bail!("MinerU completed but returned no content URL.");
                }
              }
              Ok(res) if res.state == "failed" => {
                anyhow::bail!(
                  "MinerU extraction failed: {}",
                  res.err_msg.unwrap_or_default()
                );
              }
              Ok(res) => {
                let status = if !res.state.is_empty() {
                  res.state
                } else {
                  "pending".to_string()
                };
                print!(
                  "\r{} [MinerU] Status: {} ({}s) ",
                  "✦".cyan(),
                  status,
                  attempts
                );
                io::stdout().flush()?;
              }
              Err(e) => anyhow::bail!("MinerU polling error: {}", e),
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            if attempts > 300 {
              anyhow::bail!("MinerU task timed out.");
            }
          }
        }
        Err(e) => anyhow::bail!("MinerU upload failed: {}", e),
      }
    } else {
      anyhow::bail!("Unsupported file format or MinerU not configured.")
    }
  }

  async fn paste_image(&mut self) -> Result<String> {
    println!("{} Accessing clipboard...", "✦".yellow());

    #[cfg(target_os = "macos")]
    {
      // 1. Try pbpaste for file paths first
      let output = std::process::Command::new("pbpaste").output();
      if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !text.is_empty() {
          let path_str = text.trim_start_matches("file://").trim();
          let path = std::path::PathBuf::from(path_str);
          if path.exists() && path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ["png", "jpg", "jpeg", "webp", "bmp"].contains(&ext.to_lowercase().as_str()) {
              println!("{} Detected image path in clipboard.", "✦".yellow());
              return self.analyze_complex_file(path.clone()).await;
            }
          }
        }
      }

      // 2. Use osascript to capture raw image to temp file then use GLM VLM for analysis as requested
      println!("{} Capturing raw image from clipboard...", "✦".yellow());
      let temp_path = "/tmp/seekcli_paste.png";
      let script = format!(
        "set theFile to (POSIX file \"{}\")\n\
         try\n\
           set theData to the clipboard as «class PNGf»\n\
           set theOpenFile to open for access theFile with write permission\n\
           set eof theOpenFile to 0\n\
           write theData to theOpenFile\n\
           close access theOpenFile\n\
           return \"success\"\n\
         on error\n\
           return \"no image\"\n\
         end try",
        temp_path
      );

      let _output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output();

      // Implementation of VLM analysis for /paste
      if let Some(ref vlm) = self.vlm_sensor {
        println!(
          "{} [VLM] Using {} for visual analysis...",
          "✦".yellow(),
          self.config.sensor.vlm_model
        );
        let bytes = std::fs::read(temp_path)?;
        let base64_image = base64::engine::general_purpose::STANDARD.encode(bytes);

        let messages = vec![Message::new_user_image(
          "请详细分析这张图片的内容。".to_string(),
          base64_image,
          "image/png",
        )];

        print!("{}", "VLM Thinking: ".italic().bright_black());
        let mut stream = vlm
          .call_api_with_params(&self.config.sensor.vlm_model, messages, "none", None)
          .await?;
        let mut description = String::new();

        while let Some(item) = stream.next().await {
          if let StreamItem::Content(c) = item? {
            print!("{}", c.italic().bright_black());
            description.push_str(&c);
          }
        }
        println!("\n{} VLM analysis completed.", "Success:".green());
        return Ok(description);
      }
    }

    anyhow::bail!("No image or supported path found in clipboard, or not on macOS.")
  }

  async fn handle_command(&mut self, line: &str) -> Result<bool> {
    let re_args = Regex::new(r#""([^"]*)"|'([^']*)'|(\S+)"#)?;
    let mut parts = Vec::new();
    for cap in re_args.captures_iter(line) {
      if let Some(m) = cap.get(1).or(cap.get(2)).or(cap.get(3)) {
        parts.push(m.as_str());
      }
    }

    if parts.is_empty() {
      return Ok(false);
    }
    let cmd = parts[0];

    match cmd {
      "/quit" | "/exit" => return Ok(true),
      "/image" | "/paste" => match self.paste_image().await {
        Ok(_) => {
          // Result is already printed in paste_image or analyze_complex_file
        }
        Err(e) => println!("{} Failed to analyze clipboard: {}", "Error:".red(), e),
      },
      "/file" => {
        if parts.len() > 1 {
          let path_str = parts[1];
          let path = std::path::PathBuf::from(path_str);
          if path.exists() {
            match self.analyze_complex_file(path.clone()).await {
              Ok(_) => {
                // Result is already printed in analyze_complex_file
              }
              Err(e) => println!("{} Failed to analyze file: {}", "Error:".red(), e),
            }
          } else {
            println!("{} File not found: {:?}", "Error:".red(), path_str);
          }
        } else {
          println!("{} Usage: /file <path>", "Info:".blue());
        }
      }
      "/web" => {
        if parts.len() > 1 {
          let url = parts[1];
          println!("{} Fetching web content from: {}...", "✦".cyan(), url);
          match self.brain.fetch_web_markdown(url).await {
            Ok(md) => {
              println!(
                "\n{}",
                "┏━━━━━━━━━━━━━━━━━━━━━ 网页解析预览 ━━━━━━━━━━━━━━━━━━━━━┓".cyan()
              );
              let skin = termimad::MadSkin::default();
              skin.print_text(&md);
              println!(
                "{}",
                "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛".cyan()
              );
            }
            Err(e) => println!("{} Failed to fetch web content: {}", "Error:".red(), e),
          }
        } else {
          println!("{} Usage: /web <url>", "Info:".blue());
        }
      }
      "/search" => {
        if parts.len() > 1 {
          let query = parts[1];
          println!("{} GLM Searching: {}...", "✦".cyan(), query);
          match self.brain.glm_web_search(query).await {
            Ok(res) => {
              println!(
                "\n{}",
                "┏━━━━━━━━━━━━━━━━━━━━━ GLM 搜索结果 ━━━━━━━━━━━━━━━━━━━━━┓".cyan()
              );
              let skin = termimad::MadSkin::default();
              skin.print_text(&res);
              println!(
                "{}",
                "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛".cyan()
              );
            }
            Err(e) => println!("{} GLM Search failed: {}", "Error:".red(), e),
          }
        } else {
          println!("{} Usage: /search <query>", "Info:".blue());
        }
      }
      "/tavily" => {
        if parts.len() > 1 {
          let query = parts[1];
          println!("{} Tavily Searching: {}...", "✦".cyan(), query);
          match self.brain.tavily_search(query).await {
            Ok(res) => {
              println!(
                "\n{}",
                "┏━━━━━━━━━━━━━━━━━━━━━ Tavily 搜索结果 ━━━━━━━━━━━━━━━━━━━━━┓".cyan()
              );
              let skin = termimad::MadSkin::default();
              skin.print_text(&res);
              println!(
                "{}",
                "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛".cyan()
              );
            }
            Err(e) => println!("{} Tavily Search failed: {}", "Error:".red(), e),
          }
        } else {
          println!("{} Usage: /tavily <query>", "Info:".blue());
        }
      }
      "/help" => self.print_help(),
      "/clear" => {
        self.current_session = self.history.create_session(self.model.clone());
        self.current_skill = None;
        println!("{}", "Conversation reset.".yellow());
      }
      "/skill" => {
        if parts.len() > 1 {
          match parts[1] {
            "list" => {
              let skills = self.skill_manager.load_skills()?;
              for s in skills {
                println!("- {}: {}", s.name.bold(), s.description);
              }
            }
            "auto" => {
              if parts.len() > 2 {
                self.auto_route = parts[2] == "on";
                println!("Auto-route: {}", if self.auto_route { "ON" } else { "OFF" });
              }
            }
            _ => {
              let skills = self.skill_manager.load_skills()?;
              if let Some(skill) = skills.into_iter().find(|s| s.name == parts[1]) {
                self.activate_skill(skill);
              }
            }
          }
        }
      }
      "/model" => {
        if parts.len() > 1 {
          self.model = match parts[1] {
            "flash" => self.config.brain.flash_model.clone(),
            "pro" => self.config.brain.pro_model.clone(),
            _ => self.model.clone(),
          };
        }
        println!("Model: {}", self.model.cyan());
      }
      "/thinking" => {
        if parts.len() > 1 {
          self.thinking_mode = match parts[1] {
            "n" => ThinkingMode::None,
            "h" => ThinkingMode::High,
            "m" => ThinkingMode::Max,
            _ => self.thinking_mode,
          };
        }
        println!("Thinking: {:?}", self.thinking_mode);
      }
      "/history" => {
        let sessions = self.history.list_sessions()?;
        for s in sessions.iter().take(10) {
          println!(
            "- {} ({})",
            s.title,
            s.id.chars().take(8).collect::<String>()
          );
        }
      }
      "/copy" => {
        if parts.len() > 1 {
          if let Ok(idx) = parts[1].parse::<usize>() {
            if idx > 0 && idx <= self.last_code_blocks.len() {
              #[cfg(target_os = "macos")]
              {
                let code = &self.last_code_blocks[idx - 1];
                use std::io::Write;
                let mut child = std::process::Command::new("pbcopy")
                  .stdin(std::process::Stdio::piped())
                  .spawn()?;
                if let Some(mut stdin) = child.stdin.take() {
                  stdin.write_all(code.trim().as_bytes())?;
                }
                child.wait()?;
                println!(
                  "{} Code block {} copied to clipboard!",
                  "Success:".green(),
                  idx
                );
              }
            } else {
              println!(
                "{} Invalid index. Range: 1-{}",
                "Error:".red(),
                self.last_code_blocks.len()
              );
            }
          }
        } else if !self.last_code_blocks.is_empty() {
          println!(
            "{} Specify index (1-{}) to copy.",
            "Info:".blue(),
            self.last_code_blocks.len()
          );
        } else {
          println!("{} No code blocks found in last response.", "Info:".blue());
        }
      }
      _ => println!("Unknown command"),
    }
    Ok(false)
  }

  async fn chat(&mut self, content: &str) -> Result<()> {
    let mut file_context = String::new();
    let re_file = Regex::new(r#"@(?:"([^"]+)"|'([^']*)'|([^\s]+))"#)?;

    for cap in re_file.captures_iter(content) {
      let path_str = cap
        .get(1)
        .or(cap.get(2))
        .or(cap.get(3))
        .map(|m| m.as_str())
        .unwrap_or("");

      if path_str == "image" || path_str == "img" || path_str == "paste" || path_str == "clipboard"
      {
        match self.paste_image().await {
          Ok(desc) => {
            file_context.push_str(&format!(
              "\n\n--- CONTENT FROM CLIPBOARD IMAGE ---\n{}\n",
              desc
            ));
          }
          Err(e) => {
            println!("{} Failed to analyze clipboard: {}", "Error:".red(), e);
          }
        }
        continue;
      }

      if path_str == "search" {
        match self.brain.glm_web_search(content).await {
          Ok(res) => {
            file_context.push_str(&format!("\n\n--- CONTENT FROM GLM SEARCH ---\n{}\n", res));
            println!("{} Injected GLM search results.", "Success:".green());
          }
          Err(e) => {
            println!("{} GLM Search failed: {}", "Error:".red(), e);
          }
        }
        continue;
      }

      if path_str == "tavily" {
        match self.brain.tavily_search(content).await {
          Ok(res) => {
            file_context.push_str(&format!(
              "\n\n--- CONTENT FROM TAVILY SEARCH ---\n{}\n",
              res
            ));
            println!("{} Injected Tavily search results.", "Success:".green());
          }
          Err(e) => {
            println!("{} Tavily Search failed: {}", "Error:".red(), e);
          }
        }
        continue;
      }

      if path_str.starts_with("http://") || path_str.starts_with("https://") {
        match self.brain.fetch_web_markdown(path_str).await {
          Ok(md) => {
            file_context.push_str(&format!(
              "\n\n--- CONTENT FROM WEB: {} ---\n{}\n",
              path_str, md
            ));
            println!("{} Injected web content: {}", "Success:".green(), path_str);
          }
          Err(e) => {
            println!("{} Failed to fetch web content: {}", "Error:".red(), e);
          }
        }
        continue;
      }

      let path = std::path::PathBuf::from(path_str);
      if path.exists() && path.is_file() {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext.to_lowercase().as_str() {
          "pdf" | "docx" | "pptx" | "xlsx" | "png" | "jpg" | "jpeg" | "webp" | "bmp" => {
            match self.analyze_complex_file(path.clone()).await {
              Ok(md) => {
                file_context.push_str(&format!(
                  "\n\n--- CONTENT FROM FILE: {:?} ---\n{}\n",
                  path, md
                ));
              }
              Err(e) => {
                println!("{} Failed to parse {:?}: {}", "Error:".red(), path, e);
              }
            }
          }
          _ => {
            if let Ok(text) = std::fs::read_to_string(&path) {
              file_context.push_str(&format!(
                "\n\n--- CONTENT FROM FILE: {:?} ---\n{}\n",
                path, text
              ));
              println!("{} Injected file: {:?}", "Success:".green(), path);
            }
          }
        }
      }
    }

    let final_input = if file_context.is_empty() {
      content.to_string()
    } else {
      format!("{}\n\nContext Files:{}", content, file_context)
    };

    self.current_session.messages.push(Message::Simple {
      role: "user".to_string(),
      content: api::MessageContent::Text(final_input),
      reasoning_content: None,
      tool_calls: None,
    });

    let tools = self.current_skill.as_ref().and_then(|s| s.to_api_tools());

    let mut stream = self
      .brain
      .call_api_with_params(
        &self.model,
        self.current_session.messages.clone(),
        self.thinking_mode.as_str(),
        tools,
      )
      .await?;

    let mut assistant_content = String::new();
    let mut assistant_reasoning = String::new();
    let mut tool_calls = Vec::new();
    let mut is_reasoning = false;

    while let Some(item) = stream.next().await {
      match item? {
        StreamItem::Reasoning(r) => {
          if !is_reasoning {
            print!("\n{}", "Thinking: ".italic().bright_black());
            is_reasoning = true;
          }
          print!("{}", r.italic().bright_black());
          assistant_reasoning.push_str(&r);
        }
        StreamItem::Content(c) => {
          if is_reasoning {
            println!();
            is_reasoning = false;
          }
          print!("{}", c);
          assistant_content.push_str(&c);
        }
        StreamItem::ToolCall(tc) => {
          println!(
            "\n{} Called: {}",
            "Skill:".yellow(),
            tc.function.name.cyan()
          );
          tool_calls.push(tc);
        }
        StreamItem::Finish(reason) => {
          println!();
          if let Some(r) = reason
            && r == "length"
          {
            println!("\n{}", "[Note: Max output limit reached.]".yellow());
          }
        }
      }
      io::stdout().flush()?;
    }

    let re = Regex::new(r"```(?:[a-zA-Z0-9]*)\n([\s\S]*?)```")?;
    self.last_code_blocks = re
      .captures_iter(&assistant_content)
      .map(|cap| cap[1].to_string())
      .collect();

    if !assistant_content.is_empty() {
      println!(
        "\n{}",
        "┏━━━━━━━━━━━━━━━━━━━━━ 智能渲染视图 ━━━━━━━━━━━━━━━━━━━━━┓"
          .blue()
          .bold()
      );
      let skin = MadSkin::default();
      let mut current_pos = 0;
      let mut block_idx = 0;
      let ps = syntect::parsing::SyntaxSet::load_defaults_newlines();
      let ts = syntect::highlighting::ThemeSet::load_defaults();
      let theme = &ts.themes["base16-ocean.dark"];
      let block_re = Regex::new(r"(?m)^```([a-zA-Z0-9]*)\n([\s\S]*?)^```")?;

      for cap in block_re.captures_iter(&assistant_content) {
        let entire_match = cap.get(0).unwrap();
        let lang = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let code_content = cap.get(2).unwrap().as_str();

        let pre_text = &assistant_content[current_pos..entire_match.start()];
        if !pre_text.trim().is_empty() {
          skin.print_text(pre_text);
        }

        block_idx += 1;
        let block_title = if lang.is_empty() {
          "CODE".to_string()
        } else {
          lang.to_uppercase()
        };
        println!(
          "{}",
          format!(" ── {} Block [{}] ──", block_title, block_idx).dimmed()
        );

        let syntax = ps
          .find_syntax_by_token(lang)
          .unwrap_or_else(|| ps.find_syntax_plain_text());
        let mut h = syntect::easy::HighlightLines::new(syntax, theme);
        for line in syntect::util::LinesWithEndings::from(code_content) {
          let ranges: Vec<(syntect::highlighting::Style, &str)> =
            h.highlight_line(line, &ps).unwrap_or_default();
          let escaped = syntect::util::as_24_bit_terminal_escaped(&ranges[..], false);
          print!("{}", escaped);
        }
        println!("\x1b[0m");
        println!(
          "{}",
          format!("复制代码请使用: /copy {}", block_idx)
            .bright_black()
            .italic()
        );
        println!();
        current_pos = entire_match.end();
      }

      if current_pos < assistant_content.len() {
        skin.print_text(&assistant_content[current_pos..]);
      }
      println!(
        "{}",
        "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛"
          .blue()
          .bold()
      );
    }

    self.current_session.messages.push(Message::Simple {
      role: "assistant".to_string(),
      content: api::MessageContent::Text(assistant_content),
      reasoning_content: if assistant_reasoning.is_empty() {
        None
      } else {
        Some(assistant_reasoning)
      },
      tool_calls: if tool_calls.is_empty() {
        None
      } else {
        Some(tool_calls)
      },
    });

    if self.current_session.title == "New Chat" {
      self.current_session.title = content.chars().take(30).collect::<String>();
    }

    self.history.save_session(&self.current_session)?;
    Ok(())
  }
}

#[tokio::main]
async fn main() -> Result<()> {
  let mut app = App::new()?;
  app.run().await
}
