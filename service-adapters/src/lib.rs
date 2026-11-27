//! Provider-specific outbound contracts shared by the server and credential node.

pub mod ifttt;
pub mod ifttt_mcp;
pub mod response_location;
pub mod secret_json;
mod secret_text;

pub use secret_text::{SecretText, render_credential_frame};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
