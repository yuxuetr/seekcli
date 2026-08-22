//! Retry, backoff and timeout, applied as a decorator around any
//! `LlmProvider`.
//!
//! Same shape as `observability::cost::CostTracker`: the agent loop must not
//! grow retry code (see `docs/architecture/design-principles.md` §3). Wrapping
//! the provider keeps `engine.rs` unaware that a request was ever retried.
//!
//! Scope note: only the **request** is retried — the call that returns the
//! stream. Once bytes are flowing we never retry, because a stream that dies
//! after emitting `tool_calls` has already caused side effects downstream, and
//! replaying it would duplicate them. A mid-stream failure is handed to L1's
//! Error Recovery instead. What the stream *does* get is an idle timeout, so a
//! server that accepts the request and then goes quiet cannot hang the agent
//! forever.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Result;
use futures_util::Stream;

use super::{LlmError, LlmProvider, Message, StreamItem, StreamResult, Tool};

#[derive(Debug, Clone)]
pub struct RetryPolicy {
  pub max_attempts: u32,
  pub base_delay: Duration,
  /// Cap on a single backoff sleep, so a large `Retry-After` or a high
  /// attempt count cannot park the agent for minutes.
  pub max_delay: Duration,
  /// How long to wait for response headers. Not a cap on total generation —
  /// the body streams for as long as the model keeps talking.
  pub request_timeout: Duration,
  /// How long the stream may go silent between items before we give up.
  pub stream_idle_timeout: Duration,
}

impl Default for RetryPolicy {
  fn default() -> Self {
    Self {
      max_attempts: 4,
      base_delay: Duration::from_millis(500),
      max_delay: Duration::from_secs(30),
      request_timeout: Duration::from_secs(120),
      stream_idle_timeout: Duration::from_secs(60),
    }
  }
}

/// Whether another attempt could plausibly succeed.
///
/// The dividing line is not "did it fail" but "is the failure about this
/// request or about the world". A 400 means the payload is wrong and will be
/// wrong again; a 429 or 503 means try later.
pub fn is_retryable(err: &anyhow::Error) -> bool {
  match err.downcast_ref::<LlmError>() {
    Some(LlmError::Transport { .. }) => true,
    Some(LlmError::Status { status, .. }) => match status {
      429 => true,
      408 => true,            // request timeout — the server invites a retry
      s if *s >= 500 => true, // 5xx: server-side, may pass
      _ => false,             // other 4xx: our payload is wrong, retrying repeats it
    },
    // An error we did not classify. Retrying an unknown failure risks
    // duplicating whatever it was, so fail fast and let the user see it.
    None => false,
  }
}

fn retry_after(err: &anyhow::Error) -> Option<Duration> {
  match err.downcast_ref::<LlmError>() {
    Some(LlmError::Status { retry_after, .. }) => *retry_after,
    _ => None,
  }
}

/// Backoff for `attempt` (0-based), honouring a server-supplied `Retry-After`.
///
/// Jitter is deterministic rather than random: `Math.random`-style jitter would
/// make the retry sequence untestable, and the only thing jitter must achieve
/// here is that a single client's repeated retries are not perfectly periodic.
pub fn backoff(policy: &RetryPolicy, attempt: u32, server_hint: Option<Duration>) -> Duration {
  if let Some(hint) = server_hint {
    return hint.min(policy.max_delay);
  }
  let factor = 1u32 << attempt.min(16);
  let base = policy.base_delay.saturating_mul(factor);
  // +0..25% spread, derived from the attempt number.
  let jitter = base / 4 * u32::from(attempt % 2 == 1) / 1;
  (base + jitter).min(policy.max_delay)
}

pub struct Resilient {
  inner: Box<dyn LlmProvider>,
  policy: RetryPolicy,
}

impl Resilient {
  pub fn new(inner: Box<dyn LlmProvider>, policy: RetryPolicy) -> Self {
    Self { inner, policy }
  }
}

