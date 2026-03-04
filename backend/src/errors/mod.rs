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
    /// Browser URL to complete consent flow (consent_required only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_url: Option<String>,
    /// Approval request ID (approval_required only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
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

    #[error("Role not found: {0}")]
    RoleNotFound(String),

    #[error("Group not found: {0}")]
    GroupNotFound(String),

    #[error("Consent not found")]
    ConsentNotFound,

    #[error("Role already assigned")]
    RoleAlreadyAssigned,

    #[error("User already a member of this group")]
    GroupMembershipExists,

    #[error("Cannot modify system role: {0}")]
    SystemRoleProtected(String),

    #[error("Duplicate slug: {0}")]
    DuplicateSlug(String),

    #[error("Circular group hierarchy detected")]
    CircularGroupHierarchy,

    #[error("Service account not found: {0}")]
    ServiceAccountNotFound(String),

    #[error("Service account is inactive")]
    ServiceAccountInactive,

    #[error("Social authentication failed: {0}")]
    SocialAuthFailed(String),

    #[error("Social auth conflict: email already linked to another provider")]
    SocialAuthConflict,

    #[error("Social auth: no verified email from provider")]
    SocialAuthNoEmail,

    #[error("Social auth: account is deactivated")]
    SocialAuthDeactivated,

    #[error("Consent required")]
    ConsentRequired { consent_url: String },

    #[error("Unsupported grant type: {0}")]
    UnsupportedGrantType(String),

    #[error("Approval required")]
    ApprovalRequired { request_id: String },

    #[error("External token verification failed: {0}")]
    ExternalTokenInvalid(String),

    #[error("External provider not configured: {0}")]
    ExternalProviderNotConfigured(String),
}

