//! Token estimation.
//!
//! The compaction threshold used to count **bytes**, which drifts badly with
//! script: one CJK character is 3 UTF-8 bytes but roughly one token, while
//! four ASCII bytes are roughly one token. A Chinese conversation therefore
//! tripped compaction at around a third of the context a English one did, for
//! the same real token cost.
//!
//! This is an estimate, not a tokenizer. A real BPE tokenizer means shipping
//! vocabulary files per model, and the threshold only needs to be right to
//! within a few percent to decide "is this conversation getting long". The
//! trait exists so a genuine tokenizer can replace the heuristic later without
//! touching the caller.

use super::Message;

pub trait TokenCounter: Send + Sync {
  fn count_text(&self, text: &str) -> usize;

  fn count_messages(&self, messages: &[Message]) -> usize {
    messages
      .iter()
      .map(|m| {
        let body = match m {
          Message::Simple {
            content,
            reasoning_content,
            tool_calls,
            images,
            ..
          } => {
            let mut n = self.count_text(content);
            if let Some(r) = reasoning_content {
              n += self.count_text(r);
            }
            if let Some(calls) = tool_calls {
              for c in calls {
                n += self.count_text(&c.function.name) + self.count_text(&c.function.arguments);
              }
            }
            // An image's cost has nothing to do with how many characters its
            // base64 has. Counting the encoded text would be wrong by orders of
            // magnitude in the *other* direction; ignoring it is wrong by an
            // order of magnitude too. A flat floor at least stops the estimate
            // from claiming a screenshot is free
            // (`docs/architecture/L4-memory.md` §4.6.3).
            n + images.len() * IMAGE_TOKENS
          }
          Message::ToolResponse { content, .. } => self.count_text(content),
        };
        // Per-message envelope: role, delimiters, and the wire framing every
        // provider adds. Undercounting here makes the threshold optimistic
        // exactly when a conversation has many small messages.
        body + 4
      })
      .sum()
  }
}

/// Conservative floor for one image.
///
/// Measured 2026-09-13 against the live API: a 16×16 PNG took the prompt from
/// 31 to 224 tokens, a 64×64 one to 236. Real screenshots cost more, and the
/// cost scales with dimensions rather than bytes, so this is a floor and the
/// estimate stays an estimate — it exists to stop compaction from believing a
/// conversation full of screenshots is small.
const IMAGE_TOKENS: usize = 200;

/// Script-aware ratio estimate, zero dependencies.
pub struct Heuristic;

/// Bytes per token for Latin-script text.
const ASCII_BYTES_PER_TOKEN: f64 = 4.0;
/// Bytes per token for CJK. Each character is 3 UTF-8 bytes and lands close to
/// one token, so the ratio is far lower than Latin text.
const WIDE_BYTES_PER_TOKEN: f64 = 3.0;

impl TokenCounter for Heuristic {
  fn count_text(&self, text: &str) -> usize {
    let mut ascii_bytes = 0usize;
    let mut wide_bytes = 0usize;
    for ch in text.chars() {
      let len = ch.len_utf8();
      if len > 1 {
        wide_bytes += len;
      } else {
        ascii_bytes += len;
      }
    }
    let estimate =
      ascii_bytes as f64 / ASCII_BYTES_PER_TOKEN + wide_bytes as f64 / WIDE_BYTES_PER_TOKEN;
    estimate.ceil() as usize
  }
}

#[cfg(test)]
mod tests {

  /// The estimate must not report a screenshot as nearly free: compaction
  /// decides "is this conversation long" from this number.
  #[test]
  fn an_image_is_not_counted_as_free() {
    let text_only = vec![Message::new_user_text("hi".into())];
    let with_image = vec![Message::new_user_with_images(
      "hi".into(),
      vec![crate::api::ImagePart {
        media_type: "image/png".into(),
        data_base64: "QQ==".into(),
      }],
    )];
    let bare = Heuristic.count_messages(&text_only);
    let imaged = Heuristic.count_messages(&with_image);
    assert!(
      imaged >= bare + IMAGE_TOKENS,
      "image added only {} tokens",
      imaged - bare
    );
  }

  use super::*;

  #[test]
  fn latin_text_uses_roughly_four_bytes_per_token() {
    let text = "the quick brown fox jumps over the lazy dog"; // 43 bytes
    let n = Heuristic.count_text(text);
    assert!((10..=12).contains(&n), "got {}", n);
  }

  /// The bug this replaces: byte counting made CJK look ~3x more expensive
  /// than it is, so a Chinese session compacted at a third of the context.
  #[test]
  fn cjk_is_not_charged_three_times_over() {
    let cjk = "这是一段中文对话内容"; // 10 chars, 30 bytes
    let tokens = Heuristic.count_text(cjk);
    assert!(
      (10..=12).contains(&tokens),
      "10 CJK chars should be ~10 tokens, got {}",
      tokens
    );
    // Byte counting would have said 30.
    assert!(tokens < cjk.len(), "must be cheaper than raw bytes");
  }

  #[test]
  fn mixed_script_lands_between_the_two_ratios() {
    let mixed = "read 文件 and report";
    let n = Heuristic.count_text(mixed);
    assert!(n > 0 && n < mixed.len());
  }

  #[test]
  fn message_counting_includes_tool_call_arguments() {
    use crate::api::{FunctionCall, ToolCall};
    let bare = vec![Message::Simple {
      images: Vec::new(),
      role: "assistant".into(),
      content: "ok".into(),
      reasoning_content: None,
      tool_calls: None,
    }];
    let with_call = vec![Message::Simple {
      images: Vec::new(),
      role: "assistant".into(),
      content: "ok".into(),
      reasoning_content: None,
      tool_calls: Some(vec![ToolCall {
        id: "c1".into(),
        tool_type: "function".into(),
        function: FunctionCall {
          name: "read_file".into(),
          arguments: r#"{"path":"some/long/path.rs"}"#.into(),
        },
      }]),
    }];
    // Arguments are context the model pays for; ignoring them would make a
    // tool-heavy conversation look far cheaper than it is.
    assert!(Heuristic.count_messages(&with_call) > Heuristic.count_messages(&bare));
  }

  #[test]
  fn empty_input_is_zero_plus_envelope() {
    assert_eq!(Heuristic.count_text(""), 0);
    assert_eq!(Heuristic.count_messages(&[]), 0);
  }
}
