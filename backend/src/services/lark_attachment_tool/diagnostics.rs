//! Metadata for one attachment tool call, shared by preparation, child effects,
//! activity and audit, plus typed Lark field-response validation. Unrelated
//! calls do not allocate a context or read the DB.
//! Only these closed codes (never AppError prose) cross the diagnostic boundary.
use super::eligibility;
use crate::{errors::AppError, models::lark_base_write::WriteState};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub(crate) enum Reason {
    Eligibility(eligibility::Reason),
    InvalidArguments,
    AttachmentUnreadable,
    AttachmentSize,
    AttachmentIncompleteSet,
    AttachmentIntegrity,
    FieldReadTransport,
    FieldReadProviderError,
    FieldReadInvalidResponse,
    FieldMissing,
    FieldNotAttachment,
    FieldValueInvalid,
    UploadTransport,
    UploadProviderError,
    UploadInvalidResponse,
    CredentialPreparation,
    AuthorityChanged,
    AdmissionRefused,
    Database,
    Internal,
    Ledger(WriteState),
    LedgerChanged,
    Timeout,
    CreateProviderError,
    CreateUncertain,
}
impl Reason {
    pub(crate) fn code(self) -> String {
        match self {
            Self::Eligibility(reason) => return format!("eligibility:{}", reason.code()),
            Self::Ledger(state) => {
                return format!(
                    "ledger:{}",
                    match state {
                        WriteState::Uploading => "uploading",
                        WriteState::Dispatching => "dispatching",
                        WriteState::Succeeded => "succeeded",
                        WriteState::FailedDefinite => "failed_definite",
                        WriteState::FailedPreDispatch => "failed_pre_dispatch",
                        WriteState::Uncertain => "uncertain",
                    }
                );
            }
            Self::InvalidArguments => "invalid_arguments",
            Self::AttachmentUnreadable => "attachment_unreadable",
            Self::AttachmentSize => "attachment_size",
            Self::AttachmentIncompleteSet => "attachment_incomplete_set",
            Self::AttachmentIntegrity => "attachment_integrity",
            Self::FieldReadTransport => "field_read_transport",
            Self::FieldReadProviderError => "field_read_provider_error",
            Self::FieldReadInvalidResponse => "field_read_invalid_response",
            Self::FieldMissing => "field_missing",
            Self::FieldNotAttachment => "field_not_attachment",
            Self::FieldValueInvalid => "field_value_invalid",
            Self::UploadTransport => "upload_transport",
            Self::UploadProviderError => "upload_provider_error",
            Self::UploadInvalidResponse => "upload_invalid_response",
            Self::CredentialPreparation => "credential_preparation",
            Self::AuthorityChanged => "authority_changed",
            Self::AdmissionRefused => "admission_refused",
            Self::Database => "database_unavailable",
            Self::Internal => "internal",
            Self::LedgerChanged => "ledger_changed",
            Self::Timeout => "timeout",
            Self::CreateProviderError => "create_provider_error",
            Self::CreateUncertain => "create_uncertain",
        }
        .into()
    }
    pub(crate) fn error(self, error: AppError) -> AppError {
        failure(self, None, None);
        error
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    NotSent,
    ProviderRefused,
    Succeeded,
    Uncertain,
}
impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::NotSent => "not_sent",
            Self::ProviderRefused => "provider_refused",
            Self::Succeeded => "succeeded",
            Self::Uncertain => "uncertain",
        }
    }
}
#[derive(Clone)]
pub(crate) struct Diagnostic {
    pub enabled: bool,
    pub service_id: Option<String>,
    pub reason: Option<Reason>,
    pub status: Option<u16>,
    provider_code: Option<i64>,
    outcome: Outcome,
}
impl Default for Diagnostic {
    fn default() -> Self {
        Self {
            enabled: true,
            service_id: None,
            reason: None,
            status: None,
            provider_code: None,
            outcome: Outcome::NotSent,
        }
    }
}
impl Diagnostic {
    pub(crate) fn fields(&self) -> Value {
        json!({"outcome":self.outcome.name(), "reason_code":self.reason.map(Reason::code),
            "response_status":self.status, "provider_code":self.provider_code})
    }
}
pub(crate) type Handle = Arc<Mutex<Diagnostic>>;
tokio::task_local! { static CURRENT: Handle; }
pub(crate) async fn scope<T>(handle: Handle, work: impl std::future::Future<Output = T>) -> T {
    CURRENT.scope(handle, work).await
}
pub(crate) fn current() -> Option<Handle> {
    CURRENT.try_with(Clone::clone).ok()
}
pub(crate) fn active() -> bool {
    CURRENT
        .try_with(|cell| cell.lock().expect("attachment diagnostic").enabled)
        .unwrap_or(false)
}
pub(crate) fn enable() {
    update(|diagnostic| diagnostic.enabled = true);
}
fn update(f: impl FnOnce(&mut Diagnostic)) {
    let _ = CURRENT.try_with(|cell| f(&mut cell.lock().expect("attachment diagnostic")));
}
pub(crate) fn bind(service_id: &str) {
    update(|d| {
        *d = Diagnostic {
            enabled: d.enabled,
            service_id: Some(service_id.into()),
            ..Default::default()
        };
    });
}
pub(crate) fn failure(reason: Reason, status: Option<u16>, provider_code: Option<i64>) {
    update(|d| {
        d.reason = Some(reason);
        d.status = status;
        d.provider_code = provider_code;
    });
}
pub(crate) fn outcome(outcome: Outcome) {
    update(|d| d.outcome = outcome);
}
pub(crate) fn provider_failure(reason: Reason, status: u16, code: Option<i64>) {
    failure(reason, Some(status), code);
    outcome(Outcome::ProviderRefused);
}
pub(crate) fn success(status: Option<u16>) {
    update(|d| {
        d.outcome = Outcome::Succeeded;
        d.reason = None;
        d.status = status;
        d.provider_code = None;
    });
}
/// Supply a closed fallback for earlier infrastructure/admission failures. A
/// more specific source diagnostic always wins; errors are never string-parsed.
pub(crate) fn error(error: &AppError) {
    fallback(Reason::Internal, error);
}
pub(crate) fn fallback(reason: Reason, error: &AppError) {
    update(|d| {
        if d.reason.is_none() {
            d.reason = Some(match error {
                AppError::DatabaseError(_) => Reason::Database,
                AppError::Forbidden(_)
                | AppError::AssistantTurnRequired
                | AppError::AssistantModelAuthorityChanged => Reason::AuthorityChanged,
                AppError::BadRequest(_) | AppError::ValidationError(_) => Reason::InvalidArguments,
                _ => reason,
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn lark_attachment_diagnostics_fields_boundary_requires_resolved_activation() {
        let handle = Handle::default();
        handle.lock().unwrap().enabled = false;
        scope(handle.clone(), async {
            bind("custom-connection");
            assert!(!active());
            enable();
            assert!(active());
            assert_eq!(
                current().unwrap().lock().unwrap().service_id.as_deref(),
                Some("custom-connection")
            );
        })
        .await;
        assert!(!active());
    }

    #[tokio::test]
    async fn lark_attachment_diagnostics_are_call_scoped_and_contain_no_error_text() {
        let handle = Handle::default();
        scope(handle.clone(), async {
            let _ = Reason::FieldMissing.error(AppError::BadRequest("private field value".into()));
            error(&AppError::Internal("secret".into()));
        })
        .await;
        assert!(current().is_none());
        let value = handle.lock().unwrap().fields();
        assert_eq!(value["reason_code"], "field_missing");
        assert!(!value.to_string().contains("secret"));
        assert!(!value.to_string().contains("private"));
    }
}
