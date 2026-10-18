#![allow(
    clippy::collapsible_if,
    clippy::too_many_arguments,
    clippy::needless_borrow,
    clippy::items_after_test_module
)]

use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::{
    errors::{AppError, AppResult},
    models::{platform_binding::*, platform_bot_config::PlatformBotConfig},
};
use chrono::{Duration, Utc};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, doc},
    options::{IndexOptions, ReturnDocument},
};
use zeroize::Zeroizing;

fn unavailable() -> AppError {
    AppError::NotFound("Onboarding link is unavailable or expired".into())
}

/// Committed separately from a grant decision, including failed guesses. This
/// sliding timestamp enforces spacing across fixed-window boundaries.
async fn admit_decision(db: &Database, actor: &str) -> AppResult<()> {
    let spacing = db.collection::<bson::Document>("platform_onboarding_decision_spacing");
    // Seed independently: an upsert cannot use the $expr admission predicate.
    // Concurrent seeds may lose the unique _id race, which is harmless.
    match spacing
        .update_one(
            doc! {"_id": actor},
            doc! {"$setOnInsert": {"last_decision_at": bson::DateTime::from_millis(0)}},
        )
        .upsert(true)
        .await
    {
        Ok(_) => {}
        Err(error) if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000) =>
            {}
        Err(error) => return Err(error.into()),
    }
    let admitted = spacing.update_one(
        doc! {"_id": actor, "$expr": {"$lte": ["$last_decision_at", {"$subtract": ["$$NOW", 750_i64]}]}},
        vec![doc! {"$set": {"last_decision_at": "$$NOW"}}],
    ).await?;
    if admitted.matched_count == 1 {
        Ok(())
    } else {
        Err(AppError::RateLimited)
    }
}

/// Legacy display formatting is part of the issued proof contract.
/// This only normalizes syntax; it does not authenticate or consume a proof.
pub(crate) fn normalize_code(code: &str) -> Option<String> {
    if code.len() > 32 {
        return None;
    }
    let normalized = code
        .chars()
        .filter(|c| *c != '-' && !c.is_ascii_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect::<String>();
    (normalized.len() == 8 && normalized.bytes().all(|c| c.is_ascii_alphanumeric()))
        .then_some(normalized)
}

pub fn code_hash(key: &[u8], code: &str) -> AppResult<String> {
    let normalized = normalize_code(code).ok_or_else(unavailable)?;
    Ok(super::auth_device_service::hmac_hex(
        key,
        format!("platform-onboarding-binding:v1:{normalized}").as_bytes(),
    ))
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<PlatformBindingRequest>(REQUESTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"approved_user_id": 1_i64, "state": 1_i64})
                .build(),
        )
        .await?;
    db.collection::<PlatformBinding>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform_config_id": 1, "manager_bot_id": 1, "telegram_sender_id": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<PlatformBinding>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "is_active": 1})
                .build(),
        )
        .await?;
    db.collection::<PlatformBindingRequest>(REQUESTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"code_hmac": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<PlatformBindingRequest>(REQUESTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"purge_after": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("platform_native_replies")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"source_id": 1, "sender": 1, "update_id": 1, "kind": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("platform_native_replies")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"purge_after": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("platform_guide_selections")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"binding_id": 1, "generation": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("platform_guide_selections")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"purge_after": 1})
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

pub(crate) async fn fence_profile(
    db: &Database,
    id: &str,
    generation: i64,
    session: &mut ClientSession,
) -> AppResult<PlatformBotConfig> {
    db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
        .find_one_and_update(doc! {"_id": id, "generation": generation, "enabled": true, "deleted_at": bson::Bson::Null},
            doc! {"$inc": {"binding_revision": 1_i64}}).return_document(ReturnDocument::After).session(session).await?
        .ok_or_else(unavailable)
}

