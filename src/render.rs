//! Streaming markdown rendering for the answer stream.
//!
//! The model streams markdown; the terminal shows raw `**bold**` and fence
//! markers. This turns the former into ANSI and the latter into something a
//! mouse selection can survive.
//!
//! **Line-buffered, and that is the whole design.** Every markdown construct
//! that changes how a line is drawn — fence, heading, bullet — is decided by
//! the line's *first* characters, so a renderer that holds a partial line
//! until its newline arrives needs no lookahead, no parse tree, and no
//! re-drawing of text already on screen. The cost is that output appears a
//! line at a time rather than a token at a time; that is the price of
//! deciding anything at all about a line before printing it.
//!
//! **Code blocks are the point.** Inside a fence every line is emitted
//! byte-for-byte at column 0: no indent, no `│` gutter, no background fill.
//! Those are exactly the decorations a mouse selection picks up, and code
//! pasted with a gutter is code that has to be cleaned by hand. The fence
//! itself becomes a dim label line carrying the block's `/copy` index, which
//! is the only lossless way to get the code out — see [`Renderer::push`].

use colored::Colorize;

/// Column the `/copy N` hint starts at on a label line.
///
/// Fixed rather than terminal-relative: reading the width would make the
/// renderer's output depend on the environment, and the tests would then be
/// asserting against whatever `$COLUMNS` happened to be.
const LABEL_WIDTH: usize = 32;

/// Line-buffered markdown renderer for one model response.
///
/// One per LLM call, because the `/copy` index it prints is scoped to one
/// call: `engine::App::extract_code_blocks` re-fills `last_code_blocks` from
/// each assistant message, so block numbering restarts exactly when this does.
#[derive(Default)]
pub struct Renderer {
  /// Received but not yet terminated by a newline.
  pending: String,
  in_fence: bool,
  /// Code blocks opened so far — the `/copy` index.
  blocks: usize,
  /// Whether the last emitted line was blank, so runs collapse to one.
  last_blank: bool,
}

impl Renderer {
  pub fn new() -> Self {
    Self::default()
  }

  /// Feed one streamed chunk; returns whatever is now ready to print.
  ///
  /// Chunk boundaries are the provider's business, not markdown's: the same
  /// text must render identically whether it arrives in one piece or one byte
  /// at a time.
  pub fn push(&mut self, chunk: &str) -> String {
    self.pending.push_str(chunk);
    let mut out = String::new();
    while let Some(idx) = self.pending.find('\n') {
      let line: String = self.pending.drain(..=idx).collect();
      if let Some(rendered) = self.line(line.trim_end_matches('\n').trim_end_matches('\r')) {
        out.push_str(&rendered);
        out.push('\n');
      }
    }
    out
  }

  /// Flush a last line that never got its newline.
  ///
  /// No trailing newline of its own: the caller owns the break after the
  /// answer, exactly as raw passthrough did.
  pub fn finish(&mut self) -> String {
    if self.pending.is_empty() {
      return String::new();
    }
    let line = std::mem::take(&mut self.pending);
    self.line(&line).unwrap_or_default()
  }

  /// Render one complete line. `None` suppresses it entirely.
  fn line(&mut self, raw: &str) -> Option<String> {
    let trimmed = raw.trim_start();

    if trimmed.starts_with("```") {
      if self.in_fence {
        self.in_fence = false;
        // A blank line, not a rule, closes the block: whitespace is the one
        // bottom edge a mouse selection cannot drag in with the code.
        return self.blank();
      }
      self.in_fence = true;
      self.blocks += 1;
      self.last_blank = false;
      return Some(self.fence_label(trimmed.trim_start_matches('`')));
    }

    if self.in_fence {
      // Verbatim. Blank lines inside code are code, so they do not collapse.
      self.last_blank = raw.trim().is_empty();
      return Some(raw.to_string());
    }

    if raw.trim().is_empty() {
      return self.blank();
    }
    self.last_blank = false;

    // `#` .. `######` followed by a space. Styled as a whole rather than run
    // through `inline`: nesting ANSI would let the inner reset cancel the
    // heading's own colour for everything after it.
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
      return Some(trimmed[hashes + 1..].trim().bold().cyan().to_string());
    }

    // Bullet: swap the marker, keep the indentation — the indentation is what
    // nesting is made of.
    let indent = raw.len() - trimmed.len();
    if let Some(rest) = trimmed
      .strip_prefix("- ")
      .or_else(|| trimmed.strip_prefix("* "))
      .or_else(|| trimmed.strip_prefix("+ "))
    {
      return Some(format!("{}{} {}", &raw[..indent], "•".cyan(), inline(rest)));
    }

    Some(inline(raw))
  }

  fn blank(&mut self) -> Option<String> {
    if self.last_blank {
      return None;
    }
    self.last_blank = true;
    Some(String::new())
  }

  /// `racket                          /copy 1`
  fn fence_label(&self, info: &str) -> String {
    // An info string can carry more than the language (` ```rust,ignore `);
    // only the first word names it.
    let lang = info.split_whitespace().next().unwrap_or("");
    let lang = if lang.is_empty() { "code" } else { lang };
    let pad = LABEL_WIDTH.saturating_sub(lang.chars().count()).max(1);
    format!(
      "{}{}{}",
      lang.dimmed(),
      " ".repeat(pad),
      format!("/copy {}", self.blocks).dimmed()
    )
  }
}

/// Render the inline spans of one line: `` `code` `` and `**bold**`.
///
/// One pass with code spans winning, because a `**` inside backticks is
/// someone's pointer type, not emphasis. An unpaired delimiter stays literal:
/// models write `*` as multiplication often enough that swallowing it would
/// silently corrupt the text.
fn inline(line: &str) -> String {
  spans(line, false)
}