impl AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) | Self::ValidationError(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) | Self::AuthenticationFailed(_) | Self::TokenExpired => {
                StatusCode::UNAUTHORIZED
            }
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::MfaRequired { .. } => StatusCode::FORBIDDEN,
            Self::PkceVerificationFailed | Self::InvalidRedirectUri | Self::InvalidScope(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::RoleNotFound(_) | Self::GroupNotFound(_) | Self::ConsentNotFound => {
                StatusCode::NOT_FOUND
            }
            Self::RoleAlreadyAssigned | Self::GroupMembershipExists => StatusCode::CONFLICT,
            Self::SystemRoleProtected(_) => StatusCode::FORBIDDEN,
            Self::DuplicateSlug(_) => StatusCode::CONFLICT,
            Self::CircularGroupHierarchy => StatusCode::BAD_REQUEST,
            Self::ServiceAccountNotFound(_) => StatusCode::NOT_FOUND,
            Self::ServiceAccountInactive => StatusCode::FORBIDDEN,
            Self::SocialAuthFailed(_) | Self::SocialAuthNoEmail => StatusCode::BAD_REQUEST,
            Self::SocialAuthConflict => StatusCode::CONFLICT,
            Self::SocialAuthDeactivated => StatusCode::FORBIDDEN,
            Self::ConsentRequired { .. } => StatusCode::FORBIDDEN,
            Self::UnsupportedGrantType(_) => StatusCode::BAD_REQUEST,
            Self::ApprovalRequired { .. } => StatusCode::FORBIDDEN,
            Self::ExternalTokenInvalid(_) | Self::ExternalProviderNotConfigured(_) => {
                StatusCode::BAD_REQUEST
            }
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
            Self::RoleNotFound(_) => 4000,
            Self::GroupNotFound(_) => 4001,
            Self::ConsentNotFound => 4002,
            Self::RoleAlreadyAssigned => 4003,
            Self::GroupMembershipExists => 4004,
            Self::SystemRoleProtected(_) => 4005,
            Self::DuplicateSlug(_) => 4006,
            Self::CircularGroupHierarchy => 4007,
            Self::ServiceAccountNotFound(_) => 5000,
            Self::ServiceAccountInactive => 5001,
            Self::SocialAuthFailed(_) => 6000,
            Self::SocialAuthConflict => 6001,
            Self::SocialAuthNoEmail => 6002,
            Self::SocialAuthDeactivated => 6003,
            Self::ConsentRequired { .. } => 3003,
            Self::UnsupportedGrantType(_) => 3004,
            Self::ApprovalRequired { .. } => 7000,
            Self::ExternalTokenInvalid(_) => 6004,
            Self::ExternalProviderNotConfigured(_) => 6005,
        }
    }

    /// RFC 6749 §5.2 OAuth error code for token endpoint responses.
    /// Each variant maps to a standard OAuth error string — no string matching.
    pub(crate) fn oauth_error_code(&self) -> &'static str {
        match self {
            Self::UnsupportedGrantType(_) => "unsupported_grant_type",
            Self::PkceVerificationFailed | Self::InvalidRedirectUri => "invalid_grant",
            Self::InvalidScope(_) => "invalid_scope",
            Self::Unauthorized(_)
            | Self::AuthenticationFailed(_)
            | Self::ServiceAccountNotFound(_)
            | Self::ServiceAccountInactive => "invalid_client",
            Self::NotFound(_) => "invalid_grant",
            Self::ConsentRequired { .. } => "consent_required",
            Self::ExternalTokenInvalid(_) => "invalid_grant",
            Self::ExternalProviderNotConfigured(_) => "invalid_request",
            _ => "invalid_request",
        }
    }

    /// HTTP status to use when emitting an RFC 6749 §5.2 error response.
    pub(crate) fn oauth_status(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_)
            | Self::AuthenticationFailed(_)
            | Self::ServiceAccountNotFound(_)
            | Self::ServiceAccountInactive => StatusCode::UNAUTHORIZED,
            Self::Internal(_) | Self::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
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
            Self::RoleNotFound(_) => "role_not_found",
            Self::GroupNotFound(_) => "group_not_found",
            Self::ConsentNotFound => "consent_not_found",
            Self::RoleAlreadyAssigned => "role_already_assigned",
            Self::GroupMembershipExists => "group_membership_exists",
            Self::SystemRoleProtected(_) => "system_role_protected",
            Self::DuplicateSlug(_) => "duplicate_slug",
            Self::CircularGroupHierarchy => "circular_group_hierarchy",
            Self::ServiceAccountNotFound(_) => "service_account_not_found",
            Self::ServiceAccountInactive => "service_account_inactive",
            Self::SocialAuthFailed(_) => "social_auth_failed",
            Self::SocialAuthConflict => "social_auth_conflict",
            Self::SocialAuthNoEmail => "social_auth_no_email",
            Self::SocialAuthDeactivated => "social_auth_deactivated",
            Self::ConsentRequired { .. } => "consent_required",
            Self::UnsupportedGrantType(_) => "unsupported_grant_type",
            Self::ApprovalRequired { .. } => "approval_required",
            Self::ExternalTokenInvalid(_) => "external_token_invalid",
            Self::ExternalProviderNotConfigured(_) => "external_provider_not_configured",
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
        let consent_url = match &self {
            AppError::ConsentRequired { consent_url } => Some(consent_url.clone()),
            _ => None,
        };
        let approval_request_id = match &self {
            AppError::ApprovalRequired { request_id } => Some(request_id.clone()),
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
                AppError::MfaRequired { .. } => "MFA verification required".to_string(),
                AppError::ConsentRequired { .. } => {
                    "Consent required. Complete authorization in browser flow.".to_string()
                }
                AppError::ApprovalRequired { .. } => {
                    "Approval required. A notification has been sent to the resource owner."
                        .to_string()
                }
                other => other.to_string(),
            },
            session_token: mfa_session_token,
            consent_url,
            request_id: approval_request_id,
        };

        (status, axum::Json(body)).into_response()
    }
}