pub(crate) async fn begin(
    db: &Database,
    key: &[u8],
    profile: &PlatformBotConfig,
    sender: &str,
    chat: &str,
    initiation_ref: i64,
) -> AppResult<(PlatformBindingRequest, Zeroizing<String>)> {
    if sender != chat || sender.parse::<u64>().ok().is_none_or(|id| id == 0) {
        return Err(AppError::Forbidden(
            "Linking requires the initiating private Telegram chat".into(),
        ));
    }
    if initiation_ref < 0 {
        return Err(unavailable());
    }
    // Stable metadata-only idempotency; recover the same public code on a
    // redelivered verified update without storing its plaintext.
    let material = Zeroizing::new(super::auth_device_service::hmac_hex(
        key,
        format!(
            "platform-binding-init:v1:{}:{}:{}:{sender}:{chat}:{initiation_ref}",
            profile.id, profile.manager_bot_id, profile.generation
        )
        .as_bytes(),
    ));
    let bytes = Zeroizing::new(hex::decode(material.as_str()).map_err(|_| unavailable())?);
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let code = Zeroizing::new(
        bytes
            .iter()
            .take(8)
            .map(|b| ALPHABET[usize::from(b & 31)] as char)
            .collect::<String>(),
    );
    let hash = code_hash(key, &code)?;
    let now = Utc::now();
    let row = PlatformBindingRequest {
        id: uuid::Uuid::new_v4().to_string(),
        platform_config_id: profile.id.clone(),
        manager_bot_id: profile.manager_bot_id.clone(),
        profile_generation: profile.generation,
        telegram_sender_id: sender.into(),
        telegram_chat_id: chat.into(),
        code_hmac: hash,
        initiation_ref,
        state: BindingRequestState::Pending,
        approved_user_id: None,
        replace_existing: false,
        binding_id: None,
        binding_generation_at_review: None,
        created_at: now,
        expires_at: now + Duration::minutes(10),
        purge_after: now + Duration::days(1),
    };
    let db = db.clone();
    let mut session = db.client().start_session().await?;
    let saved = session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_profile(&db, &row.platform_config_id, row.profile_generation, session).await?;
            if let Some(existing) = db.collection::<PlatformBindingRequest>(REQUESTS).find_one(doc! {"code_hmac": &row.code_hmac}).session(&mut *session).await? {
                if existing.platform_config_id == row.platform_config_id && existing.profile_generation == row.profile_generation
                    && existing.telegram_sender_id == row.telegram_sender_id && existing.initiation_ref == row.initiation_ref
                    && existing.expires_at > Utc::now() && matches!(existing.state, BindingRequestState::Pending | BindingRequestState::AwaitingChatConfirmation)
                { return Ok(existing); }
                return Err(unavailable());
            }
            for (bucket, identity, cap) in [("platform_binding_sender", format!("{}:{}", row.platform_config_id, row.telegram_sender_id), 3),
                ("platform_binding_platform", row.platform_config_id.clone(), 100)] {
                if !super::coordination_service::RateWindowStore::admit_in_session(&db, bucket, &identity, cap, std::time::Duration::from_secs(3600), session).await? { return Err(AppError::RateLimited); }
            }
            db.collection::<PlatformBindingRequest>(REQUESTS).update_many(doc! {"platform_config_id": &row.platform_config_id,
                "telegram_sender_id": &row.telegram_sender_id, "state": {"$in": ["pending", "awaiting_chat_confirmation"]}},
                doc! {"$set": {"state": "cancelled"}}).session(&mut *session).await?;
            db.collection::<PlatformBindingRequest>(REQUESTS).insert_one(&row).session(&mut *session).await?;
            Ok(row.clone())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)?;
    Ok((saved, code))
}

/// Non-mutating. No account identifier, credential, private poll secret or draft
/// content is projected to a public code holder.
pub async fn preview(db: &Database, key: &[u8], code: &str) -> AppResult<PlatformBindingRequest> {
    let row = db
        .collection::<PlatformBindingRequest>(REQUESTS)
        .find_one(
            doc! {"code_hmac": code_hash(key, code)?, "expires_at": {"$gt": bson::DateTime::now()},
            "state": {"$in": ["pending", "awaiting_chat_confirmation", "bound"]}},
        )
        .await?
        .ok_or_else(unavailable)?;
    let profile = super::platform_bot_config_service::get(db, &row.platform_config_id).await?;
    if !profile.enabled
        || profile.deleted_at.is_some()
        || profile.generation != row.profile_generation
    {
        return Err(unavailable());
    }
    Ok(row)
}

