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

use super::{api_key_mutation_service as transactions, trigger_schedule as schedules};
use crate::models::trigger_schedule::{OverlapPolicy, ScheduleSpec, ScheduleState, TriggerSource};

pub const TRIGGER_SECRET_PREFIX: &str = "nyx_trg_";
pub const TRIGGER_DELIVERY_SECRET_PREFIX: &str = "nyx_twh_";
pub const TRIGGER_ENVELOPE_OVERHEAD_BYTES: usize = 4 * 1024;
const METADATA_ONLY_RETENTION_HOURS: i64 = 72;
const MAX_LABEL_LEN: usize = 128;
const MAX_EVENT_ID_LEN: usize = 128;

#[derive(Debug)]
pub struct CreateInput {
    pub source: TriggerSource,
    pub setup_watch_id: Option<String>,
    pub schedule: Option<ScheduleSpec>,
    pub overlap: OverlapPolicy,
    pub user_id: String,
    pub label: String,
    pub user_service_id: Option<String>,
    pub verification: TriggerVerification,
    pub delivery: TriggerDelivery,
}

#[derive(Debug)]
pub struct UpdateInput {
    pub schedule: Option<ScheduleSpec>,
    pub overlap: Option<OverlapPolicy>,
    pub label: Option<String>,
    pub status: Option<TriggerStatus>,
    pub delivery: Option<TriggerDelivery>,
}

pub struct CreatedTrigger {
    pub trigger: Trigger,
    pub raw_secret: Zeroizing<String>,
    pub delivery_signing_secret: Option<Zeroizing<String>>,
    pub delivery_signing_key_id: Option<String>,
}

pub struct UpdatedTrigger {
    pub trigger: Trigger,
    pub delivery_signing_secret: Option<Zeroizing<String>>,
    pub delivery_signing_key_id: Option<String>,
}

pub struct RotatedDeliverySecret {
    pub trigger: Trigger,
    pub raw_secret: Zeroizing<String>,
    pub key_id: String,
}
crate::errors::redacted_debug!(CreatedTrigger, UpdatedTrigger, RotatedDeliverySecret);

#[cfg(test)]
tokio::task_local! {
    static UPDATE_COMMITTED: (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);
}

pub enum WebhookAdmission {
    Accepted {
        record: Box<TriggerDeliveryRecord>,
        body: Zeroizing<Vec<u8>>,
    },
    Duplicate,
}

fn new_secret(prefix: &str) -> Zeroizing<String> {
    let random = Zeroizing::new(generate_random_token());
    let mut secret = Zeroizing::new(String::with_capacity(prefix.len() + random.len()));
    secret.push_str(prefix);
    secret.push_str(&random);
    secret
}

#[derive(serde::Serialize)]
pub struct TriggerEventEnvelope {
    pub event_id: String,
    pub trigger_id: String,
    pub source: &'static str,
    pub received_at: chrono::DateTime<Utc>,
    pub payload: serde_json::Value,
}

