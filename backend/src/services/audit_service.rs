use chrono::Utc;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::mw::auth::AuthUser;

/// Fire-and-forget audit log entry.
///
/// Spawns a background task to write the audit record so that the calling
/// handler is not blocked by the database write. Errors are logged but
/// do not propagate.
#[allow(clippy::too_many_arguments)]
pub fn log_async(
    db: mongodb::Database,
    user_id: Option<String>,
    event_type: String,
    event_data: Option<serde_json::Value>,
    ip_address: Option<String>,
    user_agent: Option<String>,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
) {
    let entry = build_audit_entry(
        user_id,
        event_type.clone(),
        event_data,
        ip_address,
        user_agent,
        api_key_id,
        api_key_name,
    );
    tokio::spawn(async move {
        let _ = write_audit_entry(db, entry, event_type).await;
    });
}

/// Fire-and-forget audit log entry attributed to an authenticated request.
///
/// Pulls user_id, IP, User-Agent, and API key identity from `AuthUser` so
/// every audit record carries consistent forensic context. Prefer this over
/// `log_async` when the call site already has an `AuthUser` extractor.
pub fn log_for_user(
    db: mongodb::Database,
    auth_user: &AuthUser,
    event_type: impl Into<String>,
    event_data: Option<serde_json::Value>,
) {
    log_async(
        db,
        Some(auth_user.user_id.to_string()),
        event_type.into(),
        event_data,
        auth_user.ip_address.clone(),
        auth_user.user_agent.clone(),
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );
}

#[allow(clippy::too_many_arguments)]
fn build_audit_entry(
    user_id: Option<String>,
    event_type: String,
    event_data: Option<serde_json::Value>,
    ip_address: Option<String>,
    user_agent: Option<String>,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
) -> AuditLog {
    AuditLog {
        id: Uuid::new_v4().to_string(),
        user_id,
        event_type,
        event_data,
        ip_address,
        user_agent,
        api_key_id,
        api_key_name,
        created_at: Utc::now(),
    }
}

async fn write_audit_entry(
    db: mongodb::Database,
    entry: AuditLog,
    event_type: String,
) -> Result<(), mongodb::error::Error> {
    let result = db
        .collection::<AuditLog>(AUDIT_LOG)
        .insert_one(&entry)
        .await;
    if let Err(e) = &result {
        tracing::error!(event_type = %event_type, error = %e, "Failed to write audit log");
    }
    #[cfg(test)]
    if result.is_ok() {
        notify_test_audit_write(&entry);
    }
    result.map(|_| ())
}

#[cfg(test)]
struct AuditWriteWatcher {
    event_type: String,
    pending_credential_id: Option<String>,
    user_id: Option<String>,
    sender: tokio::sync::oneshot::Sender<String>,
}

#[cfg(test)]
static AUDIT_WRITE_WATCHERS: OnceLock<Mutex<Vec<AuditWriteWatcher>>> = OnceLock::new();

#[cfg(test)]
fn audit_write_watchers() -> &'static Mutex<Vec<AuditWriteWatcher>> {
    AUDIT_WRITE_WATCHERS.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(test)]
pub(crate) fn notify_on_audit_write(
    event_type: impl Into<String>,
    pending_credential_id: Option<String>,
) -> tokio::sync::oneshot::Receiver<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    audit_write_watchers()
        .lock()
        .expect("audit watcher mutex")
        .push(AuditWriteWatcher {
            event_type: event_type.into(),
            pending_credential_id,
            user_id: None,
            sender,
        });
    receiver
}

#[cfg(test)]
pub(crate) fn notify_on_audit_write_for_user(
    event_type: impl Into<String>,
    user_id: impl Into<String>,
) -> tokio::sync::oneshot::Receiver<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    audit_write_watchers()
        .lock()
        .expect("audit watcher mutex")
        .push(AuditWriteWatcher {
            event_type: event_type.into(),
            pending_credential_id: None,
            user_id: Some(user_id.into()),
            sender,
        });
    receiver
}

