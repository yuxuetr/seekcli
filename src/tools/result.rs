//! Structured tool outcomes.
//!
//! Stage 8.3 deferred this and settled for string prefixes — `[USER DENIED]`,
//! `[PATH DENIED]`, `[BAD ARGS]`. That works for *telling the model* what
//! happened, but the program then had to recover the same information with
//! `contains`, so every consumer re-derived semantics from prose. Adding a new
//! outcome meant finding every `contains` in the codebase and hoping.
//!
//! The prefixes stay — they are how the model learns what happened, and it has
//! been trained on them by the system prompt. What changes is that the program
//! no longer reads them: `kind` carries the meaning, and the prefix is
//! rendered *from* the kind rather than parsed back into one.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
  /// Ran and produced output.
  Ok,
  /// Refused by policy or by the user. Not an error — retrying is pointless
  /// and the model is told so.
  Denied,
  /// Ran and failed. Error Recovery attaches a hint.
  Failed,
  /// Arguments were not valid JSON, or a required one was missing.
  BadArgs,
  /// Exceeded its deadline.
  TimedOut,
}

impl ToolKind {
  /// The marker the model sees. Empty for success: a successful read should
  /// return the file, not a status line.
  pub fn prefix(self) -> &'static str {
    match self {
      ToolKind::Ok => "",
      ToolKind::Denied => "[USER DENIED] ",
      ToolKind::Failed => "[FAILED] ",
      ToolKind::BadArgs => "[BAD ARGS] ",
      ToolKind::TimedOut => "[TIMED OUT] ",
    }
  }

  /// Whether this outcome should trigger the Two-Stage micro trigger and an
  /// Error Recovery hint.
  ///
  /// `Denied` is excluded on purpose: a refusal is a *decision*, not a
  /// malfunction. Treating it as failure would make the model re-plan its way
  /// around a policy, which is precisely what the policy exists to prevent.
  pub fn is_failure(self) -> bool {
    matches!(
      self,
      ToolKind::Failed | ToolKind::BadArgs | ToolKind::TimedOut
    )
  }
}

/// What a tool produced, before classification.
///
/// Exists so the guarded pipeline can carry images without every built-in tool
/// changing signature: they keep returning `Result<String>` and convert through
/// `From`. The alternatives considered — widening every tool's return type, a
/// shared-mutable side channel, or letting the caller fill images in after the
/// fact — each either touched tools that will never return an image or put a
/// second way into the one guarded path
/// (`docs/architecture/L4-memory.md` §4.6.4).
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
  pub text: String,
  pub images: Vec<crate::api::ImagePart>,
}

impl From<String> for ToolOutput {
  fn from(text: String) -> Self {
    Self {
      text,
      images: Vec::new(),
    }
  }
}

#[derive(Debug, Clone)]
pub struct ToolResult {
  pub kind: ToolKind,
  pub content: String,
  /// Images the tool produced. Empty for every built-in.
  pub images: Vec<crate::api::ImagePart>,
}

impl ToolResult {
  pub fn ok(content: impl Into<String>) -> Self {
    Self {
      kind: ToolKind::Ok,
      content: content.into(),
      images: Vec::new(),
    }
  }

  pub fn denied(content: impl Into<String>) -> Self {
    Self {
      kind: ToolKind::Denied,
      content: content.into(),
      images: Vec::new(),
    }
  }

  pub fn failed(content: impl Into<String>) -> Self {
    Self {
      kind: ToolKind::Failed,
      content: content.into(),
      images: Vec::new(),
    }
  }

  pub fn bad_args(content: impl Into<String>) -> Self {
    Self {
      kind: ToolKind::BadArgs,
      content: content.into(),
      images: Vec::new(),
    }
  }

  pub fn timed_out(content: impl Into<String>) -> Self {
    Self {
      kind: ToolKind::TimedOut,
      content: content.into(),
      images: Vec::new(),
    }
  }

  /// Adapt a legacy `Result<String>`.
  ///
  /// Existing tools already encode denial in their text; classifying by
  /// prefix here — in exactly one place — is what lets them migrate without
  /// each being rewritten, while the rest of the codebase stops guessing.
  pub fn from_legacy(outcome: anyhow::Result<ToolOutput>) -> Self {
    match outcome {
      Ok(ToolOutput { text, images }) => {
        let mut result = Self::classify_text(text);
        result.images = images;
        result
      }
      Err(e) => Self::failed(format!("{e:#}")),
    }
  }

