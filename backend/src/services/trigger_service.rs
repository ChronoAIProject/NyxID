use std::sync::Arc;

use chrono::Utc;
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use mongodb::{Database, bson, bson::doc, options::ReturnDocument};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::crypto::aes::EncryptionKeys;
use crate::crypto::jwt::JwtKeys;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::channel_conversation::{
    COLLECTION_NAME as CHANNEL_CONVERSATIONS, ChannelConversation,
};
use crate::models::trigger::{
    COLLECTION_NAME as TRIGGERS, Trigger, TriggerDelivery, TriggerStatus, TriggerTokenLocation,
    TriggerVerification,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::rate_limit::PerChannelEventLimiter;
use crate::services::channel_event_service::{self, EventEnvelope};
use crate::services::event_dedup_cache::EventDedupCache;
use crate::services::push_service::{ApnsAuth, FcmAuth};
use crate::services::webhook_delivery_service::{self, DeliveryFailure, SignatureContract};
use crate::services::{audit_service, notification_service};

pub const TRIGGER_SECRET_PREFIX: &str = "nyx_trg_";
const MAX_LABEL_LEN: usize = 128;
const MAX_EVENT_ID_LEN: usize = 128;

#[derive(Debug)]
pub struct CreateInput {
    pub user_id: String,
    pub label: String,
    pub user_service_id: Option<String>,
    pub verification: TriggerVerification,
    pub delivery: TriggerDelivery,
}

#[derive(Debug)]
pub struct UpdateInput {
    pub label: Option<String>,
    pub status: Option<TriggerStatus>,
    pub delivery: Option<TriggerDelivery>,
}

pub struct CreatedTrigger {
    pub trigger: Trigger,
    pub raw_secret: String,
    pub delivery_signing_secret: Option<String>,
}

pub struct UpdatedTrigger {
    pub trigger: Trigger,
    pub delivery_signing_secret: Option<String>,
}

#[derive(serde::Serialize)]
pub struct TriggerEventEnvelope {
    pub event_id: String,
    pub trigger_id: String,
    pub source: &'static str,
    pub received_at: chrono::DateTime<Utc>,
    pub payload: serde_json::Value,
}

pub async fn create(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    input: CreateInput,
) -> AppResult<CreatedTrigger> {
    let label = validate_label(&input.label)?;
    validate_associations(
        db,
        &input.user_id,
        input.user_service_id.as_deref(),
        &input.delivery,
    )
    .await?;
    validate_verification(&input.verification)?;
    let (delivery, delivery_secret_encrypted, delivery_signing_secret) =
        prepare_delivery(encryption_keys, input.delivery).await?;
    let raw_secret = format!("{TRIGGER_SECRET_PREFIX}{}", generate_random_token());
    let verification_secret_encrypted = match input.verification {
        TriggerVerification::HmacSha256 { .. } => {
            Some(encryption_keys.encrypt(raw_secret.as_bytes()).await?)
        }
        TriggerVerification::Token { .. } => None,
    };
    let now = Utc::now();
    let trigger = Trigger {
        id: Uuid::new_v4().to_string(),
        user_id: input.user_id,
        label,
        user_service_id: input.user_service_id,
        status: TriggerStatus::Active,
        secret_hash: hash_token(&raw_secret),
        verification: input.verification,
        verification_secret_encrypted,
        delivery,
        delivery_secret_encrypted,
        created_at: now,
        updated_at: now,
    };
    db.collection::<Trigger>(TRIGGERS)
        .insert_one(&trigger)
        .await?;
    Ok(CreatedTrigger {
        trigger,
        raw_secret,
        delivery_signing_secret,
    })
}

pub async fn list_for_owner(db: &Database, owner_user_id: &str) -> AppResult<Vec<Trigger>> {
    db.collection::<Trigger>(TRIGGERS)
        .find(doc! { "user_id": owner_user_id })
        .sort(doc! { "created_at": -1, "_id": 1 })
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub async fn get_for_actor(db: &Database, actor_user_id: &str, id: &str) -> AppResult<Trigger> {
    let trigger = db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! { "_id": id })
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    let access =
        crate::services::org_service::resolve_owner_access(db, actor_user_id, &trigger.user_id)
            .await?;
    if !access.can_read() {
        return Err(AppError::TriggerNotFound);
    }
    Ok(trigger)
}

pub async fn ensure_actor_can_write(
    db: &Database,
    actor_user_id: &str,
    id: &str,
) -> AppResult<Trigger> {
    let trigger = db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! { "_id": id })
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    let access =
        crate::services::org_service::resolve_owner_access(db, actor_user_id, &trigger.user_id)
            .await?;
    if !access.can_write() {
        return Err(AppError::TriggerNotFound);
    }
    Ok(trigger)
}

