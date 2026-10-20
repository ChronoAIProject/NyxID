//! Server-authenticated xAI relay. Browser JSON is never forwarded upstream.
mod actor;
mod arbiter;
mod gate;
mod protocol;
mod relay;
#[cfg(test)]
mod tests;
pub use actor::Handle;
pub use protocol::connect;
pub use relay::{ClientInput, Grok, Output};
