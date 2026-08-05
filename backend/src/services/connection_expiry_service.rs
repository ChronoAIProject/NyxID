use std::sync::Arc;

use mongodb::Database;
use mongodb::bson::{self, doc};
use reqwest::Client;

use crate::config::AppConfig;
use crate::errors::AppResult;
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::audit_service::{self, AuditActor};
use crate::services::notification_service::{self, ConnectionExpiredNotificationContext};
use crate::services::push_service::{ApnsAuth, FcmAuth};

#[derive(Clone)]
pub struct ConnectionExpiryNotifier {
    config: Arc<AppConfig>,
    http_client: Client,
    fcm_auth: Option<Arc<FcmAuth>>,
    apns_auth: Option<Arc<ApnsAuth>>,
    #[cfg(test)]
    test_delivery_tx:
        Option<tokio::sync::mpsc::UnboundedSender<ConnectionExpiredNotificationContext>>,
}

impl ConnectionExpiryNotifier {
    pub fn new(
        config: Arc<AppConfig>,
        http_client: Client,
        fcm_auth: Option<Arc<FcmAuth>>,
        apns_auth: Option<Arc<ApnsAuth>>,
    ) -> Self {
        Self {
            config,
            http_client,
            fcm_auth,
            apns_auth,
            #[cfg(test)]
            test_delivery_tx: None,
        }
    }

    fn enabled(&self) -> bool {
        self.config.connection_expiry_notifications
    }

    async fn send(
        &self,
        db: &Database,
        recipient_user_id: &str,
        context: &ConnectionExpiredNotificationContext,
    ) -> AppResult<()> {
        #[cfg(test)]
        if let Some(tx) = &self.test_delivery_tx {
            tx.send(context.clone()).map_err(|_| {
                crate::errors::AppError::Internal(
                    "connection-expiry test notification receiver closed".to_string(),
                )
            })?;
            return Ok(());
        }

        notification_service::send_connection_expired_notification(
            db,
            &self.config,
            &self.http_client,
            self.fcm_auth.as_deref(),
            self.apns_auth.as_deref(),
            recipient_user_id,
            context,
        )
        .await?;
        Ok(())
    }

    #[cfg(test)]
    fn with_test_delivery(
        config: Arc<AppConfig>,
        tx: tokio::sync::mpsc::UnboundedSender<ConnectionExpiredNotificationContext>,
    ) -> Self {
        Self {
            config,
            http_client: Client::new(),
            fcm_auth: None,
            apns_auth: None,
            test_delivery_tx: Some(tx),
        }
    }
}