pub async fn update(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    current: &Trigger,
    input: UpdateInput,
) -> AppResult<UpdatedTrigger> {
    let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(Utc::now()) };
    let mut delivery_signing_secret = None;
    if let Some(label) = input.label {
        set_doc.insert("label", validate_label(&label)?);
    }
    if let Some(status) = input.status {
        set_doc.insert(
            "status",
            bson::to_bson(&status).map_err(serialization_error)?,
        );
    }
    if let Some(delivery) = input.delivery {
        validate_associations(
            db,
            &current.user_id,
            current.user_service_id.as_deref(),
            &delivery,
        )
        .await?;
        let (delivery, encrypted, raw) = prepare_delivery(encryption_keys, delivery).await?;
        set_doc.insert(
            "delivery",
            bson::to_bson(&delivery).map_err(serialization_error)?,
        );
        delivery_signing_secret = raw;
        match encrypted {
            Some(encrypted) => set_doc.insert(
                "delivery_secret_encrypted",
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
            ),
            None => {
                let updated = db
                    .collection::<Trigger>(TRIGGERS)
                    .find_one_and_update(
                        doc! { "_id": &current.id, "user_id": &current.user_id },
                        doc! {
                            "$set": set_doc,
                            "$unset": { "delivery_secret_encrypted": "" },
                        },
                    )
                    .return_document(ReturnDocument::After)
                    .await?
                    .ok_or(AppError::TriggerNotFound)?;
                return Ok(UpdatedTrigger {
                    trigger: updated,
                    delivery_signing_secret,
                });
            }
        };
    }
    let updated = db
        .collection::<Trigger>(TRIGGERS)
        .find_one_and_update(
            doc! { "_id": &current.id, "user_id": &current.user_id },
            doc! { "$set": set_doc },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    Ok(UpdatedTrigger {
        trigger: updated,
        delivery_signing_secret,
    })
}

pub async fn delete(db: &Database, current: &Trigger) -> AppResult<()> {
    let result = db
        .collection::<Trigger>(TRIGGERS)
        .delete_one(doc! { "_id": &current.id, "user_id": &current.user_id })
        .await?;
    if result.deleted_count == 1 {
        Ok(())
    } else {
        Err(AppError::TriggerNotFound)
    }
}