  /// Classify a tool's text by the prefix it already carries.
  ///
  /// Existing tools encode denial in their text; doing this in exactly one
  /// place is what let them migrate without each being rewritten.
  fn classify_text(text: String) -> Self {
    if text.starts_with("[USER DENIED]")
      || text.starts_with("[PATH DENIED]")
      || text.starts_with("[MODE DENIED]")
    {
      Self::denied(text)
    } else if text.starts_with("[BAD ARGS]") {
      Self::bad_args(text)
    } else if text.starts_with("[ERROR]") || text.starts_with("[BAD PATTERN]") {
      Self::failed(text)
    } else {
      Self::ok(text)
    }
  }

  /// What the model receives: the marker followed by the content, with the
  /// marker suppressed when the content already carries one.
  pub fn render(&self) -> String {
    let prefix = self.kind.prefix();
    if prefix.is_empty() || self.content.starts_with('[') {
      self.content.clone()
    } else {
      format!("{}{}", prefix, self.content)
    }
  }
}

impl fmt::Display for ToolResult {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.render())
  }
}

#[cfg(test)]
mod tests {

  /// Text-only tools keep their `Result<String>` signature; the conversion is
  /// what let the pipeline learn images without touching any of them.
  #[test]
  fn a_plain_string_converts_to_an_imageless_output() {
    let out: ToolOutput = "hello".to_string().into();
    assert_eq!(out.text, "hello");
    assert!(out.images.is_empty());
  }

  #[test]
  fn images_survive_classification() {
    let out = ToolOutput {
      text: "captured".into(),
      images: vec![crate::api::ImagePart {
        media_type: "image/png".into(),
        data_base64: "QQ==".into(),
      }],
    };
    let r = ToolResult::from_legacy(Ok(out));
    assert_eq!(r.kind, ToolKind::Ok);
    assert_eq!(r.images.len(), 1, "images were dropped by classification");
  }

  /// A denial must not carry images: the tool never ran.
  #[test]
  fn a_denial_has_no_images() {
    assert!(ToolResult::denied("[MODE DENIED] no").images.is_empty());
  }

  use super::*;

  #[test]
  fn a_refusal_is_a_decision_not_a_malfunction() {
    // If Denied counted as failure the loop would re-plan around the policy,
    // which is exactly what the policy exists to prevent.
    assert!(!ToolKind::Denied.is_failure());
    assert!(!ToolKind::Ok.is_failure());
    assert!(ToolKind::Failed.is_failure());
    assert!(ToolKind::BadArgs.is_failure());
    assert!(ToolKind::TimedOut.is_failure());
  }

  #[test]
  fn success_renders_the_content_alone() {
    assert_eq!(ToolResult::ok("file contents").render(), "file contents");
  }

  #[test]
  fn a_marker_is_never_doubled() {
    let already = ToolResult::denied("[USER DENIED] blocked by policy");
    assert_eq!(already.render(), "[USER DENIED] blocked by policy");
    assert!(!already.render().starts_with("[USER DENIED] [USER DENIED]"));
  }

  #[test]
  fn legacy_text_is_classified_in_exactly_one_place() {
    let cases = [
      ("[USER DENIED] nope", ToolKind::Denied),
      ("[PATH DENIED] outside", ToolKind::Denied),
      ("[MODE DENIED] read-only", ToolKind::Denied),
      ("[BAD ARGS] not json", ToolKind::BadArgs),
      ("[BAD PATTERN] bad regex", ToolKind::Failed),
      ("[ERROR] restricted", ToolKind::Failed),
      ("ordinary output", ToolKind::Ok),
    ];
    for (text, expected) in cases {
      let r = ToolResult::from_legacy(Ok(text.to_string().into()));
      assert_eq!(r.kind, expected, "misclassified: {}", text);
    }
  }

  #[test]
  fn an_error_becomes_a_failure_with_the_full_chain() {
    let err = anyhow::anyhow!("outer").context("context");
    let r = ToolResult::from_legacy(Err(err));
    assert_eq!(r.kind, ToolKind::Failed);
    assert!(r.content.contains("context"), "got: {}", r.content);
  }
}