/// Atomically transition a healthy OAuth-backed key to a dead status.
///
/// The compare-and-set is also the exactly-once gate for the audit and user
/// notification. A key that is already dead, revoked, or concurrently
/// refreshed does not emit side effects. Re-authorizing the key moves it back
/// to `active`, allowing a later independent failure to notify again.
pub async fn transition_oauth_key_to_dead(
    db: &Database,
    api_key: &UserApiKey,
    dead_status: &str,
    error_message: &str,
    notifier: Option<&ConnectionExpiryNotifier>,
) -> AppResult<bool> {
    let now = chrono::Utc::now();
    let result = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .update_one(
            doc! {
                "_id": &api_key.id,
                "updated_at": bson::DateTime::from_chrono(api_key.updated_at),
                "credential_type": "oauth2",
                "status": "active",
            },
            doc! { "$set": {
                "status": dead_status,
                "error_message": error_message,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.modified_count == 0 {
        return Ok(false);
    }

    spawn_transition_side_effects(db.clone(), api_key.clone(), notifier.cloned());
    Ok(true)
}

fn spawn_transition_side_effects(
    db: Database,
    api_key: UserApiKey,
    notifier: Option<ConnectionExpiryNotifier>,
) {
    tokio::spawn(async move {
        let user_service = match db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! {
                "user_id": &api_key.user_id,
                "api_key_id": &api_key.id,
            })
            .sort(doc! { "created_at": 1, "_id": 1 })
            .await
        {
            Ok(service) => service,
            Err(error) => {
                tracing::warn!(
                    api_key_id = %api_key.id,
                    error = %error,
                    "Failed to resolve service metadata for expired connection"
                );
                None
            }
        };

        let event_data = serde_json::json!({
            "user_id": &api_key.user_id,
            "user_service_id": user_service.as_ref().map(|service| service.id.as_str()),
            "user_service_slug": user_service.as_ref().map(|service| service.slug.as_str()),
            "api_key_id": &api_key.id,
            "provider_config_id": api_key.provider_config_id.as_deref(),
            "credential_type": &api_key.credential_type,
        });
        let actor = AuditActor {
            user_id: api_key.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        if let Err(error) = audit_service::log_actor_event(
            db.clone(),
            &actor,
            "connection_expired",
            Some(event_data),
        )
        .await
        {
            tracing::warn!(
                api_key_id = %api_key.id,
                error = %error,
                "Failed to persist connection expiry audit event"
            );
        }

        let (Some(notifier), Some(user_service)) = (notifier, user_service) else {
            return;
        };
        if !notifier.enabled() {
            return;
        }

        let context = ConnectionExpiredNotificationContext {
            service_label: api_key.label.clone(),
            service_slug: user_service.slug,
            user_service_id: user_service.id,
            api_key_id: api_key.id.clone(),
        };
        let mut recipients =
            match crate::services::org_service::list_admin_user_ids(&db, &api_key.user_id).await {
                Ok(recipients) => recipients,
                Err(error) => {
                    tracing::warn!(
                        owner_id = %api_key.user_id,
                        error = %error,
                        "Failed to resolve org admins for connection expiry notification"
                    );
                    Vec::new()
                }
            };
        if recipients.is_empty() {
            recipients.push(api_key.user_id.clone());
        }

        for recipient in recipients {
            if let Err(error) = notifier.send(&db, &recipient, &context).await {
                tracing::warn!(
                    user_id = %recipient,
                    api_key_id = %api_key.id,
                    error = %error,
                    "Connection expiry notification delivery failed"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use mongodb::bson::doc;
    use tokio::time::{Duration, timeout};
    use uuid::Uuid;

    use super::*;
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOGS};
    use crate::models::ssh_auth_mode::SshAuthMode;
    use crate::test_utils::{connect_test_database, test_app_config};

    fn oauth_key(user_id: &str) -> UserApiKey {
        let now = Utc::now();
        UserApiKey {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            label: "GitHub work account".to_string(),
            credential_type: "oauth2".to_string(),
            credential_encrypted: None,
            access_token_encrypted: Some(vec![1]),
            refresh_token_encrypted: Some(vec![2]),
            token_scopes: None,
            expires_at: Some(now),
            provider_config_id: Some("github-provider".to_string()),
            connection_id: Some(Uuid::new_v4().to_string()),
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            credential_source: Some("platform".to_string()),
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn user_service(user_id: &str, api_key_id: &str) -> UserService {
        let now = Utc::now();
        UserService {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            slug: "github-work".to_string(),
            endpoint_id: Uuid::new_v4().to_string(),
            api_key_id: Some(api_key_id.to_string()),
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            catalog_service_id: Some("github-catalog".to_string()),
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            ssh_auth_mode: SshAuthMode::ProxyOnly,
            admin_only: false,
            ssh_node_keys_stale: false,
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            is_active: true,
            source: Some("auto_provision".to_string()),
            source_id: None,
            source_app_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    async fn event_count(db: &Database, user_id: &str) -> u64 {
        db.collection::<AuditLog>(AUDIT_LOGS)
            .count_documents(doc! {
                "user_id": user_id,
                "event_type": "connection_expired",
            })
            .await
            .expect("count audit events")
    }

    async fn wait_for_event_count(db: &Database, user_id: &str, expected: u64) {
        timeout(Duration::from_secs(3), async {
            loop {
                if event_count(db, user_id).await == expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("connection expiry audit event should be persisted");
    }

    #[tokio::test]
    async fn transition_notifies_and_audits_once_then_recovery_allows_next_death() {
        let Some(db) = connect_test_database("connection_expiry_transition").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let key = oauth_key(&user_id);
        let service = user_service(&user_id, &key.id);
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let notifier =
            ConnectionExpiryNotifier::with_test_delivery(Arc::new(test_app_config()), tx);

        assert!(
            transition_oauth_key_to_dead(&db, &key, "failed", "refresh rejected", Some(&notifier))
                .await
                .unwrap()
        );
        let first = timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.service_slug, "github-work");
        wait_for_event_count(&db, &user_id, 1).await;

        assert!(
            !transition_oauth_key_to_dead(
                &db,
                &key,
                "failed",
                "refresh rejected again",
                Some(&notifier),
            )
            .await
            .unwrap()
        );
        assert!(
            timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        assert_eq!(event_count(&db, &user_id).await, 1);

        let recovered_at = Utc::now() + chrono::Duration::milliseconds(10);
        db.collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! { "_id": &key.id },
                doc! { "$set": {
                    "status": "active",
                    "updated_at": bson::DateTime::from_chrono(recovered_at),
                }},
            )
            .await
            .unwrap();
        let recovered = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert!(
            transition_oauth_key_to_dead(
                &db,
                &recovered,
                "failed",
                "refresh rejected after reconnect",
                Some(&notifier),
            )
            .await
            .unwrap()
        );
        timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .expect("second notification after recovery");
        wait_for_event_count(&db, &user_id, 2).await;
    }

    #[tokio::test]
    async fn notification_failure_does_not_fail_dead_transition() {
        let Some(db) = connect_test_database("connection_expiry_delivery_failure").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let key = oauth_key(&user_id);
        let service = user_service(&user_id, &key.id);
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        drop(rx);
        let notifier =
            ConnectionExpiryNotifier::with_test_delivery(Arc::new(test_app_config()), tx);

        assert!(
            transition_oauth_key_to_dead(&db, &key, "failed", "refresh rejected", Some(&notifier))
                .await
                .expect("notification delivery cannot fail refresh transition")
        );
        wait_for_event_count(&db, &user_id, 1).await;
        let stored = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, "failed");
    }
}
