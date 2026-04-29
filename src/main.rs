use anyhow::{Context, Result};
use arboard::Clipboard;
use colored::*;
use futures_util::StreamExt;
use regex::Regex;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::env;
use std::io::{self, Write};
use termimad::MadSkin;

mod api;
mod history;
mod skills;

#[cfg(test)]
mod test;

pub use api::{ApiClient, Message, StreamItem, ToolCall};
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
  client: ApiClient,
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
  fn new(api_key: String) -> Result<Self> {
    let history = HistoryManager::new()?;
    let skill_manager = SkillManager::new()?;
    let model = "deepseek-v4-flash".to_string();
    let current_session = history.create_session(model.clone());

    Ok(Self {
      client: ApiClient::new(api_key),
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
      content: route_prompt,
      reasoning_content: None,
      tool_calls: None,
    }];

    let mut stream = self
      .client
      .call_api_with_params("deepseek-v4-flash", messages, "none", None)
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
      "Welcome to SeekCLI (DeepSeek V4 1M Context Powered)"
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
    // Do NOT clear history, just inject the persona into the flow if starting new,
    // or append if switching topics. For V4 1M, we keep it simple.
    if self.current_session.messages.is_empty() {
      self.current_session.messages.push(Message::Simple {
        role: "system".to_string(),
        content: skill.system_prompt,
        reasoning_content: None,
        tool_calls: None,
      });
    }
  }

  async fn handle_command(&mut self, line: &str) -> Result<bool> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let cmd = parts[0];

    match cmd {
      "/quit" | "/exit" => return Ok(true),
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
            "flash" => "deepseek-v4-flash".to_string(),
            "pro" => "deepseek-v4-pro".to_string(),
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
        println!("Thinking: {}", self.thinking_mode.label().magenta());
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
              let code = &self.last_code_blocks[idx - 1];
              match Clipboard::new() {
                Ok(mut cb) => {
                  if cb.set_text(code.trim().to_string()).is_ok() {
                    println!(
                      "{} Code block {} copied to clipboard!",
                      "Success:".green(),
                      idx
                    );
                  } else {
                    println!("{} Failed to set clipboard text.", "Error:".red());
                  }
                }
                Err(e) => println!("{} Failed to open clipboard: {}", "Error:".red(), e),
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
    self.current_session.messages.push(Message::Simple {
      role: "user".to_string(),
      content: content.to_string(),
      reasoning_content: None,
      tool_calls: None,
    });

    let tools = self.current_skill.as_ref().and_then(|s| s.to_api_tools());

    // V4 1M Context means we can send thousands of messages without trimming!
    let mut stream = self
      .client
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
            println!("\n{}", "[Note: Max output limit reached. Single response capped at 8K tokens. Use '/continue' if needed.]".yellow());
          }
        }
      }
      io::stdout().flush()?;
    }

    // Extract code blocks for the /copy command
    let re = Regex::new(r"```(?:[a-zA-Z0-9]*)\n([\s\S]*?)```")?;
    self.last_code_blocks = re
      .captures_iter(&assistant_content)
      .map(|cap| cap[1].to_string())
      .collect();

    // Render formatted markdown with syntax highlighting and simplified copy instructions
    if !assistant_content.is_empty() {
      println!("\n{}", "┏━━━━━━━━━━━━━━━━━━━━━ 智能渲染视图 ━━━━━━━━━━━━━━━━━━━━━┓".blue().bold());

      let skin = MadSkin::default();
      let mut current_pos = 0;
      let mut block_idx = 0;

      // Initialize syntect for syntax highlighting
      let ps = syntect::parsing::SyntaxSet::load_defaults_newlines();
      let ts = syntect::highlighting::ThemeSet::load_defaults();
      let theme = &ts.themes["base16-ocean.dark"];

      // Improved Regex: Match backticks only at the START of a line (?m)^ to handle nested doc comments
      let block_re = Regex::new(r"(?m)^```([a-zA-Z0-9]*)\n([\s\S]*?)^```")?;

      for cap in block_re.captures_iter(&assistant_content) {
        let entire_match = cap.get(0).unwrap();
        let lang = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let code_content = cap.get(2).unwrap().as_str();

        // 1. Print preceding text
        let pre_text = &assistant_content[current_pos..entire_match.start()];
        if !pre_text.trim().is_empty() {
          skin.print_text(pre_text);
        }

        // 2. Print syntax-highlighted code block
        block_idx += 1;
        let block_title = if lang.is_empty() { "CODE".to_string() } else { lang.to_uppercase() };
        println!("{}", format!(" ── {} Block [{}] ──", block_title, block_idx).dimmed());

        // Use syntect for syntax highlighting
        let syntax = ps.find_syntax_by_token(lang).unwrap_or_else(|| ps.find_syntax_plain_text());
        let mut h = syntect::easy::HighlightLines::new(syntax, theme);

        for line in syntect::util::LinesWithEndings::from(code_content) {
          let ranges: Vec<(syntect::highlighting::Style, &str)> = h.highlight_line(line, &ps).unwrap_or_default();
          let escaped = syntect::util::as_24_bit_terminal_escaped(&ranges[..], false);
          print!("{}", escaped);
        }
        println!("\x1b[0m"); // Reset ANSI colors

        // Simplified copy instruction as requested
        println!("{}", format!("复制代码请使用: /copy {}", block_idx).bright_black().italic());
        println!();

        current_pos = entire_match.end();
      }

      // 3. Print remaining text
      if current_pos < assistant_content.len() {
        skin.print_text(&assistant_content[current_pos..]);
      }

      println!("{}", "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛".blue().bold());
    }

    self.current_session.messages.push(Message::Simple {
      role: "assistant".to_string(),
      content: assistant_content,
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
  let api_key = env::var("DEEPSEEK_API_KEY")
    .or_else(|_| env::var("DASHSCOPE_API_KEY"))
    .context("Please set DEEPSEEK_API_KEY")?;

  let mut app = App::new(api_key)?;
  app.run().await
}
