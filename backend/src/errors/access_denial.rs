//! Fixed, metadata-only recovery guidance. Never carry a caller or resource ID.
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    ApiKey,
    Delegated,
    Relay,
    ServiceAccount,
    OauthClient,
}

pub const HUMAN_HINT: &str = "Ask a human to run `nyxid login` and choose account access, or use an existing human account profile. Do not switch identity automatically.";
pub const SCOPE_HINT: &str = "Ask the owner to review the required scopes and grant only the access needed. Do not switch identity or widen permissions automatically.";

#[derive(Debug, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum AccessDenial {
    CredentialTypeUnsupported {
        credential_type: CredentialType,
        /// Supported recovery credential, not an exhaustive route ACL.
        accepted: [&'static str; 1],
        hint: &'static str,
    },
    InsufficientScope {
        hint: &'static str,
    },
}

impl super::AppError {
    pub fn unsupported_credential(message: &str, credential_type: CredentialType) -> Self {
        Self::ForbiddenWithGuidance {
            message: message.into(),
            guidance: Box::new(AccessDenial::CredentialTypeUnsupported {
                credential_type,
                accepted: ["user_session"],
                hint: HUMAN_HINT,
            }),
        }
    }

    pub fn insufficient_scope(message: impl Into<String>) -> Self {
        Self::ForbiddenWithGuidance {
            message: message.into(),
            guidance: Box::new(AccessDenial::InsufficientScope { hint: SCOPE_HINT }),
        }
    }
}
