//! Fixed, identifier-free diagnostics; the wrapped error retains its wire contract.
use super::AppError;
use serde::Serialize;

impl AppError {
    /// Remove only additive refresh diagnostics when matching the original
    /// refusal. The /auth/refresh response must retain the wrapper.
    pub(crate) fn into_refresh_source(self) -> Self {
        match self {
            Self::RefreshRefused { source, .. } => (*source).into_refresh_source(),
            source => source,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshRefusalReason {
    RefreshExpired,
    RefreshReused,
    RefreshRevoked,
    RefreshNotFound,
    SessionExpired,
    SessionRevoked,
    /// A deleted/unknown or non-authenticated parent cannot be diagnosed as
    /// expired: TTL cleanup and explicit removal are indistinguishable.
    SessionUnavailable,
}

impl RefreshRefusalReason {
    pub fn with_error(self, source: AppError) -> AppError {
        AppError::RefreshRefused {
            source: Box::new(source),
            reason: self,
        }
    }

    pub fn unauthorized(self, message: &str) -> AppError {
        self.with_error(AppError::Unauthorized(message.into()))
    }
}
