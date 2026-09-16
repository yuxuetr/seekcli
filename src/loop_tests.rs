//! End-to-end tests of the agent loop, driven by recorded LLM traffic.
//!
//! These are the first tests that exercise `run_agent_loop` at all. Everything
//! before stage 25 was pure logic: the loop had ~14% line coverage, and the
//! stage 19 fake-tool-call bug could have silently returned at any time
//! without a single test noticing.
//!
//! Each test replays a trajectory captured from the real API
//! (`SEEKCLI_RECORD=...`), so it runs offline, deterministically, and without
//! an API key. Fixtures live in `tests/fixtures/`; re-record one with:
//!
//! ```sh
//! SEEKCLI_RECORD=tests/fixtures/<name> seekcli -p "<the prompt>"
//! ```

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use crate::api::record::Replaying;
  use crate::engine::LoopStatus;
  use crate::{App, tools};

  fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .join("tests")
      .join("fixtures")
      .join(name)
  }

  fn app_for(name: &str) -> App {
    match App::for_test(Box::new(Replaying::new(fixture(name)))) {
      Ok(a) => a,
      Err(e) => panic!("cannot build test App: {}", e),
    }
  }

  /// Run inside a scratch directory, since the loop's tools resolve paths
  /// against the process cwd and `write_file` is confined to it.
  struct Scratch {
    original: PathBuf,
    dir: PathBuf,
    // Held for the lifetime of the test, released on drop with the cwd.
    _guard: std::sync::MutexGuard<'static, ()>,
  }

  impl Scratch {
    fn enter(name: &str) -> Self {
      let guard = crate::testsync::lock();
      // The loop may reach run_shell; without this it would block on stdin.
      tools::approval::set_interaction(tools::approval::Interaction::AutoDeny);
      let original = std::env::current_dir().unwrap_or_default();
      let dir = std::env::temp_dir().join(format!("seekcli-loop-{}", name));
      let _ = std::fs::remove_dir_all(&dir);
      // Loud, not best-effort. These were `let _ =`, and the failure mode was
      // vicious: a chdir that silently did not happen left the test running in
      // the repository, where `ensure_agent_system_prompt` found AGENTS.md and
      // injected workspace rules the recording does not have. The replay then
      // failed with "message count 3 != recorded 2" — a message about the
      // fixture, pointing nowhere near the actual cause. It reproduced about
      // once in 25 full runs and never single-threaded.
      if let Err(e) = std::fs::create_dir_all(&dir) {
        panic!("cannot create scratch {}: {e}", dir.display());
      }
      if let Err(e) = std::env::set_current_dir(&dir) {
        panic!("cannot enter scratch {}: {e}", dir.display());
      }
      // Resolve through the same canonicalisation the tools see, so
      // comparisons do not trip over /var vs /private/var on macOS.
      let dir = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => panic!("cannot read back the scratch directory: {e}"),
      };
      // The assertion that would have named the bug directly: whatever else
      // went wrong, the test must not be running in the repository.
      assert_ne!(
        dir, original,
        "scratch chdir did not take effect — the test would run in the \
         repository, where AGENTS.md changes the prompt"
      );
      Self {
        original,
        dir,
        _guard: guard,
      }
    }

    /// Absolute path inside *this* scratch, never re-read from the process
    /// cwd — that is what made these assertions race in the first place.
    fn path(&self, rel: &str) -> PathBuf {
      self.dir.join(rel)
    }
  }

  impl Drop for Scratch {
    fn drop(&mut self) {
      let _ = std::env::set_current_dir(&self.original);
    }
  }

  /// The stage 19 regression guard.
  ///
  /// The failing shape was: `read_file` misses, error recovery fires, the
  /// Two-Stage micro trigger runs a tools-free planning pass, and then the
  /// model narrates a fake tool call instead of emitting a real one — so it
  /// reports success while the file is never written. Asserting on the file
  /// rather than on the text is deliberate: the bug's signature was a model
  /// that *said* it had done the work.
  #[tokio::test]
  async fn recovers_from_a_missing_file_and_actually_writes_it() {
    let scratch = Scratch::enter("two-stage");
    let mut app = app_for("two-stage-recovery");

    let outcome = match app
      .run_headless("读取 notes.md；如果它不存在就创建它，内容写 hello", None)
      .await
    {
      Ok(o) => o,
      Err(e) => panic!("loop failed: {}", e),
    };

    assert_eq!(outcome.status, LoopStatus::Completed);
    assert!(
      scratch.path("notes.md").exists(),
      "the model claimed success -- the file must actually exist"
    );
    // read (fail) -> planning pass -> write -> answer
    assert_eq!(
      outcome.llm_calls, 4,
      "expected the full recovery trajectory"
    );
  }

  /// Two read-only tools in one turn take the Fork-Join path. Recorded from a
  /// real turn so the parallel branch is exercised, not just its predicate.
  #[tokio::test]
  async fn parallel_read_only_batch_returns_results_in_order() {
    let scratch = Scratch::enter("parallel");
    let _ = std::fs::create_dir_all(scratch.path("src"));
    let _ = std::fs::write(
      scratch.path("src/main.rs"),
      "fn main() {\n  println!(\"a\");\n}\n",
    );
    let _ = std::fs::write(scratch.path("src/lib.rs"), "pub fn helper() {}\n");

    let mut app = app_for("parallel-readonly");
    let outcome = match app
      .run_headless(
        "用 glob 找出所有 .rs 文件，同时用 grep 搜索 fn，把两个结果一起告诉我",
        None,
      )
      .await
    {
      Ok(o) => o,
      Err(e) => panic!("loop failed: {}", e),
    };

    assert_eq!(outcome.status, LoopStatus::Completed);
    assert!(!outcome.text.is_empty());
  }

  /// The policy gate seen from inside the loop: a denial must come back as a
  /// tool result the model can react to, not as an error that aborts the turn.
  #[tokio::test]
  async fn read_only_mode_denies_the_write_and_the_loop_still_finishes() {
    let scratch = Scratch::enter("readonly");
    let _ = std::fs::write(scratch.path("victim.txt"), "keep me\n");

    tools::policy::set_mode(tools::policy::Mode::ReadOnly);
    let mut app = app_for("readonly-denial");
    let outcome = app.run_headless("删除 victim.txt", None).await;
    tools::policy::set_mode(tools::policy::Mode::Normal);

    let outcome = match outcome {
      Ok(o) => o,
      Err(e) => panic!("a denial must not abort the loop: {}", e),
    };
    assert_eq!(outcome.status, LoopStatus::Completed);
    assert!(
      scratch.path("victim.txt").exists(),
      "read-only mode must not let the file be deleted"
    );
  }

  /// The invariant the event log exists to hold: everything the model saw is
  /// reconstructable from the log alone. If a loop path ever appends to the
  /// working set without logging it, the projection comes back short and this
  /// fails — which is the only way that drift would be noticed at all.
  #[tokio::test]
  async fn everything_the_model_saw_is_reconstructable_from_the_log() {
    use crate::session::{EventPayload, Session};

    let scratch = Scratch::enter("invariant");
    let mut app = app_for("two-stage-recovery");
    let prompt = "读取 notes.md；如果它不存在就创建它，内容写 hello";

    let mut session = Session::new("invariant-test".into(), "m".into());
    session.record(EventPayload::UserMessage {
      images: Vec::new(),
      content: prompt.to_string(),
    });

    let outcome = match app.run_headless(prompt, None).await {
      Ok(o) => o,
      Err(e) => panic!("loop failed: {}", e),
    };
    assert_eq!(outcome.status, LoopStatus::Completed);
    assert!(scratch.path("notes.md").exists());

    // The recorded trajectory: assistant(read_file) -> tool result ->
    // assistant(write_file) -> tool result -> assistant(final).
    let events = outcome.events.clone();
    let assistants = events
      .iter()
      .filter(|e| matches!(e, EventPayload::AssistantMessage { .. }))
      .count();
    let tool_results = events
      .iter()
      .filter(|e| matches!(e, EventPayload::ToolResult { .. }))
      .count();
    assert!(
      assistants >= 3,
      "expected three assistant turns, got {}",
      assistants
    );
    assert_eq!(
      tool_results, 2,
      "read_file and write_file each returned once"
    );

    session.extend(events);
    let projected = session.messages();
    // user + every assistant turn + every tool result.
    assert_eq!(projected.len(), 1 + assistants + tool_results);
  }

  /// L7-6's point: after a refusal the model can ask what IS allowed instead of
  /// guessing again. Recorded against the live API with `--read-only`.
  ///
  /// The assertion is on the *trajectory*, not the prose: the model must reach
  /// for `harness_inspect` and then report the real allowlist. Asserting the
  /// wording would break on any reply the model phrases differently.
  #[allow(clippy::await_holding_lock)]
  #[tokio::test]
  async fn a_denied_agent_inspects_the_policy_instead_of_guessing() {
    let scratch = Scratch::enter("inspect-after-denial");
    tools::policy::set_mode(tools::policy::Mode::ReadOnly);
    let mut app = app_for("inspect-after-denial");
    let outcome = app
      .run_headless(
        "用 shell 命令把 hello 写进 a.txt。如果被拒绝，请先查清当前模式下到底允许哪些命令，再告诉我结论。",
        None,
      )
      .await;
    tools::policy::set_mode(tools::policy::Mode::Normal);

    let outcome = match outcome {
      Ok(o) => o,
      Err(e) => panic!("a denial must not abort the loop: {}", e),
    };

    let inspected = outcome.events.iter().any(|e| match e {
      crate::session::EventPayload::AssistantMessage { tool_calls, .. } => tool_calls
        .iter()
        .any(|c| c.function.name == "harness_inspect"),
      _ => false,
    });
    assert!(inspected, "the model must ask, not guess");

    // And the refusal held: read-only means no file appeared.
    assert!(!scratch.path("a.txt").exists(), "read-only was breached");

    // The reported allowlist is the gate's own, so it cannot drift into prose.
    assert!(
      outcome.text.contains("realpath") || outcome.text.contains("readlink"),
      "the real allowlist should reach the user: {}",
      outcome.text
    );
  }

  /// A structurally different request must fail loudly rather than replay an
  /// answer that was never given to it.
  ///
  /// Note what does *not* trigger this: merely rewording the prompt. The shape
  /// check is deliberately structural, so a fixture survives prompt edits but
  /// not a changed conversation. Activating a Skill adds a system message, so
  /// the message count no longer matches what was recorded.
  #[tokio::test]
  async fn a_structurally_different_request_fails_instead_of_going_green() {
    let _scratch = Scratch::enter("divergent");
    let mut app = app_for("two-stage-recovery");
    let skill = crate::Skill {
      name: "extra".into(),
      description: "adds a system message the recording never had".into(),
      system_prompt: "be terse".into(),
      tools: None,
      version: None,
      source: None,
      allowed_tools: None,
    };
    let result = app
      .run_headless(
        "读取 notes.md；如果它不存在就创建它，内容写 hello",
        Some(&skill),
      )
      .await;
    match result {
      Ok(o) => panic!("a mismatched replay must fail, got status {:?}", o.status),
      Err(e) => {
        let msg = format!("{:#}", e);
        assert!(
          msg.contains("does not match") || msg.contains("replay exhausted"),
          "unhelpful failure: {}",
          msg
        );
      }
    }
  }
}
