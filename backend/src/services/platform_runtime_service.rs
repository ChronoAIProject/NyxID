use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use super::platform_inference_service::{GuideAuthority, PublicPurpose};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        platform_binding::PlatformBinding,
        platform_installation::{COLLECTION_NAME, DeliveryTest, PlatformInstallation, TESTS},
        platform_setup::{PlatformSetup, SetupState},
    },
};
use hmac::{Hmac, Mac};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use sha2::Sha256;
use subtle::ConstantTimeEq;
fn closed() -> AppError {
    AppError::Conflict("This public bot is paused or requires a fresh owner review".into())
}

pub(crate) async fn managed(db: &Database, bot_id: &str) -> AppResult<bool> {
    let bot = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": bot_id})
        .await?;
    if bot
        .as_ref()
        .is_some_and(|bot| bot.contains_key("platform_installation_id"))
    {
        return Ok(true);
    }
    let platform = bot
        .as_ref()
        .and_then(|bot| bot.get_str("platform").ok())
        .unwrap_or("");
    let remote = bot
        .as_ref()
        .and_then(|bot| bot.get_str("platform_bot_id").ok())
        .unwrap_or("");
    let filter = super::platform_bot_authority::installation_filter(bot_id, platform, remote);
    Ok(db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .is_some())
}

pub(crate) async fn capture(
    state: &AppState,
    bot_id: &str,
    sender: i64,
    update_id: i64,
    test_only: bool,
) -> AppResult<GuideAuthority> {
    let installation = state.db.collection::<PlatformInstallation>(COLLECTION_NAME).find_one(doc! {"channel_bot_id": bot_id, "state": if test_only {"ready_for_test"} else {"active"}}).await?.ok_or_else(closed)?;
    if test_only && sender == installation.authority.telegram_sender_id {
        return Err(AppError::Forbidden(
            "Use an independent customer Telegram account for delivery testing".into(),
        ));
    }
    let setup = state
        .db
        .collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME)
        .find_one(doc! {"_id": &installation.setup_id})
        .await?
        .ok_or_else(closed)?;
    let mut profile = super::platform_bot_config_service::get(
        &state.db,
        &installation.authority.platform_config_id,
    )
    .await?;
    let binding = state
        .db
        .collection::<PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME)
        .find_one(doc! {"_id": &installation.authority.binding_id})
        .await?
        .ok_or_else(closed)?;
    let guide = profile.guide.as_mut().ok_or_else(closed)?;
    if setup.resources.runtime_model_service_id.as_deref() != Some(guide.model_service_id.as_str())
        || setup.resources.runtime_model_id.as_deref() != Some(guide.model_id.as_str())
    {
        return Err(closed());
    }
    guide.limits = setup.limits.clone();
    guide.platform_turns_per_hour = setup.limits.turns_per_hour;
    guide.platform_concurrent_turns = setup.limits.concurrent_turns;
    guide.anonymous_daily_credits = setup.limits.daily_credits;
    let a = GuideAuthority {
        profile,
        binding: Some(binding),
        sender: sender.to_string(),
        update_id,
        receipt: None,
        purpose: PublicPurpose::Customer {
            installation,
            setup,
            test_only,
        },
    };
    super::platform_inference_service::admit_effect(state, &a).await?;
    Ok(a)
}