pub async fn rotate_secret(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    current: &Trigger,
) -> AppResult<(Trigger, String)> {
    let raw_secret = format!("{TRIGGER_SECRET_PREFIX}{}", generate_random_token());
    let mut set_doc = doc! {
        "secret_hash": hash_token(&raw_secret),
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    let update = match current.verification {
        TriggerVerification::HmacSha256 { .. } => {
            let encrypted = encryption_keys.encrypt(raw_secret.as_bytes()).await?;
            set_doc.insert(
                "verification_secret_encrypted",
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
            );
            doc! { "$set": set_doc }
        }
        TriggerVerification::Token { .. } => doc! {
            "$set": set_doc,
            "$unset": { "verification_secret_encrypted": "" },
        },
    };
    let updated = db
        .collection::<Trigger>(TRIGGERS)
        .find_one_and_update(
            doc! { "_id": &current.id, "user_id": &current.user_id },
            update,
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    Ok((updated, raw_secret))
}

pub async fn load_active_for_ingress(db: &Database, id: &str) -> AppResult<Trigger> {
    db.collection::<Trigger>(TRIGGERS)
        .find_one(doc! { "_id": id, "status": "active" })
        .await?
        .ok_or(AppError::TriggerNotFound)
}

pub async fn verify_ingress(
    encryption_keys: &EncryptionKeys,
    trigger: &Trigger,
    headers: &axum::http::HeaderMap,
    query_token: Option<&str>,
    body: &[u8],
) -> AppResult<()> {
    match &trigger.verification {
        TriggerVerification::Token { location } => {
            let supplied = match location {
                TriggerTokenLocation::Bearer => headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("Bearer ")),
                TriggerTokenLocation::Query => query_token,
            }
            .ok_or(AppError::TriggerSecretInvalid)?;
            let supplied_hash = hash_token(supplied);
            if supplied_hash
                .as_bytes()
                .ct_eq(trigger.secret_hash.as_bytes())
                .unwrap_u8()
                != 1
            {
                return Err(AppError::TriggerSecretInvalid);
            }
        }
        TriggerVerification::HmacSha256 { header_name } => {
            let signature = headers
                .get(header_name)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("sha256="))
                .ok_or(AppError::TriggerSecretInvalid)?;
            let encrypted = trigger
                .verification_secret_encrypted
                .as_deref()
                .ok_or(AppError::TriggerSecretInvalid)?;
            let secret = Zeroizing::new(encryption_keys.decrypt(encrypted).await?);
            type HmacSha256 = Hmac<Sha256>;
            let mut mac =
                HmacSha256::new_from_slice(secret.as_slice()).expect("HMAC accepts any key length");
            mac.update(body);
            let expected = hex::encode(mac.finalize().into_bytes());
            if expected.as_bytes().ct_eq(signature.as_bytes()).unwrap_u8() != 1 {
                return Err(AppError::TriggerSecretInvalid);
            }
        }
    }
    Ok(())
}

pub fn event_id(
    headers: &axum::http::HeaderMap,
    payload: &serde_json::Value,
    body: &[u8],
) -> AppResult<String> {
    let event_id = headers
        .get("X-NyxID-Event-Id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("event_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| hex::encode(Sha256::digest(body)));
    if event_id.is_empty() || event_id.len() > MAX_EVENT_ID_LEN {
        return Err(AppError::ValidationError(
            "trigger event id must be 1-128 characters".to_string(),
        ));
    }
    axum::http::HeaderValue::from_str(&event_id).map_err(|_| {
        AppError::ValidationError("trigger event id must be a valid HTTP header value".to_string())
    })?;
    Ok(event_id)
}

#[allow(clippy::too_many_arguments)]
pub async fn deliver_event(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    channel_limiter: &PerChannelEventLimiter,
    channel_dedup: &Arc<EventDedupCache>,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    trigger: &Trigger,
    event_id: &str,
    payload: serde_json::Value,
) -> AppResult<()> {
    let envelope = TriggerEventEnvelope {
        event_id: event_id.to_string(),
        trigger_id: trigger.id.clone(),
        source: "inbound_webhook",
        received_at: Utc::now(),
        payload,
    };
    let value = serde_json::to_value(&envelope).map_err(serialization_error)?;
    match &trigger.delivery {
        TriggerDelivery::Webhook { .. } => {
            deliver_webhook_envelope(encryption_keys, http_client, trigger, &envelope)
                .await
                .map_err(|_| AppError::TriggerDeliveryFailed)?
        }
        TriggerDelivery::Agent { conversation_id } => {
            let agent_envelope = EventEnvelope {
                event_id: agent_event_uuid(&trigger.id, event_id),
                source: "inbound_webhook".to_string(),
                event_type: "trigger.event".to_string(),
                timestamp: envelope.received_at,
                payload: Some(value),
                metadata: None,
            };
            channel_event_service::forward_trigger_event(
                db,
                http_client,
                config,
                jwt_keys,
                channel_limiter,
                channel_dedup,
                &trigger.user_id,
                conversation_id,
                &agent_envelope,
            )
            .await?;
        }
        TriggerDelivery::Notification => {
            let mut recipients =
                crate::services::org_service::list_admin_user_ids(db, &trigger.user_id).await?;
            if recipients.is_empty() {
                recipients.push(trigger.user_id.clone());
            }
            let mut delivered = false;
            let mut delivery_failed = false;
            for recipient in recipients {
                match notification_service::send_trigger_notification(
                    db,
                    config,
                    http_client,
                    fcm_auth,
                    apns_auth,
                    &recipient,
                    &trigger.label,
                    &value,
                )
                .await
                {
                    Ok(()) => delivered = true,
                    Err(AppError::TriggerDeliveryUnsupported) => {}
                    Err(AppError::TriggerDeliveryFailed) => delivery_failed = true,
                    Err(error) => return Err(error),
                }
            }
            if !delivered {
                return Err(if delivery_failed {
                    AppError::TriggerDeliveryFailed
                } else {
                    AppError::TriggerDeliveryUnsupported
                });
            }
        }
    }
    Ok(())
}

