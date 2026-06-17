//! Surgical `edit_file` matching engine with a multi-level fuzzy-match chain.
//!
//! LLMs reliably hallucinate formatting (the "indentation hallucination"): the
//! `old_text` they emit rarely matches the file byte-for-byte. A naive exact
//! `replace` fails, the model retries with the same wrong text, and the agent
//! deadlocks. Following the harness "fault-tolerance art", we absorb that error
//! in the tool with a Chain-of-Responsibility degradation pipeline:
//!
//!   L1  exact line match (fastest, safest — assume the model is right)
//!   L2  ignore CRLF vs LF line-ending differences
//!   L3  ignore blank lines surrounding the old_text block
//!   L4  ignore per-line leading/trailing whitespace (the core fix for
//!       indentation hallucination)
//!
//! We try strictest first and only loosen when a level finds nothing — minimum
//! necessary tolerance, because looser levels risk false matches. The safety
//! backstop is the **uniqueness check**: if any level matches more than one
//! region, we refuse to edit and hand the ambiguity back to the model (it has
//! the semantic context to disambiguate via Self-Correction).
//!
//! Pure and fully testable; the I/O wrapper lives in `fs::edit_file`.

/// Outcome of attempting an edit against in-memory content.
#[derive(Debug, PartialEq)]
pub enum EditOutcome {
  /// Replaced exactly one region; carries the rebuilt content and the level
  /// (1-4) that matched, for transparency.
  Replaced { level: u8, content: String },
  /// No level found the text anywhere.
  NotFound,
  /// A level matched multiple regions — refuse and ask for more context.
  Ambiguous { level: u8, count: usize },
}

/// Apply `old_text` → `new_text` to `content` using the L1-L4 chain.
pub fn apply_edit(content: &str, old_text: &str, new_text: &str) -> EditOutcome {
  if old_text.is_empty() {
    return EditOutcome::NotFound;
  }

  // split('\n') is the exact inverse of join("\n"), so we can rebuild content
  // losslessly. CRLF files keep a trailing '\r' on each line, handled by L2+.
  let file_lines: Vec<&str> = content.split('\n').collect();
  let old_lines: Vec<&str> = old_text.split('\n').collect();
  let new_lines: Vec<&str> = new_text.split('\n').collect();

  for level in 1u8..=4 {
    // L3 strips blank lines bracketing the old block; other levels use it whole.
    let target: &[&str] = if level == 3 {
      trim_blank_edges(&old_lines)
    } else {
      &old_lines
    };
    if target.is_empty() {
      continue;
    }

    let matches = find_windows(&file_lines, target, level);
    match matches.len() {
      0 => continue,
      1 => {
        let start = matches[0];
        let end = start + target.len();
        let mut out: Vec<&str> = Vec::with_capacity(file_lines.len());
        out.extend_from_slice(&file_lines[..start]);
        out.extend_from_slice(&new_lines);
        out.extend_from_slice(&file_lines[end..]);
        return EditOutcome::Replaced {
          level,
          content: out.join("\n"),
        };
      }
      n => return EditOutcome::Ambiguous { level, count: n },
    }
  }

  EditOutcome::NotFound
}

/// Start indices in `file_lines` where a window equals `target` under `level`.
fn find_windows(file_lines: &[&str], target: &[&str], level: u8) -> Vec<usize> {
  let k = target.len();
  if k == 0 || k > file_lines.len() {
    return Vec::new();
  }
  let mut hits = Vec::new();
  for start in 0..=(file_lines.len() - k) {
    if (0..k).all(|j| line_eq(file_lines[start + j], target[j], level)) {
      hits.push(start);
    }
  }
  hits
}

/// Per-level line equality (increasing tolerance).
fn line_eq(a: &str, b: &str, level: u8) -> bool {
  match level {
    1 => a == b,
    // L2/L3 tolerate CRLF vs LF.
    2 | 3 => a.trim_end_matches('\r') == b.trim_end_matches('\r'),
    // L4 tolerates any leading/trailing whitespace — the indentation fix.
    _ => a.trim() == b.trim(),
  }
}

/// Drop leading and trailing whitespace-only lines from a block.
fn trim_blank_edges<'a>(lines: &'a [&'a str]) -> &'a [&'a str] {
  let mut start = 0;
  let mut end = lines.len();
  while start < end && lines[start].trim().is_empty() {
    start += 1;
  }
  while end > start && lines[end - 1].trim().is_empty() {
    end -= 1;
  }
  &lines[start..end]
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn l1_exact_match() {
    let r = apply_edit("a\nb\nc\n", "b", "B");
    assert_eq!(
      r,
      EditOutcome::Replaced {
        level: 1,
        content: "a\nB\nc\n".to_string()
      }
    );
  }

  #[test]
  fn l1_multiline_exact() {
    let src = "fn f() {\n  return a - b;\n}\n";
    let r = apply_edit(src, "  return a - b;", "  return a + b;");
    assert_eq!(
      r,
      EditOutcome::Replaced {
        level: 1,
        content: "fn f() {\n  return a + b;\n}\n".to_string()
      }
    );
  }

  #[test]
  fn l4_absorbs_indentation_hallucination() {
    // File has 4-space indent; model emits old_text with NO indent.
    let src = "fn f() {\n    let x = compute();\n}\n";
    let r = apply_edit(src, "let x = compute();", "let x = compute2();");
    match r {
      EditOutcome::Replaced { level, content } => {
        assert_eq!(level, 4);
        // The replacement uses the new_text verbatim (model controls new indent).
        assert!(content.contains("let x = compute2();"));
        assert!(!content.contains("compute();"));
      }
      other => panic!("expected L4 replace, got {other:?}"),
    }
  }

  #[test]
  fn l2_crlf_tolerance() {
    let src = "alpha\r\nbeta\r\ngamma\r\n";
    // model emits LF-only old_text
    let r = apply_edit(src, "beta", "BETA");
    match r {
      EditOutcome::Replaced { level, content } => {
        assert_eq!(level, 2);
        assert!(content.contains("BETA"));
      }
      other => panic!("expected L2, got {other:?}"),
    }
  }

  #[test]
  fn uniqueness_check_refuses_multiple_matches() {
    let src = "x = 1\ny = 2\nx = 1\n";
    let r = apply_edit(src, "x = 1", "x = 9");
    assert_eq!(r, EditOutcome::Ambiguous { level: 1, count: 2 });
  }

  #[test]
  fn not_found_when_absent() {
    assert_eq!(apply_edit("a\nb\n", "zzz", "q"), EditOutcome::NotFound);
  }

  #[test]
  fn empty_old_text_is_not_found() {
    assert_eq!(apply_edit("a\nb\n", "", "q"), EditOutcome::NotFound);
  }

  #[test]
  fn stricter_level_wins_over_looser() {
    // An exact match exists, so L1 must win even though L4 would also match.
    let src = "  keep\nkeep\n";
    let r = apply_edit(src, "keep", "DONE");
    // "keep" (no indent) exactly matches line 2 only at L1 -> unique L1.
    assert_eq!(
      r,
      EditOutcome::Replaced {
        level: 1,
        content: "  keep\nDONE\n".to_string()
      }
    );
  }
}
