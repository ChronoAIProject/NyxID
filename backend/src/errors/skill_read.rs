//! Closed metadata for bound skill reads. Never retain Ornn response prose,
//! package bytes, connection credentials or signed identities in a refusal.
use super::AppError;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillReadReason {
    SkillNotBound,
    OwnerConnectionUnavailable,
    OrnnPrincipalDenied,
    PinnedVersionMismatch,
}

impl SkillReadReason {
    pub fn legacy_error(self) -> AppError {
        match self {
            Self::SkillNotBound | Self::OrnnPrincipalDenied => AppError::Forbidden(
                "Skill unavailable or not visible through your Ornn access".into(),
            ),
            Self::OwnerConnectionUnavailable => {
                AppError::Forbidden("No usable Ornn connection for this agent's owner".into())
            }
            Self::PinnedVersionMismatch => AppError::ValidationError(
                "Pinned skill version or SHA-256 mismatch; content refused".into(),
            ),
        }
    }

    pub fn error_key(self) -> &'static str {
        match self {
            Self::PinnedVersionMismatch => "validation_error",
            _ => "forbidden",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_read_refusals_have_only_fixed_metadata_and_legacy_classes() {
        for (reason, code) in [
            (SkillReadReason::SkillNotBound, "skill_not_bound"),
            (
                SkillReadReason::OwnerConnectionUnavailable,
                "owner_connection_unavailable",
            ),
            (
                SkillReadReason::OrnnPrincipalDenied,
                "ornn_principal_denied",
            ),
            (
                SkillReadReason::PinnedVersionMismatch,
                "pinned_version_mismatch",
            ),
        ] {
            let error = AppError::SkillRead(reason);
            let body = serde_json::to_value(error.response_body()).unwrap();
            assert_eq!(body["details"], serde_json::json!({"reason":code}));
            assert_eq!(error.status_code(), reason.legacy_error().status_code());
            assert_eq!(error.error_code(), reason.legacy_error().error_code());
            assert_eq!(error.error_key(), reason.error_key());
            assert_eq!(
                body["message"],
                reason.legacy_error().response_body().message
            );
        }
    }
}
