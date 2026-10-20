//! Developer-app connection lifecycle webhook configuration and delivery.

use chrono::{DateTime, Utc};
use mongodb::{ClientSession, Database, bson::doc, options::ReturnDocument};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::token::generate_random_token;
use crate::errors::{AppError, AppResult};
use crate::models::audit_outbox::{AuditOwner, AuditPrincipal, AuditStage};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::services::audit_service;
use crate::services::coordination_service::RateWindowStore;
use crate::services::webhook_delivery_service::{self, DeliveryFailure, SignatureContract};
use crate::services::{
    audit_outbox_service::{self, AuditEvent},
    audit_runtime::{AuditRuntime, policy::AuditTier, transaction},
};

pub const CONNECTION_WEBHOOK_SECRET_PREFIX: &str = "nyx_cwh_";
pub const CONNECTION_WEBHOOK_MAX_BODY_BYTES: usize = 16 * 1024;
const APP_WEBHOOK_MAX_PER_MINUTE: u32 = 120;

#[derive(Clone)]
pub struct DeveloperWebhookDispatcher {
    http_client: reqwest::Client,
    encryption_keys: Arc<EncryptionKeys>,
}

impl std::fmt::Debug for DeveloperWebhookDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeveloperWebhookDispatcher")
            .field("http_client", &"configured")
            .field("encryption_keys", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
pub struct ConnectionWebhookEnvelope {
    pub event_id: String,
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub data: Value,
}

impl DeveloperWebhookDispatcher {
    pub fn new(http_client: reqwest::Client, encryption_keys: Arc<EncryptionKeys>) -> Self {
        Self {
            http_client,
            encryption_keys,
        }
    }

    pub fn dispatch(&self, db: Database, app_id: String, event_type: &str, data: Value) {
        let dispatcher = self.clone();
        let event_type = event_type.to_string();
        let event_id = Uuid::new_v4().to_string();
        tokio::spawn(async move {
            if let Err(failure) = dispatcher
                .deliver_for_app(&db, &app_id, &event_id, &event_type, data)
                .await
            {
                record_final_failure_for_app(&db, &app_id, &event_id, &event_type, failure).await;
            }
        });
    }

    pub async fn deliver_for_app(
        &self,
        db: &Database,
        app_id: &str,
        event_id: &str,
        event_type: &str,
        data: Value,
    ) -> Result<(), DeliveryFailure> {
        self.deliver_for_app_inner(db, app_id, event_id, event_type, data, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn deliver_for_app_hardened(
        &self,
        db: &Database,
        app_id: &str,
        event_id: &str,
        event_type: &str,
        data: Value,
        occurred_at: DateTime<Utc>,
    ) -> Result<(), DeliveryFailure> {
        self.deliver_for_app_inner(db, app_id, event_id, event_type, data, Some(occurred_at))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn deliver_for_app_inner(
        &self,
        db: &Database,
        app_id: &str,
        event_id: &str,
        event_type: &str,
        data: Value,
        occurred_at: Option<DateTime<Utc>>,
    ) -> Result<(), DeliveryFailure> {
        if data
            .get("user_id")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|value| value.is_empty())
        {
            return Err(DeliveryFailure {
                attempts: 0,
                reason: "tenant_identity_missing",
                last_status: None,
            });
        }
        let mut client = match db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": app_id, "is_active": true })
            .await
        {
            Ok(Some(client)) => client,
            Ok(None) => return Ok(()),
            Err(error) => {
                tracing::warn!(app_id, %error, "failed to load developer webhook configuration");
                return Err(DeliveryFailure {
                    attempts: 0,
                    reason: "configuration_load_failed",
                    last_status: None,
                });
            }
        };
        if !client.connection_webhook_enabled {
            return Ok(());
        }
        let admitted = match RateWindowStore::admit(
            db,
            if occurred_at.is_some() {
                "developer_channel_webhook"
            } else {
                "developer_webhook"
            },
            app_id,
            u64::from(APP_WEBHOOK_MAX_PER_MINUTE),
            std::time::Duration::from_secs(60),
        )
        .await
        {
            Ok(admission) => admission.allowed,
            Err(error) => {
                tracing::error!(app_id, %error, "developer webhook rate admission failed");
                return Err(DeliveryFailure {
                    attempts: 0,
                    reason: "rate_limit_unavailable",
                    last_status: None,
                });
            }
        };
        if !admitted {
            return Err(DeliveryFailure {
                attempts: 0,
                reason: "app_rate_limited",
                last_status: None,
            });
        }
        if client.connection_webhook_key_id.is_none() {
            let generated = webhook_delivery_service::generate_signing_key_id();
            client = match db
                .collection::<OauthClient>(OAUTH_CLIENTS)
                .find_one_and_update(
                    doc! {
                        "_id": app_id,
                        "$or": [
                            { "connection_webhook_key_id": null },
                            { "connection_webhook_key_id": { "$exists": false } },
                        ],
                    },
                    doc! { "$set": { "connection_webhook_key_id": &generated } },
                )
                .return_document(ReturnDocument::After)
                .await
            {
                Ok(Some(updated)) => updated,
                Ok(None) => db
                    .collection::<OauthClient>(OAUTH_CLIENTS)
                    .find_one(doc! { "_id": app_id, "is_active": true })
                    .await
                    .ok()
                    .flatten()
                    .ok_or(DeliveryFailure {
                        attempts: 0,
                        reason: "key_id_load_failed",
                        last_status: None,
                    })?,
                Err(error) => {
                    tracing::warn!(app_id, %error, "failed to persist webhook signing key id");
                    return Err(DeliveryFailure {
                        attempts: 0,
                        reason: "key_id_store_failed",
                        last_status: None,
                    });
                }
            };
        }
        let (Some(url), Some(encrypted_secret), Some(key_id)) = (
            client.connection_webhook_url.as_deref(),
            client.connection_webhook_secret_encrypted.as_deref(),
            client.connection_webhook_key_id.as_deref(),
        ) else {
            return Ok(());
        };
        let secret = match self.encryption_keys.decrypt(encrypted_secret).await {
            Ok(secret) => Zeroizing::new(secret),
            Err(error) => {
                tracing::warn!(app_id, %error, "failed to decrypt developer webhook secret");
                return Err(DeliveryFailure {
                    attempts: 0,
                    reason: "secret_decrypt_failed",
                    last_status: None,
                });
            }
        };
        let envelope = ConnectionWebhookEnvelope {
            event_id: event_id.to_string(),
            event_type: event_type.to_string(),
            occurred_at: occurred_at.unwrap_or_else(Utc::now),
            data,
        };
        let body = match serde_json::to_vec(&envelope) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(app_id, %error, "failed to serialize developer webhook event");
                return Err(DeliveryFailure {
                    attempts: 0,
                    reason: "serialization_failed",
                    last_status: None,
                });
            }
        };
        if body.len() > CONNECTION_WEBHOOK_MAX_BODY_BYTES {
            return Err(DeliveryFailure {
                attempts: 0,
                reason: "body_too_large",
                last_status: None,
            });
        }

        let hardened_client = if occurred_at.is_some() {
            Some(
                super::channel_media_service::media_client(url, None, None)
                    .await
                    .map_err(|_| DeliveryFailure {
                        attempts: 0,
                        reason: "destination_unavailable",
                        last_status: None,
                    })?,
            )
        } else {
            None
        };
        webhook_delivery_service::deliver_signed_body(
            hardened_client.as_ref().unwrap_or(&self.http_client),
            url,
            secret.as_slice(),
            event_type,
            event_id,
            &body,
            SignatureContract::Timestamped,
            Some(key_id),
        )
        .await?;
        tracing::debug!(app_id, event_id, event_type, "developer webhook delivered");
        Ok(())
    }
}

async fn record_final_failure_for_app(
    db: &Database,
    app_id: &str,
    event_id: &str,
    event_type: &str,
    failure: DeliveryFailure,
) {
    let client = match db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": app_id })
        .await
    {
        Ok(Some(client)) => client,
        _ => {
            tracing::error!(
                app_id,
                event_id,
                event_type,
                "developer webhook delivery exhausted"
            );
            return;
        }
    };
    tracing::error!(
        app_id = %client.id,
        event_id,
        event_type,
        attempts = failure.attempts,
        reason = failure.reason,
        last_status = failure.last_status,
        "developer webhook delivery exhausted"
    );
    audit_service::log_async(
        db.clone(),
        client.created_by.clone(),
        "connection_webhook_delivery_failed".to_string(),
        Some(serde_json::json!({
            "app_id": &client.id,
            "event_id": event_id,
            "event_type": event_type,
            "attempts": failure.attempts,
            "reason": failure.reason,
            "last_status": failure.last_status,
        })),
        None,
        None,
        None,
        None,
    );
}

pub async fn record_terminal_delivery_failure(
    db: &Database,
    app_id: &str,
    event_id: &str,
    event_type: &str,
    failure: DeliveryFailure,
) {
    record_final_failure_for_app(db, app_id, event_id, event_type, failure).await;
}

pub async fn configure(
    db: &Database,
    runtime: &AuditRuntime,
    encryption_keys: &EncryptionKeys,
    principal: &AuditPrincipal,
    actor_id: &str,
    client_id: &str,
    url: &str,
) -> AppResult<(OauthClient, Zeroizing<String>, String)> {
    let url = webhook_delivery_service::validate_webhook_url(url, "connection_webhook_url").await?;
    store_configuration(
        db,
        runtime,
        encryption_keys,
        principal,
        actor_id,
        client_id,
        &url,
    )
    .await
}

struct WebhookSecret {
    raw: Zeroizing<String>,
    encrypted: Zeroizing<Vec<u8>>,
    key_id: String,
}

impl WebhookSecret {
    async fn prepare(keys: &EncryptionKeys) -> AppResult<Self> {
        let raw = Zeroizing::new(format!(
            "{CONNECTION_WEBHOOK_SECRET_PREFIX}{}",
            generate_random_token()
        ));
        Ok(Self {
            encrypted: Zeroizing::new(keys.encrypt(raw.as_bytes()).await?),
            raw,
            key_id: webhook_delivery_service::generate_signing_key_id(),
        })
    }
}

#[derive(Clone, Copy)]
enum WebhookChange<'a> {
    Configure(&'a str),
    Rotate,
    Disable,
}

struct WebhookConfiguration<'a> {
    db: &'a Database,
    runtime: &'a AuditRuntime,
    principal: &'a AuditPrincipal,
    actor_id: &'a str,
    client_id: &'a str,
    change: WebhookChange<'a>,
    secret: Option<&'a WebhookSecret>,
    event_id: Uuid,
    operation_id: Uuid,
    at: DateTime<Utc>,
}

