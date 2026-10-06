//! Startup diagnostics contain identifiers and fixed categories, never provider prose.
use super::{AppError, ErrorResponse};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Flag,
    Thread,
    Origin,
    Credential,
    BillingReservation,
    ProviderCreate,
    ProviderAnswer,
    Transport,
}
impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Thread => "thread",
            Self::Origin => "origin",
            Self::Credential => "credential",
            Self::BillingReservation => "billing_reservation",
            Self::ProviderCreate => "provider_create",
            Self::ProviderAnswer => "provider_answer",
            Self::Transport => "transport",
        }
    }
    pub fn error(self, error: AppError) -> AppError {
        if matches!(error, AppError::VoiceStartFailed(_)) {
            return error;
        }
        AppError::VoiceStartFailed(Box::new(Failure {
            source: error,
            stage: self,
            provider: None,
        }))
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Openai,
    Xai,
}

#[derive(Debug, Serialize)]
pub struct ProviderFailure {
    pub provider: Provider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_param: Option<String>,
}
impl ProviderFailure {
    pub fn new(provider: Provider, status: Option<u16>, body: &[u8]) -> Self {
        let value: Value = if body.len() <= 16 * 1024 {
            serde_json::from_slice(body).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        Self {
            provider,
            provider_status: status,
            provider_type: identifier(&value["error"]["type"]),
            provider_code: identifier(&value["error"]["code"]),
            provider_param: identifier(&value["error"]["param"]),
        }
    }
    pub fn error(self, stage: Stage) -> AppError {
        AppError::VoiceStartFailed(Box::new(Failure {
            source: AppError::VoiceProviderUnavailable,
            stage,
            provider: Some(self),
        }))
    }
}
// Accept bounded machine identifiers only. Reject prose, URLs, whitespace and
// credential-shaped values rather than truncating and exposing their prefixes.
fn identifier(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    (!s.is_empty()
        && s.len() <= 128
        && !["sk-", "sk_", "nyx_", "Bearer"]
            .iter()
            .any(|p| s.starts_with(p))
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.[]".contains(&b)))
    .then(|| s.to_owned())
}

pub struct Failure {
    pub(super) source: AppError,
    pub stage: Stage,
    pub provider: Option<ProviderFailure>,
}
impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceStartFailure")
            .field("stage", &self.stage)
            .field("error_code", &self.source.error_code())
            .field("provider", &self.provider)
            .finish()
    }
}
impl Failure {
    pub fn details(&self) -> Value {
        let mut value = self
            .provider
            .as_ref()
            .map_or_else(|| json!({}), |p| json!(p));
        let mut reason = self.stage.as_str().to_owned();
        if let Some(p) = &self.provider {
            if let Some(status) = p.provider_status {
                reason.push_str(&format!(":{status}"));
            }
            for s in [
                p.provider_code.as_ref().or(p.provider_type.as_ref()),
                p.provider_param.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                reason.push(':');
                reason.push_str(s);
            }
        }
        value["stage"] = json!(self.stage);
        value["reason"] = json!(reason);
        if self.stage == Stage::BillingReservation {
            value["billing_error_code"] = json!(self.source.error_code());
        }
        value
    }
    pub fn response(&self) -> ErrorResponse {
        let mut response = ErrorResponse {
            error: self.source.error_key().into(),
            error_code: self.source.error_code(),
            message: String::new(),
            details: None,
            session_token: None,
            consent_url: None,
            request_id: None,
            approve_url: None,
        };
        response.message = match &self.source {
            AppError::Conflict(s) if s == "End the current call first" => {
                "End the current call first"
            }
            AppError::InsufficientCredits => "Insufficient voice credits",
            AppError::WalletSuspended => "Voice billing wallet is suspended",
            _ => match self.stage {
                Stage::Flag => "Voice is not enabled for this account",
                Stage::Thread => "Voice is unavailable for this conversation",
                Stage::Origin => "Voice origin was refused",
                Stage::Credential => "Voice credential is unavailable or not authorized",
                Stage::BillingReservation => "Voice billing could not be reserved",
                Stage::ProviderCreate
                    if self
                        .provider
                        .as_ref()
                        .and_then(|p| p.provider_status)
                        .is_some() =>
                {
                    "Voice provider rejected the session"
                }
                Stage::ProviderCreate => "Voice provider could not create the session",
                Stage::ProviderAnswer => "Voice provider returned an invalid session answer",
                Stage::Transport => "Voice transport could not connect",
            },
        }
        .into();
        response.details = Some(self.details());
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_start_preserves_codes_and_status_but_never_renders_source_errors() {
        for stage in [
            Stage::Flag,
            Stage::Thread,
            Stage::Origin,
            Stage::Credential,
            Stage::BillingReservation,
            Stage::ProviderCreate,
            Stage::ProviderAnswer,
            Stage::Transport,
        ] {
            let error = stage.error(AppError::BillingProviderUnavailable(
                "SECRET URL body".into(),
            ));
            assert_eq!(error.error_code(), 11302);
            assert_eq!(
                error.status_code(),
                axum::http::StatusCode::PAYMENT_REQUIRED
            );
            let body = error.response_body();
            assert_eq!(body.details.as_ref().unwrap()["stage"], stage.as_str());
            assert!(!serde_json::to_string(&body).unwrap().contains("SECRET"));
            assert!(!format!("{error:?} {error}").contains("SECRET"));
        }
    }
    #[test]
    fn voice_provider_metadata_refuses_prose_keys_unbounded_or_wrong_types() {
        for value in [
            json!("sk-secret"),
            json!("nyx_secret"),
            json!("Bearer secret"),
            json!("https://secret"),
            json!("private words"),
            json!("x".repeat(129)),
            json!({"key":"secret"}),
            json!(123),
        ] {
            let body =
                json!({"error":{"message":"never", "type":value,"code":value,"param":value}});
            let p = ProviderFailure::new(Provider::Openai, Some(400), body.to_string().as_bytes());
            assert!(
                p.provider_type.is_none()
                    && p.provider_code.is_none()
                    && p.provider_param.is_none()
            );
        }
        let p = ProviderFailure::new(Provider::Xai, Some(502), b"<html>secret</html>");
        assert_eq!(p.provider_status, Some(502));
        assert!(p.provider_code.is_none());
        let p = ProviderFailure::new(Provider::Openai, Some(400), &vec![b'x'; 16385]);
        assert!(p.provider_type.is_none());
    }
}
