use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Structured JSON error response returned by all API error paths.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Machine-readable error category (e.g. "unauthorized")
    pub error: String,
    /// Numeric error code for client-side mapping
    pub error_code: u32,
    /// Human-readable error description
    pub message: String,
    /// MFA session token, only present when error_code == 2002 (mfa_required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
}

/// Application-level error variants.
/// Each variant maps to a specific HTTP status code and error payload.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] mongodb::error::Error),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Token expired")]
    TokenExpired,

    #[error("MFA required")]
    MfaRequired { session_token: String },

    #[error("PKCE verification failed")]
    PkceVerificationFailed,

    #[error("Invalid redirect URI")]
    InvalidRedirectUri,

    #[error("Invalid scope: {0}")]
    InvalidScope(String),
}

impl AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) | Self::ValidationError(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_)
            | Self::AuthenticationFailed(_)
            | Self::TokenExpired => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::MfaRequired { .. } => StatusCode::FORBIDDEN,
            Self::PkceVerificationFailed
            | Self::InvalidRedirectUri
            | Self::InvalidScope(_) => StatusCode::BAD_REQUEST,
            Self::Internal(_) | Self::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_code(&self) -> u32 {
        match self {
            Self::BadRequest(_) => 1000,
            Self::Unauthorized(_) => 1001,
            Self::Forbidden(_) => 1002,
            Self::NotFound(_) => 1003,
            Self::Conflict(_) => 1004,
            Self::RateLimited => 1005,
            Self::Internal(_) => 1006,
            Self::DatabaseError(_) => 1007,
            Self::ValidationError(_) => 1008,
            Self::AuthenticationFailed(_) => 2000,
            Self::TokenExpired => 2001,
            Self::MfaRequired { .. } => 2002,
            Self::PkceVerificationFailed => 3000,
            Self::InvalidRedirectUri => 3001,
            Self::InvalidScope(_) => 3002,
        }
    }

    pub(crate) fn error_key(&self) -> &str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::RateLimited => "rate_limited",
            Self::Internal(_) => "internal_error",
            Self::DatabaseError(_) => "database_error",
            Self::ValidationError(_) => "validation_error",
            Self::AuthenticationFailed(_) => "authentication_failed",
            Self::TokenExpired => "token_expired",
            Self::MfaRequired { .. } => "mfa_required",
            Self::PkceVerificationFailed => "pkce_verification_failed",
            Self::InvalidRedirectUri => "invalid_redirect_uri",
            Self::InvalidScope(_) => "invalid_scope",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();

        // Log server errors at error level; client errors at warn level
        match &self {
            AppError::Internal(msg) => tracing::error!(error = %msg, "Internal server error"),
            AppError::DatabaseError(err) => tracing::error!(error = %err, "Database error"),
            _ => tracing::warn!(error = %self, "Client error"),
        }

        // Extract MFA session token before consuming self in the message match
        let mfa_session_token = match &self {
            AppError::MfaRequired { session_token } => Some(session_token.clone()),
            _ => None,
        };

        let body = ErrorResponse {
            error: self.error_key().to_string(),
            error_code: self.error_code(),
            message: match &self {
                // Never leak internal details to clients
                AppError::Internal(_) | AppError::DatabaseError(_) => {
                    "An internal error occurred".to_string()
                }
                AppError::MfaRequired { .. } => {
                    "MFA verification required".to_string()
                }
                other => other.to_string(),
            },
            session_token: mfa_session_token,
        };

        (status, axum::Json(body)).into_response()
    }
}

/// Convenience type alias for handler return types.
pub type AppResult<T> = Result<T, AppError>;
