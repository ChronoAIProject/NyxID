//! Owner-visible outcomes of a skill write. Only fixed diagnostics cross the
//! HTTP, MCP and activity boundaries.
use serde::Serialize;

#[derive(Debug, Serialize, thiserror::Error)]
#[error("Conflict: {reason}: {message}")]
pub struct SkillPublicationFailure {
    pub reason: &'static str,
    pub recovery: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
    #[serde(skip)]
    message: &'static str,
}

impl SkillPublicationFailure {
    pub fn ambiguous(status: Option<u16>) -> Self {
        Self {
            reason: "publication_ambiguous",
            recovery: "check",
            upstream_status: status,
            message: "The skill save was not confirmed. NyxID will not repeat the write. Use Check again on the existing review, if available, or check Ornn.",
        }
    }

    pub(crate) fn registry_unavailable(reason: &str, status: Option<u16>) -> Option<Self> {
        let status = status.filter(|status| (500..600).contains(status))?;
        let (reason, recovery, message) = match reason {
            "source_skill_unavailable" => (
                "source_skill_unavailable",
                "retry_later",
                "Ornn could not provide the source skill. NyxID did not save an update. Retry the draft when Ornn is available.",
            ),
            "ornn_unavailable" => (
                "ornn_unavailable",
                "retry_later",
                "Ornn was unavailable before publication. NyxID did not save the skill. Retry the same draft when Ornn is available.",
            ),
            "verify_failed" => (
                "verify_failed",
                "check",
                "NyxID could not verify the saved version or attach it. Check the existing version in Ornn.",
            ),
            _ => return None,
        };
        Some(Self {
            reason,
            recovery,
            upstream_status: Some(status),
            message,
        })
    }

    pub(crate) fn refused(reason: &'static str, recovery: &'static str, status: u16) -> Self {
        Self {
            reason,
            recovery,
            upstream_status: Some(status),
            message: match recovery {
                "revise_draft" => {
                    "Ornn refused the save. The skill was not saved. Review the draft and the reported reason."
                }
                _ => {
                    "Ornn refused the save. The skill was not saved. Check access to the skill in Ornn."
                }
            },
        }
    }

    pub fn activity(&self) -> crate::models::assistant_conversation::ToolFailure {
        crate::models::assistant_conversation::ToolFailure {
            kind: "tool_error".into(),
            status: self.upstream_status,
            reason: Some(self.reason.into()),
        }
    }
}

impl super::AppError {
    /// Keep the existing HTTP conflict for the mandatory 5xx checks. The
    /// native drafting tool exposes the actionable diagnostic for these cases.
    pub(crate) fn with_legacy_skill_registry_error(self) -> Self {
        match self {
            Self::SkillPublication(failure)
                if matches!(failure.reason, "ornn_unavailable" | "verify_failed") =>
            {
                Self::Conflict("ornn_unavailable".into())
            }
            error => error,
        }
    }
}
