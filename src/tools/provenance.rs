//! What this session actually looked at.
//!
//! The citation requirement — "say where the conclusion came from" — is the
//! kind of thing that usually lives in a system prompt and is therefore
//! unenforceable. This module makes it a fact the harness holds: which URLs
//! were *searched up* (a snippet the engine wrote) and which were *fetched*
//! (text the agent actually read). `harness_inspect{what:"sources"}` renders
//! the two separately.
//!
//! The distinction is the whole point. A conclusion supported only by search
//! snippets rests on something nobody opened, and that is visible here without
//! anyone having to take the model's word for it.
//!
//! Session-scoped and in-memory: this is a view of the current run, not a
//! durable record. What belongs on disk is already in the event log — the tool
//! call and its result are both there.

use std::sync::Mutex;

/// One source, and how closely it was actually examined.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
  pub url: String,
  /// `None` for a search hit; the fetch timestamp once it has been read.
  pub fetched_at: Option<String>,
}

static SOURCES: Mutex<Vec<Source>> = Mutex::new(Vec::new());

/// Note the URLs a search surfaced. They are candidates, not evidence, so they
/// land with no fetch time.
pub fn record_search(_query: &str, urls: &[String]) {
  let Ok(mut sources) = SOURCES.lock() else {
    return;
  };
  for url in urls {
    if !sources.iter().any(|s| &s.url == url) {
      sources.push(Source {
        url: url.clone(),
        fetched_at: None,
      });
    }
  }
}

/// Note that a URL's actual text was read. Promotes an existing search hit
/// rather than duplicating it — the same URL found and then opened is one
/// source examined two ways, not two sources.
pub fn record_fetch(url: &str, at: &str) {
  let Ok(mut sources) = SOURCES.lock() else {
    return;
  };
  match sources.iter_mut().find(|s| s.url == url) {
    Some(existing) => existing.fetched_at = Some(at.to_string()),
    None => sources.push(Source {
      url: url.to_string(),
      fetched_at: Some(at.to_string()),
    }),
  }
}

/// Everything seen this session, in the order it was first encountered.
pub fn snapshot() -> Vec<Source> {
  SOURCES.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Forget everything — `/clear` and `/load` start a different conversation,
/// and carrying the old session's sources into it would misattribute them.
pub fn reset() {
  if let Ok(mut sources) = SOURCES.lock() {
    sources.clear();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Finding a URL and then opening it is one source examined twice, not two
  /// sources. Counting it twice would inflate "how much did we actually read".
  #[test]
  fn fetching_a_searched_url_promotes_it_rather_than_duplicating() {
    reset();
    record_search("q", &["https://a.test".to_string()]);
    record_fetch("https://a.test", "2026-09-16T00:00:00Z");
    let all = snapshot();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].fetched_at.as_deref(), Some("2026-09-16T00:00:00Z"));
  }

  /// The distinction the whole module exists for: a snippet is not a read.
  #[test]
  fn a_searched_url_is_not_marked_as_read() {
    reset();
    record_search("q", &["https://b.test".to_string()]);
    let all = snapshot();
    assert_eq!(all.len(), 1);
    assert!(
      all[0].fetched_at.is_none(),
      "a search hit must not look like something we read: {all:?}"
    );
    reset();
  }

  #[test]
  fn reset_clears_between_conversations() {
    reset();
    record_fetch("https://c.test", "t");
    assert_eq!(snapshot().len(), 1);
    reset();
    assert!(snapshot().is_empty());
  }
}