fn proof(key: &[u8], test: &str, value: &str) -> AppResult<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| closed())?;
    mac.update(format!("platform-customer-receipt-v1:{test}:{value}").as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub(crate) async fn begin_test(
    state: &AppState,
    a: &GuideAuthority,
) -> AppResult<(DeliveryTest, zeroize::Zeroizing<String>)> {
    let PublicPurpose::Customer {
        installation,
        setup,
        test_only: true,
    } = &a.purpose
    else {
        return Err(closed());
    };
    if a.sender == installation.authority.telegram_sender_id.to_string() {
        return Err(AppError::Forbidden(
            "Use an independent customer Telegram account for the delivery test".into(),
        ));
    }
    let code = zeroize::Zeroizing::new(hex::encode(rand::random::<[u8; 8]>()));
    let id = uuid::Uuid::new_v4().to_string();
    let row = DeliveryTest {
        id: id.clone(),
        installation_id: installation.id.clone(),
        installation_generation: installation.generation,
        setup_revision: setup.revision,
        digest: installation.authority.digest.clone(),
        customer_sender_id: a.sender.clone(),
        customer_chat_id: a.sender.clone(),
        challenge_hmac: proof(state.auth_device_hmac_key.as_slice(), &id, code.as_str())?,
        reply_message_id: None,
        state: "prepared".into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
    };
    super::platform_inference_service::admit_effect(state, a).await?;
    state
        .db
        .collection::<DeliveryTest>(TESTS)
        .insert_one(&row)
        .await?;
    Ok((row, code))
}

pub(crate) async fn observed_test_send(
    state: &AppState,
    a: &GuideAuthority,
    test: &DeliveryTest,
    message: i64,
) -> AppResult<()> {
    if message <= 0 {
        return Err(closed());
    }
    super::platform_inference_service::admit_effect(state, a).await?;
    state.db.collection::<DeliveryTest>(TESTS).update_one(doc! {"_id": &test.id, "installation_generation": test.installation_generation, "state": "prepared", "$expr": {"$gt": ["$expires_at", "$$NOW"]}},
        doc! {"$set": {"state": "awaiting_customer_reply", "reply_message_id": message.to_string()}}).await?;
    Ok(())
}

pub(crate) async fn confirm_test(
    state: &AppState,
    bot_id: &str,
    sender: i64,
    reply_to: i64,
    code: &str,
    update_id: i64,
) -> AppResult<bool> {
    let observed = state
        .db
        .collection::<PlatformInstallation>(COLLECTION_NAME)
        .find_one(doc! {"channel_bot_id": bot_id})
        .await?
        .ok_or_else(closed)?;
    // Count every attempt before capture performs authority coordination writes.
    // Rejected guesses consume these durable budgets outside the decision transaction.
    let sender_key = format!("{}:{sender}", observed.id);
    let sender_admission = super::coordination_service::RateWindowStore::admit(
        &state.db,
        "platform_customer_proof_sender",
        &sender_key,
        5,
        std::time::Duration::from_secs(60),
    )
    .await?;
    if !sender_admission.allowed {
        return Ok(false);
    }
    let installation_admission = super::coordination_service::RateWindowStore::admit(
        &state.db,
        "platform_customer_proof_installation",
        &observed.id,
        100,
        std::time::Duration::from_secs(60),
    )
    .await?;
    if !installation_admission.allowed {
        return Ok(false);
    }
    let a = capture(state, bot_id, sender, update_id, true).await?;
    let PublicPurpose::Customer {
        installation,
        setup,
        ..
    } = &a.purpose
    else {
        return Err(closed());
    };
    let db = state.db.clone();
    let installation = installation.clone();
    let setup = setup.clone();
    let code = zeroize::Zeroizing::new(code.to_owned());
    let key = state.auth_device_hmac_key.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<bool> = async {
            let live = super::platform_provisioning_service::fence_authority(&db, &installation.authority, session).await?;
            if live.state != SetupState::ReadyForTest || sender == installation.authority.telegram_sender_id { return Err(closed()); }
            if db.collection::<PlatformInstallation>(COLLECTION_NAME).update_one(doc! {"_id": &installation.id, "generation": installation.generation, "state": "ready_for_test"}, doc! {"$inc": {"test_fence": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            if !super::platform_native_receipt_service::admit_proof_in_session(&db, &installation.id, sender, update_id, session).await? { return Ok(false); }
            let Some(test) = db.collection::<DeliveryTest>(TESTS).find_one(doc! {"installation_id": &installation.id, "installation_generation": installation.generation,
                "setup_revision": setup.revision, "digest": &installation.authority.digest, "customer_sender_id": sender.to_string(), "customer_chat_id": sender.to_string(),
                "reply_message_id": reply_to.to_string(), "state": "awaiting_customer_reply", "$expr": {"$gt": ["$expires_at", "$$NOW"]}}).session(&mut *session).await? else { return Ok(false) };
            if code.len() != 16 || !bool::from(proof(key.as_slice(), &test.id, code.as_str())?.as_bytes().ct_eq(test.challenge_hmac.as_bytes())) { return Ok(false); }
            if db.collection::<DeliveryTest>(TESTS).update_one(doc! {"_id": &test.id, "state": "awaiting_customer_reply", "installation_generation": installation.generation, "$expr": {"$gt": ["$expires_at", "$$NOW"]}}, doc! {"$set": {"state": "confirmed"}}).session(&mut *session).await?.matched_count != 1 { return Ok(false); }
            if db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).update_one(doc! {"_id": &setup.id, "revision": setup.revision, "approved_digest": &installation.authority.digest, "state": "ready_for_test"},
                doc! {"$set": {"delivery_tested": true, "delivery_test_message_id": &test.reply_message_id}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            Ok(true)
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn activate_owned(
    state: &AppState,
    owner: &str,
    id: &str,
    revision: i64,
) -> AppResult<PlatformSetup> {
    let row = super::platform_setup_service::get_owned(&state.db, owner, id).await?;
    if row.revision != revision || row.state != SetupState::ReadyForTest || !row.delivery_tested {
        return Err(closed());
    }
    let installation = state
        .db
        .collection::<PlatformInstallation>(COLLECTION_NAME)
        .find_one(doc! {"setup_id": id, "owner_user_id": owner, "state": "ready_for_test"})
        .await?
        .ok_or_else(closed)?;
    let profile = super::platform_bot_config_service::get(
        &state.db,
        &installation.authority.platform_config_id,
    )
    .await?;
    super::platform_setup_service::validate_facts(
        row.template,
        &row.published.as_ref().ok_or_else(closed)?.facts,
        true,
    )?;
    super::platform_setup_service::validate_limits(&row.limits)?;
    let resolved = state
        .billing
        .owner_resolver()
        .resolve_for_execution(
            owner,
            owner,
            crate::models::usage_meter::CredentialClass::NyxidManagedMaster,
        )
        .await?;
    if resolved.owner_id != owner {
        return Err(closed());
    }
    super::platform_inference_service::probe_funding(state, &profile, owner, &row.limits).await?;
    let db = state.db.clone();
    let owner = owner.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            let live = super::platform_provisioning_service::fence_authority(&db, &installation.authority, session).await?;
            if live.state != SetupState::ReadyForTest || !live.delivery_tested || live.owner_user_id.as_deref() != Some(owner.as_str()) { return Err(closed()); }
            if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id": &installation.channel_bot_id, "user_id": &owner,
                "platform_installation_id": &installation.id, "is_active": true, "status": "active", "webhook_registered": true}, doc! {"$inc": {"public_effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            if db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(doc! {"_id": &installation.agent_id, "user_id": &owner,
                "platform_setup_id": &live.id, "destroyed_at": bson::Bson::Null}, doc! {"$inc": {"public_effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            if db.collection::<DeliveryTest>(TESTS).find_one(doc! {"installation_id": &installation.id, "installation_generation": installation.generation,
                "setup_revision": revision, "digest": &installation.authority.digest, "state": "confirmed", "customer_sender_id": {"$ne": installation.authority.telegram_sender_id.to_string()},
                "$expr": {"$gt": ["$expires_at", "$$NOW"]}}).session(&mut *session).await?.is_none() { return Err(closed()); }
            let native = db.collection::<crate::models::telegram_bot_request::TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME).find_one(doc! {"_id": &installation.channel_bot_id, "status": "connected"}).session(&mut *session).await?.ok_or_else(closed)?;
            super::platform_provisioning_service::fence_request(&db, &native, true, session).await?;
            if db.collection::<PlatformInstallation>(COLLECTION_NAME).update_one(doc! {"_id": &installation.id, "generation": installation.generation, "state": "ready_for_test"},
                doc! {"$set": {"state": "active", "updated_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            super::platform_setup_notice_service::queue(&db, &installation.authority, "active", session).await?;
            db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).find_one_and_update(doc! {"_id": &live.id, "owner_user_id": &owner, "revision": revision, "state": "ready_for_test"},
                doc! {"$set": {"state": "active", "expires_at": bson::Bson::Null, "purge_after": bson::Bson::Null, "updated_at": bson::DateTime::now()}}).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?.ok_or_else(closed)
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}