impl Drop for TriggerEventEnvelope {
    fn drop(&mut self) {
        crate::crypto::secret_json::clear_json_strings(&mut self.payload);
    }
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
    let settings = if input.source == TriggerSource::Schedule {
        Some(super::assistant_settings_service::get(db, &input.user_id).await?)
    } else {
        None
    };
    let mut schedule_state = ScheduleState::default();
    if input.source == TriggerSource::Schedule {
        let spec = input
            .schedule
            .as_ref()
            .ok_or_else(|| AppError::ValidationError("schedule is required".into()))?;
        schedules::compute::validate(spec, settings.as_ref().unwrap().schedule_minimum_minutes)?;
        schedule_state.next_due_at = schedules::compute::next(spec, Utc::now())?;
        if schedule_state.next_due_at.is_none() {
            return Err(AppError::ValidationError(
                "Schedule has no future occurrence".into(),
            ));
        }
    } else if input.schedule.is_some()
        || matches!(input.verification, TriggerVerification::Schedule)
    {
        return Err(AppError::ValidationError(
            "Webhook triggers require inbound verification and no schedule".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let (delivery, delivery_secret_encrypted, delivery_signing_secret, delivery_key_id) =
        prepare_delivery(encryption_keys, &id, &input.user_id, input.delivery).await?;
    let raw_secret = if input.source == TriggerSource::Webhook {
        new_secret(TRIGGER_SECRET_PREFIX)
    } else {
        Zeroizing::new(String::new())
    };
    let verification_secret_encrypted = if input.source == TriggerSource::Schedule {
        None
    } else {
        match input.verification {
            TriggerVerification::HmacSha256 { .. } => Some(
                super::trigger_record_service::encrypt_secret(
                    encryption_keys,
                    &id,
                    &input.user_id,
                    "verification_secret_encrypted",
                    raw_secret.as_bytes(),
                )
                .await?,
            ),
            TriggerVerification::Token { .. } | TriggerVerification::Schedule => None,
        }
    };
    let now = Utc::now();
    let mut secret_context_versions = std::collections::BTreeMap::new();
    for (field, present) in [
        (
            "verification_secret_encrypted",
            verification_secret_encrypted.is_some(),
        ),
        (
            "delivery_secret_encrypted",
            delivery_secret_encrypted.is_some(),
        ),
    ] {
        if present {
            super::credential_record_service::mark_version(
                &mut secret_context_versions,
                encryption_keys,
                field,
            );
        }
    }
    let trigger = Trigger {
        secret_context_versions,
        id: id.clone(),
        source: input.source,
        setup_watch_id: input.setup_watch_id,
        schedule: input.schedule,
        schedule_state,
        overlap: input.overlap,
        user_id: input.user_id,
        label,
        user_service_id: input.user_service_id,
        status: TriggerStatus::Active,
        secret_hash: if input.source == TriggerSource::Schedule {
            format!("schedule:{id}")
        } else {
            hash_token(&raw_secret)
        },
        verification: if input.source == TriggerSource::Schedule {
            TriggerVerification::Schedule
        } else {
            input.verification
        },
        verification_secret_encrypted,
        delivery,
        delivery_secret_encrypted,
        delivery_key_id: delivery_key_id.clone(),
        created_at: now,
        updated_at: now,
    };
    let saved = trigger.clone();
    let db_copy = db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                super::erasure_admission::fence_in_session(&db_copy, &saved.user_id, session)
                    .await?;
                if let Some(watch_id) = &saved.setup_watch_id {
                    if saved.source != TriggerSource::Webhook
                        || !matches!(saved.delivery, TriggerDelivery::Assistant { .. })
                    {
                        return Err(AppError::ValidationError(
                            "Setup links require a webhook assistant automation".into(),
                        ));
                    }
                    let consumed = db_copy
                        .collection::<bson::Document>(
                            crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
                        )
                        .update_one(
                            doc! {
                                "_id": watch_id,
                                "user_id": &saved.user_id,
                                "kind": "trigger_created",
                                "status": "pending",
                                "trigger_id": null,
                                "expires_at": { "$gt": schedules::date(Utc::now()) },
                            },
                            doc! { "$set": {"trigger_id": &saved.id} },
                        )
                        .session(&mut *session)
                        .await?;
                    if consumed.modified_count != 1 {
                        return Err(AppError::ValidationError(
                            "Webhook setup link expired, was used, or belongs to another owner"
                                .into(),
                        ));
                    }
                }
                if saved.source == TriggerSource::Schedule {
                    db_copy
                        .collection::<bson::Document>(schedules::OWNER_LIMITS)
                        .update_one(
                            doc! { "_id": &saved.user_id },
                            doc! { "$inc": {"version":1} },
                        )
                        .upsert(true)
                        .session(&mut *session)
                        .await?;
                    if db_copy
                        .collection::<Trigger>(TRIGGERS)
                        .count_documents(doc! { "user_id": &saved.user_id, "source": "schedule" })
                        .session(&mut *session)
                        .await?
                        >= 100
                    {
                        return Err(AppError::ValidationError(
                            "An owner can have at most 100 schedules".into(),
                        ));
                    }
                }
                db_copy
                    .collection::<Trigger>(TRIGGERS)
                    .insert_one(&saved)
                    .session(&mut *session)
                    .await?;
                schedules::set_schedule_job(&db_copy, &saved, session).await?;
                Ok(())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
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
    let source = capture_source(db, current).await?;
    let mut next = current.clone();
    next.schedule_state.terminal_failure_code = None;
    next.schedule_state.terminal_failure_count = 0;
    next.updated_at = Utc::now().max(current.updated_at + chrono::Duration::milliseconds(1));
    let mut delivery_signing_secret = None;
    if let Some(label) = input.label {
        next.label = validate_label(&label)?;
    }
    if let Some(status) = input.status {
        next.status = status;
        next.schedule_state.pause_reason = None;
    }
    if let Some(overlap) = input.overlap {
        next.overlap = overlap;
    }
    if let Some(delivery) = input
        .delivery
        .filter(|delivery| *delivery != current.delivery)
    {
        validate_associations(
            db,
            &current.user_id,
            current.user_service_id.as_deref(),
            &delivery,
        )
        .await?;
        let (delivery, encrypted, raw, key) =
            prepare_delivery(encryption_keys, &current.id, &current.user_id, delivery).await?;
        next.delivery = delivery;
        next.delivery_secret_encrypted = encrypted;
        next.secret_context_versions
            .remove("delivery_secret_encrypted");
        if next.delivery_secret_encrypted.is_some() {
            super::credential_record_service::mark_version(
                &mut next.secret_context_versions,
                encryption_keys,
                "delivery_secret_encrypted",
            );
        }
        next.delivery_key_id = key;
        delivery_signing_secret = raw;
        if next.delivery != current.delivery {
            next.schedule_state.dedicated_thread_id = None;
        }
    }
    if let Some(spec) = input.schedule {
        if current.source != TriggerSource::Schedule {
            return Err(AppError::ValidationError(
                "A webhook source cannot be changed to a schedule".into(),
            ));
        }
        let settings = super::assistant_settings_service::get(db, &current.user_id).await?;
        schedules::compute::validate(&spec, settings.schedule_minimum_minutes)?;
        next.schedule = Some(spec);
    }
    if next.source == TriggerSource::Schedule
        && (input.status == Some(TriggerStatus::Active) || next.schedule != current.schedule)
    {
        let spec = next
            .schedule
            .as_ref()
            .ok_or_else(|| AppError::ValidationError("Schedule specification missing".into()))?;
        next.schedule_state.next_due_at = if spec
            .max_runs
            .is_some_and(|max| next.schedule_state.occurrences >= max)
        {
            None
        } else {
            schedules::compute::next(spec, Utc::now())?
        };
    }
    let schedule_changed = next.schedule != current.schedule || input.status.is_some();
    let target_changed = match (&next.delivery, &current.delivery) {
        (
            TriggerDelivery::Assistant {
                agent_id: a,
                thread_policy: ap,
                ..
            },
            TriggerDelivery::Assistant {
                agent_id: b,
                thread_policy: bp,
                ..
            },
        ) => a != b || ap != bp,
        _ => next.delivery != current.delivery,
    };
    let db_copy = db.clone();
    let saved = next.clone();
    let mut session = db.client().start_session().await?;
    let trigger = session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<Trigger> = async {
                super::erasure_admission::fence_in_session(&db_copy,&saved.user_id,session).await?;
                let encrypted = saved
                    .delivery_secret_encrypted
                    .as_ref()
                    .map(|bytes| {
                        bson::Bson::Binary(bson::Binary {
                            subtype: bson::spec::BinarySubtype::Generic,
                            bytes: bytes.clone(),
                        })
                    })
                    .unwrap_or(bson::Bson::Null);
                let mut set = doc! {
                    "label": &saved.label,
                    "status": bson::to_bson(&saved.status).map_err(serialization_error)?,
                    "delivery": bson::to_bson(&saved.delivery).map_err(serialization_error)?,
                    "delivery_secret_encrypted": encrypted,
                    "secret_context_versions":bson::to_bson(&saved.secret_context_versions).map_err(serialization_error)?,
                    "delivery_key_id": &saved.delivery_key_id,
                    "overlap": bson::to_bson(&saved.overlap).map_err(serialization_error)?,
                    "schedule": bson::to_bson(&saved.schedule).map_err(serialization_error)?,
                    "updated_at": schedules::date(saved.updated_at),
                    "schedule_state.pause_reason": &saved.schedule_state.pause_reason,
                    "schedule_state.terminal_failure_code": null,
                    "schedule_state.terminal_failure_count": 0,
                };
                let mut saved = saved.clone();
                if schedule_changed {
                    let live = db_copy
                        .collection::<Trigger>(TRIGGERS)
                        .find_one(doc! { "_id": &saved.id })
                        .session(&mut *session)
                        .await?
                        .ok_or(AppError::TriggerNotFound)?;
                    saved.schedule_state.occurrences = live.schedule_state.occurrences;
                    if let Some(spec) = &saved.schedule {
                        saved.schedule_state.next_due_at = if spec
                            .max_runs
                            .is_some_and(|max| live.schedule_state.occurrences >= max)
                        {
                            None
                        } else {
                            schedules::compute::next(spec, Utc::now())?
                        };
                    }
                    set.insert(
                        "schedule_state.next_due_at",
                        saved.schedule_state.next_due_at.map(schedules::date),
                    );
                }
                if target_changed {
                    set.insert("schedule_state.dedicated_thread_id", bson::Bson::Null);
                }
                let committed = db_copy
                    .collection::<Trigger>(TRIGGERS)
                    .find_one_and_update(
                        super::record_storage::exact_document_predicate(&source),
                        doc! { "$set": set },
                    )
                    .return_document(ReturnDocument::After)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::Conflict(
                        "Trigger changed; reload and retry".into(),
                    ))?;
                if schedule_changed {
                    schedules::set_schedule_job(&db_copy, &saved, session).await?;
                }
                Ok(committed)
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    #[cfg(test)]
    if let Ok((entered, release)) = UPDATE_COMMITTED.try_with(Clone::clone) {
        entered.notify_one();
        release.notified().await;
    }
    Ok(UpdatedTrigger {
        delivery_signing_key_id: trigger.delivery_key_id.clone(),
        trigger,
        delivery_signing_secret,
    })
}

pub async fn delete(db: &Database, current: &Trigger) -> AppResult<()> {
    let result = db
        .collection::<Trigger>(TRIGGERS)
        .delete_one(doc! { "_id": &current.id, "user_id": &current.user_id })
        .await?;
    if result.deleted_count == 1 {
        db.collection::<bson::Document>(schedules::WORK)
            .delete_many(doc! {"trigger_id":&current.id,"kind":"schedule"})
            .await?;
        Ok(())
    } else {
        Err(AppError::TriggerNotFound)
    }
}

pub async fn rotate_secret(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    current: &Trigger,
) -> AppResult<(Trigger, Zeroizing<String>)> {
    if current.source == TriggerSource::Schedule {
        return Err(AppError::ValidationError(
            "Scheduled automations have no inbound secret".into(),
        ));
    }
    let source = capture_source(db, current).await?;
    let raw_secret = new_secret(TRIGGER_SECRET_PREFIX);
    let mut set_doc = doc! {
        "secret_hash": hash_token(&raw_secret),
        "schedule_state.terminal_failure_code": null,
        "schedule_state.terminal_failure_count": 0,
        "updated_at": bson::DateTime::from_chrono(Utc::now().max(current.updated_at + chrono::Duration::milliseconds(1))),
    };
    let update = match current.verification {
        TriggerVerification::HmacSha256 { .. } => {
            let encrypted = super::trigger_record_service::encrypt_secret(
                encryption_keys,
                &current.id,
                &current.user_id,
                "verification_secret_encrypted",
                raw_secret.as_bytes(),
            )
            .await?;
            if encryption_keys.contextual_record_writes() {
                set_doc.insert(
                    "secret_context_versions.verification_secret_encrypted",
                    3_i32,
                );
            } else {
                set_doc.insert(
                    "secret_context_versions",
                    bson::to_bson(&{
                        let mut versions = current.secret_context_versions.clone();
                        versions.remove("verification_secret_encrypted");
                        versions
                    })
                    .map_err(serialization_error)?,
                );
            }
            set_doc.insert(
                "verification_secret_encrypted",
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
            );
            doc! { "$set": set_doc }
        }
        TriggerVerification::Schedule => return Err(AppError::TriggerDeliveryUnsupported),
        TriggerVerification::Token { .. } => doc! {
            "$set": set_doc,
            "$unset": { "verification_secret_encrypted": "", "secret_context_versions.verification_secret_encrypted":"" },
        },
    };
    let updated = commit_rotation(db, current, &source, update).await?;
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
    let source = capture_source(db, current).await?;
    let raw_secret = new_secret(TRIGGER_DELIVERY_SECRET_PREFIX);
    let encrypted = super::trigger_record_service::encrypt_secret(
        encryption_keys,
        &current.id,
        &current.user_id,
        "delivery_secret_encrypted",
        raw_secret.as_bytes(),
    )
    .await?;
    let mut versions = current.secret_context_versions.clone();
    super::credential_record_service::mark_version(
        &mut versions,
        encryption_keys,
        "delivery_secret_encrypted",
    );
    let key_id = webhook_delivery_service::generate_signing_key_id();
    let trigger=commit_rotation(db,current,&source,doc! { "$set": {
                "delivery_secret_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
                "delivery_key_id": &key_id,
                "secret_context_versions":bson::to_bson(&versions).map_err(serialization_error)?,
                "schedule_state.terminal_failure_code": null,
                "schedule_state.terminal_failure_count": 0,
                "updated_at": bson::DateTime::from_chrono(Utc::now().max(current.updated_at + chrono::Duration::milliseconds(1))),
            }}).await?;
    Ok(RotatedDeliverySecret {
        trigger,
        raw_secret,
        key_id,
    })
}

async fn capture_source(db: &Database, current: &Trigger) -> AppResult<bson::Document> {
    let raw = db
        .collection::<bson::Document>(TRIGGERS)
        .find_one(doc! {"_id":&current.id,"user_id":&current.user_id})
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    let stored: Trigger = bson::from_document(raw.clone()).map_err(serialization_error)?;
    if bson::to_document(&stored).map_err(serialization_error)?
        != bson::to_document(current).map_err(serialization_error)?
    {
        return Err(AppError::Conflict(
            "Trigger changed; reload and retry".into(),
        ));
    }
    Ok(raw)
}

async fn commit_rotation(
    db: &Database,
    current: &Trigger,
    source: &bson::Document,
    update: bson::Document,
) -> AppResult<Trigger> {
    let mut tx = super::erasure_admission::begin_write(db, &current.user_id).await?;
    let result: AppResult<Trigger> = async {
        db.collection::<Trigger>(TRIGGERS)
            .find_one_and_update(
                super::record_storage::exact_document_predicate(source),
                update,
            )
            .return_document(ReturnDocument::After)
            .session(&mut tx)
            .await?
            .ok_or_else(|| AppError::Conflict("Trigger changed; reload and retry".into()))
    }
    .await;
    super::org_membership_authority::finish(tx, result).await
}

pub async fn load_active_for_ingress(db: &Database, id: &str) -> AppResult<Trigger> {
    let trigger = db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! { "_id": id, "status": "active", "source": {"$ne":"schedule"} })
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
        TriggerVerification::Schedule => return Err(AppError::TriggerNotFound),
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
            let secret = super::trigger_record_service::secret(
                encryption_keys,
                trigger,
                "verification_secret_encrypted",
            )
            .await?;
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
    let mut envelope = TriggerEventEnvelope {
        event_id: event_id.to_string(),
        trigger_id: trigger.id.clone(),
        source: if trigger.source == TriggerSource::Schedule {
            "schedule"
        } else {
            "inbound_webhook"
        },
        received_at: Utc::now(),
        payload,
    };
    let mut value = crate::crypto::secret_json::SecretJson(
        serde_json::to_value(&envelope).map_err(serialization_error)?,
    );
    match &trigger.delivery {
        TriggerDelivery::Assistant { .. } => return Err(AppError::TriggerDeliveryUnsupported),
        TriggerDelivery::Webhook { .. } => {
            let max_body_bytes = config
                .trigger_payload_max_bytes
                .saturating_add(TRIGGER_ENVELOPE_OVERHEAD_BYTES);
            if trigger.source == TriggerSource::Schedule {
                let admission = admit_webhook_delivery(
                    db,
                    encryption_keys,
                    trigger,
                    event_id,
                    std::mem::take(&mut envelope.payload),
                    config.trigger_delivery_retention_hours,
                    max_body_bytes,
                )
                .await?;
                if let WebhookAdmission::Accepted { record, body } = admission {
                    let outcome = deliver_webhook_body(
                        encryption_keys,
                        http_client,
                        trigger,
                        event_id,
                        &body,
                        max_body_bytes,
                    )
                    .await;
                    update_delivery_after_attempt(db, &record, &outcome).await?;
                    outcome.map_err(|_| AppError::TriggerDeliveryFailed)?;
                }
            } else {
                let body = Zeroizing::new(
                    crate::crypto::secret_json::to_secret_string(&envelope)
                        .map_err(serialization_error)?
                        .as_bytes()
                        .to_vec(),
                );
                deliver_webhook_body(
                    encryption_keys,
                    http_client,
                    trigger,
                    event_id,
                    &body,
                    max_body_bytes,
                )
                .await
                .map_err(|_| AppError::TriggerDeliveryFailed)?;
            }
        }
        TriggerDelivery::Agent { conversation_id } => {
            let agent_envelope = EventEnvelope {
                event_id: agent_event_uuid(&trigger.id, event_id),
                source: envelope.source.to_string(),
                event_type: "trigger.event".to_string(),
                timestamp: envelope.received_at,
                payload: Some(std::mem::take(&mut value.0)),
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
        source: if trigger.source == TriggerSource::Schedule {
            "schedule"
        } else {
            "inbound_webhook"
        },
        received_at: now,
        payload,
    };
    let body = Zeroizing::new(
        crate::crypto::secret_json::to_secret_string(&envelope)
            .map_err(serialization_error)?
            .as_bytes()
            .to_vec(),
    );
    if body.len() > max_body_bytes {
        return Err(AppError::TriggerPayloadTooLarge);
    }

    let retained_hours = if retention_hours == 0 {
        METADATA_ONLY_RETENTION_HOURS
    } else {
        i64::try_from(retention_hours).unwrap_or(i64::MAX)
    };
    let mut record = TriggerDeliveryRecord {
        secret_context_versions: Default::default(),
        id: delivery_record_id(&trigger.id, event_id),
        trigger_id: trigger.id.clone(),
        user_id: trigger.user_id.clone(),
        event_id: event_id.to_string(),
        status: TriggerDeliveryRecordStatus::Pending,
        attempts: 0,
        last_status_code: None,
        envelope_encrypted: None,
        created_at: now,
        updated_at: now,
        expires_at: now + chrono::Duration::hours(retained_hours),
        delivered_at: None,
    };
    if retention_hours != 0 {
        record.envelope_encrypted = Some(
            super::trigger_record_service::encrypt_envelope(encryption_keys, &record, &body)
                .await?,
        );
        super::credential_record_service::mark_version(
            &mut record.secret_context_versions,
            encryption_keys,
            "envelope_encrypted",
        );
    }
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        super::erasure_admission::fence_in_session(db, &trigger.user_id, &mut session).await?;
        if db
            .collection::<Trigger>(TRIGGERS)
            .find_one(doc! {"_id":&trigger.id,"user_id":&trigger.user_id,"status":"active"})
            .session(&mut session)
            .await?
            .is_none()
        {
            return Err(AppError::TriggerNotFound);
        }
        db.collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
            .insert_one(&record)
            .session(&mut session)
            .await?;
        Ok(())
    }
    .await;
    match super::org_membership_authority::finish(session, result).await {
        Ok(()) => Ok(WebhookAdmission::Accepted {
            record: Box::new(record),
            body,
        }),
        Err(AppError::DatabaseError(error)) if is_duplicate_key_error(&error) => {
            Ok(WebhookAdmission::Duplicate)
        }
        Err(error) => Err(error),
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
    body: Zeroizing<Vec<u8>>,
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
    if trigger.delivery_secret_encrypted.is_none() {
        return Err(DeliveryFailure {
            attempts: 0,
            reason: "signing_secret_missing",
            last_status: None,
        });
    }
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
    let secret = super::trigger_record_service::secret(
        encryption_keys,
        trigger,
        "delivery_secret_encrypted",
    )
    .await
    .map_err(|_| DeliveryFailure {
        attempts: 0,
        reason: "secret_decrypt_failed",
        last_status: None,
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
    let body = super::trigger_record_service::envelope(encryption_keys, &record).await?;
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
    id: &str,
    owner: &str,
    delivery: TriggerDelivery,
) -> AppResult<(
    TriggerDelivery,
    Option<Vec<u8>>,
    Option<Zeroizing<String>>,
    Option<String>,
)> {
    match delivery {
        TriggerDelivery::Webhook { url } => {
            let url = webhook_delivery_service::validate_webhook_url(&url, "delivery.url").await?;
            let raw = new_secret(TRIGGER_DELIVERY_SECRET_PREFIX);
            let encrypted = super::trigger_record_service::encrypt_secret(
                encryption_keys,
                id,
                owner,
                "delivery_secret_encrypted",
                raw.as_bytes(),
            )
            .await?;
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

pub(crate) async fn validate_associations(
    db: &Database,
    user_id: &str,
    user_service_id: Option<&str>,
    delivery: &TriggerDelivery,
) -> AppResult<()> {
    if let Some(service_id) = user_service_id {
        let exists = crate::services::service_history::collection::<UserService>(db, USER_SERVICES)
            .find_one(doc! { "_id": service_id, "user_id": user_id })
            .await?
            .is_some();
        if !exists {
            return Err(AppError::TriggerNotFound);
        }
    }
    if let TriggerDelivery::Assistant {
        agent_id,
        instruction,
        ..
    } = delivery
    {
        if instruction.trim().is_empty() || instruction.len() > 8192 {
            return Err(AppError::ValidationError(
                "Instruction must contain 1–8192 bytes".into(),
            ));
        }
        let agent = super::assistant_team_service::agent(db, user_id, agent_id).await?;
        if agent.destroyed_at.is_some() {
            return Err(AppError::ValidationError(
                "Target agent was destroyed".into(),
            ));
        }
        super::assistant_nyxagent::require_enabled(db, user_id).await?;
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

    #[tokio::test]
    async fn update_returns_its_committed_delivery_secret_pair_during_successor_rotation() {
        let db =
            crate::test_utils::connect_transaction_test_database("trigger_response_pair").await;
        let keys = test_encryption_keys();
        let owner = Uuid::new_v4().to_string();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let created = create(
            &db,
            &keys,
            CreateInput {
                source: Default::default(),
                setup_watch_id: None,
                schedule: None,
                overlap: Default::default(),
                user_id: owner,
                label: "Pairing".into(),
                user_service_id: None,
                verification: TriggerVerification::HmacSha256 {
                    header_name: "X-Signature".into(),
                },
                delivery: TriggerDelivery::Notification,
            },
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let change = webhook_delivery_service::with_test_webhook_target(
            address,
            UPDATE_COMMITTED.scope(
                (entered.clone(), release.clone()),
                update(
                    &db,
                    &keys,
                    &created.trigger,
                    UpdateInput {
                        schedule: None,
                        overlap: None,
                        label: None,
                        status: None,
                        delivery: Some(TriggerDelivery::Webhook {
                            url: format!("http://{address}/hook"),
                        }),
                    },
                ),
            ),
        );
        let rotate = async {
            tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
                .await
                .expect("update did not reach committed-response barrier");
            let committed = db
                .collection::<Trigger>(TRIGGERS)
                .find_one(doc! {"_id":&created.trigger.id})
                .await
                .unwrap()
                .unwrap();
            let successor = rotate_delivery_secret(&db, &keys, &committed)
                .await
                .unwrap();
            release.notify_one();
            Ok::<_, AppError>((committed, successor))
        };
        // An early update error cancels the paired waiter. Bound the entire
        // barrier as well so a missing notification cannot hang the suite.
        let (updated, (committed, successor)) =
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                tokio::try_join!(change, rotate)
            })
            .await
            .expect("committed-response pairing timed out")
            .unwrap();
        assert_eq!(updated.trigger.delivery_key_id, committed.delivery_key_id);
        assert_eq!(updated.delivery_signing_key_id, committed.delivery_key_id);
        let secret = super::super::trigger_record_service::secret(
            &keys,
            &committed,
            "delivery_secret_encrypted",
        )
        .await
        .unwrap();
        assert_eq!(
            updated.delivery_signing_secret.as_ref().unwrap().as_bytes(),
            secret.as_slice()
        );
        assert_ne!(
            updated.delivery_signing_key_id.as_deref(),
            Some(successor.key_id.as_str())
        );
        let live = db
            .collection::<Trigger>(TRIGGERS)
            .find_one(doc! {"_id":&created.trigger.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            live.delivery_key_id.as_deref(),
            Some(successor.key_id.as_str())
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn live_hmac_rotation_and_delivery_support_staging_migration_and_strict_reads() {
        let db = connect_test_database("trigger_contextual_lifecycle")
            .await
            .expect("MongoDB fixture required");
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let sent = sent.clone();
                async move {
                    sent.send((headers, body)).unwrap();
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut staged = test_encryption_keys();
        staged.set_contextual_record_writes(false);
        let migration = test_encryption_keys();
        let mut strict = test_encryption_keys();
        strict.set_allow_legacy_record_reads(false);
        let owner = Uuid::new_v4().to_string();
        let created = webhook_delivery_service::with_test_webhook_target(
            address,
            create(
                &db,
                &staged,
                CreateInput {
                    source: Default::default(),
                    setup_watch_id: None,
                    schedule: None,
                    overlap: Default::default(),
                    user_id: owner,
                    label: "Contextual webhook".into(),
                    user_service_id: None,
                    verification: TriggerVerification::HmacSha256 {
                        header_name: "X-Hub-Signature-256".into(),
                    },
                    delivery: TriggerDelivery::Webhook {
                        url: format!("http://{address}/hook"),
                    },
                },
            ),
        )
        .await
        .unwrap();
        assert!(created.trigger.secret_context_versions.is_empty());
        // The reader stage retains the exact pre-cutover encrypted format.
        assert_eq!(
            staged
                .decrypt(
                    created
                        .trigger
                        .verification_secret_encrypted
                        .as_deref()
                        .unwrap()
                )
                .await
                .unwrap()
                .as_slice(),
            created.raw_secret.as_bytes()
        );
        let body = br#"{"action":"opened"}"#;
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Hub-Signature-256",
            HeaderValue::from_str(&format!(
                "sha256={}",
                webhook_delivery_service::compute_body_signature(
                    created.raw_secret.as_bytes(),
                    body
                )
            ))
            .unwrap(),
        );
        verify_ingress(&staged, &created.trigger, &headers, None, body)
            .await
            .unwrap();
        assert!(
            verify_ingress(&strict, &created.trigger, &headers, None, body)
                .await
                .is_err()
        );
        let WebhookAdmission::Accepted {
            record,
            body: retained,
        } = admit_webhook_delivery(
            &db,
            &staged,
            &created.trigger,
            "before-cutover",
            serde_json::json!({"action":"opened"}),
            72,
            1024 * 1024,
        )
        .await
        .unwrap()
        else {
            panic!("first delivery must be admitted")
        };
        assert!(record.secret_context_versions.is_empty());
        assert_eq!(
            staged
                .decrypt(record.envelope_encrypted.as_deref().unwrap())
                .await
                .unwrap()
                .as_slice(),
            retained.as_slice()
        );
        for (collection, id) in [
            (TRIGGERS, &created.trigger.id),
            (TRIGGER_DELIVERIES, &record.id),
        ] {
            let raw = db
                .collection::<bson::Document>(collection)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                super::super::trigger_record_service::migrate_snapshot(
                    &db, &migration, collection, &raw
                )
                .await
                .unwrap()
            );
            let current = db
                .collection::<bson::Document>(collection)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap();
            super::super::trigger_record_service::verify_current_snapshot(
                &strict, collection, &current,
            )
            .await
            .unwrap();
            assert!(
                !super::super::trigger_record_service::migrate_snapshot(
                    &db, &migration, collection, &current
                )
                .await
                .unwrap()
            );
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .find_one(doc! {"_id":id})
                    .await
                    .unwrap()
                    .unwrap(),
                current
            );
        }
        let live = load_active_for_ingress(&db, &created.trigger.id)
            .await
            .unwrap();
        verify_ingress(&strict, &live, &headers, None, body)
            .await
            .unwrap();
        let replayed = redeliver(
            &db,
            &strict,
            &reqwest::Client::new(),
            &live,
            "before-cutover",
            1024 * 1024,
        )
        .await
        .unwrap();
        assert_eq!(replayed.status, TriggerDeliveryRecordStatus::Delivered);
        let (wire_headers, wire_body) = received.recv().await.unwrap();
        assert_eq!(wire_body.as_ref(), retained.as_slice());
        let expected = webhook_delivery_service::compute_timestamped_signature(
            created.delivery_signing_secret.as_ref().unwrap().as_bytes(),
            wire_headers["X-NyxID-Timestamp"].to_str().unwrap(),
            &wire_body,
        );
        assert_eq!(
            wire_headers["X-NyxID-Signature"],
            format!("sha256={expected}")
        );
        let (rotated, secret) = rotate_secret(&db, &strict, &live).await.unwrap();
        assert_eq!(
            rotated
                .secret_context_versions
                .get("verification_secret_encrypted"),
            Some(&3)
        );
        assert!(
            verify_ingress(&strict, &rotated, &headers, None, body)
                .await
                .is_err()
        );
        headers.insert(
            "X-Hub-Signature-256",
            HeaderValue::from_str(&format!(
                "sha256={}",
                webhook_delivery_service::compute_body_signature(secret.as_bytes(), body)
            ))
            .unwrap(),
        );
        verify_ingress(&strict, &rotated, &headers, None, body)
            .await
            .unwrap();
        let outgoing = rotate_delivery_secret(&db, &strict, &rotated)
            .await
            .unwrap();
        assert_eq!(
            outgoing
                .trigger
                .secret_context_versions
                .get("delivery_secret_encrypted"),
            Some(&3)
        );
        let WebhookAdmission::Accepted { record, .. } = admit_webhook_delivery(
            &db,
            &strict,
            &outgoing.trigger,
            "after-cutover",
            serde_json::json!({"action":"closed"}),
            72,
            1024 * 1024,
        )
        .await
        .unwrap() else {
            panic!("new event must be admitted")
        };
        assert_eq!(
            record.secret_context_versions.get("envelope_encrypted"),
            Some(&3)
        );
        let mut moved = (*record).clone();
        moved.trigger_id = Uuid::new_v4().to_string();
        assert!(
            super::super::trigger_record_service::envelope(&strict, &moved)
                .await
                .is_err()
        );
        moved = (*record).clone();
        moved.event_id = "different-event".into();
        assert!(
            super::super::trigger_record_service::envelope(&strict, &moved)
                .await
                .is_err()
        );
        moved = (*record).clone();
        moved.user_id = Uuid::new_v4().to_string();
        assert!(
            super::super::trigger_record_service::envelope(&strict, &moved)
                .await
                .is_err()
        );
        server.abort();
        db.drop().await.unwrap();
    }

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
            secret_context_versions: Default::default(),
            source: Default::default(),
            setup_watch_id: None,
            schedule: None,
            schedule_state: Default::default(),
            overlap: Default::default(),
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
        db.collection::<Trigger>(TRIGGERS)
            .insert_one(&trigger)
            .await
            .unwrap();
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
        let plaintext = super::super::trigger_record_service::envelope(&keys, &record)
            .await
            .unwrap();
        assert_eq!(plaintext.as_slice(), body.as_slice());

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
    async fn webhook_admission_rejects_missing_or_deleted_live_parent() {
        let db = connect_test_database("trigger_delivery_missing_parent")
            .await
            .expect("Mongo8 required");
        let keys = test_encryption_keys();
        let trigger = webhook_trigger(&keys, "https://events.example.test/hook".into()).await;
        for previously_stored in [false, true] {
            if previously_stored {
                db.collection::<Trigger>(TRIGGERS)
                    .insert_one(&trigger)
                    .await
                    .unwrap();
                db.collection::<Trigger>(TRIGGERS)
                    .delete_one(doc! {"_id":&trigger.id})
                    .await
                    .unwrap();
            }
            assert!(matches!(
                admit_webhook_delivery(
                    &db,
                    &keys,
                    &trigger,
                    "missing-parent-event",
                    serde_json::json!({"action":"opened"}),
                    72,
                    256 * 1024 + TRIGGER_ENVELOPE_OVERHEAD_BYTES
                )
                .await,
                Err(AppError::TriggerNotFound)
            ));
            assert_eq!(
                db.collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn zero_retention_keeps_metadata_without_replayable_payload() {
        let Some(db) = connect_test_database("trigger_delivery_metadata_only").await else {
            return;
        };
        let keys = test_encryption_keys();
        let trigger = webhook_trigger(&keys, "https://events.example.test/hook".to_string()).await;
        db.collection::<Trigger>(TRIGGERS)
            .insert_one(&trigger)
            .await
            .unwrap();
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
        db.collection::<Trigger>(TRIGGERS)
            .insert_one(&trigger)
            .await
            .unwrap();
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
                source: Default::default(),
                setup_watch_id: None,
                schedule: None,
                overlap: Default::default(),
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
        assert!(!format!("{:?}", created.trigger).contains(created.raw_secret.as_str()));
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
                schedule: None,
                overlap: None,
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
        assert!(rotated.updated_at > updated.trigger.updated_at);
        assert!(matches!(
            rotate_secret(&db, &keys, &updated.trigger).await,
            Err(AppError::Conflict(_))
        ));

        delete(&db, &rotated).await.expect("delete trigger");
        assert!(matches!(
            rotate_secret(&db, &keys, &rotated).await,
            Err(AppError::TriggerNotFound)
        ));
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
        assert!(rotated.trigger.updated_at > current.updated_at);
        assert!(matches!(
            rotate_delivery_secret(&db, &keys, &current).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            rotated.trigger.delivery_key_id.as_deref(),
            Some(rotated.key_id.as_str())
        );
        let decrypted = super::super::trigger_record_service::secret(
            &keys,
            &rotated.trigger,
            "delivery_secret_encrypted",
        )
        .await
        .unwrap();
        assert_eq!(decrypted.as_slice(), rotated.raw_secret.as_bytes());
        db.collection::<Trigger>(TRIGGERS)
            .delete_one(doc! {"_id": &current.id})
            .await
            .unwrap();
        assert!(matches!(
            rotate_delivery_secret(&db, &keys, &rotated.trigger).await,
            Err(AppError::TriggerNotFound)
        ));
        current.delivery = TriggerDelivery::Notification;
        assert!(matches!(
            rotate_delivery_secret(&db, &keys, &current).await,
            Err(AppError::TriggerDeliveryUnsupported)
        ));
    }
}