pub async fn decide(
    db: &Database,
    key: &[u8],
    actor: &str,
    code: &str,
    matched_code: &str,
    approve: bool,
    replace: bool,
) -> AppResult<PlatformBindingRequest> {
    admit_decision(db, actor).await?;
    let hash = code_hash(key, code)?;
    if hash != code_hash(key, matched_code)? {
        return Err(AppError::ValidationError(
            "Match the code shown in the initiating chat".into(),
        ));
    }
    let db = db.clone();
    let actor = actor.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            super::platform_setup_service::fence_owner(&db, &actor, session).await?;
            let request = db.collection::<PlatformBindingRequest>(REQUESTS).find_one(doc! {"code_hmac": &hash, "state": "pending", "expires_at": {"$gt": bson::DateTime::now()}})
                .session(&mut *session).await?.ok_or_else(unavailable)?;
            fence_profile(&db, &request.platform_config_id, request.profile_generation, session).await?;
            let previous = db.collection::<PlatformBinding>(COLLECTION_NAME).find_one(doc! {"platform_config_id": &request.platform_config_id,
                "manager_bot_id": &request.manager_bot_id, "telegram_sender_id": &request.telegram_sender_id}).session(&mut *session).await?;
            if approve && !replace && previous.as_ref().is_some_and(|b| b.is_active && b.user_id != actor) {
                return Err(AppError::Conflict("This chat has another account binding; explicitly review account replacement".into()));
            }
            db.collection::<PlatformBindingRequest>(REQUESTS).find_one_and_update(doc! {"_id": &request.id, "state": "pending"},
                doc! {"$set": {"state": if approve {"awaiting_chat_confirmation"} else {"denied"}, "approved_user_id": if approve {bson::Bson::String(actor.clone())} else {bson::Bson::Null}, "replace_existing": replace,
                    "binding_generation_at_review": previous.map(|b| bson::Bson::Int64(b.generation)).unwrap_or(bson::Bson::Null)}})
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(unavailable)
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn confirm_from_chat(
    db: &Database,
    key: &[u8],
    profile: &PlatformBotConfig,
    sender: &str,
    chat: &str,
    code: &str,
) -> AppResult<PlatformBinding> {
    if sender != chat {
        return Err(unavailable());
    }
    let hash = code_hash(key, code)?;
    let db = db.clone();
    let profile = profile.clone();
    let sender = sender.to_owned();
    let chat = chat.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_profile(&db, &profile.id, profile.generation, session).await?;
            let request = db.collection::<PlatformBindingRequest>(REQUESTS).find_one(doc! {"code_hmac": &hash, "platform_config_id": &profile.id,
                "manager_bot_id": &profile.manager_bot_id, "profile_generation": profile.generation, "telegram_sender_id": &sender, "telegram_chat_id": &chat,
                "state": "awaiting_chat_confirmation", "expires_at": {"$gt": bson::DateTime::now()}}).session(&mut *session).await?.ok_or_else(unavailable)?;
            let actor = request.approved_user_id.as_deref().ok_or_else(unavailable)?;
            super::platform_setup_service::fence_owner(&db, actor, session).await?;
            let filter = doc! {"platform_config_id": &profile.id, "manager_bot_id": &profile.manager_bot_id, "telegram_sender_id": &sender};
            let previous = db.collection::<PlatformBinding>(COLLECTION_NAME).find_one(filter.clone()).session(&mut *session).await?;
            if previous.as_ref().map(|b| b.generation) != request.binding_generation_at_review { return Err(unavailable()); }
            if previous.as_ref().is_some_and(|b| b.is_active && b.user_id != actor) && !request.replace_existing { return Err(unavailable()); }
            let id = previous.as_ref().map(|b| b.id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            if let Some(previous) = &previous { invalidate_setups(&db, &previous.id, &previous.user_id, previous.generation, !(previous.is_active && previous.user_id == actor), session).await?; }
            cancel_pending_requests(&db, &profile.id, &sender, Some(&request.id), session).await?;
            let now = Utc::now();
            let row = PlatformBinding { id: id.clone(), platform_config_id: profile.id.clone(), manager_bot_id: profile.manager_bot_id.clone(),
                profile_generation: profile.generation, telegram_sender_id: sender.clone(), telegram_chat_id: chat.clone(), user_id: actor.into(),
                generation: previous.as_ref().map_or(1, |b| b.generation + 1), revocation_generation: previous.as_ref().map_or(0, |b| b.revocation_generation + i64::from(!b.is_active || b.user_id != actor)), is_active: true, created_at: previous.as_ref().map_or(now, |b| b.created_at), updated_at: now, expires_at: now + Duration::days(30) };
            db.collection::<PlatformBinding>(COLLECTION_NAME).replace_one(filter, &row).upsert(true).session(&mut *session).await?;
            db.collection::<PlatformBindingRequest>(REQUESTS).update_one(doc! {"_id": &request.id, "state": "awaiting_chat_confirmation"}, doc! {"$set": {"state": "bound", "binding_id": &id}}).session(&mut *session).await?;
            Ok(row)
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn live_for_sender(
    db: &Database,
    profile: &PlatformBotConfig,
    sender: &str,
    chat: &str,
) -> AppResult<Option<PlatformBinding>> {
    let row = db.collection::<PlatformBinding>(COLLECTION_NAME).find_one(doc! {"platform_config_id": &profile.id, "manager_bot_id": &profile.manager_bot_id,
        "profile_generation": profile.generation, "telegram_sender_id": sender, "telegram_chat_id": chat, "is_active": true, "expires_at": {"$gt": bson::DateTime::now()}}).await?;
    if let Some(row) = row {
        if db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id": &row.user_id, "is_active": true})
            .await?
            .is_some()
        {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

pub(crate) async fn fence_reply(
    db: &Database,
    profile: &PlatformBotConfig,
    binding: Option<&PlatformBinding>,
    session: &mut ClientSession,
) -> AppResult<()> {
    fence_profile(db, &profile.id, profile.generation, session).await?;
    if let Some(binding) = binding {
        super::platform_setup_service::fence_owner(db, &binding.user_id, session).await?;
        if db.collection::<PlatformBinding>(COLLECTION_NAME).update_one(doc! {"_id": &binding.id, "user_id": &binding.user_id,
                    "generation": binding.generation, "platform_config_id": &profile.id, "profile_generation": profile.generation,
                    "telegram_sender_id": &binding.telegram_sender_id, "telegram_chat_id": &binding.telegram_chat_id, "is_active": true,
                    "$expr": {"$gt": ["$expires_at", "$$NOW"]}}, doc! {"$inc": {"reply_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 {
                    return Err(unavailable());
                }
    }
    Ok(())
}

pub(crate) async fn admit_reply(
    db: &Database,
    profile: &PlatformBotConfig,
    binding: Option<&PlatformBinding>,
) -> AppResult<()> {
    let db = db.clone();
    let profile = profile.clone();
    let binding = binding.cloned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                fence_reply(&db, &profile, binding.as_ref(), session).await?;
                Ok(())
            }
            .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)
}

async fn invalidate_setups(
    db: &Database,
    binding: &str,
    owner: &str,
    generation: i64,
    include_active: bool,
    session: &mut ClientSession,
) -> AppResult<()> {
    let states = if include_active {
        vec!["cancelled", "expired"]
    } else {
        vec!["cancelled", "expired", "active"]
    };
    let generations = if include_active {
        bson::Bson::Document(doc! {"$lte": generation})
    } else {
        generation.into()
    };
    db.collection::<bson::Document>(crate::models::platform_setup::COLLECTION_NAME).update_many(doc! {"binding_id": binding, "owner_user_id": owner, "binding_generation": generations,
        "state": {"$nin": states}}, doc! {"$set": {"state": "paused", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null,
            "delivery_tested": false, "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}).session(session).await?;
    Ok(())
}

async fn cancel_pending_requests(
    db: &Database,
    profile: &str,
    sender: &str,
    except: Option<&str>,
    session: &mut ClientSession,
) -> AppResult<()> {
    let mut filter = doc! {"platform_config_id": profile, "telegram_sender_id": sender, "state": {"$in": ["pending", "awaiting_chat_confirmation"]}};
    if let Some(id) = except {
        filter.insert("_id", doc! {"$ne": id});
    }
    db.collection::<PlatformBindingRequest>(REQUESTS)
        .update_many(filter, doc! {"$set": {"state": "cancelled"}})
        .session(session)
        .await?;
    Ok(())
}

pub async fn revoke_owned(db: &Database, actor: &str, id: &str, generation: i64) -> AppResult<()> {
    let db = db.clone();
    let actor = actor.to_owned();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            super::platform_setup_service::fence_owner(&db, &actor, session).await?;
            let binding = db.collection::<PlatformBinding>(COLLECTION_NAME).find_one(doc! {"_id": &id, "user_id": &actor, "generation": generation, "is_active": true}).session(&mut *session).await?.ok_or_else(unavailable)?;
            if db.collection::<PlatformBinding>(COLLECTION_NAME).update_one(doc! {"_id": &id, "user_id": &actor, "generation": generation, "is_active": true},
                doc! {"$set": {"is_active": false, "updated_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64, "revocation_generation": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(unavailable()); }
            cancel_pending_requests(&db, &binding.platform_config_id, &binding.telegram_sender_id, None, session).await?;
            invalidate_setups(&db, &id, &binding.user_id, generation, true, session).await
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn list_owned(db: &Database, actor: &str) -> AppResult<Vec<PlatformBinding>> {
    use futures::TryStreamExt;
    // Archived requests expire separately; settings only need revocable live bindings.
    db.collection::<PlatformBinding>(COLLECTION_NAME)
        .find(doc! {"user_id": actor, "is_active": true})
        .sort(doc! {"updated_at": -1, "_id": -1})
        .limit(100)
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub(crate) async fn unlink_sender(
    db: &Database,
    profile: &PlatformBotConfig,
    sender: &str,
    chat: &str,
) -> AppResult<()> {
    if sender != chat {
        return Err(unavailable());
    }
    let db = db.clone();
    let profile = profile.clone();
    let sender = sender.to_owned();
    let chat = chat.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            fence_profile(&db, &profile.id, profile.generation, session).await?;
            cancel_pending_requests(&db, &profile.id, &sender, None, session).await?;
            if let Some(binding) = db.collection::<PlatformBinding>(COLLECTION_NAME).find_one_and_update(
                doc! {"platform_config_id": &profile.id, "manager_bot_id": &profile.manager_bot_id, "telegram_sender_id": &sender, "telegram_chat_id": &chat, "is_active": true},
                doc! {"$set": {"is_active": false, "updated_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64, "revocation_generation": 1_i64}}
            ).session(&mut *session).await? {
                invalidate_setups(&db, &binding.id, &binding.user_id, binding.generation, true, session).await?;
            }
            Ok(())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

/// Joins broad personal `/unlink`; the caller owns manager identity fencing and
/// commit. This deliberately does not require the old guide to be enabled.
pub(crate) async fn unlink_sender_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    profile_id: &str,
    manager_bot_id: &str,
    sender: &str,
    chat: &str,
) -> AppResult<()> {
    if sender != chat {
        return Err(unavailable());
    }
    cancel_pending_requests(db, profile_id, sender, None, session).await?;
    if let Some(binding) = db.collection::<PlatformBinding>(COLLECTION_NAME).find_one_and_update(
        doc! {"platform_config_id":profile_id,"manager_bot_id":manager_bot_id,"telegram_sender_id":sender,"telegram_chat_id":chat,"is_active":true},
        doc! {"$set":{"is_active":false,"updated_at":bson::DateTime::now()},"$inc":{"generation":1_i64,"revocation_generation":1_i64}}
    ).session(&mut *session).await? {
        invalidate_setups(db, &binding.id, &binding.user_id, binding.generation, true, session).await?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::test_utils::connect_transaction_test_database;

    #[tokio::test]
    async fn platform_onboarding_decision_spacing_counts_invalid_attempts_with_server_clock() {
        let db = connect_transaction_test_database("platform_binding_spacing").await;
        let actor = uuid::Uuid::new_v4().to_string();
        assert!(
            decide(
                &db,
                b"test-only-hmac-key",
                &actor,
                "bad",
                "bad",
                true,
                false
            )
            .await
            .is_err()
        );
        assert!(matches!(
            decide(
                &db,
                b"test-only-hmac-key",
                &actor,
                "AAAAAAAA",
                "AAAAAAAA",
                true,
                false
            )
            .await,
            Err(AppError::RateLimited)
        ));
        let spacing = db.collection::<bson::Document>("platform_onboarding_decision_spacing");
        // Exact boundary is tested with a future timestamp to avoid scheduler delay.
        spacing
            .update_one(
                doc! {"_id": &actor},
                vec![doc! {"$set": {"last_decision_at": {"$add": ["$$NOW", 60_000_i64]}}}],
            )
            .await
            .unwrap();
        assert!(matches!(
            admit_decision(&db, &actor).await,
            Err(AppError::RateLimited)
        ));
        spacing
            .update_one(
                doc! {"_id": &actor},
                vec![doc! {"$set": {"last_decision_at": {"$subtract": ["$$NOW", 751_i64]}}}],
            )
            .await
            .unwrap();
        admit_decision(&db, &actor).await.unwrap();
        let results = tokio::join!(admit_decision(&db, &actor), admit_decision(&db, &actor));
        assert!(matches!(results.0, Err(AppError::RateLimited)));
        assert!(matches!(results.1, Err(AppError::RateLimited)));
        db.drop().await.unwrap();
    }
    async fn fixture(name: &str) -> (Database, PlatformBotConfig, String) {
        let db = connect_transaction_test_database(name).await;
        ensure_indexes(&db).await.unwrap();
        let actor = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &actor,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let now = Utc::now();
        let profile = PlatformBotConfig {
            identity_version: None,
            native_entry: None,
            guide: None,
            id: uuid::Uuid::new_v4().to_string(),
            channel_bot_id: uuid::Uuid::new_v4().to_string(),
            manager_bot_id: "991100".into(),
            observation_id: uuid::Uuid::new_v4().to_string(),
            execution_profile:
                crate::models::platform_bot_config::PlatformExecutionProfile::PlatformOnboarding,
            generation: 1,
            enabled: true,
            funding_owner_user_id: uuid::Uuid::new_v4().to_string(),
            configured_by: actor.clone(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
            .insert_one(&profile)
            .await
            .unwrap();
        (db, profile, actor)
    }

    #[tokio::test]
    async fn platform_onboarding_binding_deduplicates_and_unlink_cancels_unbound_approval() {
        let (db, profile, actor) = fixture("platform_binding_unbound_unlink").await;
        let key = b"test-only-hmac-key";
        let (request, code) = begin(&db, key, &profile, "123", "123", 10).await.unwrap();
        for _ in 0..5 {
            let (duplicate, duplicate_code) =
                begin(&db, key, &profile, "123", "123", 10).await.unwrap();
            assert_eq!(duplicate.id, request.id);
            assert_eq!(duplicate_code.as_str(), code.as_str());
        }
        assert_eq!(
            db.collection::<PlatformBindingRequest>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        decide(&db, key, &actor, &code, &code, true, false)
            .await
            .unwrap();
        assert!(
            confirm_from_chat(&db, key, &profile, "124", "124", &code)
                .await
                .is_err()
        );
        unlink_sender(&db, &profile, "123", "123").await.unwrap();
        assert!(
            confirm_from_chat(&db, key, &profile, "123", "123", &code)
                .await
                .is_err()
        );
        assert!(
            live_for_sender(&db, &profile, "123", "123")
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_binding_revocation_fences_prepared_reply_and_confirmation() {
        let (db, profile, actor) = fixture("platform_binding_revoke_reply").await;
        let key = b"test-only-hmac-key";
        let (_, code) = begin(&db, key, &profile, "123", "123", 10).await.unwrap();
        decide(&db, key, &actor, &code, &code, true, false)
            .await
            .unwrap();
        let binding = confirm_from_chat(&db, key, &profile, "123", "123", &code)
            .await
            .unwrap();
        admit_reply(&db, &profile, Some(&binding)).await.unwrap();
        let stranger = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &stranger,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        assert!(list_owned(&db, &stranger).await.unwrap().is_empty());
        assert_eq!(list_owned(&db, &actor).await.unwrap().len(), 1);
        assert!(
            revoke_owned(&db, &stranger, &binding.id, binding.generation)
                .await
                .is_err()
        );
        let (_, second_code) = begin(&db, key, &profile, "123", "123", 11).await.unwrap();
        db.collection::<bson::Document>("platform_onboarding_decision_spacing")
            .update_one(
                doc! {"_id": &actor},
                vec![doc! {"$set": {"last_decision_at": {"$subtract": ["$$NOW", 751_i64]}}}],
            )
            .await
            .unwrap();
        decide(&db, key, &actor, &second_code, &second_code, true, false)
            .await
            .unwrap();
        revoke_owned(&db, &actor, &binding.id, binding.generation)
            .await
            .unwrap();
        assert!(admit_reply(&db, &profile, Some(&binding)).await.is_err());
        assert!(
            confirm_from_chat(&db, key, &profile, "123", "123", &second_code)
                .await
                .is_err()
        );
        // An inactive user also cannot admit a prepared account-specific reply.
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id": &actor}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        assert!(admit_reply(&db, &profile, Some(&binding)).await.is_err());
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn platform_onboarding_binding_expiry_sender_and_generation_fences() {
        let (db, profile, actor) = fixture("platform_binding_expiry_fences").await;
        let key = b"test-only-hmac-key";
        assert!(begin(&db, key, &profile, "123", "124", 1).await.is_err());
        assert!(begin(&db, key, &profile, "0", "0", 1).await.is_err());
        let (request, code) = begin(&db, key, &profile, "123", "123", 1).await.unwrap();
        decide(&db, key, &actor, &code, &code, true, false)
            .await
            .unwrap();
        assert!(
            confirm_from_chat(&db, key, &profile, "124", "124", &code)
                .await
                .is_err()
        );
        db.collection::<bson::Document>(REQUESTS)
            .update_one(
                doc! {"_id": &request.id},
                vec![doc! {"$set": {"expires_at": {"$subtract": ["$$NOW", 1_i64]}}}],
            )
            .await
            .unwrap();
        assert!(preview(&db, key, &code).await.is_err());
        assert!(
            confirm_from_chat(&db, key, &profile, "123", "123", &code)
                .await
                .is_err()
        );
        db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &profile.id},
                doc! {"$set": {"enabled": false}, "$inc": {"generation": 1_i64, "revocation_generation": 1_i64}},
            )
            .await
            .unwrap();
        assert!(begin(&db, key, &profile, "123", "123", 2).await.is_err());
        assert!(admit_reply(&db, &profile, None).await.is_err());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_decision_spacing_concurrent_replicas_admit_one() {
        let db = connect_transaction_test_database("platform_binding_spacing_concurrent").await;
        let actor = uuid::Uuid::new_v4().to_string();
        let results = tokio::join!(admit_decision(&db, &actor), admit_decision(&db, &actor));
        assert_eq!(
            usize::from(results.0.is_ok()) + usize::from(results.1.is_ok()),
            1
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn platform_onboarding_rejected_code_transaction_does_not_refund_spacing() {
        let (db, _, actor) = fixture("platform_binding_rejected_guess").await;
        assert!(matches!(
            decide(
                &db,
                b"test-only-key",
                &actor,
                "AAAAAAAA",
                "AAAAAAAA",
                true,
                false
            )
            .await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            decide(
                &db,
                b"test-only-key",
                &actor,
                "BBBBBBBB",
                "BBBBBBBB",
                true,
                false
            )
            .await,
            Err(AppError::RateLimited)
        ));
        db.drop().await.unwrap();
    }
}

/// Durable, metadata-only admission for native text replies, including denied
/// model/quota attempts. Redelivery cannot consume another reply allowance.
pub(crate) async fn admit_native_reply_in_session(
    db: Database,
    profile: PlatformBotConfig,
    binding: Option<PlatformBinding>,
    source: String,
    sender: String,
    update_id: i64,
    kind: &'static str,
    allow_paused: bool,
    session: &mut ClientSession,
) -> AppResult<bool> {
    if allow_paused {
        if db
            .collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &profile.id, "generation": profile.generation},
                doc! {"$inc": {"native_reply_revision": 1_i64}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(unavailable());
        }
    } else {
        fence_reply(&db, &profile, binding.as_ref(), session).await?;
    }
    let replies = db.collection::<bson::Document>("platform_native_replies");
    if replies
        .find_one(
            doc! {"source_id": &source, "sender": &sender, "update_id": update_id, "kind": kind},
        )
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    if !super::coordination_service::RateWindowStore::admit_in_session(
        &db,
        "platform_native_reply_sender",
        &format!("{source}:{sender}"),
        10,
        std::time::Duration::from_secs(60),
        session,
    )
    .await?
        || !super::coordination_service::RateWindowStore::admit_in_session(
            &db,
            "platform_native_reply_profile",
            &source,
            100,
            std::time::Duration::from_secs(60),
            session,
        )
        .await?
    {
        return Ok(false);
    }
    replies.insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "source_id": &source, "sender": &sender, "update_id": update_id, "kind": kind,
                "purge_after": bson::DateTime::from_chrono(Utc::now() + Duration::days(1))}).session(&mut *session).await?;
    Ok(true)
}
