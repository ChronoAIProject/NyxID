//! Provider-specific outbound contracts shared by the server and credential node.

pub mod ifttt;
pub mod ifttt_mcp;
pub mod response_location;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
