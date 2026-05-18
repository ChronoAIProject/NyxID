//! Shared signing primitives for cloud-provider auth methods used by the
//! NyxID backend proxy and node agent.
//!
//! Two surfaces:
//! - [`aws_sigv4`]: AWS Signature Version 4 request signing. Used for the
//!   `aws_sigv4` proxy auth method (e.g. AWS Cost Explorer).
//! - [`gcp_oauth`]: Google service-account JWT-bearer OAuth flow with an
//!   in-process access-token cache. Used for the `gcp_service_account`
//!   proxy auth method (e.g. GCP Cloud Billing API, BigQuery).
//!
//! Both backend (`backend/src/services/proxy_service.rs`) and the node agent
//! (`cli/src/node/...`) consume the same primitives so a credential stored
//! in either place signs requests identically. See NyxID#716.

pub mod aws_sigv4;
pub mod error;
pub mod gcp_oauth;

pub use error::{CloudAuthError, CloudAuthResult};
