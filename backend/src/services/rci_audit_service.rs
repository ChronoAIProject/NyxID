use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::errors::PENDING_CREDENTIAL_NODE_OFFLINE_CODE;
use crate::models::node_pending_credential::{NodePendingCredential, RemoteCryptoState};
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, node_pending_credential_service::PendingCredentialAuditSummary,
};

pub const PENDING_CREDENTIAL_DECRYPT_FAILED_CODE: u32 = 8006;
pub const PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE: u32 = 8007;
pub const PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE: u32 = 8008;
pub const PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE: u32 = 8009;
pub const PENDING_CREDENTIAL_QUEUE_FULL_CODE: u32 = 8011;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RciAuditSubject {
    pub node_id: String,
    pub pending_credential_id: String,
    pub service_slug: String,
    pub owner_user_id: String,
    pub remote_state: Option<RemoteCryptoState>,
    pub pending_created_at: DateTime<Utc>,
    pub pending_expires_at: DateTime<Utc>,
    pub ciphertext_queued_at: Option<DateTime<Utc>>,
    pub ciphertext_expires_at: Option<DateTime<Utc>>,
}

impl RciAuditSubject {
    pub fn from_pending(pending: &NodePendingCredential) -> Self {
        Self {
            node_id: pending.node_id.clone(),
            pending_credential_id: pending.id.clone(),
            service_slug: pending.service_slug.clone(),
            owner_user_id: pending.owner_user_id.clone(),
            remote_state: pending.remote_state.clone(),
            pending_created_at: pending.created_at,
            pending_expires_at: pending.expires_at,
            ciphertext_queued_at: pending.ciphertext_queued_at,
            ciphertext_expires_at: pending.ciphertext_expires_at,
        }
    }

    pub fn from_summary(summary: &PendingCredentialAuditSummary) -> Self {
        Self {
            node_id: summary.node_id.clone(),
            pending_credential_id: summary.pending_credential_id.clone(),
            service_slug: summary.service_slug.clone(),
            owner_user_id: summary.owner_user_id.clone(),
            remote_state: summary.remote_state.clone(),
            pending_created_at: summary.pending_created_at,
            pending_expires_at: summary.pending_expires_at,
            ciphertext_queued_at: summary.ciphertext_queued_at,
            ciphertext_expires_at: summary.ciphertext_expires_at,
        }
    }

    pub fn pending_is_rci(pending: &NodePendingCredential) -> bool {
        pending.crypto.is_some() || pending.remote_state.is_some()
    }

