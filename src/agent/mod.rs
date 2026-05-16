pub mod compressor;
pub mod prompt;

/// Maximum number of ReAct iterations per chat turn.
/// Hard cap to prevent runaway tool loops if the model misbehaves.
pub const MAX_ITER: usize = 25;

/// Maximum nesting depth for sub-agent delegation.
pub const MAX_SUBAGENT_DEPTH: usize = 3;
