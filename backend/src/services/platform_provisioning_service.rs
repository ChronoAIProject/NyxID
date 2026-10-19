#![allow(
    clippy::too_many_arguments,
    clippy::collapsible_if,
    clippy::needless_borrow
)]

use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_binding::PlatformBinding,
        platform_installation::*,
        platform_setup::{PlatformSetup, SetupState},
        telegram_bot_request::TelegramBotRequest,
    },
};
use chrono::Utc;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};

fn stale() -> AppError {
    AppError::Conflict("Setup authority changed; review the current setup".into())
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<PlatformInstallation>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_user_id": 1_i64, "owner_retirement": 1_i64})
                .build(),
        )
        .await?;
    db.collection::<PlatformInstallation>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"channel_bot_id": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<PlatformInstallation>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"setup_id": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<PlatformInstallation>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"telegram_bot_id": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"telegram_bot_id": {"$type": "string"}})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<SetupStepJob>(STEPS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"phase": 1, "retry_at": 1})
                .build(),
        )
        .await?;
    db.collection::<SetupNotice>(NOTICES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"setup_id": 1, "revision": 1, "kind": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<SetupNotice>(NOTICES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<DeliveryTest>(TESTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub(crate) async fn fence_step(
    db: &Database,
    job: &SetupStepJob,
    session: &mut ClientSession,
) -> AppResult<()> {
    if job.claim_id.is_none() || db.collection::<Document>(STEPS).update_one(
        doc! {"_id": &job.id, "setup_id": &job.setup_id, "generation": job.generation, "claim_id": &job.claim_id,
            "authority": bson::to_bson(&job.authority).map_err(|_| stale())?, "phase": "running", "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
        doc! {"$inc": {"effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
    Ok(())
}

pub(crate) async fn fence_authority(
    db: &Database,
    a: &SetupStepAuthority,
    session: &mut ClientSession,
) -> AppResult<PlatformSetup> {
    super::platform_setup_service::fence_owner(db, &a.owner_user_id, session).await?;
    let states = match a.step {
        SetupStep::BeginTelegram => vec!["provisioning", "awaiting_child_review"],
        SetupStep::ConnectTelegram => vec!["provisioning", "ready_for_test", "active"],
    };
    let setup = db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).find_one_and_update(
        doc! {"_id": &a.setup_id, "owner_user_id": &a.owner_user_id, "revision": a.setup_revision, "authority_generation": a.authority_generation,
            "approved_revision": a.setup_revision, "approved_digest": &a.digest, "state": {"$in": states},
            "$expr": {"$or": [{"$eq": [{"$ifNull": ["$expires_at", bson::Bson::Null]}, bson::Bson::Null]}, {"$gt": ["$expires_at", "$$NOW"]}]}},
        doc! {"$inc": {"execution_fence": 1_i64}}).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
    if setup.template_version != 1
        || setup.platform_config_id.as_deref() != Some(a.platform_config_id.as_str())
        || setup.profile_generation != Some(a.profile_generation)
        || setup.binding_id.as_deref() != Some(a.binding_id.as_str())
        || setup.binding_generation != Some(a.binding_generation)
    {
        return Err(stale());
    }
    let profile = super::platform_binding_service::fence_profile(
        db,
        &a.platform_config_id,
        a.profile_generation,
        session,
    )
    .await?;
    if profile.manager_bot_id != a.manager_bot_id.to_string() {
        return Err(stale());
    }
    let credentials = db
        .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
        .find_one(
            doc! {"provider": "telegram-new", "fields.manager_bot_id": &profile.manager_bot_id,
            "fields.observation_id": &profile.observation_id, "fields.webhook_ready": "true"},
        )
        .session(&mut *session)
        .await?;
    if credentials.is_none() {
        return Err(stale());
    }
    if db.collection::<PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME).update_one(
        doc! {"_id": &a.binding_id, "user_id": &a.owner_user_id, "generation": a.binding_generation, "platform_config_id": &a.platform_config_id,
            "profile_generation": a.profile_generation, "manager_bot_id": a.manager_bot_id.to_string(), "telegram_sender_id": a.telegram_sender_id.to_string(),
            "telegram_chat_id": a.telegram_sender_id.to_string(), "is_active": true, "$expr": {"$gt": ["$expires_at", "$$NOW"]}},
        doc! {"$inc": {"setup_effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
    if a.step == SetupStep::ConnectTelegram && db.collection::<Document>(crate::models::telegram_bot_request::MANAGED_BOTS).find_one_and_update(
        doc! {"manager_bot_id": a.manager_bot_id, "telegram_bot_id": a.approved_child_bot_id, "telegram_user_id": a.telegram_sender_id,
            "revision": a.approved_child_revision, "observation_id": &profile.observation_id}, doc! {"$inc": {"setup_effect_revision": 1_i64}})
        .session(&mut *session).await?.is_none() { return Err(stale()); }
    if a.step == SetupStep::ConnectTelegram
        && (a.approved_child_bot_id.is_none()
            || a.approved_child_revision.is_none()
            || setup.resources.approved_telegram_bot_id.as_deref()
                != a.approved_child_bot_id.map(|id| id.to_string()).as_deref()
            || setup.resources.observed_creation_revision != a.approved_child_revision)
    {
        return Err(stale());
    }
    Ok(setup)
}

pub(crate) async fn fence_request(
    db: &Database,
    request: &TelegramBotRequest,
    connect: bool,
    session: &mut ClientSession,
) -> AppResult<Option<PlatformSetup>> {
    let installations = db.collection::<PlatformInstallation>(COLLECTION_NAME);
    let Some(a) = &request.setup_authority else {
        if installations
            .find_one(doc! {"channel_bot_id": &request.id})
            .session(&mut *session)
            .await?
            .is_some()
        {
            return Err(stale());
        }
        return Ok(None);
    };
    if a.owner_user_id != request.owner_user_id
        || request.actor_user_id != request.owner_user_id
        || a.manager_bot_id != request.manager_bot_id
        || (connect
            && (a.step != SetupStep::ConnectTelegram
                || a.approved_child_bot_id != request.telegram_bot_id
                || a.approved_child_revision != request.manager_revision))
    {
        return Err(stale());
    }
    let row = fence_authority(db, a, session).await?;
    let profile = db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &a.platform_config_id, "generation": a.profile_generation})
        .session(&mut *session)
        .await?
        .ok_or_else(stale)?;
    if profile.observation_id != request.observation_id {
        return Err(stale());
    }
    if row.resources.creation_request_id.as_deref() != Some(request.id.as_str())
        || row.resources.channel_bot_id.as_deref() != Some(request.id.as_str())
    {
        return Err(stale());
    }
    let current = db.collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME).find_one_and_update(
        doc! {"_id": &request.id, "connection_generation": request.connection_generation, "setup_authority": bson::to_bson(a).map_err(|_| stale())?,
            "telegram_user_id": bson::to_bson(&request.telegram_user_id).map_err(|_| stale())?,
            "telegram_bot_id": bson::to_bson(&request.telegram_bot_id).map_err(|_| stale())?,
            "manager_revision": bson::to_bson(&request.manager_revision).map_err(|_| stale())?,
            "observation_id": &request.observation_id,
            "$or": [
                {"status": {"$in": ["provisioning", "connected"]}},
                {"status": {"$in": ["waiting_telegram", "waiting_bot", "waiting_consent", "ready"]}, "active": true,
                 "$expr": {"$gt": ["$expires_at", "$$NOW"]}}
            ]}, doc! {"$inc": {"setup_effect_revision": 1_i64}})
        .session(&mut *session).await?.ok_or_else(stale)?;
    if current.get_str("observation_id").ok() != Some(request.observation_id.as_str()) {
        return Err(stale());
    }
    if request
        .telegram_user_id
        .is_some_and(|id| id != a.telegram_sender_id)
    {
        return Err(stale());
    }
    if let (Some(child), Some(revision)) = (request.telegram_bot_id, request.manager_revision) {
        if db.collection::<Document>(crate::models::telegram_bot_request::MANAGED_BOTS).find_one_and_update(
            doc! {"manager_bot_id": a.manager_bot_id, "telegram_bot_id": child, "telegram_user_id": a.telegram_sender_id, "revision": revision,
                "observation_id": &request.observation_id}, doc! {"$inc": {"setup_effect_revision": 1_i64}}).session(&mut *session).await?.is_none() { return Err(stale()); }
    }
    let installation = installations.find_one_and_update(doc! {"channel_bot_id": &request.id, "setup_id": &a.setup_id, "owner_user_id": &a.owner_user_id,
        "authority": bson::to_bson(a).map_err(|_| stale())?, "state": {"$nin": ["cancelled", "retired"]}}, doc! {"$inc": {"effect_revision": 1_i64}})
        .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
    if connect
        && installation.telegram_bot_id.as_deref()
            != request.telegram_bot_id.map(|id| id.to_string()).as_deref()
    {
        return Err(stale());
    }
    Ok(Some(row))
}

pub(crate) async fn admit_request(
    db: &Database,
    request: &TelegramBotRequest,
    connect: bool,
) -> AppResult<Option<PlatformSetup>> {
    let db = db.clone();
    let request = request.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transaction_result(fence_request(&db, &request, connect, session).await)
        })
        .await
        .map_err(map_transaction_error)
}

pub(crate) async fn capture(
    db: &Database,
    row: &PlatformSetup,
    digest: String,
    step: SetupStep,
) -> AppResult<SetupStepAuthority> {
    let binding = db.collection::<PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME).find_one(doc! {"_id": row.binding_id.as_deref().ok_or_else(stale)?, "user_id": row.owner_user_id.as_deref().ok_or_else(stale)?, "generation": row.binding_generation.ok_or_else(stale)?}).await?.ok_or_else(stale)?;
    Ok(SetupStepAuthority {
        setup_id: row.id.clone(),
        owner_user_id: row.owner_user_id.clone().ok_or_else(stale)?,
        setup_revision: row.revision,
        digest,
        authority_generation: row.authority_generation,
        platform_config_id: row.platform_config_id.clone().ok_or_else(stale)?,
        profile_generation: row.profile_generation.ok_or_else(stale)?,
        binding_id: binding.id,
        binding_generation: binding.generation,
        binding_revocation_generation: binding.revocation_generation,
        manager_bot_id: binding.manager_bot_id.parse().map_err(|_| stale())?,
        telegram_sender_id: binding.telegram_sender_id.parse().map_err(|_| stale())?,
        step,
        approved_child_bot_id: row
            .resources
            .approved_telegram_bot_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| stale())?,
        approved_child_revision: row.resources.observed_creation_revision,
    })
}

pub async fn approve_owned(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    owner: &str,
    id: &str,
    revision: i64,
    digest: &str,
) -> AppResult<PlatformSetup> {
    let row = super::platform_setup_service::get_owned(db, owner, id).await?;
    let facts = super::platform_setup_service::decrypt_facts(keys, &row).await?;
    super::platform_setup_service::validate_facts(row.template, &facts, true)?;
    super::platform_setup_service::validate_limits(&row.limits)?;
    if row.revision != revision
        || row
            .published
            .as_ref()
            .is_none_or(|published| published.facts != facts)
        || super::platform_setup_service::approval_digest(&row, &facts)? != digest
    {
        return Err(stale());
    }
    if row.approved_revision == Some(revision) && row.approved_digest.as_deref() == Some(digest) {
        return Ok(row);
    }
    if row.state != SetupState::AwaitingApproval
        || row.resources.creation_request_id.is_none()
        || row.resources.agent_id.is_none()
        || row.resources.runtime_model_service_id.is_none()
        || row.resources.runtime_model_id.is_none()
    {
        return Err(stale());
    }
    let existing = db
        .collection::<PlatformInstallation>(COLLECTION_NAME)
        .find_one(doc! {"setup_id": &row.id})
        .await?;
    let step = if existing.is_some() && row.resources.approved_telegram_bot_id.is_some() {
        SetupStep::ConnectTelegram
    } else {
        SetupStep::BeginTelegram
    };
    let a = capture(db, &row, digest.into(), step).await?;
    let db = db.clone();
    let owner = owner.to_owned();
    let row = row.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            super::platform_setup_service::fence_owner(&db, &owner, session).await?;
            let now = Utc::now();
            let approved = db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).find_one_and_update(doc! {"_id": &row.id, "owner_user_id": &owner, "revision": row.revision, "state": "awaiting_approval"},
                doc! {"$set": {"state": "provisioning", "approved_revision": row.revision, "approved_digest": &a.digest, "approved_by": &owner, "approved_at": bson::DateTime::from_chrono(now), "updated_at": bson::DateTime::from_chrono(now)}})
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
            fence_authority(&db, &a, session).await?;
            let request = approved.resources.creation_request_id.clone().ok_or_else(stale)?;
            let installation = PlatformInstallation {id: uuid::Uuid::new_v4().to_string(), setup_id: row.id.clone(), owner_user_id: owner.clone(), channel_bot_id: request.clone(), agent_id: approved.resources.agent_id.clone().ok_or_else(stale)?,
                telegram_bot_id: None, native_creation: None, generation: 1, state: "provisioning".into(), authority: a.clone(), created_at: now, updated_at: now};
            if let Some(old) = &existing {
                if old.owner_user_id != owner || old.channel_bot_id != request || old.agent_id != installation.agent_id || old.state == "retired" { return Err(stale()); }
                let native = db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME)
                    .find_one(doc! {"_id": &request, "owner_user_id": &owner}).session(&mut *session).await?;
                if a.step == SetupStep::ConnectTelegram {
                    if old.telegram_bot_id.as_deref() != a.approved_child_bot_id.map(|id| id.to_string()).as_deref() { return Err(stale()); }
                    let saved = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(doc! {"_id": &request}).session(&mut *session).await?;
                    if let Some(saved) = saved {
                        if saved.get_str("user_id").ok() != Some(owner.as_str()) || saved.get_str("platform_installation_id").ok() != Some(old.id.as_str())
                            || saved.get_str("platform_bot_id").ok() != old.telegram_bot_id.as_deref() || saved.get_bool("is_active").ok() != Some(true)
                            || !matches!(saved.get_str("status").ok(), Some("pending" | "active")) { return Err(stale()); }
                    }
                    let provenance = old.native_creation.as_ref().ok_or_else(stale)?;
                    if provenance.sender_id != a.telegram_sender_id || provenance.child_revision != a.approved_child_revision.ok_or_else(stale)? { return Err(stale()); }
                    if db.collection::<Document>(crate::models::telegram_bot_request::MANAGED_BOTS).find_one(
                        doc! {"manager_bot_id": a.manager_bot_id, "telegram_bot_id": a.approved_child_bot_id, "telegram_user_id": a.telegram_sender_id,
                            "revision": a.approved_child_revision, "created_by": a.telegram_sender_id, "observation_id": &provenance.observation_id})
                        .session(&mut *session).await?.is_none() { return Err(stale()); }
                }
                if db.collection::<PlatformInstallation>(COLLECTION_NAME).update_one(doc! {"_id": &old.id, "generation": old.generation},
                    doc! {"$set": {"authority": bson::to_bson(&a).map_err(|_| stale())?, "state": "provisioning", "updated_at": bson::DateTime::now()}, "$unset": {"owner_retirement": ""}, "$inc": {"generation": 1_i64}})
                    .session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
                if let Some(native) = native {
                    let profile = db.collection::<crate::models::platform_bot_config::PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME).find_one(doc! {"_id": &a.platform_config_id}).session(&mut *session).await?.ok_or_else(stale)?;
                    if native.manager_bot_id != a.manager_bot_id || native.observation_id != profile.observation_id
                        || native.telegram_user_id.is_some_and(|id| id != a.telegram_sender_id) { return Err(stale()); }
                    let started = matches!(native.status, crate::models::telegram_bot_request::TelegramRequestStatus::Provisioning | crate::models::telegram_bot_request::TelegramRequestStatus::Connected);
                    let status = if started {bson::to_bson(&native.status).map_err(|_| stale())?}
                        else if a.step == SetupStep::ConnectTelegram {"ready".into()}
                        else if old.telegram_bot_id.is_some() {"waiting_consent".into()} else {"waiting_telegram".into()};
                    db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME).update_one(
                        doc! {"_id": &request, "connection_generation": native.connection_generation},
                        doc! {"$set": {"setup_authority": bson::to_bson(&a).map_err(|_| stale())?, "auto_connect": false, "status": status,
                            "active": native.status != crate::models::telegram_bot_request::TelegramRequestStatus::Connected,
                            "expires_at": bson::DateTime::from_chrono(now + chrono::Duration::minutes(15)), "purge_after": bson::Bson::Null}, "$inc": {"connection_generation": 1_i64}})
                        .session(&mut *session).await?;
                } else if old.telegram_bot_id.is_some() {
                    let rebuilt = rebuild_request(&db, old, &a, session).await?;
                    db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME).insert_one(&rebuilt).session(&mut *session).await?;
                }
                db.collection::<SetupStepJob>(STEPS).update_one(doc! {"_id": &request, "setup_id": &row.id},
                    doc! {"$set": {"authority": bson::to_bson(&a).map_err(|_| stale())?, "phase": "queued", "claim_id": bson::Bson::Null, "attempts": 0_i64, "error_code": bson::Bson::Null, "retry_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64}})
                    .session(&mut *session).await?;
            } else {
                db.collection::<PlatformInstallation>(COLLECTION_NAME).insert_one(installation).session(&mut *session).await?;
                db.collection::<SetupStepJob>(STEPS).insert_one(SetupStepJob {id: request, setup_id: row.id.clone(), authority: a.clone(), phase: "queued".into(), generation: 1, claim_id: None, attempts: 0, error_code: None, retry_at: now, created_at: now}).session(&mut *session).await?;
            }
            Ok(approved)
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

/// Record the provider-observed child without granting connect authority. The
/// durable remote identity survives draft expiry or cancellation.
pub(crate) async fn observe_child(
    db: &Database,
    request: &TelegramBotRequest,
    child: i64,
    username: &str,
    update_id: i64,
) -> AppResult<()> {
    let db = db.clone();
    let request = request.clone();
    let username = username.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let row = fence_request(&db, &request, false, session).await?.ok_or_else(stale)?;
            db.collection::<Document>(crate::models::platform_settings::COLLECTION_NAME).update_one(
                doc! {"_id": crate::models::platform_settings::PLATFORM_SETTINGS_ID}, doc! {"$inc": {"channel_bot_registration_revision": 1_i64}})
                .upsert(true).session(&mut *session).await?;
            if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(
                doc! {"platform": {"$in": ["telegram", "telegram-new"]}, "platform_bot_id": child.to_string(), "is_active": true})
                .session(&mut *session).await?.is_some() {
                return Err(AppError::Conflict("This child is already connected; retire that connection and review a fresh creation".into()));
            }
            let a = request.setup_authority.as_ref().ok_or_else(stale)?;
            if a.step != SetupStep::BeginTelegram || request.start_update_id.is_none_or(|start| update_id <= start) { return Err(stale()); }
            let event = db.collection::<crate::models::telegram_bot_request::ManagedBotEvents>(crate::models::telegram_bot_request::MANAGED_BOTS)
                .find_one(doc! {"manager_bot_id": a.manager_bot_id, "telegram_bot_id": child, "telegram_user_id": a.telegram_sender_id,
                    "created_by": a.telegram_sender_id, "observation_id": &request.observation_id, "retired": {"$ne": true}, "revision": 1_i64})
                .session(&mut *session).await?.ok_or_else(stale)?;
            if event.bot_username.as_deref() != Some(username.as_str()) { return Err(stale()); }
            if db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME).update_one(
                doc! {"_id": &request.id, "revision": request.revision, "connection_generation": request.connection_generation, "status": "waiting_bot"},
                doc! {"$set": {"telegram_bot_id": child, "bot_username": &username, "manager_revision": event.revision, "status": "waiting_consent", "auto_connect": false},
                    "$inc": {"revision": 1_i64, "connection_generation": 1_i64}, "$unset": {"consent_hash": ""}}
            ).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
            db.collection::<PlatformInstallation>(COLLECTION_NAME).update_one(doc! {"setup_id": &row.id, "channel_bot_id": &request.id},
                doc! {"$set": {"telegram_bot_id": child.to_string(), "state": "awaiting_child_review", "updated_at": bson::DateTime::now(),
                    "native_creation": bson::to_bson(&NativeCreationProof {observation_id: request.observation_id.clone(), sender_id: a.telegram_sender_id,
                        start_update_id: request.start_update_id.ok_or_else(stale)?, child_revision: event.revision, child_username: username.clone(), request_created_at: request.created_at}).map_err(|_| stale())?}}).session(&mut *session).await?;
            db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).update_one(doc! {"_id": &row.id, "revision": row.revision},
                doc! {"$set": {"state": "awaiting_child_review", "resources.observed_telegram_bot_id": child.to_string(), "resources.observed_creation_revision": event.revision, "updated_at": bson::DateTime::now()}}).session(&mut *session).await?;
            super::platform_setup_notice_service::queue(&db, a, "awaiting_child_review", session).await?;
            Ok(())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn child_candidate(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    owner: &str,
    id: &str,
) -> AppResult<(PlatformSetup, TelegramBotRequest, String)> {
    let row = super::platform_setup_service::get_owned(db, owner, id).await?;
    if row.state != SetupState::AwaitingChildReview {
        return Err(stale());
    }
    let request = db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME)
        .find_one(doc! {"_id": row.resources.creation_request_id.as_deref().ok_or_else(stale)?, "owner_user_id": owner,
            "status": "waiting_consent", "active": true}).await?.ok_or_else(stale)?;
    admit_request(db, &request, false).await?;
    let child = request.telegram_bot_id.ok_or_else(stale)?;
    if row.resources.observed_telegram_bot_id.as_deref() != Some(child.to_string().as_str()) {
        return Err(stale());
    }
    let mut candidate = row;
    candidate.revision = candidate.revision.checked_add(1).ok_or_else(stale)?;
    candidate.authority_generation = candidate
        .authority_generation
        .checked_add(1)
        .ok_or_else(stale)?;
    candidate.resources.approved_telegram_bot_id = Some(child.to_string());
    candidate.resources.observed_creation_revision = request.manager_revision;
    candidate.state = SetupState::Provisioning;
    let facts = super::platform_setup_service::decrypt_facts(keys, &candidate).await?;
    let digest = super::platform_setup_service::approval_digest(&candidate, &facts)?;
    Ok((candidate, request, digest))
}

