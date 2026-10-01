// CLI commands sit beside the runtime, which is also exercised by backend
// integration tests against the real node implementation.
pub mod commands;
pub mod setup;
include!("runtime.rs");