impl<'a> WebhookConfiguration<'a> {
    fn new(
        db: &'a Database,
        runtime: &'a AuditRuntime,
        principal: &'a AuditPrincipal,
        actor_id: &'a str,
        client_id: &'a str,
        change: WebhookChange<'a>,
        secret: Option<&'a WebhookSecret>,
    ) -> Self {
        Self {
            db,
            runtime,
            principal,
            actor_id,
            client_id,
            change,
            secret,
            event_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            at: Utc::now(),
        }
    }

    async fn commit(&self) -> AppResult<OauthClient> {
        transaction::run(self.db, self, |session, change| {
            Box::pin(configuration_in_session(session, change))
        })
        .await
    }
}

async fn store_configuration(
    db: &Database,
    runtime: &AuditRuntime,
    encryption_keys: &EncryptionKeys,
    principal: &AuditPrincipal,
    actor_id: &str,
    client_id: &str,
    url: &str,
) -> AppResult<(OauthClient, Zeroizing<String>, String)> {
    let secret = WebhookSecret::prepare(encryption_keys).await?;
    let updated = WebhookConfiguration::new(
        db,
        runtime,
        principal,
        actor_id,
        client_id,
        WebhookChange::Configure(url),
        Some(&secret),
    )
    .commit()
    .await?;
    // No secret is returned until the business mutation and safe audit commit.
    Ok((updated, secret.raw, secret.key_id))
}

