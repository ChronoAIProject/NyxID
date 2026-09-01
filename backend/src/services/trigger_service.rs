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
use crate::models::trigger_delivery::{
    COLLECTION_NAME as TRIGGER_DELIVERIES, TriggerDeliveryRecord, TriggerDeliveryRecordStatus,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::rate_limit::PerChannelEventLimiter;
use crate::services::channel_event_service::{self, EventEnvelope};
use crate::services::push_service::{ApnsAuth, FcmAuth};
use crate::services::webhook_delivery_service::{self, DeliveryFailure, SignatureContract};
use crate::services::{audit_service, notification_service};

pub const TRIGGER_SECRET_PREFIX: &str = "nyx_trg_";
pub const TRIGGER_DELIVERY_SECRET_PREFIX: &str = "nyx_twh_";
pub const TRIGGER_ENVELOPE_OVERHEAD_BYTES: usize = 4 * 1024;
const METADATA_ONLY_RETENTION_HOURS: i64 = 72;
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
    pub delivery_signing_key_id: Option<String>,
}

pub struct UpdatedTrigger {
    pub trigger: Trigger,
    pub delivery_signing_secret: Option<String>,
    pub delivery_signing_key_id: Option<String>,
}

pub struct RotatedDeliverySecret {
    pub trigger: Trigger,
    pub raw_secret: String,
    pub key_id: String,
}

