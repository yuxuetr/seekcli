//! Observability layer (L7): non-invasive accounting and tracing that wraps
//! the engine rather than polluting its control flow.

pub mod cost;
pub mod trace;