pub async fn rotate_secret(
    db: &Database,
    runtime: &AuditRuntime,
    encryption_keys: &EncryptionKeys,
    principal: &AuditPrincipal,
    actor_id: &str,
    client_id: &str,
) -> AppResult<(OauthClient, Zeroizing<String>, String)> {
    let secret = WebhookSecret::prepare(encryption_keys).await?;
    let updated = WebhookConfiguration::new(
        db,
        runtime,
        principal,
        actor_id,
        client_id,
        WebhookChange::Rotate,
        Some(&secret),
    )
    .commit()
    .await?;
    Ok((updated, secret.raw, secret.key_id))
}

pub async fn disable(
    db: &Database,
    runtime: &AuditRuntime,
    principal: &AuditPrincipal,
    actor_id: &str,
    client_id: &str,
) -> AppResult<OauthClient> {
    WebhookConfiguration::new(
        db,
        runtime,
        principal,
        actor_id,
        client_id,
        WebhookChange::Disable,
        None,
    )
    .commit()
    .await
}

async fn configuration_in_session(
    session: &mut ClientSession,
    change: &WebhookConfiguration<'_>,
) -> AppResult<OauthClient> {
    use crate::models::{org_membership, user};
    use crate::services::org_service::{self, OwnerAccess};

    let db = change.db;
    let acting_owner = match change.principal {
        AuditPrincipal::Person { id } => id,
        AuditPrincipal::ApiKey { owner_id, .. } => owner_id,
        _ => return Err(AppError::Forbidden("Developer app owner required".into())),
    };
    if acting_owner != change.actor_id {
        return Err(AppError::Forbidden("Developer app owner required".into()));
    }
    let client = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! {"_id": change.client_id, "is_active": true})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".into()))?;
    let owner_id = client
        .created_by
        .as_deref()
        .ok_or_else(|| AppError::NotFound("OAuth client not found".into()))?;
    let access =
        org_service::resolve_owner_access_in_session(db, session, change.actor_id, owner_id)
            .await?;
    if !access.can_read() {
        return Err(AppError::NotFound("OAuth client not found".into()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this OAuth client".into(),
        ));
    }
    let owner = db
        .collection::<user::User>(user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner_id, "is_active": true})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".into()))?;
    if matches!(change.change, WebhookChange::Rotate)
        && (!client.connection_webhook_enabled
            || client.connection_webhook_url.is_none()
            || client.connection_webhook_secret_encrypted.is_none())
    {
        return Err(AppError::NotFound(
            "Connection webhook not configured".into(),
        ));
    }
    if matches!(change.change, WebhookChange::Disable)
        && !client.connection_webhook_enabled
        && client.connection_webhook_secret_encrypted.is_none()
        && client.connection_webhook_key_id.is_none()
    {
        return Ok(client);
    }
    // Snapshot reads alone do not fence a concurrent membership revocation or
    // owner deletion. Touch the same documents so contention retries the entire
    // authority check and mutation in a fresh snapshot.
    if let OwnerAccess::AsOrgAdmin { membership_id, .. } = access {
        let fenced = db
            .collection::<bson::Document>(org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"_id": membership_id, "revoked_at": bson::Bson::Null, "role": "admin"},
                doc! {"$inc": {"audit_mutation_fence": 1_i64}},
            )
            .session(&mut *session)
            .await?;
        if fenced.matched_count != 1 {
            return Err(AppError::AuditIntegrityConflict);
        }
    }
    let fenced = db
        .collection::<bson::Document>(user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": owner_id, "is_active": true},
            doc! {"$inc": {"audit_mutation_fence": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if fenced.matched_count != 1 {
        return Err(AppError::AuditIntegrityConflict);
    }
    let mut set = doc! {"updated_at": bson::DateTime::from_chrono(change.at)};
    let (action, tier, update) = match change.change {
        WebhookChange::Disable => {
            set.insert("connection_webhook_enabled", false);
            (
                "connection_webhook_disabled",
                AuditTier::Narrowing,
                doc! {
                    "$set": set,
                    "$unset": {"connection_webhook_secret_encrypted": "", "connection_webhook_key_id": ""},
                },
            )
        }
        WebhookChange::Configure(_) | WebhookChange::Rotate => {
            let secret = change.secret.ok_or(AppError::AuditIntegrityConflict)?;
            set.insert(
                "connection_webhook_secret_encrypted",
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: secret.encrypted.to_vec(),
                },
            );
            set.insert("connection_webhook_key_id", &secret.key_id);
            set.insert("connection_webhook_enabled", true);
            let action = if let WebhookChange::Configure(url) = change.change {
                set.insert("connection_webhook_url", url);
                "connection_webhook_configured"
            } else {
                "connection_webhook_secret_rotated"
            };
            (action, AuditTier::Standard, doc! {"$set": set})
        }
    };
    let updated = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one_and_update(
            doc! {"_id": change.client_id, "created_by": owner_id, "is_active": true},
            update,
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
        .ok_or(AppError::AuditIntegrityConflict)?;
    let event = AuditEvent {
        operation_id: change.operation_id.to_string(),
        stage: AuditStage::Committed,
        principal: change.principal.clone(),
        owner: if owner.user_type.is_org() {
            AuditOwner::Org { id: owner.id }
        } else {
            AuditOwner::Person { id: owner.id }
        },
        action,
        target_id: Some(client.id),
    };
    audit_outbox_service::enqueue_admitted_in_session(
        db,
        session,
        change.runtime,
        tier,
        &change.event_id,
        &event,
    )
    .await?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
    use chrono::{TimeZone, Utc};

    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOGS};
    use crate::models::oauth_client::ScopeProvenance;
    use crate::test_utils::{connect_test_database, test_encryption_keys};

    fn client(id: &str, owner: &str) -> OauthClient {
        let now = Utc::now();
        OauthClient {
            id: id.to_string(),
            client_name: "Webhook Test App".to_string(),
            client_secret_hash: "redacted-hash".to_string(),
            redirect_uris: vec!["https://app.example.test/callback".to_string()],
            allowed_scopes: "openid".to_string(),
            scope_provenance: ScopeProvenance::Explicit,
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: Some(owner.to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn connection_webhook_envelope_timestamp_wire_format_is_stable() {
        let envelope = ConnectionWebhookEnvelope {
            event_id: "3cc24472-c0b4-436c-a42a-17f43087f3e7".to_string(),
            event_type: "connect_link.completed".to_string(),
            occurred_at: Utc
                .with_ymd_and_hms(2026, 8, 6, 9, 30, 0)
                .single()
                .expect("fixture timestamp")
                + chrono::Duration::milliseconds(123),
            data: serde_json::json!({
                "user_id": "user-uuid",
                "connect_link_id": "6c02c84a-3d97-430f-8468-c96b609d9563",
                "service_id": "catalog-service-id",
                "service_slug": "service-slug",
                "status": "completed",
                "user_service_id": "user-service-id",
                "completed_at": "2026-08-06T09:29:59.000Z",
                "expires_at": "2026-08-06T09:45:00.000Z",
            }),
        };

        assert_eq!(
            serde_json::to_string(&envelope).expect("serialize envelope"),
            r#"{"event_id":"3cc24472-c0b4-436c-a42a-17f43087f3e7","event_type":"connect_link.completed","occurred_at":"2026-08-06T09:30:00.123Z","data":{"user_id":"user-uuid","connect_link_id":"6c02c84a-3d97-430f-8468-c96b609d9563","service_id":"catalog-service-id","service_slug":"service-slug","status":"completed","user_service_id":"user-service-id","completed_at":"2026-08-06T09:29:59.000Z","expires_at":"2026-08-06T09:45:00.000Z"}}"#
        );
    }

    #[tokio::test]
    async fn connection_webhook_body_ceiling_is_enforced_before_send() {
        let Some(db) = connect_test_database("developer_connection_webhook_body_ceiling").await
        else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let client_id = uuid::Uuid::new_v4().to_string();
        let keys = Arc::new(test_encryption_keys());
        let mut configured = client(&client_id, &owner);
        configured.connection_webhook_url = Some("http://127.0.0.1:1/events".to_string());
        configured.connection_webhook_secret_encrypted =
            Some(keys.encrypt(b"body-limit-secret").await.unwrap());
        configured.connection_webhook_key_id = Some("key_body_limit".to_string());
        configured.connection_webhook_enabled = true;
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(configured)
            .await
            .unwrap();

        let failure = DeveloperWebhookDispatcher::new(reqwest::Client::new(), keys)
            .deliver_for_app(
                &db,
                &client_id,
                "event-id",
                "connect_link.completed",
                serde_json::json!({
                    "user_id": "user-id",
                    "oversized": "x".repeat(CONNECTION_WEBHOOK_MAX_BODY_BYTES),
                }),
            )
            .await
            .expect_err("oversized metadata envelope must be rejected");
        assert_eq!(failure.reason, "body_too_large");
        assert_eq!(failure.attempts, 0);
    }

    #[tokio::test]
    async fn configuration_returns_secret_once_rotation_replaces_it_and_disable_clears_it() {
        let Some(db) = connect_test_database("developer_connection_webhook_config").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let client_id = uuid::Uuid::new_v4().to_string();
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(client(&client_id, &owner))
            .await
            .expect("insert client");
        let keys = test_encryption_keys();
        let runtime = AuditRuntime::new(Default::default(), false);
        let principal = AuditPrincipal::Person { id: owner.clone() };
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();

        let (configured, first_secret, first_key_id) = store_configuration(
            &db,
            &runtime,
            &keys,
            &principal,
            &owner,
            &client_id,
            "https://receiver.example.test/events",
        )
        .await
        .expect("configure webhook");
        assert!(first_secret.starts_with(CONNECTION_WEBHOOK_SECRET_PREFIX));
        assert!(configured.connection_webhook_enabled);
        assert_eq!(
            configured.connection_webhook_key_id.as_deref(),
            Some(first_key_id.as_str())
        );
        assert!(!format!("{configured:?}").contains(first_secret.as_str()));
        let stored_first = Zeroizing::new(
            keys.decrypt(
                configured
                    .connection_webhook_secret_encrypted
                    .as_deref()
                    .expect("encrypted secret"),
            )
            .await
            .expect("decrypt first secret"),
        );
        assert_eq!(stored_first.as_slice(), first_secret.as_bytes());

        let (rotated, second_secret, second_key_id) =
            rotate_secret(&db, &runtime, &keys, &principal, &owner, &client_id)
                .await
                .expect("rotate webhook secret");
        assert_ne!(first_secret.as_str(), second_secret.as_str());
        assert_ne!(first_key_id, second_key_id);
        let stored_second = Zeroizing::new(
            keys.decrypt(
                rotated
                    .connection_webhook_secret_encrypted
                    .as_deref()
                    .expect("rotated encrypted secret"),
            )
            .await
            .expect("decrypt rotated secret"),
        );
        assert_eq!(stored_second.as_slice(), second_secret.as_bytes());

        let disabled = disable(&db, &runtime, &principal, &owner, &client_id)
            .await
            .expect("disable webhook");
        assert!(!disabled.connection_webhook_enabled);
        assert!(disabled.connection_webhook_secret_encrypted.is_none());
        assert!(matches!(
            rotate_secret(&db, &runtime, &keys, &principal, &owner, &client_id).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn lifecycle_delivery_uses_timestamp_bound_signature_and_metadata_envelope() {
        let Some(db) = connect_test_database("developer_connection_webhook_delivery").await else {
            return;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(HeaderMap, Bytes)>();
        let app = Router::new().route(
            "/events",
            post(move |headers: HeaderMap, body: Bytes| {
                let tx = tx.clone();
                async move {
                    tx.send((headers, body)).expect("capture delivery");
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind receiver");
        let address = listener.local_addr().expect("receiver address");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve receiver") });

        let owner = uuid::Uuid::new_v4().to_string();
        let client_id = uuid::Uuid::new_v4().to_string();
        let keys = Arc::new(test_encryption_keys());
        let raw_secret = "nyx_cwh_fixture-secret";
        let mut configured = client(&client_id, &owner);
        configured.connection_webhook_url = Some(format!("http://{address}/events"));
        configured.connection_webhook_secret_encrypted = Some(
            keys.encrypt(raw_secret.as_bytes())
                .await
                .expect("encrypt signing secret"),
        );
        configured.connection_webhook_key_id = Some("key_fixture".to_string());
        configured.connection_webhook_enabled = true;
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(configured)
            .await
            .expect("insert configured client");

        let dispatcher = DeveloperWebhookDispatcher::new(reqwest::Client::new(), keys);
        dispatcher
            .deliver_for_app(
                &db,
                &client_id,
                "event-id",
                "connect_link.completed",
                serde_json::json!({
                    "user_id": &owner,
                    "connect_link_id": "link-id",
                    "service_slug": "github",
                    "status": "completed",
                }),
            )
            .await
            .expect("deliver lifecycle webhook");

        let (headers, body) = rx.recv().await.expect("captured delivery");
        let timestamp = headers
            .get("X-NyxID-Timestamp")
            .and_then(|value| value.to_str().ok())
            .expect("timestamp header");
        let supplied_signature = headers
            .get("X-NyxID-Signature")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("sha256="))
            .expect("signature header");
        let expected_signature = webhook_delivery_service::compute_timestamped_signature(
            raw_secret.as_bytes(),
            timestamp,
            &body,
        );
        assert_eq!(supplied_signature, expected_signature);
        assert_eq!(headers["X-NyxID-Event"], "connect_link.completed");
        assert_eq!(headers["X-NyxID-Delivery-Id"], "event-id");
        assert_eq!(headers["X-NyxID-Key-Id"], "key_fixture");

        let envelope: serde_json::Value =
            serde_json::from_slice(&body).expect("parse lifecycle envelope");
        assert_eq!(envelope["event_id"], "event-id");
        assert_eq!(envelope["event_type"], "connect_link.completed");
        assert_eq!(envelope["data"]["connect_link_id"], "link-id");
        assert_eq!(envelope["data"]["user_id"], owner);
        assert_eq!(envelope["data"]["status"], "completed");
        assert!(envelope["occurred_at"].is_string());
    }

    #[tokio::test]
    async fn exhausted_delivery_retries_emit_metadata_only_failure_audit() {
        let Some(db) = connect_test_database("developer_connection_webhook_failure_audit").await
        else {
            return;
        };
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler_attempts = attempts.clone();
        let app = Router::new().route(
            "/events",
            post(move || {
                let handler_attempts = handler_attempts.clone();
                async move {
                    handler_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind failing receiver");
        let address = listener.local_addr().expect("failing receiver address");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve receiver") });

        let owner = uuid::Uuid::new_v4().to_string();
        let client_id = uuid::Uuid::new_v4().to_string();
        let keys = Arc::new(test_encryption_keys());
        let raw_secret = "nyx_cwh_failure-secret";
        let mut configured = client(&client_id, &owner);
        configured.connection_webhook_url = Some(format!("http://{address}/events"));
        configured.connection_webhook_secret_encrypted = Some(
            keys.encrypt(raw_secret.as_bytes())
                .await
                .expect("encrypt signing secret"),
        );
        configured.connection_webhook_key_id = Some("key_failure".to_string());
        configured.connection_webhook_enabled = true;
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(configured)
            .await
            .expect("insert configured client");

        let failure = DeveloperWebhookDispatcher::new(reqwest::Client::new(), keys)
            .deliver_for_app(
                &db,
                &client_id,
                "event-id",
                "connect_link.expired",
                serde_json::json!({
                    "user_id": &owner,
                    "connect_link_id": "private-event-payload",
                }),
            )
            .await
            .expect_err("receiver rejects delivery");
        record_terminal_delivery_failure(
            &db,
            &client_id,
            "event-id",
            "connect_link.expired",
            failure,
        )
        .await;
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);

        let audit = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(audit) = db
                    .collection::<AuditLog>(AUDIT_LOGS)
                    .find_one(mongodb::bson::doc! {
                        "user_id": &owner,
                        "event_type": "connection_webhook_delivery_failed",
                    })
                    .await
                    .expect("query failure audit")
                {
                    break audit;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("failure audit should be persisted");
        let data = audit.event_data.expect("failure audit metadata");
        assert_eq!(data["app_id"], client_id);
        assert_eq!(data["event_type"], "connect_link.expired");
        assert_eq!(data["attempts"], 3);
        let serialized = serde_json::to_string(&data).expect("serialize audit metadata");
        assert!(!serialized.contains(raw_secret));
        assert!(!serialized.contains(&address.to_string()));
        assert!(!serialized.contains("private-event-payload"));
    }
    async fn w8_coreb_configuration_fixture() -> (Database, String, String, EncryptionKeys) {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_coreb_webhook_config").await;
        let owner = Uuid::new_v4().to_string();
        let app = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(client(&app, &owner))
            .await
            .unwrap();
        (db, owner, app, test_encryption_keys())
    }

    #[tokio::test]
    async fn w8_coreb_webhook_configuration_abort_commit_and_safe_attribution() {
        use crate::models::audit_outbox::{PENDING_COLLECTION_NAME, PendingAuditEvent};
        let (db, owner, app, keys) = w8_coreb_configuration_fixture().await;
        let runtime = AuditRuntime::new(Default::default(), false);
        let principal = AuditPrincipal::ApiKey {
            id: Uuid::new_v4().to_string(),
            owner_id: owner.clone(),
        };
        let secret = WebhookSecret::prepare(&keys).await.unwrap();
        let change = WebhookConfiguration::new(
            &db,
            &runtime,
            &principal,
            &owner,
            &app,
            WebhookChange::Configure("https://receiver.example.test/events"),
            Some(&secret),
        );
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        configuration_in_session(&mut session, &change)
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        drop(session);
        let unchanged = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! {"_id": &app})
            .await
            .unwrap()
            .unwrap();
        assert!(!unchanged.connection_webhook_enabled);
        assert!(unchanged.connection_webhook_secret_encrypted.is_none());
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let saved = change.commit().await.unwrap();
        assert!(saved.connection_webhook_enabled);
        let pending = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .find_one(doc! {"_id": change.event_id.to_string()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.event.principal, principal);
        assert_eq!(
            pending.event.owner,
            AuditOwner::Person { id: owner.clone() }
        );
        assert_eq!(pending.event.target_id.as_deref(), Some(app.as_str()));
        let evidence = serde_json::to_string(&pending).unwrap();
        for forbidden in [
            secret.raw.as_str(),
            secret.key_id.as_str(),
            "https://receiver.example.test/events",
            "encrypted",
        ] {
            assert!(!evidence.contains(forbidden));
        }
        assert_eq!(
            crate::services::audit_state_service::load(&db)
                .await
                .unwrap()
                .pending_count,
            1
        );
        disable(&db, &runtime, &principal, &owner, &app)
            .await
            .unwrap();
        disable(&db, &runtime, &principal, &owner, &app)
            .await
            .unwrap();
        assert_eq!(
            crate::services::audit_state_service::load(&db)
                .await
                .unwrap()
                .pending_count,
            2
        );
    }

    #[tokio::test]
    async fn w8_coreb_webhook_rotation_is_standard_and_disable_survives_closed_admission() {
        use crate::services::audit_runtime::policy::{AdmissionLimits, TierLimit};
        let (db, owner, app, keys) = w8_coreb_configuration_fixture().await;
        let principal = AuditPrincipal::Person { id: owner.clone() };
        let defaults = AdmissionLimits::default();
        let runtime = AuditRuntime::new(
            AdmissionLimits {
                pending_count: 1,
                standard: TierLimit {
                    pending_count: 1,
                    ..defaults.standard
                },
                ..defaults
            },
            true,
        );
        let (configured, _, _) = store_configuration(
            &db,
            &runtime,
            &keys,
            &principal,
            &owner,
            &app,
            "https://receiver.example.test/events",
        )
        .await
        .unwrap();
        assert!(matches!(
            rotate_secret(&db, &runtime, &keys, &principal, &owner, &app).await,
            Err(AppError::AuditAdmissionClosed { .. })
        ));
        let current = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! {"_id": &app})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            current.connection_webhook_secret_encrypted,
            configured.connection_webhook_secret_encrypted
        );
        assert_eq!(
            current.connection_webhook_key_id,
            configured.connection_webhook_key_id
        );
        let disabled = disable(&db, &runtime, &principal, &owner, &app)
            .await
            .unwrap();
        assert!(!disabled.connection_webhook_enabled);
        assert!(disabled.connection_webhook_secret_encrypted.is_none());
        assert!(disabled.connection_webhook_key_id.is_none());
    }

    #[tokio::test]
    async fn w8_coreb_webhook_owner_acl_and_principal_binding_are_transactional() {
        use crate::models::{
            org_membership::{self, OrgRole},
            user::{self, UserType},
        };
        let (db, owner, app, keys) = w8_coreb_configuration_fixture().await;
        let runtime = AuditRuntime::new(Default::default(), false);
        for principal in [
            AuditPrincipal::Node { id: owner.clone() },
            AuditPrincipal::ApiKey {
                id: Uuid::new_v4().to_string(),
                owner_id: Uuid::new_v4().to_string(),
            },
        ] {
            assert!(matches!(
                store_configuration(
                    &db,
                    &runtime,
                    &keys,
                    &principal,
                    &owner,
                    &app,
                    "https://receiver.example.test/events"
                )
                .await,
                Err(AppError::Forbidden(_))
            ));
        }
        let stranger = Uuid::new_v4().to_string();
        assert!(matches!(
            store_configuration(
                &db,
                &runtime,
                &keys,
                &AuditPrincipal::Person {
                    id: stranger.clone()
                },
                &stranger,
                &app,
                "https://receiver.example.test/events"
            )
            .await,
            Err(AppError::NotFound(_))
        ));
        let org = Uuid::new_v4().to_string();
        db.collection::<user::User>(user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(&org, UserType::Org))
            .await
            .unwrap();
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .update_one(doc! {"_id": &app}, doc! {"$set": {"created_by": &org}})
            .await
            .unwrap();
        let member = crate::test_utils::test_membership(&org, &owner, OrgRole::Member, None);
        db.collection::<org_membership::OrgMembership>(org_membership::COLLECTION_NAME)
            .insert_one(&member)
            .await
            .unwrap();
        let principal = AuditPrincipal::Person { id: owner.clone() };
        assert!(matches!(
            store_configuration(
                &db,
                &runtime,
                &keys,
                &principal,
                &owner,
                &app,
                "https://receiver.example.test/events"
            )
            .await,
            Err(AppError::OrgRoleInsufficient(_))
        ));
        db.collection::<bson::Document>(org_membership::COLLECTION_NAME)
            .update_one(doc! {"_id": &member.id}, doc! {"$set": {"role": "admin"}})
            .await
            .unwrap();
        store_configuration(
            &db,
            &runtime,
            &keys,
            &principal,
            &owner,
            &app,
            "https://receiver.example.test/events",
        )
        .await
        .unwrap();
        let pending = db
            .collection::<crate::models::audit_outbox::PendingAuditEvent>(
                crate::models::audit_outbox::PENDING_COLLECTION_NAME,
            )
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.event.owner, AuditOwner::Org { id: org });
        assert_eq!(pending.event.principal, principal);
        db.collection::<bson::Document>(org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &member.id},
                doc! {"$set": {"revoked_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert!(matches!(
            rotate_secret(&db, &runtime, &keys, &principal, &owner, &app).await,
            Err(AppError::NotFound(_))
        ));
    }
}
