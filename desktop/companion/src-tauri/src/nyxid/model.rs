use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NyxIdView {
    Checking,
    SignedOut,
    Authorizing {
        user_code: String,
        verification_url: String,
        expires_at: String,
    },
    Connected {
        user: NyxIdUser,
        capabilities: NyxIdCapabilities,
    },
    Denied {
        message: String,
    },
    Expired {
        message: String,
    },
    Error {
        error: NyxIdPublicError,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdUser {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdCapabilities {
    pub enabled_count: usize,
    pub disabled_count: usize,
    pub attention_count: usize,
    pub checked_at: String,
    pub services: Vec<NyxIdService>,
}

impl NyxIdCapabilities {
    pub fn from_services(services: Vec<NyxIdService>, checked_at: DateTime<Utc>) -> Self {
        let enabled_count = services
            .iter()
            .filter(|service| service.state == NyxIdServiceState::Enabled)
            .count();
        let disabled_count = services
            .iter()
            .filter(|service| service.state == NyxIdServiceState::Disabled)
            .count();
        let attention_count = services
            .iter()
            .filter(|service| service.state == NyxIdServiceState::Attention)
            .count();
        Self {
            enabled_count,
            disabled_count,
            attention_count,
            checked_at: checked_at.to_rfc3339(),
            services,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdService {
    pub id: String,
    pub slug: String,
    pub label: String,
    pub state: NyxIdServiceState,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NyxIdServiceState {
    Enabled,
    Disabled,
    Attention,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdPublicError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub retry_action: Option<NyxIdRetryAction>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NyxIdRetryAction {
    Connect,
    Cancel,
    Refresh,
    Logout,
}

impl NyxIdView {
    pub fn error(code: &str, message: &str, retry_action: Option<NyxIdRetryAction>) -> Self {
        Self::Error {
            error: NyxIdPublicError {
                code: code.to_owned(),
                message: message.to_owned(),
                retryable: retry_action.is_some(),
                retry_action,
            },
        }
    }
}

pub(crate) struct CredentialBundle {
    pub(crate) access_token: Zeroizing<String>,
    pub(crate) refresh_token: Zeroizing<String>,
    pub(crate) access_expires_at: DateTime<Utc>,
    pub(crate) session_id: String,
}

#[derive(Clone)]
pub(crate) struct PendingLoginRecovery {
    pub(crate) device_code: Zeroizing<String>,
    pub(crate) recovery_secret: Zeroizing<String>,
    pub(crate) attempt_id: String,
    pub(crate) phase: PendingLoginRecoveryPhase,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingLoginRecoveryPhase {
    AwaitingDelivery,
    DeliveryCommitting,
}

impl PendingLoginRecovery {
    pub(crate) fn new(
        device_code: String,
        recovery_secret: String,
        attempt_id: String,
    ) -> Option<Self> {
        valid_device_code(&device_code)
            .then_some(())
            .and_then(|()| valid_recovery_secret(&recovery_secret).then_some(()))
            .and_then(|()| {
                is_session_id(&attempt_id).then_some(Self {
                    device_code: Zeroizing::new(device_code),
                    recovery_secret: Zeroizing::new(recovery_secret),
                    attempt_id,
                    phase: PendingLoginRecoveryPhase::AwaitingDelivery,
                })
            })
    }

    pub(crate) fn mark_delivery_committing(&mut self) {
        self.phase = PendingLoginRecoveryPhase::DeliveryCommitting;
    }
}

impl fmt::Debug for PendingLoginRecovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingLoginRecovery")
            .field("device_code", &"[REDACTED]")
            .field("recovery_secret", &"[REDACTED]")
            .field("attempt_id", &self.attempt_id)
            .field("phase", &self.phase)
            .finish()
    }
}

impl CredentialBundle {
    pub(crate) fn new(
        access_token: String,
        refresh_token: String,
        access_expires_at: DateTime<Utc>,
    ) -> Option<Self> {
        if access_token.trim().is_empty() || refresh_token.trim().is_empty() {
            return None;
        }
        Some(Self {
            access_token: Zeroizing::new(access_token),
            refresh_token: Zeroizing::new(refresh_token),
            access_expires_at,
            session_id: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub(crate) fn for_session_id(mut self, session_id: String) -> Option<Self> {
        is_session_id(&session_id).then(|| {
            self.session_id = session_id;
            self
        })
    }
}

impl fmt::Debug for CredentialBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialBundle")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("access_expires_at", &self.access_expires_at)
            .field("session_id", &self.session_id)
            .finish()
    }
}

pub(crate) fn is_session_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 4)
}

pub(crate) fn valid_device_code(value: &str) -> bool {
    value
        .strip_prefix("nyx_adc_")
        .is_some_and(|encoded| valid_base64url_bytes(encoded, 32))
}

fn valid_recovery_secret(value: &str) -> bool {
    valid_base64url_bytes(value, 32)
}

fn valid_base64url_bytes(value: &str, expected_len: usize) -> bool {
    URL_SAFE_NO_PAD.decode(value).is_ok_and(|decoded| {
        decoded.len() == expected_len && URL_SAFE_NO_PAD.encode(decoded) == value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_view_serialization_contains_no_session_secrets() {
        let bundle =
            CredentialBundle::new("access-secret".into(), "refresh-secret".into(), Utc::now())
                .unwrap();
        let view = NyxIdView::Connected {
            user: NyxIdUser {
                id: "user-1".into(),
                email: "user@example.com".into(),
                display_name: None,
                avatar_url: None,
            },
            capabilities: NyxIdCapabilities::from_services(Vec::new(), Utc::now()),
        };
        let json = serde_json::to_string(&view).unwrap();
        let debug = format!("{bundle:?}");
        assert!(!json.contains("access-secret"));
        assert!(!json.contains("refresh-secret"));
        assert!(!debug.contains("access-secret"));
        assert!(!debug.contains("refresh-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn pending_login_recovery_validates_and_redacts_both_capabilities() {
        let device_code = format!("nyx_adc_{}", URL_SAFE_NO_PAD.encode([7_u8; 32]));
        let recovery_secret = URL_SAFE_NO_PAD.encode([9_u8; 32]);
        let recovery = PendingLoginRecovery::new(
            device_code.clone(),
            recovery_secret.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();

        let debug = format!("{recovery:?}");
        assert!(!debug.contains(&device_code));
        assert!(!debug.contains(&recovery_secret));
        assert_eq!(debug.matches("[REDACTED]").count(), 2);
        let public_json = serde_json::to_string(&NyxIdView::Authorizing {
            user_code: "ABCD-EFGH".into(),
            verification_url: "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH".into(),
            expires_at: Utc::now().to_rfc3339(),
        })
        .unwrap();
        assert!(!public_json.contains(&device_code));
        assert!(!public_json.contains(&recovery_secret));
        assert!(
            PendingLoginRecovery::new(
                "nyx_adc_not-canonical".into(),
                recovery_secret,
                uuid::Uuid::new_v4().to_string(),
            )
            .is_none()
        );
    }

    #[test]
    fn authorizing_fields_are_camel_case() {
        let value = serde_json::to_value(NyxIdView::Authorizing {
            user_code: "ABCD-EFGH".into(),
            verification_url: "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH".into(),
            expires_at: "2026-10-09T12:00:00Z".into(),
        })
        .unwrap();
        assert_eq!(value["state"], "authorizing");
        assert_eq!(value["userCode"], "ABCD-EFGH");
        assert!(value.get("user_code").is_none());
    }

    #[test]
    fn public_error_exposes_camel_case_retry_action_without_secrets() {
        let retryable = serde_json::to_value(NyxIdView::error(
            "temporary",
            "Try again",
            Some(NyxIdRetryAction::Refresh),
        ))
        .unwrap();
        assert_eq!(retryable["error"]["retryable"], true);
        assert_eq!(retryable["error"]["retryAction"], "refresh");
        assert!(retryable["error"].get("retry_action").is_none());

        let terminal = serde_json::to_value(NyxIdView::error("terminal", "Stop", None)).unwrap();
        assert_eq!(terminal["error"]["retryable"], false);
        assert!(terminal["error"]["retryAction"].is_null());
    }
}