/// Convenience type alias for handler return types.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes() {
        assert_eq!(
            AppError::BadRequest("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::Unauthorized("x".into()).status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::Forbidden("x".into()).status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::NotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::Conflict("x".into()).status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            AppError::RateLimited.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            AppError::Internal("x".into()).status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            AppError::ValidationError("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::AuthenticationFailed("x".into()).status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::TokenExpired.status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::MfaRequired {
                session_token: "tok".into()
            }
            .status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::PkceVerificationFailed.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::InvalidRedirectUri.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::InvalidScope("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::RoleNotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::GroupNotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::ConsentNotFound.status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::RoleAlreadyAssigned.status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            AppError::GroupMembershipExists.status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            AppError::SystemRoleProtected("x".into()).status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::DuplicateSlug("x".into()).status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            AppError::CircularGroupHierarchy.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::ServiceAccountNotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::ServiceAccountInactive.status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::SocialAuthFailed("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::SocialAuthConflict.status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            AppError::SocialAuthNoEmail.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::SocialAuthDeactivated.status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::UnsupportedGrantType("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::ApprovalRequired {
                request_id: "x".into()
            }
            .status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::ExternalTokenInvalid("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::ExternalProviderNotConfigured("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn error_codes_unique() {
        let codes = vec![
            AppError::BadRequest("".into()).error_code(),
            AppError::Unauthorized("".into()).error_code(),
            AppError::Forbidden("".into()).error_code(),
            AppError::NotFound("".into()).error_code(),
            AppError::Conflict("".into()).error_code(),
            AppError::RateLimited.error_code(),
            AppError::Internal("".into()).error_code(),
            AppError::ValidationError("".into()).error_code(),
            AppError::AuthenticationFailed("".into()).error_code(),
            AppError::TokenExpired.error_code(),
            AppError::MfaRequired {
                session_token: "".into(),
            }
            .error_code(),
            AppError::PkceVerificationFailed.error_code(),
            AppError::InvalidRedirectUri.error_code(),
            AppError::InvalidScope("".into()).error_code(),
            AppError::RoleNotFound("".into()).error_code(),
            AppError::GroupNotFound("".into()).error_code(),
            AppError::ConsentNotFound.error_code(),
            AppError::RoleAlreadyAssigned.error_code(),
            AppError::GroupMembershipExists.error_code(),
            AppError::SystemRoleProtected("".into()).error_code(),
            AppError::DuplicateSlug("".into()).error_code(),
            AppError::CircularGroupHierarchy.error_code(),
            AppError::ServiceAccountNotFound("".into()).error_code(),
            AppError::ServiceAccountInactive.error_code(),
            AppError::SocialAuthFailed("".into()).error_code(),
            AppError::SocialAuthConflict.error_code(),
            AppError::SocialAuthNoEmail.error_code(),
            AppError::SocialAuthDeactivated.error_code(),
            AppError::UnsupportedGrantType("".into()).error_code(),
            AppError::ApprovalRequired {
                request_id: "".into(),
            }
            .error_code(),
            AppError::ExternalTokenInvalid("".into()).error_code(),
            AppError::ExternalProviderNotConfigured("".into()).error_code(),
        ];
        let unique: std::collections::HashSet<u32> = codes.iter().copied().collect();
        assert_eq!(
            codes.len(),
            unique.len(),
            "All error codes should be unique"
        );
    }

    #[test]
    fn error_keys() {
        assert_eq!(AppError::BadRequest("".into()).error_key(), "bad_request");
        assert_eq!(
            AppError::Unauthorized("".into()).error_key(),
            "unauthorized"
        );
        assert_eq!(AppError::Forbidden("".into()).error_key(), "forbidden");
        assert_eq!(AppError::NotFound("".into()).error_key(), "not_found");
        assert_eq!(AppError::Conflict("".into()).error_key(), "conflict");
        assert_eq!(AppError::RateLimited.error_key(), "rate_limited");
        assert_eq!(AppError::Internal("".into()).error_key(), "internal_error");
        assert_eq!(
            AppError::ValidationError("".into()).error_key(),
            "validation_error"
        );
        assert_eq!(
            AppError::AuthenticationFailed("".into()).error_key(),
            "authentication_failed"
        );
        assert_eq!(AppError::TokenExpired.error_key(), "token_expired");
        assert_eq!(
            AppError::MfaRequired {
                session_token: "".into()
            }
            .error_key(),
            "mfa_required"
        );
        assert_eq!(
            AppError::PkceVerificationFailed.error_key(),
            "pkce_verification_failed"
        );
        assert_eq!(
            AppError::InvalidRedirectUri.error_key(),
            "invalid_redirect_uri"
        );
        assert_eq!(
            AppError::InvalidScope("".into()).error_key(),
            "invalid_scope"
        );
        assert_eq!(
            AppError::RoleNotFound("".into()).error_key(),
            "role_not_found"
        );
        assert_eq!(
            AppError::GroupNotFound("".into()).error_key(),
            "group_not_found"
        );
        assert_eq!(AppError::ConsentNotFound.error_key(), "consent_not_found");
        assert_eq!(
            AppError::RoleAlreadyAssigned.error_key(),
            "role_already_assigned"
        );
        assert_eq!(
            AppError::GroupMembershipExists.error_key(),
            "group_membership_exists"
        );
        assert_eq!(
            AppError::SystemRoleProtected("".into()).error_key(),
            "system_role_protected"
        );
        assert_eq!(
            AppError::DuplicateSlug("".into()).error_key(),
            "duplicate_slug"
        );
        assert_eq!(
            AppError::CircularGroupHierarchy.error_key(),
            "circular_group_hierarchy"
        );
        assert_eq!(
            AppError::ServiceAccountNotFound("".into()).error_key(),
            "service_account_not_found"
        );
        assert_eq!(
            AppError::ServiceAccountInactive.error_key(),
            "service_account_inactive"
        );
        assert_eq!(
            AppError::SocialAuthFailed("".into()).error_key(),
            "social_auth_failed"
        );
        assert_eq!(
            AppError::SocialAuthConflict.error_key(),
            "social_auth_conflict"
        );
        assert_eq!(
            AppError::SocialAuthNoEmail.error_key(),
            "social_auth_no_email"
        );
        assert_eq!(
            AppError::SocialAuthDeactivated.error_key(),
            "social_auth_deactivated"
        );
        assert_eq!(
            AppError::UnsupportedGrantType("".into()).error_key(),
            "unsupported_grant_type"
        );
        assert_eq!(
            AppError::ApprovalRequired {
                request_id: "".into()
            }
            .error_key(),
            "approval_required"
        );
        assert_eq!(
            AppError::ExternalTokenInvalid("".into()).error_key(),
            "external_token_invalid"
        );
        assert_eq!(
            AppError::ExternalProviderNotConfigured("".into()).error_key(),
            "external_provider_not_configured"
        );
    }

    #[test]
    fn oauth_error_codes() {
        assert_eq!(
            AppError::UnsupportedGrantType("x".into()).oauth_error_code(),
            "unsupported_grant_type"
        );
        assert_eq!(
            AppError::PkceVerificationFailed.oauth_error_code(),
            "invalid_grant"
        );
        assert_eq!(
            AppError::InvalidRedirectUri.oauth_error_code(),
            "invalid_grant"
        );
        assert_eq!(
            AppError::InvalidScope("x".into()).oauth_error_code(),
            "invalid_scope"
        );
        assert_eq!(
            AppError::Unauthorized("x".into()).oauth_error_code(),
            "invalid_client"
        );
        assert_eq!(
            AppError::AuthenticationFailed("x".into()).oauth_error_code(),
            "invalid_client"
        );
        assert_eq!(
            AppError::ServiceAccountNotFound("x".into()).oauth_error_code(),
            "invalid_client"
        );
        assert_eq!(
            AppError::ServiceAccountInactive.oauth_error_code(),
            "invalid_client"
        );
        assert_eq!(
            AppError::NotFound("x".into()).oauth_error_code(),
            "invalid_grant"
        );
        assert_eq!(
            AppError::BadRequest("x".into()).oauth_error_code(),
            "invalid_request"
        );
        assert_eq!(
            AppError::ConsentRequired {
                consent_url: "x".into()
            }
            .oauth_error_code(),
            "consent_required"
        );
        assert_eq!(
            AppError::ExternalTokenInvalid("x".into()).oauth_error_code(),
            "invalid_grant"
        );
        assert_eq!(
            AppError::ExternalProviderNotConfigured("x".into()).oauth_error_code(),
            "invalid_request"
        );
    }

    #[test]
    fn display_messages() {
        assert_eq!(
            format!("{}", AppError::BadRequest("oops".into())),
            "Bad request: oops"
        );
        assert_eq!(format!("{}", AppError::TokenExpired), "Token expired");
        assert_eq!(format!("{}", AppError::RateLimited), "Rate limited");
        assert_eq!(
            format!("{}", AppError::PkceVerificationFailed),
            "PKCE verification failed"
        );
        assert_eq!(
            format!("{}", AppError::InvalidRedirectUri),
            "Invalid redirect URI"
        );
        assert_eq!(
            format!(
                "{}",
                AppError::MfaRequired {
                    session_token: "t".into()
                }
            ),
            "MFA required"
        );
    }

    #[test]
    fn error_response_serialization() {
        let resp = ErrorResponse {
            error: "bad_request".to_string(),
            error_code: 1000,
            message: "Invalid input".to_string(),
            session_token: None,
            consent_url: None,
            request_id: None,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["error"], "bad_request");
        assert_eq!(json["error_code"], 1000);
        assert!(json.get("session_token").is_none());
        assert!(json.get("consent_url").is_none());
        assert!(json.get("request_id").is_none());
    }

    #[test]
    fn error_response_with_session_token() {
        let resp = ErrorResponse {
            error: "mfa_required".to_string(),
            error_code: 2002,
            message: "MFA verification required".to_string(),
            session_token: Some("mfa-session-tok".to_string()),
            consent_url: None,
            request_id: None,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["session_token"], "mfa-session-tok");
    }
}
