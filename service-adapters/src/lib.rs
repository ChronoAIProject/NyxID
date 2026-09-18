//! Provider-specific outbound contracts shared by the server and credential node.

pub mod ifttt;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