/// Spawn bounded webhook delivery after the ingress handler has atomically
/// reserved the event id. Delivery failure is isolated from the provider
/// request and recorded with metadata only.
pub fn dispatch_webhook_event(
    db: Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: reqwest::Client,
    trigger: Trigger,
    event_id: String,
    payload: serde_json::Value,
) {
    tokio::spawn(async move {
        let envelope = TriggerEventEnvelope {
            event_id: event_id.clone(),
            trigger_id: trigger.id.clone(),
            source: "inbound_webhook",
            received_at: Utc::now(),
            payload,
        };
        match deliver_webhook_envelope(&encryption_keys, &http_client, &trigger, &envelope).await {
            Ok(()) => audit_service::log_async(
                db,
                Some(trigger.user_id),
                "trigger_event_forwarded".to_string(),
                Some(serde_json::json!({
                    "trigger_id": trigger.id,
                    "event_id": event_id,
                    "delivery_type": "webhook",
                })),
                None,
                None,
                None,
                None,
            ),
            Err(failure) => record_webhook_delivery_failure(&db, &trigger, &event_id, failure),
        }
    });
}

async fn deliver_webhook_envelope(
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    trigger: &Trigger,
    envelope: &TriggerEventEnvelope,
) -> Result<(), DeliveryFailure> {
    let TriggerDelivery::Webhook { url } = &trigger.delivery else {
        return Err(DeliveryFailure {
            attempts: 0,
            reason: "delivery_type_mismatch",
            last_status: None,
        });
    };
    let Some(encrypted) = trigger.delivery_secret_encrypted.as_deref() else {
        return Err(DeliveryFailure {
            attempts: 0,
            reason: "signing_secret_missing",
            last_status: None,
        });
    };
    let secret = encryption_keys
        .decrypt(encrypted)
        .await
        .map(Zeroizing::new)
        .map_err(|error| {
            tracing::warn!(
                trigger_id = %trigger.id,
                %error,
                "failed to decrypt trigger webhook signing secret"
            );
            DeliveryFailure {
                attempts: 0,
                reason: "secret_decrypt_failed",
                last_status: None,
            }
        })?;
    let body = serde_json::to_vec(envelope).map_err(|error| {
        tracing::warn!(
            trigger_id = %trigger.id,
            %error,
            "failed to serialize trigger webhook envelope"
        );
        DeliveryFailure {
            attempts: 0,
            reason: "serialization_failed",
            last_status: None,
        }
    })?;
    webhook_delivery_service::deliver_signed_body(
        http_client,
        url,
        secret.as_slice(),
        "trigger.event",
        &envelope.event_id,
        &body,
        SignatureContract::Timestamped,
    )
    .await
}