/// `bold` is carried *down* rather than wrapped around the result, because an
/// ANSI reset ends every active style and not just the innermost one. Wrapping
/// `**a `b` c**` would render `a` bold, then the code span's own reset would
/// drop the bold for ` c` — the one construct in this list that models write
/// most often.
fn spans(line: &str, bold: bool) -> String {
  let emit = |s: &str| {
    if s.is_empty() {
      String::new()
    } else if bold {
      s.bold().to_string()
    } else {
      s.to_string()
    }
  };

  let mut out = String::with_capacity(line.len());
  let mut rest = line;
  loop {
    let Some(at) = rest.find(['`', '*']) else {
      out.push_str(&emit(rest));
      return out;
    };
    let (head, tail) = rest.split_at(at);
    out.push_str(&emit(head));

    if let Some(body) = tail.strip_prefix('`')
      && let Some(end) = body.find('`')
    {
      let span = body[..end].yellow();
      out.push_str(&if bold { span.bold() } else { span }.to_string());
      rest = &body[end + 1..];
      continue;
    }
    if !bold
      && let Some(body) = tail.strip_prefix("**")
      && let Some(end) = body.find("**")
    {
      out.push_str(&spans(&body[..end], true));
      rest = &body[end + 2..];
      continue;
    }

    let step = tail.chars().next().map_or(1, char::len_utf8);
    out.push_str(&emit(&tail[..step]));
    rest = &tail[step..];
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Assert on structure, not on escape codes.
  fn render(text: &str) -> String {
    colored::control::set_override(false);
    let mut r = Renderer::new();
    let mut out = r.push(text);
    out.push_str(&r.finish());
    out
  }

  #[test]
  fn code_lines_are_verbatim_and_flush_left() {
    let out = render("see:\n```racket\n(define (f x)\n  (+ x 1))\n```\ndone\n");
    // Every line of the block, exactly as the model wrote it — the indent of
    // the second line is the model's, and nothing was added to the first.
    assert!(out.contains("\n(define (f x)\n"), "{out:?}");
    assert!(out.contains("\n  (+ x 1))\n"), "{out:?}");
    // Nothing that a mouse selection would drag in with the code.
    for line in out
      .lines()
      .filter(|l| l.contains("define") || l.contains("+ x"))
    {
      assert!(!line.starts_with('│') && !line.starts_with('>'), "{line:?}");
    }
  }

  #[test]
  fn fence_becomes_a_label_carrying_the_copy_index() {
    let out = render("```rust\nfn a() {}\n```\n\n```\nplain\n```\n");
    assert!(out.contains("rust"), "{out:?}");
    assert!(out.contains("/copy 1"), "{out:?}");
    // No language on the second fence, but it still gets an index.
    assert!(out.contains("code"), "{out:?}");
    assert!(out.contains("/copy 2"), "{out:?}");
    // The fence markers themselves are gone.
    assert!(!out.contains("```"), "{out:?}");
  }

  /// Chunk boundaries are the provider's business, not markdown's.
  #[test]
  fn output_does_not_depend_on_chunking() {
    let text = "# Title\n\n- a `b` **c**\n\n```py\nx = 1\n\ny = 2\n```\ntail";
    colored::control::set_override(false);

    let whole = render(text);
    let mut r = Renderer::new();
    let mut piecemeal = String::new();
    for ch in text.chars() {
      piecemeal.push_str(&r.push(&ch.to_string()));
    }
    piecemeal.push_str(&r.finish());

    assert_eq!(whole, piecemeal);
  }

  #[test]
  fn a_code_span_wins_over_emphasis_inside_it() {
    // `**p` is a pointer, not the start of bold.
    assert_eq!(render("a `int **p` b\n"), "a int **p b\n");
  }

  /// Found by looking at real output, not by reasoning about the code: the
  /// bold branch used to swallow its body whole, so backticks inside it
  /// reached the screen as backticks.
  #[test]
  fn emphasis_containing_code_renders_both() {
    assert_eq!(render("**`<?` 默认是 `<`** x\n"), "<? 默认是 < x\n");
  }

  #[test]
  fn unpaired_delimiters_stay_literal() {
    assert_eq!(
      render("2 * 3 and ** and `open\n"),
      "2 * 3 and ** and `open\n"
    );
  }

  #[test]
  fn emphasis_and_code_are_stripped_of_their_markers() {
    assert_eq!(render("**bold** and `code`\n"), "bold and code\n");
  }

  #[test]
  fn headings_lose_their_hashes_and_bullets_keep_their_indent() {
    let out = render("### Notes\n- top\n  - nested\n");
    assert!(out.starts_with("Notes\n"), "{out:?}");
    assert!(out.contains("\n• top\n"), "{out:?}");
    assert!(out.contains("\n  • nested\n"), "{out:?}");
  }

  /// `#5` is not a heading, and `**x**` at line start is not a bullet.
  #[test]
  fn near_misses_are_left_alone() {
    assert_eq!(render("#5 wins\n"), "#5 wins\n");
    assert_eq!(render("**x** y\n"), "x y\n");
  }

  #[test]
  fn blank_runs_collapse_outside_code_but_never_inside() {
    // Two blank lines between paragraphs become one...
    let prose = render("a\n\n\n\nb\n");
    assert_eq!(prose, "a\n\nb\n");
    // ...but a blank line inside a block is code and must survive.
    let code = render("```py\nx = 1\n\n\ny = 2\n```\n");
    assert!(code.contains("x = 1\n\n\ny = 2\n"), "{code:?}");
  }

  #[test]
  fn finish_flushes_a_line_that_never_got_its_newline() {
    colored::control::set_override(false);
    let mut r = Renderer::new();
    assert_eq!(r.push("**tail**"), "");
    assert_eq!(r.finish(), "tail");
  }
}