#[cfg(test)]
fn notify_test_audit_write(entry: &AuditLog) {
    let pending_credential_id = entry
        .event_data
        .as_ref()
        .and_then(|event_data| event_data.get("pending_credential_id"))
        .or_else(|| {
            entry
                .event_data
                .as_ref()
                .and_then(|event_data| event_data.get("fanout_id"))
        })
        .and_then(serde_json::Value::as_str);
    let mut watchers = audit_write_watchers().lock().expect("audit watcher mutex");
    let mut index = 0;
    while index < watchers.len() {
        let watcher = &watchers[index];
        let pending_matches = match watcher.pending_credential_id.as_deref() {
            Some(expected) => pending_credential_id == Some(expected),
            None => true,
        };
        let user_matches = match watcher.user_id.as_deref() {
            Some(expected) => entry.user_id.as_deref() == Some(expected),
            None => true,
        };
        if watcher.event_type == entry.event_type && pending_matches && user_matches {
            let watcher = watchers.swap_remove(index);
            let _ = watcher.sender.send(entry.id.clone());
        } else {
            index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::node_pending_credential::RemoteCryptoState;
    use crate::services::rci_audit_service::{
        RciAuditDelivery, RciAuditEventKind, RciAuditSubject, rci_event_data,
    };
    use crate::test_utils::{connect_test_database, test_auth_user};
    use chrono::Duration;
    use futures::TryStreamExt;

    #[tokio::test]
    async fn write_audit_entry_inserts_audit_entry() {
        let Some(db) = connect_test_database("audit_log_async").await else {
            return;
        };

        let entry = build_audit_entry(
            Some("user-123".to_string()),
            "test_event".to_string(),
            Some(serde_json::json!({"key": "value"})),
            Some("127.0.0.1".to_string()),
            Some("test-agent".to_string()),
            None,
            None,
        );
        write_audit_entry(db.clone(), entry, "test_event".to_string())
            .await
            .expect("write audit entry");

        let entries: Vec<AuditLog> = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find(mongodb::bson::doc! { "event_type": "test_event" })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.user_id.as_deref(), Some("user-123"));
        assert_eq!(entry.event_type, "test_event");
        assert_eq!(entry.ip_address.as_deref(), Some("127.0.0.1"));
        assert_eq!(entry.user_agent.as_deref(), Some("test-agent"));
        assert!(entry.api_key_id.is_none());
        assert!(entry.api_key_name.is_none());
    }

    #[tokio::test]
    async fn write_audit_entry_with_api_key_fields() {
        let Some(db) = connect_test_database("audit_log_apikey").await else {
            return;
        };

        let entry = build_audit_entry(
            Some("user-456".to_string()),
            "proxy_request".to_string(),
            None,
            None,
            None,
            Some("key-id-1".to_string()),
            Some("my-agent".to_string()),
        );
        write_audit_entry(db.clone(), entry, "proxy_request".to_string())
            .await
            .expect("write audit entry");

        let entries: Vec<AuditLog> = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find(mongodb::bson::doc! { "event_type": "proxy_request" })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].api_key_id.as_deref(), Some("key-id-1"));
        assert_eq!(entries[0].api_key_name.as_deref(), Some("my-agent"));
    }

    #[tokio::test]
    async fn audit_entry_for_user_extracts_auth_user_fields() {
        let Some(db) = connect_test_database("audit_log_for_user").await else {
            return;
        };

        let mut auth = test_auth_user("550e8400-e29b-41d4-a716-446655440099");
        auth.ip_address = Some("10.0.0.1".to_string());
        auth.user_agent = Some("Mozilla/5.0".to_string());
        auth.api_key_id = Some("ak-1".to_string());
        auth.api_key_name = Some("agent-name".to_string());

        let entry = build_audit_entry(
            Some(auth.user_id.to_string()),
            "user_action".to_string(),
            Some(serde_json::json!({"detail": "test"})),
            auth.ip_address.clone(),
            auth.user_agent.clone(),
            auth.api_key_id.clone(),
            auth.api_key_name.clone(),
        );
        write_audit_entry(db.clone(), entry, "user_action".to_string())
            .await
            .expect("write audit entry");

        let entries: Vec<AuditLog> = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find(mongodb::bson::doc! { "event_type": "user_action" })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(
            entry.user_id.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440099")
        );
        assert_eq!(entry.ip_address.as_deref(), Some("10.0.0.1"));
        assert_eq!(entry.user_agent.as_deref(), Some("Mozilla/5.0"));
        assert_eq!(entry.api_key_id.as_deref(), Some("ak-1"));
        assert_eq!(entry.api_key_name.as_deref(), Some("agent-name"));
    }

    fn test_rci_subject() -> RciAuditSubject {
        let now = Utc::now();
        RciAuditSubject {
            node_id: "node-audit".to_string(),
            pending_credential_id: "pending-audit".to_string(),
            service_slug: "openclaw".to_string(),
            owner_user_id: "owner-audit".to_string(),
            remote_state: Some(RemoteCryptoState::PubkeyPosted),
            fan_out: false,
            generation: None,
            pending_created_at: now - Duration::minutes(5),
            pending_expires_at: now + Duration::minutes(55),
            ciphertext_queued_at: Some(now - Duration::minutes(1)),
            ciphertext_expires_at: Some(now + Duration::minutes(14)),
        }
    }

    async fn insert_rci_event(db: &mongodb::Database, kind: RciAuditEventKind) -> AuditLog {
        let event_type = kind.event_type().to_string();
        let entry = build_audit_entry(
            Some("owner-audit".to_string()),
            event_type.clone(),
            Some(rci_event_data(&test_rci_subject(), kind, Utc::now())),
            Some("127.0.0.1".to_string()),
            Some("test-agent".to_string()),
            None,
            None,
        );
        write_audit_entry(db.clone(), entry.clone(), event_type.clone())
            .await
            .expect("insert RCI audit entry");

        db.collection::<AuditLog>(AUDIT_LOG)
            .find_one(mongodb::bson::doc! { "_id": &entry.id })
            .await
            .expect("query audit entry")
            .expect("audit entry exists")
    }

    #[tokio::test]
    async fn audit_no_plaintext_in_any_event() {
        let Some(db) = connect_test_database("audit_no_plaintext_any_event").await else {
            return;
        };

        let kinds = [
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
            RciAuditEventKind::Consumed,
            RciAuditEventKind::Declined {
                reason_present: true,
            },
            RciAuditEventKind::Canceled,
            RciAuditEventKind::Expired,
        ];

        for kind in kinds {
            let entry = insert_rci_event(&db, kind).await;
            let audit_json = serde_json::to_string(&entry).expect("serialize audit log");
            for forbidden in [
                "super-secret-plaintext-fixture",
                "ciphertext-fixture",
                "nonce-fixture",
                "admin-pubkey-fixture",
                "node-pubkey-fixture",
                "hash-fixture",
                "fingerprint-fixture",
                "field-name-fixture",
                "target-url-fixture",
            ] {
                assert!(!audit_json.contains(forbidden), "{forbidden}");
            }
        }
    }

    #[tokio::test]
    async fn audit_no_plaintext_in_error_log() {
        let Some(db) = connect_test_database("audit_no_plaintext_error_event").await else {
            return;
        };

        let kinds = [
            RciAuditEventKind::DecryptFailed,
            RciAuditEventKind::VersionUnsupported,
            RciAuditEventKind::CiphertextTooLarge,
            RciAuditEventKind::PubkeyAwaiting,
            RciAuditEventKind::QueueFull,
            RciAuditEventKind::CiphertextQueued {
                delivery: RciAuditDelivery::OfflineQueue,
                node_offline: true,
            },
        ];

        for kind in kinds {
            let entry = insert_rci_event(&db, kind).await;
            let event_data = entry.event_data.expect("event data");
            let object = event_data.as_object().expect("event data object");
            assert!(object.get("error_code").is_some(), "{kind:?}");
            assert!(object.get("error_kind").is_some(), "{kind:?}");
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
                "raw_node_error",
                "raw_decline_reason",
            ] {
                assert!(
                    !object.contains_key(forbidden_key),
                    "{kind:?}: {forbidden_key}"
                );
            }
            let audit_json = event_data.to_string();
            for forbidden in [
                "super-secret-plaintext-fixture",
                "ciphertext-fixture",
                "nonce-fixture",
                "admin-pubkey-fixture",
                "node-pubkey-fixture",
                "raw-node-error-fixture",
            ] {
                assert!(!audit_json.contains(forbidden), "{forbidden}");
            }
        }
    }
}
