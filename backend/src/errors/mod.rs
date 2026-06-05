use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// Structured JSON error response returned by all API error paths.
#[derive(Debug, Serialize, ToSchema)]
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
    /// Approval request ID (approval_required / approval_failed only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// URL where the user can review pending approvals (approval_failed only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approve_url: Option<String>,
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

    #[error("Social auth conflict: social identity already linked to another account")]
    SocialAuthConflict,

    #[error("Social auth: no verified email from provider")]
    SocialAuthNoEmail,

    #[error("Social auth: account is deactivated")]
    SocialAuthDeactivated,

    #[error("Social auth: registration closed — invite code required")]
    SocialAuthRegistrationClosed,

    #[error("Email signup is disabled on this instance")]
    EmailSignupDisabled,

    #[error("SSH node key missing: {0}")]
    SshNodeKeyMissing(String),

    #[error("SSH host key mismatch: {0}")]
    SshHostKeyMismatch(String),

    #[error("SSH node exec channel closed: {0}")]
    SshNodeExecChannelClosed(String),

    #[error("SSH principal ambiguous: {0}")]
    SshPrincipalAmbiguous(String),

    #[error("SSH auth mode unsupported for operation: {0}")]
    SshAuthModeUnsupportedForOperation(String),

    #[error("Consent required")]
    ConsentRequired { consent_url: String },

    #[error("Unsupported grant type: {0}")]
    UnsupportedGrantType(String),

    #[error("Approval required")]
    ApprovalRequired { request_id: String },

    #[error("Approval failed: {reason}")]
    ApprovalFailed {
        request_id: String,
        approve_url: String,
        reason: String,
    },

    #[error("External token verification failed: {0}")]
    ExternalTokenInvalid(String),

    #[error("External provider not configured: {0}")]
    ExternalProviderNotConfigured(String),

    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("Node offline: {0}")]
    NodeOffline(String),

    #[error("Node proxy timeout")]
    NodeProxyTimeout,

    #[error("Node registration failed: {0}")]
    NodeRegistrationFailed(String),

    #[error("Node credential missing: {0}")]
    NodeCredentialMissing(String),

    #[error("WebSocket proxy downstream error: {0}")]
    WsProxyDownstream(String),

    #[error("API key scope forbidden: {0}")]
    ApiKeyScopeForbidden(String),

    #[error("API key scope inactive")]
    ApiKeyScopeInactive,

    #[error("API key scope not found: {0}")]
    ApiKeyScopeNotFound(String),

    #[error("Device code not found")]
    DeviceCodeNotFound,

    #[error("Device code expired")]
    DeviceCodeExpired,

    #[error("Device poll signature invalid: {0}")]
    DevicePollSignatureInvalid(String),

    #[error("Device user code invalid")]
    DeviceUserCodeInvalid,

    #[error("Device code pending")]
    DeviceCodePending,

    #[error("Device code already delivered")]
    DeviceCodeAlreadyDelivered,

    #[error("Device code rate limited")]
    DeviceCodeRateLimited,

    #[error("Device code locked")]
    DeviceCodeLocked,

    #[error("Device code poll interval must slow down")]
    DeviceCodeSlowDown,

    #[error("Channel bot not found: {0}")]
    ChannelBotNotFound(String),

    #[error("Channel bot inactive or invalid: {0}")]
    ChannelBotInactive(String),

    #[error("Channel bot limit reached: {0}")]
    ChannelBotLimitReached(String),

    #[error("Channel webhook verification failed: {0}")]
    ChannelWebhookVerificationFailed(String),

    #[error("Channel relay failed: {0}")]
    ChannelRelayFailed(String),

    #[error("Channel platform error: {0}")]
    ChannelPlatformError(String),

    #[error("Channel platform does not support message edits")]
    ChannelPlatformEditUnsupported,

    #[error("Device channel conversations do not support replies")]
    DeviceChannelReplyNotAllowed,

    #[error("Organization accounts cannot authenticate directly")]
    OrgCannotAuthenticate,

    #[error("Organization membership query timed out")]
    OrgQueryTimeout,

    #[error("Organization not found: {0}")]
    OrgNotFound(String),

    #[error("Organization slug is already taken: {0}")]
    OrgSlugTaken(String),

    #[error("Organization membership required")]
    OrgMembershipRequired,

    #[error("Organization role insufficient: {0}")]
    OrgRoleInsufficient(String),

    #[error("Organization invite invalid: {0}")]
    OrgInviteInvalid(String),

    #[error("Organization invite expired")]
    OrgInviteExpired,

    #[error("Organization approval policy has no admins to decide: {0}")]
    OrgApprovalNoAdmin(String),

    #[error("Invalid invite code")]
    InviteCodeInvalid,

    #[error("Invite code has been used up")]
    InviteCodeExhausted,

    #[error("Invite code has been deactivated")]
    InviteCodeDeactivated,

    #[error("Invite code has already been redeemed")]
    InviteCodeAlreadyRedeemed,
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
            Self::SocialAuthRegistrationClosed => StatusCode::FORBIDDEN,
            Self::EmailSignupDisabled => StatusCode::FORBIDDEN,
            Self::SshNodeKeyMissing(_) => StatusCode::NOT_FOUND,
            Self::SshHostKeyMismatch(_) => StatusCode::BAD_GATEWAY,
            Self::SshNodeExecChannelClosed(_) => StatusCode::BAD_GATEWAY,
            Self::SshPrincipalAmbiguous(_) | Self::SshAuthModeUnsupportedForOperation(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::ConsentRequired { .. } => StatusCode::FORBIDDEN,
            Self::UnsupportedGrantType(_) => StatusCode::BAD_REQUEST,
            Self::ApprovalRequired { .. } => StatusCode::FORBIDDEN,
            Self::ApprovalFailed { .. } => StatusCode::FORBIDDEN,
            Self::ExternalTokenInvalid(_) | Self::ExternalProviderNotConfigured(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::NodeNotFound(_) => StatusCode::NOT_FOUND,
            Self::NodeOffline(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::NodeProxyTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::NodeRegistrationFailed(_) => StatusCode::BAD_REQUEST,
            Self::NodeCredentialMissing(_) => StatusCode::BAD_GATEWAY,
            Self::WsProxyDownstream(_) => StatusCode::BAD_GATEWAY,
            Self::ApiKeyScopeForbidden(_) => StatusCode::FORBIDDEN,
            Self::ApiKeyScopeInactive => StatusCode::FORBIDDEN,
            Self::ApiKeyScopeNotFound(_) => StatusCode::NOT_FOUND,
            Self::DeviceCodeNotFound => StatusCode::BAD_REQUEST,
            Self::DeviceCodeExpired => StatusCode::GONE,
            Self::DevicePollSignatureInvalid(_) => StatusCode::FORBIDDEN,
            Self::DeviceUserCodeInvalid => StatusCode::BAD_REQUEST,
            Self::DeviceCodePending => StatusCode::BAD_REQUEST,
            Self::DeviceCodeAlreadyDelivered => StatusCode::GONE,
            Self::DeviceCodeRateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::DeviceCodeLocked => StatusCode::TOO_MANY_REQUESTS,
            Self::DeviceCodeSlowDown => StatusCode::TOO_MANY_REQUESTS,
            Self::ChannelBotNotFound(_) => StatusCode::NOT_FOUND,
            Self::ChannelBotInactive(_) => StatusCode::BAD_REQUEST,
            Self::ChannelBotLimitReached(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::ChannelWebhookVerificationFailed(_) => StatusCode::UNAUTHORIZED,
            Self::ChannelRelayFailed(_) => StatusCode::BAD_GATEWAY,
            Self::ChannelPlatformError(_) => StatusCode::BAD_GATEWAY,
            Self::ChannelPlatformEditUnsupported => StatusCode::NOT_IMPLEMENTED,
            Self::DeviceChannelReplyNotAllowed => StatusCode::BAD_REQUEST,
            Self::OrgCannotAuthenticate => StatusCode::FORBIDDEN,
            Self::OrgQueryTimeout => StatusCode::SERVICE_UNAVAILABLE,
            Self::OrgNotFound(_) => StatusCode::NOT_FOUND,
            Self::OrgSlugTaken(_) => StatusCode::CONFLICT,
            Self::OrgMembershipRequired => StatusCode::FORBIDDEN,
            Self::OrgRoleInsufficient(_) => StatusCode::FORBIDDEN,
            Self::OrgInviteInvalid(_) => StatusCode::BAD_REQUEST,
            Self::OrgInviteExpired => StatusCode::GONE,
            Self::OrgApprovalNoAdmin(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::InviteCodeInvalid
            | Self::InviteCodeExhausted
            | Self::InviteCodeDeactivated
            | Self::InviteCodeAlreadyRedeemed => StatusCode::BAD_REQUEST,
            Self::Internal(_) | Self::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub(crate) fn error_code(&self) -> u32 {
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
            Self::EmailSignupDisabled => 1009,
            Self::SshNodeKeyMissing(_) => 1011,
            Self::SshHostKeyMismatch(_) => 1012,
            Self::SshNodeExecChannelClosed(_) => 1013,
            Self::SshPrincipalAmbiguous(_) => 1014,
            Self::SshAuthModeUnsupportedForOperation(_) => 1015,
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
            Self::SocialAuthRegistrationClosed => 6006,
            Self::ConsentRequired { .. } => 3003,
            Self::UnsupportedGrantType(_) => 3004,
            Self::ApprovalRequired { .. } => 7000,
            Self::ApprovalFailed { .. } => 7001,
            Self::ExternalTokenInvalid(_) => 6004,
            Self::ExternalProviderNotConfigured(_) => 6005,
            Self::NodeNotFound(_) => 8000,
            Self::NodeOffline(_) => 8001,
            Self::NodeProxyTimeout => 8002,
            Self::NodeRegistrationFailed(_) => 8003,
            Self::NodeCredentialMissing(_) => 8004,
            Self::WsProxyDownstream(_) => 8005,
            Self::ApiKeyScopeForbidden(_) => 9000,
            Self::ApiKeyScopeInactive => 9001,
            Self::ApiKeyScopeNotFound(_) => 9002,
            Self::DeviceCodeNotFound => 9500,
            Self::DeviceCodeExpired => 9501,
            Self::DevicePollSignatureInvalid(_) => 9502,
            Self::DeviceUserCodeInvalid => 9503,
            Self::DeviceCodePending => 9504,
            Self::DeviceCodeAlreadyDelivered => 9505,
            Self::DeviceCodeRateLimited => 9506,
            Self::DeviceCodeLocked => 9507,
            Self::DeviceCodeSlowDown => 9508,
            Self::ChannelBotNotFound(_) => 10000,
            Self::ChannelBotInactive(_) => 10001,
            Self::ChannelBotLimitReached(_) => 10002,
            Self::ChannelWebhookVerificationFailed(_) => 10003,
            Self::ChannelRelayFailed(_) => 10004,
            Self::ChannelPlatformError(_) => 10005,
            Self::ChannelPlatformEditUnsupported => 10007,
            Self::DeviceChannelReplyNotAllowed => 10006,
            Self::OrgCannotAuthenticate => 1403,
            Self::OrgQueryTimeout => 8100,
            Self::OrgNotFound(_) => 8101,
            Self::OrgSlugTaken(_) => 8107,
            Self::OrgMembershipRequired => 8102,
            Self::OrgRoleInsufficient(_) => 8103,
            Self::OrgInviteInvalid(_) => 8104,
            Self::OrgInviteExpired => 8105,
            Self::OrgApprovalNoAdmin(_) => 8106,
            Self::InviteCodeInvalid => 8200,
            Self::InviteCodeExhausted => 8201,
            Self::InviteCodeDeactivated => 8202,
            Self::InviteCodeAlreadyRedeemed => 8203,
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
            Self::EmailSignupDisabled => "email_signup_disabled",
            Self::SshNodeKeyMissing(_) => "ssh_node_key_missing",
            Self::SshHostKeyMismatch(_) => "ssh_host_key_mismatch",
            Self::SshNodeExecChannelClosed(_) => "ssh_node_exec_channel_closed",
            Self::SshPrincipalAmbiguous(_) => "ssh_principal_ambiguous",
            Self::SshAuthModeUnsupportedForOperation(_) => {
                "ssh_auth_mode_unsupported_for_operation"
            }
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
            Self::SocialAuthRegistrationClosed => "social_auth_registration_closed",
            Self::ConsentRequired { .. } => "consent_required",
            Self::UnsupportedGrantType(_) => "unsupported_grant_type",
            Self::ApprovalRequired { .. } => "approval_required",
            Self::ApprovalFailed { .. } => "approval_failed",
            Self::ExternalTokenInvalid(_) => "external_token_invalid",
            Self::ExternalProviderNotConfigured(_) => "external_provider_not_configured",
            Self::NodeNotFound(_) => "node_not_found",
            Self::NodeOffline(_) => "node_offline",
            Self::NodeProxyTimeout => "node_proxy_timeout",
            Self::NodeRegistrationFailed(_) => "node_registration_failed",
            Self::NodeCredentialMissing(_) => "node_credential_missing",
            Self::WsProxyDownstream(_) => "ws_proxy_downstream",
            Self::ApiKeyScopeForbidden(_) => "api_key_scope_forbidden",
            Self::ApiKeyScopeInactive => "api_key_scope_inactive",
            Self::ApiKeyScopeNotFound(_) => "api_key_scope_not_found",
            Self::DeviceCodeNotFound => "device_code_not_found",
            Self::DeviceCodeExpired => "device_code_expired",
            Self::DevicePollSignatureInvalid(_) => "device_poll_signature_invalid",
            Self::DeviceUserCodeInvalid => "device_user_code_invalid",
            Self::DeviceCodePending => "device_code_pending",
            Self::DeviceCodeAlreadyDelivered => "device_code_already_delivered",
            Self::DeviceCodeRateLimited => "device_code_rate_limited",
            Self::DeviceCodeLocked => "device_code_locked",
            Self::DeviceCodeSlowDown => "device_code_slow_down",
            Self::ChannelBotNotFound(_) => "channel_bot_not_found",
            Self::ChannelBotInactive(_) => "channel_bot_inactive",
            Self::ChannelBotLimitReached(_) => "channel_bot_limit_reached",
            Self::ChannelWebhookVerificationFailed(_) => "channel_webhook_verification_failed",
            Self::ChannelRelayFailed(_) => "channel_relay_failed",
            Self::ChannelPlatformError(_) => "channel_platform_error",
            Self::ChannelPlatformEditUnsupported => "edit_unsupported",
            Self::DeviceChannelReplyNotAllowed => "device_channel_reply_not_allowed",
            Self::OrgCannotAuthenticate => "org_cannot_authenticate",
            Self::OrgQueryTimeout => "org_query_timeout",
            Self::OrgNotFound(_) => "org_not_found",
            Self::OrgSlugTaken(_) => "org_slug_taken",
            Self::OrgMembershipRequired => "org_membership_required",
            Self::OrgRoleInsufficient(_) => "org_role_insufficient",
            Self::OrgInviteInvalid(_) => "org_invite_invalid",
            Self::OrgInviteExpired => "org_invite_expired",
            Self::OrgApprovalNoAdmin(_) => "org_approval_no_admin",
            Self::InviteCodeInvalid => "invite_code_invalid",
            Self::InviteCodeExhausted => "invite_code_exhausted",
            Self::InviteCodeDeactivated => "invite_code_deactivated",
            Self::InviteCodeAlreadyRedeemed => "invite_code_already_redeemed",
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
            AppError::ApprovalFailed { request_id, .. } => Some(request_id.clone()),
            _ => None,
        };
        let approve_url = match &self {
            AppError::ApprovalFailed { approve_url, .. } => Some(approve_url.clone()),
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
                AppError::ApprovalFailed {
                    reason,
                    approve_url,
                    ..
                } => {
                    format!("Approval failed: {reason}. Review pending approvals at {approve_url}")
                }
                other => other.to_string(),
            },
            session_token: mfa_session_token,
            consent_url,
            request_id: approval_request_id,
            approve_url,
        };

        (status, axum::Json(body)).into_response()
    }
}

/// Convenience type alias for handler return types.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;

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
            AppError::SocialAuthRegistrationClosed.status_code(),
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
            AppError::ApprovalFailed {
                request_id: "x".into(),
                approve_url: "https://example.com/approvals".into(),
                reason: "rejected".into(),
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
        assert_eq!(
            AppError::NodeNotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::NodeOffline("x".into()).status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            AppError::NodeProxyTimeout.status_code(),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            AppError::NodeRegistrationFailed("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::NodeCredentialMissing("x".into()).status_code(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            AppError::WsProxyDownstream("x".into()).status_code(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            AppError::ApiKeyScopeForbidden("x".into()).status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::ApiKeyScopeInactive.status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::ApiKeyScopeNotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::DeviceCodeNotFound.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(AppError::DeviceCodeExpired.status_code(), StatusCode::GONE);
        assert_eq!(
            AppError::DevicePollSignatureInvalid("x".into()).status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            AppError::DeviceUserCodeInvalid.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::DeviceCodePending.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::DeviceCodeAlreadyDelivered.status_code(),
            StatusCode::GONE
        );
        assert_eq!(
            AppError::DeviceCodeRateLimited.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            AppError::DeviceCodeLocked.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            AppError::DeviceCodeSlowDown.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            AppError::ChannelBotNotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::ChannelBotInactive("x".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::ChannelBotLimitReached("x".into()).status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            AppError::ChannelWebhookVerificationFailed("x".into()).status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::ChannelRelayFailed("x".into()).status_code(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            AppError::ChannelPlatformError("x".into()).status_code(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            AppError::InviteCodeInvalid.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::InviteCodeExhausted.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::InviteCodeDeactivated.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::InviteCodeAlreadyRedeemed.status_code(),
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
            AppError::EmailSignupDisabled.error_code(),
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
            AppError::SocialAuthRegistrationClosed.error_code(),
            AppError::UnsupportedGrantType("".into()).error_code(),
            AppError::ApprovalRequired {
                request_id: "".into(),
            }
            .error_code(),
            AppError::ApprovalFailed {
                request_id: "".into(),
                approve_url: "".into(),
                reason: "".into(),
            }
            .error_code(),
            AppError::ExternalTokenInvalid("".into()).error_code(),
            AppError::ExternalProviderNotConfigured("".into()).error_code(),
            AppError::NodeNotFound("".into()).error_code(),
            AppError::NodeOffline("".into()).error_code(),
            AppError::NodeProxyTimeout.error_code(),
            AppError::NodeRegistrationFailed("".into()).error_code(),
            AppError::NodeCredentialMissing("".into()).error_code(),
            AppError::WsProxyDownstream("".into()).error_code(),
            AppError::ApiKeyScopeForbidden("".into()).error_code(),
            AppError::ApiKeyScopeInactive.error_code(),
            AppError::ApiKeyScopeNotFound("".into()).error_code(),
            AppError::DeviceCodeNotFound.error_code(),
            AppError::DeviceCodeExpired.error_code(),
            AppError::DevicePollSignatureInvalid("".into()).error_code(),
            AppError::DeviceUserCodeInvalid.error_code(),
            AppError::DeviceCodePending.error_code(),
            AppError::DeviceCodeAlreadyDelivered.error_code(),
            AppError::DeviceCodeRateLimited.error_code(),
            AppError::DeviceCodeLocked.error_code(),
            AppError::DeviceCodeSlowDown.error_code(),
            AppError::ChannelBotNotFound("".into()).error_code(),
            AppError::ChannelBotInactive("".into()).error_code(),
            AppError::ChannelBotLimitReached("".into()).error_code(),
            AppError::ChannelWebhookVerificationFailed("".into()).error_code(),
            AppError::ChannelRelayFailed("".into()).error_code(),
            AppError::ChannelPlatformError("".into()).error_code(),
            AppError::InviteCodeInvalid.error_code(),
            AppError::InviteCodeExhausted.error_code(),
            AppError::InviteCodeDeactivated.error_code(),
            AppError::InviteCodeAlreadyRedeemed.error_code(),
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
            AppError::EmailSignupDisabled.error_key(),
            "email_signup_disabled"
        );
        assert_eq!(AppError::EmailSignupDisabled.error_code(), 1009);
        assert_eq!(
            AppError::EmailSignupDisabled.status_code(),
            StatusCode::FORBIDDEN
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
            AppError::SocialAuthRegistrationClosed.error_key(),
            "social_auth_registration_closed"
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
            AppError::ApprovalFailed {
                request_id: "".into(),
                approve_url: "".into(),
                reason: "".into(),
            }
            .error_key(),
            "approval_failed"
        );
        assert_eq!(
            AppError::ExternalTokenInvalid("".into()).error_key(),
            "external_token_invalid"
        );
        assert_eq!(
            AppError::ExternalProviderNotConfigured("".into()).error_key(),
            "external_provider_not_configured"
        );
        assert_eq!(
            AppError::NodeNotFound("".into()).error_key(),
            "node_not_found"
        );
        assert_eq!(AppError::NodeOffline("".into()).error_key(), "node_offline");
        assert_eq!(AppError::NodeProxyTimeout.error_key(), "node_proxy_timeout");
        assert_eq!(
            AppError::NodeRegistrationFailed("".into()).error_key(),
            "node_registration_failed"
        );
        assert_eq!(
            AppError::NodeCredentialMissing("".into()).error_key(),
            "node_credential_missing"
        );
        assert_eq!(
            AppError::NodeCredentialMissing("".into()).error_code(),
            8004
        );
        assert_eq!(
            AppError::WsProxyDownstream("".into()).error_key(),
            "ws_proxy_downstream"
        );
        assert_eq!(AppError::WsProxyDownstream("".into()).error_code(), 8005);
        assert_eq!(
            AppError::ApiKeyScopeForbidden("".into()).error_key(),
            "api_key_scope_forbidden"
        );
        assert_eq!(
            AppError::ApiKeyScopeInactive.error_key(),
            "api_key_scope_inactive"
        );
        assert_eq!(
            AppError::ApiKeyScopeNotFound("".into()).error_key(),
            "api_key_scope_not_found"
        );
        assert_eq!(AppError::ApiKeyScopeForbidden("".into()).error_code(), 9000);
        assert_eq!(AppError::ApiKeyScopeInactive.error_code(), 9001);
        assert_eq!(AppError::ApiKeyScopeNotFound("".into()).error_code(), 9002);
        assert_eq!(
            AppError::DeviceCodeNotFound.error_key(),
            "device_code_not_found"
        );
        assert_eq!(AppError::DeviceCodeNotFound.error_code(), 9500);
        assert_eq!(
            AppError::DeviceCodeExpired.error_key(),
            "device_code_expired"
        );
        assert_eq!(AppError::DeviceCodeExpired.error_code(), 9501);
        assert_eq!(
            AppError::DevicePollSignatureInvalid("".into()).error_key(),
            "device_poll_signature_invalid"
        );
        assert_eq!(
            AppError::DevicePollSignatureInvalid("".into()).error_code(),
            9502
        );
        assert_eq!(
            AppError::DeviceUserCodeInvalid.error_key(),
            "device_user_code_invalid"
        );
        assert_eq!(AppError::DeviceUserCodeInvalid.error_code(), 9503);
        assert_eq!(
            AppError::DeviceCodePending.error_key(),
            "device_code_pending"
        );
        assert_eq!(AppError::DeviceCodePending.error_code(), 9504);
        assert_eq!(
            AppError::DeviceCodeAlreadyDelivered.error_key(),
            "device_code_already_delivered"
        );
        assert_eq!(AppError::DeviceCodeAlreadyDelivered.error_code(), 9505);
        assert_eq!(
            AppError::DeviceCodeRateLimited.error_key(),
            "device_code_rate_limited"
        );
        assert_eq!(AppError::DeviceCodeRateLimited.error_code(), 9506);
        assert_eq!(AppError::DeviceCodeLocked.error_key(), "device_code_locked");
        assert_eq!(AppError::DeviceCodeLocked.error_code(), 9507);
        assert_eq!(
            AppError::DeviceCodeSlowDown.error_key(),
            "device_code_slow_down"
        );
        assert_eq!(AppError::DeviceCodeSlowDown.error_code(), 9508);
        assert_eq!(
            AppError::ChannelBotNotFound("".into()).error_key(),
            "channel_bot_not_found"
        );
        assert_eq!(
            AppError::ChannelBotInactive("".into()).error_key(),
            "channel_bot_inactive"
        );
        assert_eq!(
            AppError::ChannelBotLimitReached("".into()).error_key(),
            "channel_bot_limit_reached"
        );
        assert_eq!(
            AppError::ChannelWebhookVerificationFailed("".into()).error_key(),
            "channel_webhook_verification_failed"
        );
        assert_eq!(
            AppError::ChannelRelayFailed("".into()).error_key(),
            "channel_relay_failed"
        );
        assert_eq!(
            AppError::ChannelPlatformError("".into()).error_key(),
            "channel_platform_error"
        );
        assert_eq!(
            AppError::InviteCodeInvalid.error_key(),
            "invite_code_invalid"
        );
        assert_eq!(AppError::InviteCodeInvalid.error_code(), 8200);
        assert_eq!(
            AppError::InviteCodeExhausted.error_key(),
            "invite_code_exhausted"
        );
        assert_eq!(AppError::InviteCodeExhausted.error_code(), 8201);
        assert_eq!(
            AppError::InviteCodeDeactivated.error_key(),
            "invite_code_deactivated"
        );
        assert_eq!(AppError::InviteCodeDeactivated.error_code(), 8202);
        assert_eq!(
            format!("{}", AppError::InviteCodeInvalid),
            "Invalid invite code"
        );
        assert_eq!(
            format!("{}", AppError::InviteCodeExhausted),
            "Invite code has been used up"
        );
        assert_eq!(
            format!("{}", AppError::InviteCodeDeactivated),
            "Invite code has been deactivated"
        );
        assert_eq!(
            AppError::InviteCodeAlreadyRedeemed.error_key(),
            "invite_code_already_redeemed"
        );
        assert_eq!(AppError::InviteCodeAlreadyRedeemed.error_code(), 8203);
        assert_eq!(
            format!("{}", AppError::InviteCodeAlreadyRedeemed),
            "Invite code has already been redeemed"
        );
    }

    #[tokio::test]
    async fn ws_proxy_downstream_response_includes_node_reason() {
        let response = AppError::WsProxyDownstream("downstream rejected handshake".to_string())
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");

        assert_eq!(json["error"], "ws_proxy_downstream");
        assert_eq!(json["error_code"], 8005);
        assert_eq!(
            json["message"],
            "WebSocket proxy downstream error: downstream rejected handshake"
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
            approve_url: None,
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
            approve_url: None,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["session_token"], "mfa-session-tok");
    }

    #[test]
    fn newer_variant_status_key_and_code_mappings() {
        let cases = vec![
            (
                AppError::SshNodeKeyMissing("missing".into()),
                StatusCode::NOT_FOUND,
                "ssh_node_key_missing",
                1011,
            ),
            (
                AppError::SshHostKeyMismatch("bad host".into()),
                StatusCode::BAD_GATEWAY,
                "ssh_host_key_mismatch",
                1012,
            ),
            (
                AppError::SshNodeExecChannelClosed("closed".into()),
                StatusCode::BAD_GATEWAY,
                "ssh_node_exec_channel_closed",
                1013,
            ),
            (
                AppError::SshPrincipalAmbiguous("alice".into()),
                StatusCode::BAD_REQUEST,
                "ssh_principal_ambiguous",
                1014,
            ),
            (
                AppError::SshAuthModeUnsupportedForOperation("node_key".into()),
                StatusCode::BAD_REQUEST,
                "ssh_auth_mode_unsupported_for_operation",
                1015,
            ),
            (
                AppError::ChannelPlatformEditUnsupported,
                StatusCode::NOT_IMPLEMENTED,
                "edit_unsupported",
                10007,
            ),
            (
                AppError::DeviceChannelReplyNotAllowed,
                StatusCode::BAD_REQUEST,
                "device_channel_reply_not_allowed",
                10006,
            ),
            (
                AppError::OrgCannotAuthenticate,
                StatusCode::FORBIDDEN,
                "org_cannot_authenticate",
                1403,
            ),
            (
                AppError::OrgQueryTimeout,
                StatusCode::SERVICE_UNAVAILABLE,
                "org_query_timeout",
                8100,
            ),
            (
                AppError::OrgNotFound("org".into()),
                StatusCode::NOT_FOUND,
                "org_not_found",
                8101,
            ),
            (
                AppError::OrgSlugTaken("slug".into()),
                StatusCode::CONFLICT,
                "org_slug_taken",
                8107,
            ),
            (
                AppError::OrgMembershipRequired,
                StatusCode::FORBIDDEN,
                "org_membership_required",
                8102,
            ),
            (
                AppError::OrgRoleInsufficient("admin".into()),
                StatusCode::FORBIDDEN,
                "org_role_insufficient",
                8103,
            ),
            (
                AppError::OrgInviteInvalid("bad".into()),
                StatusCode::BAD_REQUEST,
                "org_invite_invalid",
                8104,
            ),
            (
                AppError::OrgInviteExpired,
                StatusCode::GONE,
                "org_invite_expired",
                8105,
            ),
            (
                AppError::OrgApprovalNoAdmin("org".into()),
                StatusCode::SERVICE_UNAVAILABLE,
                "org_approval_no_admin",
                8106,
            ),
        ];

        for (err, status, key, code) in cases {
            assert_eq!(err.status_code(), status, "{err}");
            assert_eq!(err.error_key(), key, "{err}");
            assert_eq!(err.error_code(), code, "{err}");
        }
    }

    #[test]
    fn oauth_status_mappings() {
        assert_eq!(
            AppError::Unauthorized("bad client".into()).oauth_status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::ServiceAccountInactive.oauth_status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::Internal("boom".into()).oauth_status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            AppError::InvalidScope("bad".into()).oauth_status(),
            StatusCode::BAD_REQUEST
        );
    }

    async fn response_json(err: AppError) -> (StatusCode, Value) {
        let response = err.into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let json = serde_json::from_slice(&body).expect("json response");
        (status, json)
    }

    #[tokio::test]
    async fn internal_response_redacts_internal_details() {
        let (status, json) =
            response_json(AppError::Internal("database password leaked".into())).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"], "internal_error");
        assert_eq!(json["error_code"], 1006);
        assert_eq!(json["message"], "An internal error occurred");
    }

    #[tokio::test]
    async fn special_response_fields_are_serialized_for_mfa_consent_and_approval() {
        let (status, json) = response_json(AppError::MfaRequired {
            session_token: "mfa-token".into(),
        })
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "mfa_required");
        assert_eq!(json["session_token"], "mfa-token");

        let (status, json) = response_json(AppError::ConsentRequired {
            consent_url: "https://app.example.com/consent".into(),
        })
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "consent_required");
        assert_eq!(json["consent_url"], "https://app.example.com/consent");

        let (status, json) = response_json(AppError::ApprovalFailed {
            request_id: "approval-1".into(),
            approve_url: "https://app.example.com/approvals".into(),
            reason: "rejected".into(),
        })
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "approval_failed");
        assert_eq!(json["request_id"], "approval-1");
        assert_eq!(json["approve_url"], "https://app.example.com/approvals");
        assert_eq!(
            json["message"],
            "Approval failed: rejected. Review pending approvals at https://app.example.com/approvals"
        );
    }
}
