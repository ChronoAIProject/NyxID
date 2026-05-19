//! Shared signing primitives for cloud-provider auth methods used by the
//! NyxID backend proxy and node agent.
//!
//! Surface:
//! - [`aws_sigv4`]: AWS Signature Version 4 request signing. Used for the
//!   `aws_sigv4` proxy auth method (e.g. AWS Cost Explorer).
//!
//! Both backend (`backend/src/services/proxy_service.rs`) and the node agent
//! (`cli/src/node/...`) consume the same primitive so a credential stored
//! in either place signs requests identically. See NyxID#716.

pub mod aws_sigv4;
pub mod error;

pub use error::{CloudAuthError, CloudAuthResult};