pub enum WebhookAdmission {
    Accepted {
        record: Box<TriggerDeliveryRecord>,
        body: Vec<u8>,
    },
    Duplicate,
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
    let (delivery, delivery_secret_encrypted, delivery_signing_secret, delivery_key_id) =
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
        delivery_key_id: delivery_key_id.clone(),
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
        delivery_signing_key_id: delivery_key_id,
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
    let mut delivery_signing_key_id = current.delivery_key_id.clone();
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
        let (delivery, encrypted, raw, key_id) =
            prepare_delivery(encryption_keys, delivery).await?;
        set_doc.insert(
            "delivery",
            bson::to_bson(&delivery).map_err(serialization_error)?,
        );
        delivery_signing_secret = raw;
        delivery_signing_key_id = key_id.clone();
        match encrypted {
            Some(encrypted) => {
                set_doc.insert(
                    "delivery_secret_encrypted",
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: encrypted,
                    },
                );
                set_doc.insert("delivery_key_id", key_id.unwrap_or_default());
            }
            None => {
                let updated = db
                    .collection::<Trigger>(TRIGGERS)
                    .find_one_and_update(
                        doc! { "_id": &current.id, "user_id": &current.user_id },
                        doc! {
                            "$set": set_doc,
                            "$unset": {
                                "delivery_secret_encrypted": "",
                                "delivery_key_id": "",
                            },
                        },
                    )
                    .return_document(ReturnDocument::After)
                    .await?
                    .ok_or(AppError::TriggerNotFound)?;
                return Ok(UpdatedTrigger {
                    trigger: updated,
                    delivery_signing_secret,
                    delivery_signing_key_id,
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
        delivery_signing_key_id,
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

pub async fn rotate_delivery_secret(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    current: &Trigger,
) -> AppResult<RotatedDeliverySecret> {
    if !matches!(current.delivery, TriggerDelivery::Webhook { .. }) {
        return Err(AppError::TriggerDeliveryUnsupported);
    }
    let raw_secret = format!(
        "{TRIGGER_DELIVERY_SECRET_PREFIX}{}",
        generate_random_token()
    );
    let encrypted = encryption_keys.encrypt(raw_secret.as_bytes()).await?;
    let key_id = webhook_delivery_service::generate_signing_key_id();
    let trigger = db
        .collection::<Trigger>(TRIGGERS)
        .find_one_and_update(
            doc! { "_id": &current.id, "user_id": &current.user_id },
            doc! { "$set": {
                "delivery_secret_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
                "delivery_key_id": &key_id,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    Ok(RotatedDeliverySecret {
        trigger,
        raw_secret,
        key_id,
    })
}

pub async fn load_active_for_ingress(db: &Database, id: &str) -> AppResult<Trigger> {
    let trigger = db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! { "_id": id, "status": "active" })
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    let trigger = ensure_delivery_key_id(db, trigger).await?;
    if trigger.status != TriggerStatus::Active {
        return Err(AppError::TriggerNotFound);
    }
    Ok(trigger)
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
            let body = serde_json::to_vec(&envelope).map_err(serialization_error)?;
            deliver_webhook_body(
                encryption_keys,
                http_client,
                trigger,
                event_id,
                &body,
                config
                    .trigger_payload_max_bytes
                    .saturating_add(TRIGGER_ENVELOPE_OVERHEAD_BYTES),
            )
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

pub async fn admit_webhook_delivery(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    trigger: &Trigger,
    event_id: &str,
    payload: serde_json::Value,
    retention_hours: u64,
    max_body_bytes: usize,
) -> AppResult<WebhookAdmission> {
    if !matches!(trigger.delivery, TriggerDelivery::Webhook { .. }) {
        return Err(AppError::TriggerDeliveryUnsupported);
    }
    let now = Utc::now();
    let envelope = TriggerEventEnvelope {
        event_id: event_id.to_string(),
        trigger_id: trigger.id.clone(),
        source: "inbound_webhook",
        received_at: now,
        payload,
    };
    let body = serde_json::to_vec(&envelope).map_err(serialization_error)?;
    if body.len() > max_body_bytes {
        return Err(AppError::TriggerPayloadTooLarge);
    }
    let envelope_encrypted = if retention_hours == 0 {
        None
    } else {
        Some(encryption_keys.encrypt(&body).await?)
    };
    let retained_hours = if retention_hours == 0 {
        METADATA_ONLY_RETENTION_HOURS
    } else {
        i64::try_from(retention_hours).unwrap_or(i64::MAX)
    };
    let record = TriggerDeliveryRecord {
        id: delivery_record_id(&trigger.id, event_id),
        trigger_id: trigger.id.clone(),
        user_id: trigger.user_id.clone(),
        event_id: event_id.to_string(),
        status: TriggerDeliveryRecordStatus::Pending,
        attempts: 0,
        last_status_code: None,
        envelope_encrypted,
        created_at: now,
        updated_at: now,
        expires_at: now + chrono::Duration::hours(retained_hours),
        delivered_at: None,
    };
    match db
        .collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
        .insert_one(&record)
        .await
    {
        Ok(_) => Ok(WebhookAdmission::Accepted {
            record: Box::new(record),
            body,
        }),
        Err(error) if is_duplicate_key_error(&error) => Ok(WebhookAdmission::Duplicate),
        Err(error) => Err(error.into()),
    }
}

/// Spawn delivery after durable admission. Delivery failure is isolated from
/// the ingress request and reflected in the metadata-only delivery record.
pub fn dispatch_webhook_event(
    db: Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: reqwest::Client,
    trigger: Trigger,
    record: TriggerDeliveryRecord,
    body: Vec<u8>,
    max_body_bytes: usize,
) {
    tokio::spawn(async move {
        let event_id = record.event_id.clone();
        let outcome = deliver_webhook_body(
            &encryption_keys,
            &http_client,
            &trigger,
            &event_id,
            &body,
            max_body_bytes,
        )
        .await;
        if let Err(error) = update_delivery_after_attempt(&db, &record, &outcome).await {
            tracing::warn!(
                trigger_id = %trigger.id,
                event_id,
                %error,
                "failed to update trigger delivery record"
            );
        }
        match outcome {
            Ok(()) => audit_service::log_async(
                db.clone(),
                Some(trigger.user_id.clone()),
                "trigger_event_forwarded".to_string(),
                Some(serde_json::json!({
                    "trigger_id": &trigger.id,
                    "event_id": &event_id,
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

async fn update_delivery_after_attempt(
    db: &Database,
    record: &TriggerDeliveryRecord,
    outcome: &Result<(), DeliveryFailure>,
) -> AppResult<()> {
    let now = Utc::now();
    let (status, last_status_code) = match outcome {
        Ok(()) => ("delivered", None),
        Err(failure) => ("failed", failure.last_status),
    };
    let mut set_doc = doc! {
        "status": status,
        "last_status_code": bson::to_bson(&last_status_code).map_err(serialization_error)?,
        "updated_at": bson::DateTime::from_chrono(now),
    };
    if outcome.is_ok() {
        set_doc.insert("delivered_at", bson::DateTime::from_chrono(now));
    }
    db.collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
        .update_one(
            doc! { "_id": &record.id, "trigger_id": &record.trigger_id },
            doc! {
                "$set": set_doc,
                "$inc": { "attempts": 1_i32 },
            },
        )
        .await?;
    Ok(())
}

async fn deliver_webhook_body(
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    trigger: &Trigger,
    event_id: &str,
    body: &[u8],
    max_body_bytes: usize,
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
    let Some(key_id) = trigger.delivery_key_id.as_deref() else {
        return Err(DeliveryFailure {
            attempts: 0,
            reason: "signing_key_id_missing",
            last_status: None,
        });
    };
    if body.len() > max_body_bytes {
        return Err(DeliveryFailure {
            attempts: 0,
            reason: "body_too_large",
            last_status: None,
        });
    }
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
    webhook_delivery_service::deliver_signed_body(
        http_client,
        url,
        secret.as_slice(),
        "trigger.event",
        event_id,
        body,
        SignatureContract::Timestamped,
        Some(key_id),
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

pub async fn list_deliveries(
    db: &Database,
    trigger_id: &str,
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<TriggerDeliveryRecord>, u64)> {
    let page = page.max(1);
    let per_page = per_page.clamp(1, 100);
    let filter = doc! { "trigger_id": trigger_id };
    let total = db
        .collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
        .count_documents(filter.clone())
        .await?;
    let deliveries = db
        .collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
        .find(filter)
        .sort(doc! { "created_at": -1, "_id": -1 })
        .skip((page - 1).saturating_mul(per_page))
        .limit(i64::try_from(per_page).unwrap_or(100))
        .await?
        .try_collect()
        .await?;
    Ok((deliveries, total))
}

pub async fn redeliver(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    trigger: &Trigger,
    event_id: &str,
    max_body_bytes: usize,
) -> AppResult<TriggerDeliveryRecord> {
    if !matches!(trigger.delivery, TriggerDelivery::Webhook { .. }) {
        return Err(AppError::TriggerDeliveryUnsupported);
    }
    let trigger = ensure_delivery_key_id(db, trigger.clone()).await?;
    let record = db
        .collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
        .find_one(doc! { "trigger_id": &trigger.id, "event_id": event_id })
        .await?
        .ok_or(AppError::TriggerDeliveryRecordNotFound)?;
    let encrypted = record
        .envelope_encrypted
        .as_deref()
        .ok_or(AppError::TriggerDeliveryRecordNotFound)?;
    let body = Zeroizing::new(encryption_keys.decrypt(encrypted).await?);
    db.collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
        .update_one(
            doc! { "_id": &record.id, "trigger_id": &trigger.id },
            doc! {
                "$set": {
                    "status": "pending",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                },
                "$unset": { "delivered_at": "" },
            },
        )
        .await?;
    let outcome = deliver_webhook_body(
        encryption_keys,
        http_client,
        &trigger,
        event_id,
        body.as_slice(),
        max_body_bytes,
    )
    .await;
    update_delivery_after_attempt(db, &record, &outcome).await?;
    if let Err(failure) = outcome {
        record_webhook_delivery_failure(db, &trigger, event_id, failure);
    }
    db.collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
        .find_one(doc! { "_id": &record.id })
        .await?
        .ok_or(AppError::TriggerDeliveryRecordNotFound)
}

async fn ensure_delivery_key_id(db: &Database, trigger: Trigger) -> AppResult<Trigger> {
    if !matches!(trigger.delivery, TriggerDelivery::Webhook { .. })
        || trigger.delivery_key_id.is_some()
    {
        return Ok(trigger);
    }
    let key_id = webhook_delivery_service::generate_signing_key_id();
    if let Some(updated) = db
        .collection::<Trigger>(TRIGGERS)
        .find_one_and_update(
            doc! {
                "_id": &trigger.id,
                "$or": [
                    { "delivery_key_id": null },
                    { "delivery_key_id": { "$exists": false } },
                ],
            },
            doc! { "$set": { "delivery_key_id": key_id } },
        )
        .return_document(ReturnDocument::After)
        .await?
    {
        return Ok(updated);
    }
    db.collection::<Trigger>(TRIGGERS)
        .find_one(doc! { "_id": &trigger.id })
        .await?
        .ok_or(AppError::TriggerNotFound)
}

fn delivery_record_id(trigger_id: &str, event_id: &str) -> String {
    hex::encode(Sha256::digest(
        format!("{trigger_id}\0{event_id}").as_bytes(),
    ))
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error)) =
        error.kind.as_ref()
    {
        return write_error.code == 11000;
    }
    false
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
) -> AppResult<(
    TriggerDelivery,
    Option<Vec<u8>>,
    Option<String>,
    Option<String>,
)> {
    match delivery {
        TriggerDelivery::Webhook { url } => {
            let url = webhook_delivery_service::validate_webhook_url(&url, "delivery.url").await?;
            let raw = format!(
                "{TRIGGER_DELIVERY_SECRET_PREFIX}{}",
                generate_random_token()
            );
            let encrypted = encryption_keys.encrypt(raw.as_bytes()).await?;
            let key_id = webhook_delivery_service::generate_signing_key_id();
            Ok((
                TriggerDelivery::Webhook { url },
                Some(encrypted),
                Some(raw),
                Some(key_id),
            ))
        }
        other => Ok((other, None, None, None)),
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

    async fn webhook_trigger(keys: &EncryptionKeys, url: String) -> Trigger {
        let mut trigger = trigger(
            TriggerVerification::Token {
                location: TriggerTokenLocation::Bearer,
            },
            None,
        );
        trigger.delivery = TriggerDelivery::Webhook { url };
        trigger.delivery_secret_encrypted = Some(
            keys.encrypt(b"delivery-signing-secret")
                .await
                .expect("encrypt delivery secret"),
        );
        trigger.delivery_key_id = Some("key_fixture".to_string());
        trigger
    }

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
            delivery_key_id: None,
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
    async fn webhook_admission_is_durable_dedup_and_encrypts_retained_envelope() {
        let Some(db) = connect_test_database("trigger_delivery_durable_admission").await else {
            return;
        };
        let keys = test_encryption_keys();
        let trigger = webhook_trigger(&keys, "https://events.example.test/hook".to_string()).await;
        let payload = serde_json::json!({"event_id":"event-1","action":"opened"});
        let first = admit_webhook_delivery(
            &db,
            &keys,
            &trigger,
            "event-1",
            payload.clone(),
            72,
            256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
        )
        .await
        .expect("admit first event");
        let WebhookAdmission::Accepted { record, body } = first else {
            panic!("first event must be accepted")
        };
        assert!(record.envelope_encrypted.is_some());
        assert!(!format!("{record:?}").contains("opened"));
        let plaintext = keys
            .decrypt(record.envelope_encrypted.as_deref().unwrap())
            .await
            .unwrap();
        assert_eq!(plaintext, body);

        let second = admit_webhook_delivery(
            &db,
            &keys,
            &trigger,
            "event-1",
            payload,
            72,
            256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
        )
        .await
        .expect("deduplicate second event");
        assert!(matches!(second, WebhookAdmission::Duplicate));
    }

    #[tokio::test]
    async fn zero_retention_keeps_metadata_without_replayable_payload() {
        let Some(db) = connect_test_database("trigger_delivery_metadata_only").await else {
            return;
        };
        let keys = test_encryption_keys();
        let trigger = webhook_trigger(&keys, "https://events.example.test/hook".to_string()).await;
        let admitted = admit_webhook_delivery(
            &db,
            &keys,
            &trigger,
            "event-1",
            serde_json::json!({"action":"opened"}),
            0,
            256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
        )
        .await
        .unwrap();
        let WebhookAdmission::Accepted { record, .. } = admitted else {
            panic!("event must be accepted")
        };
        assert!(record.envelope_encrypted.is_none());
        assert!(matches!(
            redeliver(
                &db,
                &keys,
                &reqwest::Client::new(),
                &trigger,
                "event-1",
                256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
            )
            .await,
            Err(AppError::TriggerDeliveryRecordNotFound)
        ));
    }

    #[tokio::test]
    async fn redelivery_uses_retained_envelope_and_records_success() {
        let Some(db) = connect_test_database("trigger_delivery_redelivery").await else {
            return;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let tx = tx.clone();
                    async move {
                        tx.send((headers, body)).unwrap();
                        axum::http::StatusCode::NO_CONTENT
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let keys = test_encryption_keys();
        let trigger = webhook_trigger(&keys, format!("http://{address}/hook")).await;
        admit_webhook_delivery(
            &db,
            &keys,
            &trigger,
            "event-1",
            serde_json::json!({"action":"opened"}),
            72,
            256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
        )
        .await
        .unwrap();

        let updated = redeliver(
            &db,
            &keys,
            &reqwest::Client::new(),
            &trigger,
            "event-1",
            256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
        )
        .await
        .expect("redeliver retained envelope");
        assert_eq!(updated.status, TriggerDeliveryRecordStatus::Delivered);
        assert_eq!(updated.attempts, 1);
        assert!(updated.delivered_at.is_some());
        let (headers, body) = rx.recv().await.unwrap();
        assert_eq!(headers["X-NyxID-Event"], "trigger.event");
        assert_eq!(headers["X-NyxID-Delivery-Id"], "event-1");
        assert_eq!(headers["X-NyxID-Key-Id"], "key_fixture");
        let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(envelope["payload"]["action"], "opened");
    }

    #[tokio::test]
    async fn redelivery_assigns_stable_key_id_to_legacy_trigger() {
        let Some(db) = connect_test_database("trigger_delivery_legacy_key_id").await else {
            return;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move |headers: axum::http::HeaderMap| {
                let tx = tx.clone();
                async move {
                    tx.send(headers).unwrap();
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let keys = test_encryption_keys();
        let mut trigger = webhook_trigger(&keys, format!("http://{address}/hook")).await;
        trigger.delivery_key_id = None;
        db.collection::<Trigger>(TRIGGERS)
            .insert_one(&trigger)
            .await
            .unwrap();
        admit_webhook_delivery(
            &db,
            &keys,
            &trigger,
            "event-legacy",
            serde_json::json!({"action":"opened"}),
            72,
            256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
        )
        .await
        .unwrap();

        redeliver(
            &db,
            &keys,
            &reqwest::Client::new(),
            &trigger,
            "event-legacy",
            256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES,
        )
        .await
        .expect("redeliver legacy trigger");
        let headers = rx.recv().await.unwrap();
        let stored = db
            .collection::<Trigger>(TRIGGERS)
            .find_one(doc! { "_id": &trigger.id })
            .await
            .unwrap()
            .unwrap();
        let key_id = stored
            .delivery_key_id
            .expect("legacy trigger receives stable key id");
        assert!(key_id.starts_with("key_"));
        assert_eq!(headers["X-NyxID-Key-Id"], key_id);
    }

    #[tokio::test]
    async fn outbound_trigger_envelope_ceiling_is_enforced_at_admission() {
        let Some(db) = connect_test_database("trigger_delivery_body_ceiling").await else {
            return;
        };
        let keys = test_encryption_keys();
        let trigger = webhook_trigger(&keys, "https://events.example.test/hook".to_string()).await;
        assert!(matches!(
            admit_webhook_delivery(
                &db,
                &keys,
                &trigger,
                "event-1",
                serde_json::json!({"payload":"x".repeat(128)}),
                72,
                64,
            )
            .await,
            Err(AppError::TriggerPayloadTooLarge)
        ));
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

    #[tokio::test]
    async fn delivery_secret_rotation_replaces_secret_and_key_id() {
        let Some(db) = connect_test_database("trigger_delivery_secret_rotation").await else {
            return;
        };
        let keys = test_encryption_keys();
        let mut current =
            webhook_trigger(&keys, "https://events.example.test/hook".to_string()).await;
        let previous_key_id = current.delivery_key_id.clone();
        db.collection::<Trigger>(TRIGGERS)
            .insert_one(&current)
            .await
            .unwrap();
        let rotated = rotate_delivery_secret(&db, &keys, &current)
            .await
            .expect("rotate delivery secret");
        assert!(
            rotated
                .raw_secret
                .starts_with(TRIGGER_DELIVERY_SECRET_PREFIX)
        );
        assert_ne!(Some(rotated.key_id.clone()), previous_key_id);
        assert_eq!(
            rotated.trigger.delivery_key_id.as_deref(),
            Some(rotated.key_id.as_str())
        );
        let decrypted = keys
            .decrypt(
                rotated
                    .trigger
                    .delivery_secret_encrypted
                    .as_deref()
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(decrypted, rotated.raw_secret.as_bytes());
        current.delivery = TriggerDelivery::Notification;
        assert!(matches!(
            rotate_delivery_secret(&db, &keys, &current).await,
            Err(AppError::TriggerDeliveryUnsupported)
        ));
    }
}
