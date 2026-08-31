use std::sync::Arc;

use futures::TryStreamExt;
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
    developer_webhook_dispatcher:
        Option<Arc<crate::services::developer_webhook_service::DeveloperWebhookDispatcher>>,
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
        developer_webhook_dispatcher: Option<
            Arc<crate::services::developer_webhook_service::DeveloperWebhookDispatcher>,
        >,
    ) -> Self {
        Self {
            config,
            http_client,
            fcm_auth,
            apns_auth,
            developer_webhook_dispatcher,
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
    pub(crate) fn with_test_delivery(
        config: Arc<AppConfig>,
        tx: tokio::sync::mpsc::UnboundedSender<ConnectionExpiredNotificationContext>,
    ) -> Self {
        Self {
            config,
            http_client: Client::new(),
            fcm_auth: None,
            apns_auth: None,
            developer_webhook_dispatcher: None,
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
                "$expr": {
                    "$eq": [
                        { "$ifNull": ["$credential_epoch", 1_i64] },
                        api_key.credential_epoch,
                    ]
                },
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

    let audit_error = error_message.chars().take(200).collect();
    spawn_transition_side_effects(
        db.clone(),
        api_key.clone(),
        dead_status.to_string(),
        audit_error,
        notifier.cloned(),
    );
    Ok(true)
}

/// Transition every healthy legacy key shadowing a provider token.
/// Multi-connection keys are excluded because they own independent tokens and
/// use `transition_oauth_key_to_dead` directly during in-place refresh.
pub async fn transition_legacy_oauth_keys_to_dead(
    db: &Database,
    user_id: &str,
    provider_config_id: &str,
    dead_status: &str,
    error_message: &str,
    notifier: Option<&ConnectionExpiryNotifier>,
) -> AppResult<u64> {
    let keys: Vec<UserApiKey> = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "connection_id": null,
            "credential_type": "oauth2",
            "status": "active",
        })
        .await?
        .try_collect()
        .await?;

    let mut transitioned = 0;
    for key in &keys {
        if transition_oauth_key_to_dead(db, key, dead_status, error_message, notifier).await? {
            transitioned += 1;
        }
    }
    Ok(transitioned)
}

fn spawn_transition_side_effects(
    db: Database,
    api_key: UserApiKey,
    dead_status: String,
    audit_error: String,
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
            "error": audit_error,
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

        if let (Some(dispatcher), Some(user_service), Some(app_id)) = (
            notifier
                .as_ref()
                .and_then(|notifier| notifier.developer_webhook_dispatcher.as_deref()),
            user_service.as_ref(),
            user_service
                .as_ref()
                .and_then(|service| service.source_app_id.as_deref()),
        ) {
            let webhook_data = connection_webhook_data(&api_key, user_service, &dead_status);
            dispatcher.dispatch(
                db.clone(),
                app_id.to_string(),
                "connection.expired",
                webhook_data,
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

fn connection_webhook_data(
    api_key: &UserApiKey,
    user_service: &UserService,
    dead_status: &str,
) -> serde_json::Value {
    let mut data = serde_json::json!({
        "user_id": &api_key.user_id,
        "user_service_id": &user_service.id,
        "user_service_slug": &user_service.slug,
        "api_key_id": &api_key.id,
        "status": dead_status,
        "credential_type": &api_key.credential_type,
        "provider_config_id": api_key.provider_config_id.as_deref(),
    });
    if user_service.source.as_deref() == Some("connect_link")
        && let Some(connect_link_id) = user_service.source_id.as_deref()
        && let Some(object) = data.as_object_mut()
    {
        object.insert(
            "connect_link_id".to_string(),
            serde_json::Value::String(connect_link_id.to_string()),
        );
    }
    data
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
    use crate::models::oauth_client::{
        COLLECTION_NAME as OAUTH_CLIENTS, OauthClient, ScopeProvenance,
    };
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use crate::models::ssh_auth_mode::SshAuthMode;
    use crate::models::user_provider_token::{
        COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
    };
    use crate::services::user_token_service;
    use crate::test_utils::{connect_test_database, test_app_config, test_encryption_keys};

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
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            credential_source: Some("platform".to_string()),
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            credential_epoch: 1,
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
            state_version: 1,
            rotation_predecessor_id: None,
        }
    }

    #[test]
    fn expiry_webhook_omits_unresolvable_connect_link_id() {
        let user_id = Uuid::new_v4().to_string();
        let key = oauth_key(&user_id);
        let service = user_service(&user_id, &key.id);
        let data = connection_webhook_data(&key, &service, "expired");
        assert_eq!(data["user_id"], user_id);
        assert!(data.get("connect_link_id").is_none());
    }

    fn webhook_client(
        id: &str,
        owner: &str,
        url: String,
        encrypted_secret: Vec<u8>,
    ) -> OauthClient {
        let now = Utc::now();
        OauthClient {
            id: id.to_string(),
            client_name: "Connection Webhook App".to_string(),
            client_secret_hash: "hash".to_string(),
            redirect_uris: Vec::new(),
            allowed_scopes: "openid".to_string(),
            scope_provenance: ScopeProvenance::Explicit,
            grant_types: "authorization_code".to_string(),
            client_type: "public".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: Some(url),
            connection_webhook_secret_encrypted: Some(encrypted_secret),
            connection_webhook_key_id: None,
            connection_webhook_enabled: true,
            created_by: Some(owner.to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    fn oauth_provider(
        id: &str,
        token_url: &str,
        client_id_encrypted: Vec<u8>,
        client_secret_encrypted: Vec<u8>,
    ) -> ProviderConfig {
        let now = Utc::now();
        ProviderConfig {
            id: id.to_string(),
            slug: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://example.test/authorize".to_string()),
            token_url: Some(token_url.to_string()),
            revocation_url: None,
            revocation: None,
            default_scopes: None,
            client_id_encrypted: Some(client_id_encrypted),
            client_secret_encrypted: Some(client_secret_encrypted),
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            revocation_seed_version: 0,
            created_at: now,
            updated_at: now,
        }
    }

    async fn insert_legacy_refresh_fixture(
        db: &Database,
        token_url: &str,
    ) -> (
        crate::crypto::aes::EncryptionKeys,
        UserApiKey,
        UserProviderToken,
    ) {
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let client_id_encrypted = encryption_keys.encrypt(b"test-client-id").await.unwrap();
        let client_secret_encrypted = encryption_keys
            .encrypt(b"test-client-secret")
            .await
            .unwrap();
        let provider = oauth_provider(
            &provider_id,
            token_url,
            client_id_encrypted,
            client_secret_encrypted,
        );
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(provider)
            .await
            .unwrap();

        let mut key = oauth_key(&user_id);
        key.provider_config_id = Some(provider_id.clone());
        key.connection_id = None;
        let service = user_service(&user_id, &key.id);
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service)
            .await
            .unwrap();

        let now = Utc::now();
        let token = UserProviderToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            provider_config_id: provider_id,
            connection_id: None,
            credential_user_id: None,
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(
                encryption_keys.encrypt(b"existing-access").await.unwrap(),
            ),
            refresh_token_encrypted: Some(
                encryption_keys.encrypt(b"existing-refresh").await.unwrap(),
            ),
            token_scopes: Some("openid".to_string()),
            expires_at: Some(now - chrono::Duration::minutes(1)),
            api_key_encrypted: None,
            status: "active".to_string(),
            state_version: 1,
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: Some("Legacy connection".to_string()),
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(&token)
            .await
            .unwrap();

        (encryption_keys, key, token)
    }

    async fn spawn_token_server(
        response: serde_json::Value,
        status: axum::http::StatusCode,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(move || {
                let response = response.clone();
                async move { (status, axum::Json(response)) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}/token"), server)
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

    async fn latest_event(db: &Database, user_id: &str) -> AuditLog {
        db.collection::<AuditLog>(AUDIT_LOGS)
            .find_one(doc! {
                "user_id": user_id,
                "event_type": "connection_expired",
            })
            .sort(doc! { "created_at": -1 })
            .await
            .expect("find audit event")
            .expect("connection expiry audit event")
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

        let provider_error = "provider refresh rejection ".repeat(12);
        assert!(
            transition_oauth_key_to_dead(&db, &key, "failed", &provider_error, Some(&notifier),)
                .await
                .unwrap()
        );
        let first = timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.service_slug, "github-work");
        wait_for_event_count(&db, &user_id, 1).await;
        let event = latest_event(&db, &user_id).await;
        let expected_error: String = provider_error.chars().take(200).collect();
        assert_eq!(
            event
                .event_data
                .as_ref()
                .and_then(|data| data["error"].as_str()),
            Some(expected_error.as_str())
        );

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
    async fn legacy_transient_refresh_failure_keeps_shadow_key_active() {
        let Some(db) = connect_test_database("connection_expiry_legacy_transient").await else {
            return;
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let token_url = format!("http://{address}/token");
        let (encryption_keys, key, token) = insert_legacy_refresh_fixture(&db, &token_url).await;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let notifier =
            ConnectionExpiryNotifier::with_test_delivery(Arc::new(test_app_config()), tx);

        let active = user_token_service::get_active_token(
            &db,
            &encryption_keys,
            &token.user_id,
            &token.provider_config_id,
            Some(&notifier),
        )
        .await
        .expect("transient refresh failure should fall back to the existing access token");
        assert_eq!(active.access_token.as_deref(), Some("existing-access"));

        let stored_token = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": &token.id })
            .await
            .unwrap()
            .unwrap();
        let stored_key = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token.status, "active");
        assert_eq!(stored_key.status, "active");
        assert_eq!(event_count(&db, &token.user_id).await, 0);
        assert!(
            timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn legacy_http_refresh_failure_notifies_and_audits_once() {
        let Some(db) = connect_test_database("connection_expiry_legacy_http_failure").await else {
            return;
        };
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({ "error": "invalid_grant" }),
            axum::http::StatusCode::BAD_REQUEST,
        )
        .await;
        let (encryption_keys, key, token) = insert_legacy_refresh_fixture(&db, &token_url).await;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let notifier =
            ConnectionExpiryNotifier::with_test_delivery(Arc::new(test_app_config()), tx);

        let active = user_token_service::get_active_token(
            &db,
            &encryption_keys,
            &token.user_id,
            &token.provider_config_id,
            Some(&notifier),
        )
        .await
        .expect("terminal refresh failure should still return the existing access token once");
        assert_eq!(active.access_token.as_deref(), Some("existing-access"));

        let stored_token = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": &token.id })
            .await
            .unwrap()
            .unwrap();
        let stored_key = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token.status, "refresh_failed");
        assert_eq!(stored_key.status, "refresh_failed");
        timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .expect("legacy expiry notification");
        wait_for_event_count(&db, &token.user_id, 1).await;
        let event = latest_event(&db, &token.user_id).await;
        assert!(
            event
                .event_data
                .as_ref()
                .and_then(|data| data["error"].as_str())
                .is_some_and(|error| error.contains("invalid_grant"))
        );

        assert!(matches!(
            user_token_service::get_active_token(
                &db,
                &encryption_keys,
                &token.user_id,
                &token.provider_config_id,
                Some(&notifier),
            )
            .await,
            Err(crate::errors::AppError::NotFound(_))
        ));
        assert!(
            timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        assert_eq!(event_count(&db, &token.user_id).await, 1);
    }

    #[tokio::test]
    async fn notification_and_webhook_delivery_do_not_block_or_fail_dead_transition() {
        let Some(db) = connect_test_database("connection_expiry_delivery_failure").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let key = oauth_key(&user_id);
        let app_id = Uuid::new_v4().to_string();
        let mut service = user_service(&user_id, &key.id);
        service.source_app_id = Some(app_id.clone());
        service.source = Some("connect_link".to_string());
        service.source_id = Some("connect-link-id".to_string());
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();

        let (request_started_tx, mut request_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let receiver = axum::Router::new().route(
            "/events",
            axum::routing::post(move |body: axum::body::Bytes| {
                let request_started_tx = request_started_tx.clone();
                async move {
                    request_started_tx.send(body).expect("record request start");
                    std::future::pending::<axum::http::StatusCode>().await
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind blocking webhook receiver");
        let address = listener.local_addr().expect("webhook receiver address");
        tokio::spawn(async move {
            axum::serve(listener, receiver)
                .await
                .expect("serve blocking webhook receiver")
        });

        let keys = Arc::new(test_encryption_keys());
        let encrypted_secret = keys
            .encrypt(b"webhook-signing-secret")
            .await
            .expect("encrypt webhook secret");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(webhook_client(
                &app_id,
                &user_id,
                format!("http://{address}/events"),
                encrypted_secret,
            ))
            .await
            .expect("insert webhook client");

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        drop(rx);
        let mut notifier =
            ConnectionExpiryNotifier::with_test_delivery(Arc::new(test_app_config()), tx);
        notifier.developer_webhook_dispatcher = Some(Arc::new(
            crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
                reqwest::Client::new(),
                keys,
            ),
        ));

        assert!(
            transition_oauth_key_to_dead(&db, &key, "failed", "refresh rejected", Some(&notifier))
                .await
                .expect("side-effect delivery cannot fail refresh transition")
        );
        let webhook_body = timeout(Duration::from_secs(3), request_started_rx.recv())
            .await
            .expect("webhook delivery should start in the background")
            .expect("webhook request start");
        let envelope: serde_json::Value =
            serde_json::from_slice(&webhook_body).expect("connection expiry envelope");
        assert_eq!(envelope["event_type"], "connection.expired");
        assert_eq!(envelope["data"]["user_id"], user_id);
        assert_eq!(envelope["data"]["connect_link_id"], "connect-link-id");
        assert_eq!(envelope["data"]["status"], "failed");
        assert_eq!(envelope["data"]["user_service_id"], service.id);
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