#[async_trait::async_trait]
impl LlmProvider for Resilient {
  async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<StreamResult> {
    let mut attempt = 0u32;
    loop {
      let call =
        self
          .inner
          .call_api_with_params(model, messages.clone(), thinking_mode, tools.clone());
      let outcome = match tokio::time::timeout(self.policy.request_timeout, call).await {
        Ok(result) => result,
        Err(_) => Err(
          LlmError::Transport {
            provider: "resilience",
            source: format!(
              "no response headers within {:?}",
              self.policy.request_timeout
            ),
          }
          .into(),
        ),
      };

      match outcome {
        Ok(stream) => {
          return Ok(Box::pin(IdleTimeout::new(
            stream,
            self.policy.stream_idle_timeout,
          )));
        }
        Err(e) => {
          attempt += 1;
          if attempt >= self.policy.max_attempts || !is_retryable(&e) {
            return Err(e);
          }
          let delay = backoff(&self.policy, attempt - 1, retry_after(&e));
          // Visible, per design-principles: degrade rather than abort, but
          // never silently.
          eprintln!(
            "[LLM] {} — retrying in {:?} ({}/{})",
            e, delay, attempt, self.policy.max_attempts
          );
          tokio::time::sleep(delay).await;
        }
      }
    }
  }
}

/// Fails the stream if no item arrives within `idle`.
///
/// Needed because a request that returns 200 and then stalls looks identical
/// to a slow model: without this the agent waits forever with no output and no
/// error, which is the worst possible failure mode for an unattended run.
struct IdleTimeout {
  inner: StreamResult,
  idle: Duration,
  sleep: Pin<Box<tokio::time::Sleep>>,
}

impl IdleTimeout {
  fn new(inner: StreamResult, idle: Duration) -> Self {
    Self {
      inner,
      idle,
      sleep: Box::pin(tokio::time::sleep(idle)),
    }
  }
}