fn record_webhook_delivery_failure(
    db: &Database,
    trigger: &Trigger,
    event_id: &str,
    failure: DeliveryFailure,
) {
    tracing::error!(
        trigger_id = %trigger.id,
        event_id,
        attempts = failure.attempts,
        reason = failure.reason,
        last_status = failure.last_status,
        "trigger webhook delivery exhausted"
    );
    audit_service::log_async(
        db.clone(),
        Some(trigger.user_id.clone()),
        "trigger_webhook_delivery_failed".to_string(),
        Some(serde_json::json!({
            "trigger_id": &trigger.id,
            "event_id": event_id,
            "delivery_type": "webhook",
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

fn agent_event_uuid(trigger_id: &str, event_id: &str) -> String {
    let digest = Sha256::digest(format!("{trigger_id}.{event_id}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

async fn prepare_delivery(
    encryption_keys: &EncryptionKeys,
    delivery: TriggerDelivery,
) -> AppResult<(TriggerDelivery, Option<Vec<u8>>, Option<String>)> {
    match delivery {
        TriggerDelivery::Webhook { url } => {
            let url = webhook_delivery_service::validate_webhook_url(&url, "delivery.url").await?;
            let raw = format!("nyx_twh_{}", generate_random_token());
            let encrypted = encryption_keys.encrypt(raw.as_bytes()).await?;
            Ok((TriggerDelivery::Webhook { url }, Some(encrypted), Some(raw)))
        }
        other => Ok((other, None, None)),
    }
}

async fn validate_associations(
    db: &Database,
    user_id: &str,
    user_service_id: Option<&str>,
    delivery: &TriggerDelivery,
) -> AppResult<()> {
    if let Some(service_id) = user_service_id {
        let exists = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": service_id, "user_id": user_id })
            .await?
            .is_some();
        if !exists {
            return Err(AppError::TriggerNotFound);
        }
    }
    if let TriggerDelivery::Agent { conversation_id } = delivery {
        let exists = db
            .collection::<ChannelConversation>(CHANNEL_CONVERSATIONS)
            .find_one(doc! {
                "_id": conversation_id,
                "user_id": user_id,
                "platform": "device",
                "is_active": true,
            })
            .await?
            .is_some();
        if !exists {
            return Err(AppError::TriggerDeliveryUnsupported);
        }
    }
    Ok(())
}

fn validate_verification(verification: &TriggerVerification) -> AppResult<()> {
    if let TriggerVerification::HmacSha256 { header_name } = verification {
        if header_name.len() > 128 {
            return Err(AppError::ValidationError(
                "verification header_name must not exceed 128 characters".to_string(),
            ));
        }
        axum::http::HeaderName::from_bytes(header_name.as_bytes()).map_err(|_| {
            AppError::ValidationError("verification header_name is invalid".to_string())
        })?;
    }
    Ok(())
}

fn validate_label(label: &str) -> AppResult<String> {
    let label = label.trim();
    if label.is_empty() || label.len() > MAX_LABEL_LEN {
        return Err(AppError::ValidationError(
            "label must be 1-128 characters".to_string(),
        ));
    }
    Ok(label.to_string())
}

fn serialization_error(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("Failed to serialize trigger data: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{connect_test_database, test_encryption_keys};
    use axum::http::{HeaderMap, HeaderValue};
    use chrono::TimeZone;

    fn trigger(verification: TriggerVerification, encrypted: Option<Vec<u8>>) -> Trigger {
        Trigger {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            label: "Test trigger".to_string(),
            user_service_id: None,
            status: TriggerStatus::Active,
            secret_hash: hash_token("nyx_trg_secret"),
            verification,
            verification_secret_encrypted: encrypted,
            delivery: TriggerDelivery::Notification,
            delivery_secret_encrypted: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn trigger_event_envelope_timestamp_wire_format_is_stable() {
        let envelope = TriggerEventEnvelope {
            event_id: "provider-event-123".to_string(),
            trigger_id: "trigger-uuid".to_string(),
            source: "inbound_webhook",
            received_at: Utc
                .with_ymd_and_hms(2026, 8, 6, 9, 30, 0)
                .single()
                .expect("fixture timestamp")
                + chrono::Duration::milliseconds(123),
            payload: serde_json::json!({"action":"opened"}),
        };

        assert_eq!(
            serde_json::to_string(&envelope).expect("serialize envelope"),
            r#"{"event_id":"provider-event-123","trigger_id":"trigger-uuid","source":"inbound_webhook","received_at":"2026-08-06T09:30:00.123Z","payload":{"action":"opened"}}"#
        );
    }

    #[tokio::test]
    async fn token_verification_accepts_valid_and_rejects_wrong_secret() {
        let trigger = trigger(
            TriggerVerification::Token {
                location: TriggerTokenLocation::Bearer,
            },
            None,
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer nyx_trg_secret"),
        );
        assert!(
            verify_ingress(&test_encryption_keys(), &trigger, &headers, None, b"{}")
                .await
                .is_ok()
        );
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert!(matches!(
            verify_ingress(&test_encryption_keys(), &trigger, &headers, None, b"{}").await,
            Err(AppError::TriggerSecretInvalid)
        ));
    }

    #[tokio::test]
    async fn hmac_verification_checks_raw_body() {
        let keys = test_encryption_keys();
        let secret = b"nyx_trg_hmac_secret";
        let encrypted = keys.encrypt(secret).await.expect("encrypt secret");
        let trigger = trigger(
            TriggerVerification::HmacSha256 {
                header_name: "X-Hub-Signature-256".to_string(),
            },
            Some(encrypted),
        );
        let body = br#"{"action":"opened"}"#;
        let signature = webhook_delivery_service::compute_body_signature(secret, body);
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Hub-Signature-256",
            HeaderValue::from_str(&format!("sha256={signature}")).expect("signature header"),
        );
        assert!(
            verify_ingress(&keys, &trigger, &headers, None, body)
                .await
                .is_ok()
        );
        assert!(matches!(
            verify_ingress(&keys, &trigger, &headers, None, b"changed").await,
            Err(AppError::TriggerSecretInvalid)
        ));
    }

    #[test]
    fn agent_event_id_is_stable_uuid() {
        let first = agent_event_uuid("trigger", "event");
        assert_eq!(first, agent_event_uuid("trigger", "event"));
        assert!(Uuid::parse_str(&first).is_ok());
    }

    #[test]
    fn event_id_rejects_values_that_cannot_be_forwarded_as_headers() {
        let headers = HeaderMap::new();
        let payload = serde_json::json!({ "event_id": "line-one\nline-two" });
        assert!(matches!(
            event_id(&headers, &payload, b"{}"),
            Err(AppError::ValidationError(_))
        ));
    }

    #[tokio::test]
    async fn crud_and_secret_rotation_preserve_one_time_secret_contract() {
        let Some(db) = connect_test_database("trigger_service_crud").await else {
            return;
        };
        let keys = test_encryption_keys();
        let owner = Uuid::new_v4().to_string();
        let created = create(
            &db,
            &keys,
            CreateInput {
                user_id: owner.clone(),
                label: "Repository activity".to_string(),
                user_service_id: None,
                verification: TriggerVerification::Token {
                    location: TriggerTokenLocation::Bearer,
                },
                delivery: TriggerDelivery::Notification,
            },
        )
        .await
        .expect("create trigger");
        assert!(created.raw_secret.starts_with(TRIGGER_SECRET_PREFIX));
        assert_eq!(created.trigger.secret_hash, hash_token(&created.raw_secret));
        assert!(!format!("{:?}", created.trigger).contains(&created.raw_secret));
        assert!(created.delivery_signing_secret.is_none());

        let listed = list_for_owner(&db, &owner).await.expect("list triggers");
        assert_eq!(listed.len(), 1);
        let current = get_for_actor(&db, &owner, &created.trigger.id)
            .await
            .expect("get trigger");
        let updated = update(
            &db,
            &keys,
            &current,
            UpdateInput {
                label: Some("Deployment activity".to_string()),
                status: Some(TriggerStatus::Disabled),
                delivery: None,
            },
        )
        .await
        .expect("update trigger");
        assert_eq!(updated.trigger.label, "Deployment activity");
        assert_eq!(updated.trigger.status, TriggerStatus::Disabled);

        let (rotated, second_secret) = rotate_secret(&db, &keys, &updated.trigger)
            .await
            .expect("rotate trigger secret");
        assert_ne!(created.raw_secret, second_secret);
        assert_eq!(rotated.secret_hash, hash_token(&second_secret));

        delete(&db, &rotated).await.expect("delete trigger");
        assert!(matches!(
            get_for_actor(&db, &owner, &rotated.id).await,
            Err(AppError::TriggerNotFound)
        ));
    }
}