    fn remote_state_name(&self) -> Option<&'static str> {
        self.remote_state.as_ref().map(remote_state_name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RciAuditDelivery {
    OnlineForward,
    OfflineQueue,
    QueuedReplay,
}

impl RciAuditDelivery {
    fn as_str(self) -> &'static str {
        match self {
            Self::OnlineForward => "online_forward",
            Self::OfflineQueue => "offline_queue",
            Self::QueuedReplay => "queued_replay",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RciAuditErrorKind {
    DecryptFailed,
    VersionUnsupported,
    CiphertextTooLarge,
    PubkeyAwaiting,
    NodeOffline,
    QueueFull,
}

impl RciAuditErrorKind {
    pub fn code(self) -> u32 {
        match self {
            Self::DecryptFailed => PENDING_CREDENTIAL_DECRYPT_FAILED_CODE,
            Self::VersionUnsupported => PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE,
            Self::CiphertextTooLarge => PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE,
            Self::PubkeyAwaiting => PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE,
            Self::NodeOffline => PENDING_CREDENTIAL_NODE_OFFLINE_CODE,
            Self::QueueFull => PENDING_CREDENTIAL_QUEUE_FULL_CODE,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DecryptFailed => "pending_credential_decrypt_failed",
            Self::VersionUnsupported => "pending_credential_version_unsupported",
            Self::CiphertextTooLarge => "pending_credential_ciphertext_too_large",
            Self::PubkeyAwaiting => "pending_credential_pubkey_awaiting",
            Self::NodeOffline => "pending_credential_node_offline",
            Self::QueueFull => "pending_credential_queue_full",
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            PENDING_CREDENTIAL_DECRYPT_FAILED_CODE => Some(Self::DecryptFailed),
            PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE => Some(Self::VersionUnsupported),
            PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE => Some(Self::CiphertextTooLarge),
            PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE => Some(Self::PubkeyAwaiting),
            PENDING_CREDENTIAL_NODE_OFFLINE_CODE => Some(Self::NodeOffline),
            PENDING_CREDENTIAL_QUEUE_FULL_CODE => Some(Self::QueueFull),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RciAuditEventKind {
    PubkeyPosted,
    CiphertextReceived,
    CiphertextForwarded {
        delivery: RciAuditDelivery,
    },
    CiphertextQueued {
        delivery: RciAuditDelivery,
        node_offline: bool,
    },
    CiphertextReplayed {
        delivery: RciAuditDelivery,
    },
    DecryptSucceeded,
    DecryptFailed,
    VersionUnsupported,
    CiphertextTooLarge,
    PubkeyAwaiting,
    QueueFull,
    Consumed,
    Declined {
        reason_present: bool,
    },
    Canceled,
    Expired,
}

impl RciAuditEventKind {
    pub fn from_error_kind(error_kind: RciAuditErrorKind) -> Self {
        match error_kind {
            RciAuditErrorKind::DecryptFailed => Self::DecryptFailed,
            RciAuditErrorKind::VersionUnsupported => Self::VersionUnsupported,
            RciAuditErrorKind::CiphertextTooLarge => Self::CiphertextTooLarge,
            RciAuditErrorKind::PubkeyAwaiting => Self::PubkeyAwaiting,
            RciAuditErrorKind::NodeOffline => Self::CiphertextQueued {
                delivery: RciAuditDelivery::OfflineQueue,
                node_offline: true,
            },
            RciAuditErrorKind::QueueFull => Self::QueueFull,
        }
    }

    pub fn event_type(self) -> &'static str {
        match self {
            Self::PubkeyPosted => "node_credential_rci_pubkey_posted",
            Self::CiphertextReceived => "node_credential_rci_ciphertext_received",
            Self::CiphertextForwarded { .. } => "node_credential_rci_ciphertext_forwarded",
            Self::CiphertextQueued { .. } => "node_credential_rci_ciphertext_queued",
            Self::CiphertextReplayed { .. } => "node_credential_rci_ciphertext_replayed",
            Self::DecryptSucceeded => "node_credential_rci_decrypt_succeeded",
            Self::DecryptFailed => "node_credential_rci_decrypt_failed",
            Self::VersionUnsupported => "node_credential_rci_version_unsupported",
            Self::CiphertextTooLarge => "node_credential_rci_ciphertext_too_large",
            Self::PubkeyAwaiting => "node_credential_rci_pubkey_awaiting",
            Self::QueueFull => "node_credential_rci_queue_full",
            Self::Consumed => "node_credential_rci_consumed",
            Self::Declined { .. } => "node_credential_rci_declined",
            Self::Canceled => "node_credential_rci_canceled",
            Self::Expired => "node_credential_rci_expired",
        }
    }

    fn remote_state(self, subject: &RciAuditSubject) -> Option<&'static str> {
        match self {
            Self::PubkeyPosted => Some("pubkey_posted"),
            Self::CiphertextReceived
            | Self::CiphertextForwarded { .. }
            | Self::CiphertextReplayed { .. } => Some("ciphertext_received"),
            Self::CiphertextQueued { .. } => Some("ciphertext_queued"),
            Self::DecryptSucceeded | Self::Consumed => Some("consumed"),
            Self::DecryptFailed | Self::VersionUnsupported => Some("decrypt_failed"),
            Self::Declined { .. } => Some("declined"),
            Self::Canceled => Some("canceled"),
            Self::Expired => Some("expired"),
            Self::CiphertextTooLarge | Self::PubkeyAwaiting | Self::QueueFull => {
                subject.remote_state_name()
            }
        }
    }

    fn delivery(self) -> Option<RciAuditDelivery> {
        match self {
            Self::CiphertextForwarded { delivery }
            | Self::CiphertextQueued { delivery, .. }
            | Self::CiphertextReplayed { delivery } => Some(delivery),
            _ => None,
        }
    }

    fn error_kind(self) -> Option<RciAuditErrorKind> {
        match self {
            Self::CiphertextQueued {
                node_offline: true, ..
            } => Some(RciAuditErrorKind::NodeOffline),
            Self::DecryptFailed => Some(RciAuditErrorKind::DecryptFailed),
            Self::VersionUnsupported => Some(RciAuditErrorKind::VersionUnsupported),
            Self::CiphertextTooLarge => Some(RciAuditErrorKind::CiphertextTooLarge),
            Self::PubkeyAwaiting => Some(RciAuditErrorKind::PubkeyAwaiting),
            Self::QueueFull => Some(RciAuditErrorKind::QueueFull),
            _ => None,
        }
    }

    fn include_queue_timestamps(self) -> bool {
        matches!(
            self,
            Self::CiphertextQueued { .. } | Self::CiphertextReplayed { .. } | Self::Expired
        )
    }
}

pub fn log_rci_for_user(
    db: mongodb::Database,
    auth_user: &AuthUser,
    subject: &RciAuditSubject,
    kind: RciAuditEventKind,
) {
    audit_service::log_for_user(
        db,
        auth_user,
        kind.event_type(),
        Some(rci_event_data(subject, kind, Utc::now())),
    );
}

pub fn log_rci_for_node(
    db: mongodb::Database,
    owner_user_id: &str,
    ip_address: Option<String>,
    user_agent: Option<String>,
    subject: &RciAuditSubject,
    kind: RciAuditEventKind,
) {
    audit_service::log_async(
        db,
        Some(owner_user_id.to_string()),
        kind.event_type().to_string(),
        Some(rci_event_data(subject, kind, Utc::now())),
        ip_address,
        user_agent,
        None,
        None,
    );
}

pub(crate) fn rci_event_data(
    subject: &RciAuditSubject,
    kind: RciAuditEventKind,
    event_at: DateTime<Utc>,
) -> Value {
    let mut object = Map::new();
    object.insert(
        "flow".to_string(),
        Value::String("remote_credential_injection".to_string()),
    );
    object.insert("routed_via".to_string(), Value::String("node".to_string()));
    object.insert(
        "node_id".to_string(),
        Value::String(subject.node_id.clone()),
    );
    object.insert(
        "pending_credential_id".to_string(),
        Value::String(subject.pending_credential_id.clone()),
    );
    object.insert(
        "service_slug".to_string(),
        Value::String(subject.service_slug.clone()),
    );
    object.insert(
        "owner_user_id".to_string(),
        Value::String(subject.owner_user_id.clone()),
    );
    if let Some(remote_state) = kind.remote_state(subject) {
        object.insert(
            "remote_state".to_string(),
            Value::String(remote_state.to_string()),
        );
    }
    object.insert("event_at".to_string(), Value::String(event_at.to_rfc3339()));
    object.insert(
        "pending_created_at".to_string(),
        Value::String(subject.pending_created_at.to_rfc3339()),
    );
    object.insert(
        "pending_expires_at".to_string(),
        Value::String(subject.pending_expires_at.to_rfc3339()),
    );
    if let Some(delivery) = kind.delivery() {
        object.insert(
            "delivery".to_string(),
            Value::String(delivery.as_str().to_string()),
        );
    }
    if kind.include_queue_timestamps() {
        if let Some(queued_at) = subject.ciphertext_queued_at {
            object.insert(
                "ciphertext_queued_at".to_string(),
                Value::String(queued_at.to_rfc3339()),
            );
        }
        if let Some(expires_at) = subject.ciphertext_expires_at {
            object.insert(
                "ciphertext_expires_at".to_string(),
                Value::String(expires_at.to_rfc3339()),
            );
        }
    }
    if let Some(error_kind) = kind.error_kind() {
        object.insert(
            "error_code".to_string(),
            Value::Number(serde_json::Number::from(error_kind.code())),
        );
        object.insert(
            "error_kind".to_string(),
            Value::String(error_kind.as_str().to_string()),
        );
    }
    if let RciAuditEventKind::Declined { reason_present } = kind {
        object.insert("reason_present".to_string(), Value::Bool(reason_present));
    }

    Value::Object(object)
}

fn remote_state_name(state: &RemoteCryptoState) -> &'static str {
    match state {
        RemoteCryptoState::PubkeyPosted => "pubkey_posted",
        RemoteCryptoState::CiphertextReceived => "ciphertext_received",
        RemoteCryptoState::CiphertextQueued => "ciphertext_queued",
        RemoteCryptoState::Consumed => "consumed",
        RemoteCryptoState::DecryptFailed => "decrypt_failed",
        RemoteCryptoState::Expired => "expired",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn subject() -> RciAuditSubject {
        let now = Utc::now();
        RciAuditSubject {
            node_id: "node-audit".to_string(),
            pending_credential_id: "pending-audit".to_string(),
            service_slug: "openclaw".to_string(),
            owner_user_id: "owner-audit".to_string(),
            remote_state: Some(RemoteCryptoState::PubkeyPosted),
            pending_created_at: now - Duration::minutes(10),
            pending_expires_at: now + Duration::minutes(50),
            ciphertext_queued_at: Some(now - Duration::minutes(1)),
            ciphertext_expires_at: Some(now + Duration::minutes(14)),
        }
    }

    fn taxonomy_kinds() -> Vec<RciAuditEventKind> {
        vec![
            RciAuditEventKind::PubkeyPosted,
            RciAuditEventKind::CiphertextReceived,
            RciAuditEventKind::CiphertextForwarded {
                delivery: RciAuditDelivery::OnlineForward,
            },
            RciAuditEventKind::CiphertextQueued {
                delivery: RciAuditDelivery::OfflineQueue,
                node_offline: true,
            },
            RciAuditEventKind::CiphertextReplayed {
                delivery: RciAuditDelivery::QueuedReplay,
            },
            RciAuditEventKind::DecryptSucceeded,
            RciAuditEventKind::DecryptFailed,
            RciAuditEventKind::VersionUnsupported,
            RciAuditEventKind::CiphertextTooLarge,
            RciAuditEventKind::PubkeyAwaiting,
            RciAuditEventKind::QueueFull,
            RciAuditEventKind::Consumed,
            RciAuditEventKind::Declined {
                reason_present: true,
            },
            RciAuditEventKind::Canceled,
            RciAuditEventKind::Expired,
        ]
    }

    fn object_keys(value: &Value) -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .expect("event data object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    fn sorted(mut keys: Vec<&str>) -> Vec<String> {
        keys.sort();
        keys.into_iter().map(str::to_string).collect()
    }

    #[test]
    fn rci_event_type_taxonomy_is_stable() {
        let event_types: Vec<&str> = taxonomy_kinds()
            .into_iter()
            .map(RciAuditEventKind::event_type)
            .collect();

        assert_eq!(
            event_types,
            vec![
                "node_credential_rci_pubkey_posted",
                "node_credential_rci_ciphertext_received",
                "node_credential_rci_ciphertext_forwarded",
                "node_credential_rci_ciphertext_queued",
                "node_credential_rci_ciphertext_replayed",
                "node_credential_rci_decrypt_succeeded",
                "node_credential_rci_decrypt_failed",
                "node_credential_rci_version_unsupported",
                "node_credential_rci_ciphertext_too_large",
                "node_credential_rci_pubkey_awaiting",
                "node_credential_rci_queue_full",
                "node_credential_rci_consumed",
                "node_credential_rci_declined",
                "node_credential_rci_canceled",
                "node_credential_rci_expired",
            ]
        );
    }

    #[test]
    fn rci_event_data_exact_allowlist() {
        let subject = subject();
        let common = vec![
            "event_at",
            "flow",
            "node_id",
            "owner_user_id",
            "pending_created_at",
            "pending_credential_id",
            "pending_expires_at",
            "remote_state",
            "routed_via",
            "service_slug",
        ];

        for kind in taxonomy_kinds() {
            let event_data = rci_event_data(&subject, kind, Utc::now());
            let mut expected = common.clone();
            if kind.delivery().is_some() {
                expected.push("delivery");
            }
            if kind.include_queue_timestamps() {
                expected.push("ciphertext_queued_at");
                expected.push("ciphertext_expires_at");
            }
            if kind.error_kind().is_some() {
                expected.push("error_code");
                expected.push("error_kind");
            }
            if matches!(kind, RciAuditEventKind::Declined { .. }) {
                expected.push("reason_present");
            }
            assert_eq!(object_keys(&event_data), sorted(expected), "{kind:?}");
        }
    }

    #[test]
    fn rci_event_data_excludes_crypto_material_and_derivatives() {
        let event_json = taxonomy_kinds()
            .into_iter()
            .map(|kind| rci_event_data(&subject(), kind, Utc::now()).to_string())
            .collect::<Vec<_>>()
            .join("\n");

        for forbidden_value in [
            "super-secret-plaintext-fixture",
            "secret-value-fixture",
            "admin-pubkey-fixture",
            "node-pubkey-fixture",
            "nonce-fixture",
            "ciphertext-fixture",
            "hash-fixture",
            "fingerprint-fixture",
            "raw-node-error-fixture",
            "decline-reason-fixture",
            "target-url-fixture",
            "field-name-fixture",
        ] {
            assert!(!event_json.contains(forbidden_value), "{forbidden_value}");
        }

        for kind in taxonomy_kinds() {
            let event_data = rci_event_data(&subject(), kind, Utc::now());
            let object = event_data.as_object().expect("event data object");
            for forbidden_key in [
                "plaintext",
                "secret",
                "ciphertext",
                "nonce",
                "node_pubkey",
                "admin_pubkey",
                "hash",
                "fingerprint",
                "length",
                "bytes",
                "target_url",
                "field_name",
                "injection_method",
                "raw_version",
                "raw_status",
                "raw_node_error",
                "raw_decline_reason",
                "queue_count",
                "queued_pending_ids",
            ] {
                assert!(
                    !object.contains_key(forbidden_key),
                    "{kind:?}: {forbidden_key}"
                );
            }
        }
    }

    #[test]
    fn rci_error_code_mapping_is_fixed() {
        let cases = [
            (
                PENDING_CREDENTIAL_DECRYPT_FAILED_CODE,
                RciAuditErrorKind::DecryptFailed,
                "node_credential_rci_decrypt_failed",
                "pending_credential_decrypt_failed",
            ),
            (
                PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE,
                RciAuditErrorKind::VersionUnsupported,
                "node_credential_rci_version_unsupported",
                "pending_credential_version_unsupported",
            ),
            (
                PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE,
                RciAuditErrorKind::CiphertextTooLarge,
                "node_credential_rci_ciphertext_too_large",
                "pending_credential_ciphertext_too_large",
            ),
            (
                PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE,
                RciAuditErrorKind::PubkeyAwaiting,
                "node_credential_rci_pubkey_awaiting",
                "pending_credential_pubkey_awaiting",
            ),
            (
                PENDING_CREDENTIAL_NODE_OFFLINE_CODE,
                RciAuditErrorKind::NodeOffline,
                "node_credential_rci_ciphertext_queued",
                "pending_credential_node_offline",
            ),
            (
                PENDING_CREDENTIAL_QUEUE_FULL_CODE,
                RciAuditErrorKind::QueueFull,
                "node_credential_rci_queue_full",
                "pending_credential_queue_full",
            ),
        ];

        for (code, error_kind, event_type, error_kind_name) in cases {
            assert_eq!(RciAuditErrorKind::from_code(code), Some(error_kind));
            let event_kind = RciAuditEventKind::from_error_kind(error_kind);
            assert_eq!(event_kind.event_type(), event_type);
            assert_eq!(error_kind.as_str(), error_kind_name);

            let event_data = rci_event_data(&subject(), event_kind, Utc::now());
            assert_eq!(event_data["error_code"], code);
            assert_eq!(event_data["error_kind"], error_kind_name);
        }
    }
}