impl Stream for IdleTimeout {
  type Item = Result<StreamItem>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let this = self.get_mut();
    match this.inner.as_mut().poll_next(cx) {
      Poll::Ready(item) => {
        // Any progress resets the clock; the timeout is about silence, not
        // about total duration.
        this.sleep = Box::pin(tokio::time::sleep(this.idle));
        Poll::Ready(item)
      }
      Poll::Pending => match this.sleep.as_mut().poll(cx) {
        Poll::Ready(()) => Poll::Ready(Some(Err(
          LlmError::Transport {
            provider: "resilience",
            source: format!("stream idle for {:?}", this.idle),
          }
          .into(),
        ))),
        Poll::Pending => Poll::Pending,
      },
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn status(code: u16, retry_after: Option<Duration>) -> anyhow::Error {
    LlmError::Status {
      provider: "test",
      status: code,
      retry_after,
      body: String::new(),
    }
    .into()
  }

  #[test]
  fn transient_failures_retry_and_client_errors_do_not() {
    // The world is busy or broken -> try again.
    assert!(is_retryable(&status(429, None)), "rate limit");
    assert!(is_retryable(&status(500, None)), "server error");
    assert!(is_retryable(&status(503, None)), "unavailable");
    assert!(is_retryable(&status(408, None)), "server-side timeout");
    assert!(is_retryable(
      &LlmError::Transport {
        provider: "test",
        source: "connection refused".into(),
      }
      .into()
    ));

    // Our payload is wrong; retrying sends the same wrong payload.
    assert!(!is_retryable(&status(400, None)), "bad request");
    assert!(!is_retryable(&status(401, None)), "bad key");
    assert!(!is_retryable(&status(404, None)), "wrong endpoint");
    assert!(!is_retryable(&status(422, None)), "schema rejected");
  }

  #[test]
  fn unclassified_errors_fail_fast_rather_than_repeat_unknown_work() {
    let opaque = anyhow::anyhow!("something else entirely");
    assert!(!is_retryable(&opaque));
  }

  #[test]
  fn backoff_grows_exponentially_and_is_capped() {
    let p = RetryPolicy {
      base_delay: Duration::from_millis(100),
      max_delay: Duration::from_secs(1),
      ..RetryPolicy::default()
    };
    assert_eq!(backoff(&p, 0, None), Duration::from_millis(100));
    assert!(backoff(&p, 1, None) >= Duration::from_millis(200));
    assert!(backoff(&p, 2, None) >= Duration::from_millis(400));
    // Without the cap, attempt 20 would be years.
    assert_eq!(backoff(&p, 20, None), Duration::from_secs(1));
  }

  #[test]
  fn server_retry_after_overrides_our_own_schedule() {
    let p = RetryPolicy {
      base_delay: Duration::from_millis(100),
      max_delay: Duration::from_secs(60),
      ..RetryPolicy::default()
    };
    let hint = Duration::from_secs(7);
    assert_eq!(backoff(&p, 0, Some(hint)), hint);
    // ...but still bounded: a server asking for an hour must not park the agent.
    assert_eq!(
      backoff(&p, 0, Some(Duration::from_secs(3600))),
      Duration::from_secs(60)
    );
  }

  /// Fails the first `fail_times` calls, then succeeds. Counts calls so a
  /// test can assert the decorator actually issued another request rather
  /// than merely deciding it would.
  struct Flaky {
    fail_times: std::sync::atomic::AtomicU32,
    calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
    status_code: u16,
  }

  #[async_trait::async_trait]
  impl LlmProvider for Flaky {
    async fn call_api_with_params(
      &self,
      _model: &str,
      _messages: Vec<Message>,
      _thinking_mode: &str,
      _tools: Option<Vec<Tool>>,
    ) -> Result<StreamResult> {
      use std::sync::atomic::Ordering;
      self.calls.fetch_add(1, Ordering::SeqCst);
      if self.fail_times.load(Ordering::SeqCst) > 0 {
        self.fail_times.fetch_sub(1, Ordering::SeqCst);
        return Err(status(self.status_code, None));
      }
      Ok(Box::pin(futures_util::stream::once(async {
        Ok(StreamItem::Content("ok".to_string()))
      })))
    }
  }

  fn fast_policy() -> RetryPolicy {
    RetryPolicy {
      max_attempts: 4,
      base_delay: Duration::from_millis(1),
      max_delay: Duration::from_millis(5),
      ..RetryPolicy::default()
    }
  }

  fn flaky(
    fail_times: u32,
    status_code: u16,
  ) -> (Resilient, std::sync::Arc<std::sync::atomic::AtomicU32>) {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let inner = Flaky {
      fail_times: std::sync::atomic::AtomicU32::new(fail_times),
      calls: calls.clone(),
      status_code,
    };
    (Resilient::new(Box::new(inner), fast_policy()), calls)
  }

  #[tokio::test]
  async fn a_transient_failure_is_retried_until_it_succeeds() {
    let (provider, calls) = flaky(2, 503);
    let out = provider
      .call_api_with_params("m", vec![], "none", None)
      .await;
    assert!(out.is_ok(), "should have recovered after retries");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
  }

  #[tokio::test]
  async fn a_client_error_is_not_retried() {
    let (provider, calls) = flaky(2, 400);
    let out = provider
      .call_api_with_params("m", vec![], "none", None)
      .await;
    assert!(out.is_err(), "400 must surface immediately");
    assert_eq!(
      calls.load(std::sync::atomic::Ordering::SeqCst),
      1,
      "a bad payload must not be sent again"
    );
  }

  #[tokio::test]
  async fn retries_stop_at_max_attempts() {
    let (provider, calls) = flaky(99, 503);
    let out = provider
      .call_api_with_params("m", vec![], "none", None)
      .await;
    assert!(out.is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
  }

  /// A provider that returns 200 and then never speaks again — indistinguishable
  /// from a slow model without the idle timeout, and the worst failure mode for
  /// an unattended `--run-task` run.
  struct Silent;

  #[async_trait::async_trait]
  impl LlmProvider for Silent {
    async fn call_api_with_params(
      &self,
      _model: &str,
      _messages: Vec<Message>,
      _thinking_mode: &str,
      _tools: Option<Vec<Tool>>,
    ) -> Result<StreamResult> {
      Ok(Box::pin(futures_util::stream::pending()))
    }
  }

  #[tokio::test]
  async fn a_silent_stream_errors_instead_of_hanging_forever() {
    use futures_util::StreamExt;
    let policy = RetryPolicy {
      stream_idle_timeout: Duration::from_millis(30),
      ..fast_policy()
    };
    let provider = Resilient::new(Box::new(Silent), policy);
    let mut stream = match provider
      .call_api_with_params("m", vec![], "none", None)
      .await
    {
      Ok(s) => s,
      Err(e) => panic!("request itself should succeed: {}", e),
    };
    let item = tokio::time::timeout(Duration::from_secs(2), stream.next()).await;
    match item {
      Ok(Some(Err(e))) => assert!(format!("{}", e).contains("idle"), "got: {}", e),
      Ok(other) => panic!("expected an idle-timeout error, got {:?}", other.is_some()),
      Err(_) => panic!("idle timeout did not fire — the agent would hang here"),
    }
  }

  #[test]
  fn retry_after_header_parses_delta_seconds() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "12".parse().unwrap());
    assert_eq!(
      super::super::parse_retry_after(&headers),
      Some(Duration::from_secs(12))
    );

    // HTTP-date form is deliberately unsupported; falling back to our own
    // backoff is better than pulling in a date parser.
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
      reqwest::header::RETRY_AFTER,
      "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
    );
    assert_eq!(super::super::parse_retry_after(&headers), None);
  }
}