pub async fn approve_child_owned(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    owner: &str,
    id: &str,
    revision: i64,
    request_revision: i64,
    child: i64,
    manager_revision: i64,
    digest: &str,
) -> AppResult<PlatformSetup> {
    let (candidate, request, expected) = child_candidate(db, keys, owner, id).await?;
    if candidate.revision != revision.checked_add(1).ok_or_else(stale)?
        || request.revision != request_revision
        || request.telegram_bot_id != Some(child)
        || request.manager_revision != Some(manager_revision)
        || expected != digest
    {
        return Err(stale());
    }
    let authority = capture(db, &candidate, expected, SetupStep::ConnectTelegram).await?;
    let db = db.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_request(&db, &request, false, session).await?;
            if db.collection::<Document>(crate::models::telegram_bot_request::MANAGED_BOTS).update_one(
                doc! {"manager_bot_id": authority.manager_bot_id, "telegram_bot_id": child, "telegram_user_id": authority.telegram_sender_id,
                    "created_by": authority.telegram_sender_id, "observation_id": &request.observation_id, "revision": manager_revision, "retired": {"$ne": true}},
                doc! {"$inc": {"review_fence": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
            let approved = db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &candidate.id, "owner_user_id": &authority.owner_user_id, "revision": revision, "state": "awaiting_child_review"},
                doc! {"$set": {"revision": candidate.revision, "authority_generation": candidate.authority_generation, "resources": bson::to_bson(&candidate.resources).map_err(|_| stale())?,
                    "state": "provisioning", "approved_revision": candidate.revision, "approved_digest": &authority.digest, "approved_by": &authority.owner_user_id,
                    "approved_at": bson::DateTime::now(), "delivery_tested": false, "updated_at": bson::DateTime::now()}}
            ).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
            let next_generation = request.connection_generation.checked_add(1).ok_or_else(stale)?;
            if db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME).update_one(
                doc! {"_id": &request.id, "revision": request_revision, "connection_generation": request.connection_generation, "status": "waiting_consent"},
                doc! {"$set": {"setup_authority": bson::to_bson(&authority).map_err(|_| stale())?, "status": "ready", "auto_connect": false,
                    "connection_generation": next_generation}, "$inc": {"revision": 1_i64}, "$unset": {"consent_hash": ""}}
            ).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
            db.collection::<PlatformInstallation>(COLLECTION_NAME).update_one(doc! {"setup_id": &candidate.id, "channel_bot_id": &request.id, "telegram_bot_id": child.to_string()},
                doc! {"$set": {"authority": bson::to_bson(&authority).map_err(|_| stale())?, "state": "provisioning", "updated_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64}})
                .session(&mut *session).await?;
            fence_authority(&db, &authority, session).await?;
            db.collection::<SetupStepJob>(STEPS).update_one(doc! {"_id": &request.id, "setup_id": &candidate.id},
                doc! {"$set": {"authority": bson::to_bson(&authority).map_err(|_| stale())?, "phase": "queued", "claim_id": bson::Bson::Null, "attempts": 0_i64, "error_code": bson::Bson::Null, "retry_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64}})
                .session(&mut *session).await?;
            Ok(approved)
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn claim_step(db: &Database) -> AppResult<Option<SetupStepJob>> {
    let claim = uuid::Uuid::new_v4().to_string();
    Ok(db.collection::<SetupStepJob>(STEPS).find_one_and_update(
        doc! {"phase": {"$in": ["queued", "running"]}, "$expr": {"$lte": ["$retry_at", "$$NOW"]}},
        vec![doc! {"$set": {"phase": "running", "claim_id": &claim, "generation": {"$add": ["$generation", 1_i64]}, "attempts": {"$add": ["$attempts", 1_i64]},
            "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 120_i64}}}}]
    ).sort(doc! {"retry_at": 1}).return_document(ReturnDocument::After).await?)
}

pub(crate) async fn sweep(state: &crate::AppState) -> AppResult<()> {
    for _ in 0..8 {
        let Some(job) = claim_step(&state.db).await? else {
            break;
        };
        let claim = job.claim_id.as_deref().ok_or_else(stale)?;
        let outcome = if job.attempts > 5 {
            Err(AppError::Conflict(
                "Provisioning needs a fresh owner review".into(),
            ))
        } else {
            execute_step(state, &job).await
        };
        let code = outcome
            .as_ref()
            .err()
            .map(|error| error.error_code().to_string());
        let terminal = outcome.is_err() && job.attempts >= 5;
        let db = state.db.clone();
        let job = job.clone();
        let claim = claim.to_owned();
        let success = outcome.is_ok();
        let mut session = db.client().start_session().await?;
        session.start_transaction().and_run2(async move |session| {
            let result: AppResult<()> = async {
                let changed = db.collection::<SetupStepJob>(STEPS).update_one(
                    doc! {"_id": &job.id, "generation": job.generation, "claim_id": &claim, "phase": "running", "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
                    vec![doc! {"$set": {"phase": if success {"completed"} else if terminal {"failed"} else {"queued"}, "error_code": code.clone(),
                        "claim_id": bson::Bson::Null, "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": i64::from(job.attempts.min(5)) * 30_i64}}}}]
                ).session(&mut *session).await?;
                if terminal && changed.matched_count == 1 {
                    db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).update_one(
                        doc! {"_id": &job.setup_id, "revision": job.authority.setup_revision, "authority_generation": job.authority.authority_generation,
                            "state": {"$in": ["provisioning", "awaiting_child_review"]}},
                        doc! {"$set": {"state": "failed_retryable", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null,
                            "delivery_tested": false, "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}
                    ).session(&mut *session).await?;
                }
                Ok(())
            }.await;
            transaction_result(result)
        }).await.map_err(map_transaction_error)?;
    }
    Ok(())
}

pub(crate) async fn execute_step(state: &crate::AppState, job: &SetupStepJob) -> AppResult<()> {
    let service = super::telegram_new_service::TelegramNewService {
        db: &state.db,
        keys: &state.encryption_keys,
        config: &state.config,
        api: super::telegram_new_api::TelegramApi::new(&state.http_client),
    };
    match job.authority.step {
        SetupStep::BeginTelegram => {
            if let Some(request) = state
                .db
                .collection::<TelegramBotRequest>(
                    crate::models::telegram_bot_request::COLLECTION_NAME,
                )
                .find_one(doc! {"_id": &job.id})
                .await?
            {
                if request.setup_authority.as_ref() != Some(&job.authority) {
                    return Err(stale());
                }
                admit_request(&state.db, &request, false).await?;
                if request.telegram_bot_id.is_some() {
                    let db = state.db.clone();
                    let job = job.clone();
                    let mut session = db.client().start_session().await?;
                    session.start_transaction().and_run2(async move |session| {
                        let result: AppResult<()> = async {
                            fence_step(&db, &job, session).await?;
                            fence_request(&db, &request, false, session).await?;
                            db.collection::<Document>(crate::models::platform_setup::COLLECTION_NAME).update_one(doc! {"_id": &job.setup_id, "revision": job.authority.setup_revision}, doc! {"$set": {"state": "awaiting_child_review"}}).session(&mut *session).await?;
                            db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"setup_id": &job.setup_id}, doc! {"$set": {"state": "awaiting_child_review"}}).session(&mut *session).await?;
                            Ok(())
                        }.await; transaction_result(result)
                    }).await.map_err(map_transaction_error)?;
                }
            } else {
                let setup = super::platform_setup_service::get_owned(
                    &state.db,
                    &job.authority.owner_user_id,
                    &job.setup_id,
                )
                .await?;
                let facts =
                    super::platform_setup_service::decrypt_facts(&state.encryption_keys, &setup)
                        .await?;
                service.begin_approved(job, &facts.name).await?;
            }
        }
        SetupStep::ConnectTelegram => {
            let created = super::assistant_team_service::create_approved_specialist(&state.db, &state.encryption_keys, job,
                super::assistant_team_service::CreateRequest {
                    machines: None, logins: None, name: format!("public-{}", &job.setup_id[..8]),
                    description: "A client-owned public bot specialist. Public delivery uses only the reviewed setup publication and has no private connector or account access.".into(),
                    display_name: None, persona: None,
                    targets: super::assistant_team_service::GrantTargets {service_ids: vec![], platform_service_ids: vec![], slugs: vec![], ids_by_request: Default::default()},
                    account_read: false, specialty: None, created_by: "user",
                }).await?;
            if created.is_err() {
                return Err(AppError::Conflict(
                    "Specialist capacity or name must be resolved before retrying".into(),
                ));
            }
            let db = state.db.clone();
            let job = job.clone();
            let mut session = db.client().start_session().await?;
            session.start_transaction().and_run2(async move |session| {
                let result: AppResult<()> = async {
                    fence_authority(&db, &job.authority, session).await?;
                    if db.collection::<Document>(STEPS).update_one(doc! {"_id": &job.id, "generation": job.generation, "claim_id": &job.claim_id,
                        "phase": "running", "$expr": {"$gt": ["$retry_at", "$$NOW"]}}, doc! {"$inc": {"effect_revision": 1_i64}})
                        .session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
                    let request = db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME)
                        .find_one(doc! {"_id": &job.id}).session(&mut *session).await?.ok_or_else(stale)?;
                    let setup = fence_request(&db, &request, true, session).await?.ok_or_else(stale)?;
                    if request.status == crate::models::telegram_bot_request::TelegramRequestStatus::Connected {
                        if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(doc! {"_id": &job.id, "is_active": true, "status": "active", "webhook_registered": true}).session(&mut *session).await?.is_none() { return Err(stale()); }
                        if setup.state == SetupState::Provisioning {
                            if db.collection::<Document>(crate::models::platform_setup::COLLECTION_NAME).update_one(doc! {"_id": &job.setup_id, "revision": job.authority.setup_revision, "approved_digest": &job.authority.digest, "state": "provisioning"}, doc! {"$set": {"state": "ready_for_test", "updated_at": bson::DateTime::now()}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
                            if db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"setup_id": &job.setup_id, "authority": bson::to_bson(&job.authority).map_err(|_| stale())?, "state": "provisioning"}, doc! {"$set": {"state": "ready_for_test", "updated_at": bson::DateTime::now()}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
                        }
                        return Ok(());
                    }
                    if db.collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME).update_one(
                        doc! {"_id": &job.id, "connection_generation": request.connection_generation, "status": {"$in": ["ready", "provisioning"]}},
                        doc! {"$set": {"auto_connect": true}, "$unset": {"next_connection_attempt_at": "", "connection_error": ""}, "$inc": {"connection_generation": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
                    Ok(())
                }.await; transaction_result(result)
            }).await.map_err(map_transaction_error)?;
        }
    }
    Ok(())
}

async fn rebuild_request(
    db: &Database,
    installation: &PlatformInstallation,
    a: &SetupStepAuthority,
    session: &mut ClientSession,
) -> AppResult<TelegramBotRequest> {
    use crate::models::telegram_bot_request::TelegramRequestStatus as Status;
    let proof = installation.native_creation.as_ref().ok_or_else(stale)?;
    let profile = db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &a.platform_config_id, "generation": a.profile_generation})
        .session(&mut *session)
        .await?
        .ok_or_else(stale)?;
    if proof.sender_id != a.telegram_sender_id || proof.observation_id != profile.observation_id {
        return Err(stale());
    }
    let saved = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": &installation.channel_bot_id})
        .session(&mut *session)
        .await?;
    let status = if let Some(saved) = saved {
        if saved.get_str("platform_installation_id").ok() != Some(installation.id.as_str())
            || saved.get_bool("is_active").ok() != Some(true)
        {
            return Err(stale());
        }
        match saved.get_str("status").ok() {
            Some("active") => Status::Connected,
            Some("pending") => Status::Provisioning,
            _ => return Err(stale()),
        }
    } else if a.step == SetupStep::ConnectTelegram {
        Status::Ready
    } else {
        Status::WaitingConsent
    };
    let now = Utc::now();
    Ok(TelegramBotRequest {
        id: installation.channel_bot_id.clone(),
        setup_authority: Some(a.clone()),
        connection_generation: installation.generation.checked_add(1).ok_or_else(stale)?,
        actor_user_id: a.owner_user_id.clone(),
        owner_user_id: a.owner_user_id.clone(),
        manager_bot_id: a.manager_bot_id,
        observation_id: proof.observation_id.clone(),
        label: "Reviewed public bot".into(),
        destination: "Your reviewed public setup".into(),
        status,
        active: status != Status::Connected,
        revision: 1,
        challenge_hash: super::telegram_new_service::hash(&hex::encode(rand::random::<[u8; 24]>())),
        telegram_user_id: Some(a.telegram_sender_id),
        telegram_bot_id: installation
            .telegram_bot_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| stale())?,
        bot_username: Some(proof.child_username.clone()),
        consent_hash: None,
        manager_revision: Some(proof.child_revision),
        auto_connect: false,
        start_update_id: Some(proof.start_update_id),
        connection_attempts: 0,
        next_connection_attempt_at: None,
        connection_error: None,
        expires_at: now + chrono::Duration::minutes(15),
        created_at: proof.request_created_at,
        purge_after: None,
    })
}

pub(crate) async fn assess_owned_runtime(
    db: &Database,
    a: &SetupStepAuthority,
    session: &mut ClientSession,
) -> AppResult<()> {
    let profile = db.collection::<crate::models::platform_bot_config::PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
        .find_one(doc! {"_id": &a.platform_config_id, "generation": a.profile_generation, "enabled": true, "deleted_at": bson::Bson::Null}).session(&mut *session).await?.ok_or_else(stale)?;
    if a.step != SetupStep::ConnectTelegram
        || profile.manager_bot_id != a.manager_bot_id.to_string()
    {
        return Err(stale());
    }
    let mut binding = doc! {"_id": &a.binding_id, "user_id": &a.owner_user_id, "platform_config_id": &a.platform_config_id,
    "manager_bot_id": &profile.manager_bot_id, "telegram_sender_id": a.telegram_sender_id.to_string(), "telegram_chat_id": a.telegram_sender_id.to_string(), "is_active": true};
    if a.binding_revocation_generation == 0 {
        binding.insert(
            "$or",
            vec![
                doc! {"revocation_generation": 0_i64},
                doc! {"revocation_generation": {"$exists": false}},
            ],
        );
    } else {
        binding.insert("revocation_generation", a.binding_revocation_generation);
    }
    let setup = db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).find_one(doc! {"_id": &a.setup_id, "owner_user_id": &a.owner_user_id,
        "revision": a.setup_revision, "authority_generation": a.authority_generation, "approved_revision": a.setup_revision, "approved_digest": &a.digest, "state": "active"}).session(&mut *session).await?.ok_or_else(stale)?;
    if setup.template_version != 1 {
        return Err(stale());
    }
    let guide = profile.guide.as_ref().ok_or_else(stale)?;
    if setup.resources.runtime_model_service_id.as_deref() != Some(guide.model_service_id.as_str())
        || setup.resources.runtime_model_id.as_deref() != Some(guide.model_id.as_str())
    {
        return Err(stale());
    }
    let installation = db
        .collection::<PlatformInstallation>(COLLECTION_NAME)
        .find_one(
            doc! {"setup_id": &a.setup_id, "owner_user_id": &a.owner_user_id,
            "state": "active", "authority": bson::to_bson(a).map_err(|_| stale())?},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(stale)?;
    // Registration consumes the creation identity with retired=true. Ongoing
    // authority remains the exact manager observation/revision and live child.
    for (collection, filter) in [
        (
            crate::models::user::COLLECTION_NAME,
            doc! {"_id": &a.owner_user_id, "is_active": true, "user_type": "person"},
        ),
        (crate::models::platform_binding::COLLECTION_NAME, binding),
        (
            crate::models::platform_credential::COLLECTION_NAME,
            doc! {"provider": "telegram-new", "fields.manager_bot_id": &profile.manager_bot_id, "fields.observation_id": &profile.observation_id, "fields.webhook_ready": "true"},
        ),
        (
            crate::models::telegram_bot_request::MANAGED_BOTS,
            doc! {"manager_bot_id": a.manager_bot_id, "telegram_bot_id": a.approved_child_bot_id, "telegram_user_id": a.telegram_sender_id, "revision": a.approved_child_revision, "observation_id": &profile.observation_id, "retired": true},
        ),
        (
            crate::models::channel_bot::COLLECTION_NAME,
            doc! {"_id": &installation.channel_bot_id, "user_id": &a.owner_user_id, "platform_installation_id": &installation.id, "platform_bot_id": a.approved_child_bot_id.map(|id| id.to_string()), "is_active": true, "status": "active", "webhook_registered": true},
        ),
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            doc! {"_id": &installation.agent_id, "user_id": &a.owner_user_id, "platform_setup_id": &a.setup_id, "destroyed_at": bson::Bson::Null},
        ),
        (
            crate::models::downstream_service::COLLECTION_NAME,
            doc! {"_id": &guide.model_service_id, "is_active": true, "service_type": "http", "inference.model_list": true},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .session(&mut *session)
            .await?
            .is_none()
        {
            return Err(stale());
        }
    }
    Ok(())
}

/// Activated ownership is durable; the bounded onboarding binding is no longer
/// an execution lease. Explicit unlink/replacement still revokes its consent epoch.
pub(crate) async fn fence_owned_runtime(
    db: &Database,
    a: &SetupStepAuthority,
    session: &mut ClientSession,
) -> AppResult<PlatformSetup> {
    super::platform_setup_service::fence_owner(db, &a.owner_user_id, session).await?;
    let profile = super::platform_binding_service::fence_profile(
        db,
        &a.platform_config_id,
        a.profile_generation,
        session,
    )
    .await?;
    let setup = db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).find_one_and_update(doc! {"_id": &a.setup_id, "owner_user_id": &a.owner_user_id,
        "revision": a.setup_revision, "authority_generation": a.authority_generation, "approved_revision": a.setup_revision, "approved_digest": &a.digest, "state": "active"},
        doc! {"$inc": {"execution_fence": 1_i64}}).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
    if a.step != SetupStep::ConnectTelegram
        || setup.template_version != 1
        || profile.manager_bot_id != a.manager_bot_id.to_string()
    {
        return Err(stale());
    }
    let epoch = if a.binding_revocation_generation == 0 {
        doc! {"$or": [{"revocation_generation": 0_i64}, {"revocation_generation": {"$exists": false}}]}
    } else {
        doc! {"revocation_generation": a.binding_revocation_generation}
    };
    let mut binding = doc! {"_id": &a.binding_id, "user_id": &a.owner_user_id, "platform_config_id": &a.platform_config_id,
    "manager_bot_id": a.manager_bot_id.to_string(), "telegram_sender_id": a.telegram_sender_id.to_string(), "telegram_chat_id": a.telegram_sender_id.to_string(), "is_active": true};
    binding.extend(epoch);
    if db
        .collection::<Document>(crate::models::platform_binding::COLLECTION_NAME)
        .update_one(binding, doc! {"$inc": {"public_effect_revision": 1_i64}})
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(stale());
    }
    if db
        .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
        .find_one(
            doc! {"provider": "telegram-new", "fields.manager_bot_id": &profile.manager_bot_id,
            "fields.observation_id": &profile.observation_id, "fields.webhook_ready": "true"},
        )
        .session(&mut *session)
        .await?
        .is_none()
    {
        return Err(stale());
    }
    if db.collection::<Document>(crate::models::telegram_bot_request::MANAGED_BOTS).find_one_and_update(doc! {"manager_bot_id": a.manager_bot_id,
        "telegram_bot_id": a.approved_child_bot_id, "telegram_user_id": a.telegram_sender_id, "revision": a.approved_child_revision, "observation_id": &profile.observation_id},
        doc! {"$inc": {"public_effect_revision": 1_i64}}).session(&mut *session).await?.is_none() { return Err(stale()); }
    Ok(setup)
}

#[cfg(test)]
mod tests {
    use super::super::{
        platform_onboarding_test_support as support, platform_setup_service as setups,
        telegram_new_tests as telegram,
    };
    use super::*;

    async fn renewal_then_revocation(replace: bool) {
        let f = support::distinct_fixture().await;
        f.observe().await;
        let row = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap();
        let (_, native, digest) =
            child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &row.id)
                .await
                .unwrap();
        let approved = approve_child_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &row.id,
            row.revision,
            native.revision,
            900,
            native.manager_revision.unwrap(),
            &digest,
        )
        .await
        .unwrap();
        let installed = f
            .state
            .db
            .collection::<PlatformInstallation>(COLLECTION_NAME)
            .find_one(doc! {"setup_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::platform_setup::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"state": "active", "expires_at": bson::Bson::Null}},
            )
            .await
            .unwrap();
        let key = f.state.auth_device_hmac_key.as_slice();
        tokio::time::sleep(std::time::Duration::from_millis(760)).await;
        let (_, code) = super::super::platform_binding_service::begin(
            &f.state.db,
            key,
            &f.profile,
            "700",
            "700",
            200,
        )
        .await
        .unwrap();
        super::super::platform_binding_service::decide(
            &f.state.db,
            key,
            &f.owner,
            &code,
            &code,
            true,
            false,
        )
        .await
        .unwrap();
        let renewed = super::super::platform_binding_service::confirm_from_chat(
            &f.state.db,
            key,
            &f.profile,
            "700",
            "700",
            &code,
        )
        .await
        .unwrap();
        assert!(renewed.generation > f.binding.generation);
        assert_eq!(
            renewed.revocation_generation,
            f.binding.revocation_generation
        );
        let active = setups::get_owned(&f.state.db, &f.owner, &row.id)
            .await
            .unwrap();
        assert_eq!(active.state, SetupState::Active);
        assert_eq!(active.binding_generation, Some(f.binding.generation));
        let mut session = f.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        fence_owned_runtime(&f.state.db, &installed.authority, &mut session)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        if replace {
            let other = uuid::Uuid::new_v4().to_string();
            f.state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .insert_one(crate::test_utils::test_user(
                    &other,
                    crate::models::user::UserType::Person,
                ))
                .await
                .unwrap();
            let (_, code) = super::super::platform_binding_service::begin(
                &f.state.db,
                key,
                &f.profile,
                "700",
                "700",
                300,
            )
            .await
            .unwrap();
            super::super::platform_binding_service::decide(
                &f.state.db,
                key,
                &other,
                &code,
                &code,
                true,
                true,
            )
            .await
            .unwrap();
            super::super::platform_binding_service::confirm_from_chat(
                &f.state.db,
                key,
                &f.profile,
                "700",
                "700",
                &code,
            )
            .await
            .unwrap();
        } else {
            super::super::platform_binding_service::revoke_owned(
                &f.state.db,
                &f.owner,
                &renewed.id,
                renewed.generation,
            )
            .await
            .unwrap();
        }
        let paused = setups::get_owned(&f.state.db, &f.owner, &row.id)
            .await
            .unwrap();
        assert_eq!(paused.state, SetupState::Paused);
        assert!(paused.approved_digest.is_none());
        assert!(paused.revision > approved.revision);
        let mut session = f.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            fence_owned_runtime(&f.state.db, &installed.authority, &mut session)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_active_renewal_then_unlink_pauses_prior_generation() {
        renewal_then_revocation(false).await;
    }
    #[tokio::test]
    async fn platform_onboarding_active_renewal_then_replacement_pauses_prior_owner() {
        renewal_then_revocation(true).await;
    }

    #[tokio::test]
    async fn platform_onboarding_distinct_client_authority_and_reclaimed_specialist_claim() {
        let f = support::distinct_fixture().await;
        assert_ne!(f.operator, f.owner);
        assert_eq!(f.profile.configured_by, f.operator);
        assert_eq!(f.binding.user_id, f.owner);
        assert!(
            setups::get_owned(&f.state.db, &f.operator, &f.setup.id)
                .await
                .is_err()
        );
        let other = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &other,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        assert!(
            setups::get_owned(&f.state.db, &other, &f.setup.id)
                .await
                .is_err()
        );
        f.observe().await;
        let row = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap();
        let (_, native, digest) =
            child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &row.id)
                .await
                .unwrap();
        approve_child_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &row.id,
            row.revision,
            native.revision,
            900,
            native.manager_revision.unwrap(),
            &digest,
        )
        .await
        .unwrap();
        let old = f.claim().await;
        f.state
            .db
            .collection::<Document>(STEPS)
            .update_one(
                doc! {"_id": &old.id},
                vec![doc! {"$set": {"retry_at": {"$subtract": ["$$NOW", 1_i64]}}}],
            )
            .await
            .unwrap();
        let current = claim_step(&f.state.db).await.unwrap().unwrap();
        assert!(current.generation > old.generation);
        let request = super::super::assistant_team_service::CreateRequest {
            machines: None,
            logins: None,
            name: "client-cafe".into(),
            description: "Reviewed public cafe".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        };
        assert!(
            super::super::assistant_team_service::create_approved_specialist(
                &f.state.db,
                &f.state.encryption_keys,
                &old,
                request
            )
            .await
            .is_err()
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
                .count_documents(doc! {"platform_setup_id": &f.setup.id})
                .await
                .unwrap(),
            0
        );
        // This checks cross-client ownership and claim fencing; no billing proof is claimed.
    }

    #[tokio::test]
    async fn platform_onboarding_claimed_creation_rejects_reclaimed_worker_and_forged_first_sender()
    {
        let f = support::fixture().await;
        let old = f.claim().await;
        f.state
            .db
            .collection::<Document>(STEPS)
            .update_one(
                doc! {"_id": &old.id},
                vec![doc! {"$set": {"retry_at": {"$subtract": ["$$NOW", 1_i64]}}}],
            )
            .await
            .unwrap();
        let current = claim_step(&f.state.db).await.unwrap().unwrap();
        assert!(current.generation > old.generation);
        let base = f.server.uri();
        let service = telegram::service(&f.state, &base);
        assert!(service.begin_approved(&old, "Cafe").await.is_err());
        assert_eq!(
            f.state
                .db
                .collection::<TelegramBotRequest>(
                    crate::models::telegram_bot_request::COLLECTION_NAME
                )
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let (request, url) = service.begin_approved(&current, "Cafe").await.unwrap();
        let challenge = reqwest::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "start")
            .unwrap()
            .1
            .to_string();
        let before = request.revision;
        let mut update = telegram::message(
            serde_json::json!({"text": format!("/start {challenge}"), "from": {"id": 701, "is_bot": false}, "chat": {"id": 701, "type": "private"}}),
        );
        service
            .webhook(
                &telegram::headers(&f.state).await,
                &serde_json::to_vec(&update).unwrap(),
            )
            .await
            .unwrap();
        let unchanged = f
            .state
            .db
            .collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME)
            .find_one(doc! {"_id": &request.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.revision, before);
        assert!(unchanged.telegram_user_id.is_none());
        update["message"]["from"]["id"] = serde_json::json!(700);
        update["message"]["chat"]["id"] = serde_json::json!(700);
        service
            .webhook(
                &telegram::headers(&f.state).await,
                &serde_json::to_vec(&update).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .get(&f.owner, &request.id)
                .await
                .unwrap()
                .telegram_user_id,
            Some(700)
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_child_review_denies_unswept_expiry_and_changed_live_request_fields()
     {
        let f = support::fixture().await;
        let request = f.observe().await;
        let before = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap();
        assert_eq!(before.state, SetupState::AwaitingChildReview);
        child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &f.setup.id)
            .await
            .unwrap();
        f.state.db.collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME).update_one(doc! {"_id": &request.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}}).await.unwrap();
        assert!(
            child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &f.setup.id)
                .await
                .is_err()
        );
        assert!(admit_request(&f.state.db, &request, false).await.is_err());
        let row = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap();
        assert_eq!(row.revision, before.revision);
        f.state.db.collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME).update_one(doc! {"_id": &request.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::minutes(10)), "telegram_user_id": 701_i64}}).await.unwrap();
        assert!(admit_request(&f.state.db, &request, false).await.is_err());
        f.state
            .db
            .collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &request.id},
                doc! {"$set": {"telegram_user_id": 700_i64, "telegram_bot_id": 901_i64}},
            )
            .await
            .unwrap();
        assert!(admit_request(&f.state.db, &request, false).await.is_err());
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_reapproval_reuses_installation_before_native_request_and_after_child_observation()
     {
        let f = support::fixture().await;
        let original = f
            .state
            .db
            .collection::<PlatformInstallation>(COLLECTION_NAME)
            .find_one(doc! {"setup_id": &f.setup.id})
            .await
            .unwrap()
            .unwrap();
        let paused =
            setups::pause_or_cancel(&f.state.db, &f.owner, &f.setup.id, f.setup.revision, false)
                .await
                .unwrap();
        let reviewed = setups::request_review(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &paused.id,
            paused.revision,
        )
        .await
        .unwrap();
        let facts = setups::decrypt_facts(&f.state.encryption_keys, &reviewed)
            .await
            .unwrap();
        let digest = setups::approval_digest(&reviewed, &facts).unwrap();
        let approved = approve_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &reviewed.id,
            reviewed.revision,
            &digest,
        )
        .await
        .unwrap();
        assert_eq!(
            approved.resources.creation_request_id,
            f.setup.resources.creation_request_id
        );
        assert_eq!(
            f.state
                .db
                .collection::<PlatformInstallation>(COLLECTION_NAME)
                .find_one(doc! {"setup_id": &f.setup.id})
                .await
                .unwrap()
                .unwrap()
                .id,
            original.id
        );
        assert_eq!(
            f.state
                .db
                .collection::<PlatformInstallation>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        f.observe().await;
        let observed = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap();
        let paused = setups::pause_or_cancel(
            &f.state.db,
            &f.owner,
            &observed.id,
            observed.revision,
            false,
        )
        .await
        .unwrap();
        let reviewed = setups::request_review(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &paused.id,
            paused.revision,
        )
        .await
        .unwrap();
        let facts = setups::decrypt_facts(&f.state.encryption_keys, &reviewed)
            .await
            .unwrap();
        let digest = setups::approval_digest(&reviewed, &facts).unwrap();
        approve_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &reviewed.id,
            reviewed.revision,
            &digest,
        )
        .await
        .unwrap();
        let job = claim_step(&f.state.db).await.unwrap().unwrap();
        execute_step(&f.state, &job).await.unwrap();
        let candidate =
            child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &f.setup.id)
                .await
                .unwrap();
        let approved = approve_child_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &f.setup.id,
            candidate.0.revision - 1,
            candidate.1.revision,
            900,
            1,
            &candidate.2,
        )
        .await
        .unwrap();
        assert_eq!(
            approved.resources.creation_request_id,
            f.setup.resources.creation_request_id
        );
        assert_eq!(
            approved.resources.approved_telegram_bot_id.as_deref(),
            Some("900")
        );
        let expired = f
            .state
            .db
            .collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME);
        expired
            .delete_one(doc! {"_id": approved.resources.creation_request_id.as_ref().unwrap()})
            .await
            .unwrap();
        let paused = setups::pause_or_cancel(
            &f.state.db,
            &f.owner,
            &approved.id,
            approved.revision,
            false,
        )
        .await
        .unwrap();
        let reviewed = setups::request_review(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &paused.id,
            paused.revision,
        )
        .await
        .unwrap();
        let facts = setups::decrypt_facts(&f.state.encryption_keys, &reviewed)
            .await
            .unwrap();
        let digest = setups::approval_digest(&reviewed, &facts).unwrap();
        let resumed = approve_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &reviewed.id,
            reviewed.revision,
            &digest,
        )
        .await
        .unwrap();
        let request = f
            .state
            .db
            .collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME)
            .find_one(doc! {"_id": resumed.resources.creation_request_id.as_ref().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.telegram_bot_id, Some(900));
        assert_eq!(
            request.status,
            crate::models::telegram_bot_request::TelegramRequestStatus::Ready
        );
        assert_eq!(
            request.setup_authority.as_ref().unwrap().step,
            SetupStep::ConnectTelegram
        );
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn platform_onboarding_manual_registration_and_observation_serialize_remote_identity() {
        let f = support::distinct_fixture().await;
        let request = Box::pin(f.native_start()).await;
        f.state
            .db
            .collection::<crate::models::telegram_bot_request::ManagedBotEvents>(
                crate::models::telegram_bot_request::MANAGED_BOTS,
            )
            .insert_one(crate::models::telegram_bot_request::ManagedBotEvents {
                id: uuid::Uuid::new_v4().to_string(),
                manager_bot_id: request.manager_bot_id,
                telegram_bot_id: 900,
                telegram_user_id: 700,
                revision: 1,
                update_ids: vec![2],
                created_by: Some(700),
                bot_username: Some("CustomerBot".into()),
                created_at: Some(chrono::Utc::now()),
                observation_id: Some(request.observation_id.clone()),
                retired: false,
            })
            .await
            .unwrap();
        let live = f
            .state
            .db
            .collection::<TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME)
            .find_one(doc! {"_id": &request.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            live.status,
            crate::models::telegram_bot_request::TelegramRequestStatus::WaitingBot
        );
        assert_eq!(live.connection_generation, request.connection_generation);
        assert!(live.telegram_bot_id.is_none());
        let initial = f
            .state
            .db
            .collection::<PlatformInstallation>(COLLECTION_NAME)
            .find_one(doc! {"setup_id": &f.setup.id})
            .await
            .unwrap()
            .unwrap();
        assert!(initial.telegram_bot_id.is_none());
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .count_documents(doc! {"platform_bot_id": "900", "is_active": true})
                .await
                .unwrap(),
            0
        );
        let mut manual =
            super::super::channel_bot_service::get_bot(&f.state.db, &f.profile.channel_bot_id)
                .await
                .unwrap();
        manual.id = uuid::Uuid::new_v4().to_string();
        manual.user_id = f.owner.clone();
        manual.platform = "telegram".into();
        manual.platform_bot_id = "900".into();
        manual.credential_source = "user".into();
        manual.bot_token_encrypted = f
            .state
            .encryption_keys
            .encrypt(b"900:manual-test-token")
            .await
            .unwrap();
        let mut held = f.state.db.client().start_session().await.unwrap();
        held.start_transaction().await.unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::platform_settings::COLLECTION_NAME)
            .update_one(
                doc! {"_id": crate::models::platform_settings::PLATFORM_SETTINGS_ID},
                doc! {"$inc": {"channel_bot_registration_revision": 1_i64}},
            )
            .session(&mut held)
            .await
            .unwrap();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
        let registration_db = f.state.db.clone();
        let registration_barrier = barrier.clone();
        let manual_id = manual.id.clone();
        let registration = tokio::spawn(async move {
            registration_barrier.wait().await;
            Box::pin(super::super::channel_bot_service::insert_registered_bot(
                &registration_db,
                &manual,
                10,
                None,
            ))
            .await
        });
        let observation_db = f.state.db.clone();
        let observation_barrier = barrier.clone();
        let observation = tokio::spawn(async move {
            observation_barrier.wait().await;
            Box::pin(observe_child(
                &observation_db,
                &request,
                900,
                "CustomerBot",
                3,
            ))
            .await
        });
        barrier.wait().await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!registration.is_finished());
        assert!(!observation.is_finished());
        held.commit_transaction().await.unwrap();
        drop(held);
        let registration = registration.await.unwrap();
        let observation = observation.await.unwrap();
        assert_ne!(registration.is_ok(), observation.is_ok());
        let installation = f
            .state
            .db
            .collection::<PlatformInstallation>(COLLECTION_NAME)
            .find_one(doc! {"setup_id": &f.setup.id})
            .await
            .unwrap()
            .unwrap();
        let ordinary = f
            .state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id": manual_id})
            .await
            .unwrap();
        if observation.is_ok() {
            assert!(ordinary.is_none());
            assert_eq!(installation.telegram_bot_id.as_deref(), Some("900"));
        } else {
            assert!(ordinary.is_some());
            assert!(installation.telegram_bot_id.is_none());
            assert!(matches!(observation, Err(AppError::Conflict(_))));
        }
    }
}
