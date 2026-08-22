//! Output routing.
//!
//! Two streams with different audiences:
//!
//! * **stderr** — everything the operator watches: progress lines, spinners,
//!   approval prompts, retry notices, and (when headless) the streamed model
//!   output itself.
//! * **stdout** — the *result*, and nothing else. `seekcli -p ... --output json`
//!   must be pipeable into `jq` without filtering, so a single stray progress
//!   line on stdout is a correctness bug, not a cosmetic one.
//!
//! In the REPL the streamed answer *is* the result, so it keeps going to
//! stdout as before. Headless runs stream to stderr and print the final answer
//! (or JSON) to stdout once, which is why this is a mode flag rather than a
//! blanket redirect.
//!
//! A process-global atomic mirrors `tools::approval::init_policy`: the flag is
//! set once at startup and read from deep inside the loop, and threading it
//! through every call site would buy nothing.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static HEADLESS: AtomicBool = AtomicBool::new(false);

pub fn set_headless(value: bool) {
  HEADLESS.store(value, Ordering::SeqCst);
}

pub fn is_headless() -> bool {
  HEADLESS.load(Ordering::SeqCst)
}

/// Streamed model output (reasoning or content), routed per mode.
pub fn content(text: &str) {
  if is_headless() {
    eprint!("{}", text);
  } else {
    print!("{}", text);
  }
}

/// Flush whichever stream `content` is writing to.
pub fn flush_content() -> io::Result<()> {
  if is_headless() {
    io::stderr().flush()
  } else {
    io::stdout().flush()
  }
}

/// The final result. Always stdout, always exactly once.
pub fn result(text: &str) {
  println!("{}", text);
}
