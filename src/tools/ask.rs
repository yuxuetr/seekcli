//! `ask_user_question`: let the model ask instead of guess.
//!
//! Without it a model that lacks a detail has two options — invent one, or
//! stall — and both waste a turn. The interesting part is not the interactive
//! path but the headless one: an unattended run must **refuse immediately**
//! rather than block on input nobody will type. A tool that hangs looks
//! exactly like a hung process, and a launchd job would sit there until killed.

use anyhow::{Context, Result};
use colored::Colorize;
use serde_json::Value;
use std::io::{self, IsTerminal, Write};

pub async fn ask_user_question(args: &Value) -> Result<String> {
  let question = args
    .get("question")
    .and_then(Value::as_str)
    .context("Missing 'question' argument")?;
  let options: Vec<String> = args
    .get("options")
    .and_then(Value::as_array)
    .map(|arr| {
      arr
        .iter()
        .filter_map(|o| {
          o.get("label").and_then(Value::as_str).map(|l| {
            match o.get("description").and_then(Value::as_str) {
              Some(d) if !d.is_empty() => format!("{} — {}", l, d),
              _ => l.to_string(),
            }
          })
        })
        .collect()
    })
    .unwrap_or_default();

  if !io::stdin().is_terminal() {
    return Ok(format!(
      "[USER DENIED] No interactive user is available (non-interactive run), so \\
       this question cannot be answered. Do not ask again. Choose the most \\
       reasonable default, state the assumption you made, and continue. \\
       Question was: {question}"
    ));
  }

  eprintln!();
  eprintln!("{} {}", "[?]".cyan().bold(), question.bold());
  for (n, option) in options.iter().enumerate() {
    eprintln!("    {}. {}", n + 1, option);
  }
  if options.is_empty() {
    eprint!("    Answer: ");
  } else {
    eprint!("    Choose 1-{} or type an answer: ", options.len());
  }
  io::stderr().flush().ok();

  let mut buf = String::new();
  if io::stdin().read_line(&mut buf).is_err() {
    return Ok("[USER DENIED] Could not read an answer.".to_string());
  }
  let answer = buf.trim();
  if answer.is_empty() {
    return Ok(
      "[USER DENIED] The user gave no answer. Proceed with your best judgement.".to_string(),
    );
  }
  // A bare number selects an option; anything else is taken verbatim.
  if let Ok(choice) = answer.parse::<usize>()
    && choice >= 1
    && choice <= options.len()
  {
    return Ok(format!("User chose: {}", options[choice - 1]));
  }
  Ok(format!("User answered: {}", answer))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  /// `cargo test` runs without a tty, which is the same condition a launchd
  /// job runs under — so this test genuinely exercises the headless path.
  #[tokio::test]
  async fn a_non_interactive_run_refuses_immediately_instead_of_blocking() {
    let out = match ask_user_question(&json!({ "question": "Which port?" })).await {
      Ok(t) => t,
      Err(e) => panic!("must not error: {}", e),
    };
    assert!(out.starts_with("[USER DENIED]"), "got: {}", out);
    // Telling the model to pick a default is what keeps the run moving.
    assert!(out.contains("default"), "got: {}", out);
    assert!(
      out.contains("Which port?"),
      "the question should be echoed back"
    );
  }

  #[tokio::test]
  async fn a_missing_question_is_an_error_not_a_prompt() {
    assert!(ask_user_question(&json!({})).await.is_err());
  }
}
