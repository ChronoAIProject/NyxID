use super::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    platform_inference_service::{GuideAuthority, PublicPurpose},
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};
pub const COLLECTION: &str = "platform_native_receipts";
#[derive(Clone)]
pub(crate) struct ReceiptClaim {
    pub id: String,
    pub generation: i64,
    pub claim_id: String,
}
fn closed() -> AppError {
    AppError::Conflict("Native processing authority changed".into())
}

pub(crate) async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(COLLECTION)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"source_id": 1, "sender": 1, "update_id": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(COLLECTION)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"phase": 1, "retry_at": 1})
                .build(),
        )
        .await?;
    db.collection::<Document>(COLLECTION)
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

pub(crate) async fn fence(
    db: &Database,
    claim: &ReceiptClaim,
    session: &mut ClientSession,
) -> AppResult<()> {
    if db
        .collection::<Document>(COLLECTION)
        .update_one(
            doc! {"_id": &claim.id, "generation": claim.generation, "claim_id": &claim.claim_id,
            "phase": "processing", "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
            doc! {"$inc": {"effect_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(closed());
    }
    Ok(())
}

pub(crate) async fn admit(state: &AppState, a: &GuideAuthority) -> AppResult<Option<ReceiptClaim>> {
    let db = state.db.clone();
    let a = a.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            let (source, installation, kind) = match &a.purpose {
                PublicPurpose::Guide => (format!("guide:{}", a.profile.id), None, "guide"),
                PublicPurpose::Readiness => return Err(closed()),
                PublicPurpose::Customer {installation, ..} => (format!("child:{}", installation.id), Some(installation), "customer"),
            };
            if kind == "guide" && (!a.profile.enabled || a.profile.deleted_at.is_some()) {
                if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"_id": &a.profile.id, "generation": a.profile.generation}, doc! {"$inc": {"native_ingress_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            } else { super::platform_inference_service::fence_public(&db, &a, session).await?; }
            if db.collection::<Document>(COLLECTION).find_one(doc! {"source_id": &source, "sender": &a.sender, "update_id": a.update_id}).session(&mut *session).await?.is_some() { return Ok(None); }
            if !super::coordination_service::RateWindowStore::admit_in_session(&db, "platform_native_ingress", &source, 1000, std::time::Duration::from_secs(3600), session).await? { return Err(AppError::RateLimited); }
            let claim = ReceiptClaim {id: uuid::Uuid::new_v4().to_string(), generation: 1, claim_id: uuid::Uuid::new_v4().to_string()};
            db.collection::<Document>(COLLECTION).insert_one(doc! {"_id": &claim.id, "source_id": &source, "kind": kind, "sender": &a.sender, "update_id": a.update_id,
                "profile_id": &a.profile.id, "profile_generation": a.profile.generation, "binding_id": a.binding.as_ref().map(|b| &b.id), "binding_generation": a.binding.as_ref().map(|b| b.generation),
                "installation_id": installation.map(|i| &i.id), "installation_generation": installation.map(|i| i.generation),
                "phase": "processing", "generation": claim.generation, "claim_id": &claim.claim_id, "attempts": 0_i64,
                "retry_at": bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::seconds(150)),
                "purge_after": bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::days(1))}).session(&mut *session).await?;
            // Store the deadline using database time, including cross-replica admission.
            db.collection::<Document>(COLLECTION).update_one(doc! {"_id": &claim.id}, vec![doc! {"$set": {"retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 150_i64}}}}]).session(&mut *session).await?;
            Ok(Some(claim))
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

/// Proof decisions have only Mongo effects: their dedup row commits with the decision.
/// The caller fences live customer authority in the same transaction.
pub(crate) async fn admit_proof_in_session(
    db: &Database,
    installation: &str,
    sender: i64,
    update: i64,
    session: &mut ClientSession,
) -> AppResult<bool> {
    let source = format!("proof:{installation}");
    if db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"source_id": &source, "sender": sender.to_string(), "update_id": update})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    db.collection::<Document>(COLLECTION).insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "source_id": source,
        "sender": sender.to_string(), "update_id": update, "kind": "customer_proof", "installation_id": installation,
        "phase": "completed", "generation": 1_i64, "claim_id": bson::Bson::Null, "attempts": 0_i64,
        "purge_after": bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::days(1))}).session(session).await?;
    Ok(true)
}

pub(crate) async fn finish(db: &Database, claim: &ReceiptClaim, success: bool) -> AppResult<()> {
    db.collection::<Document>(COLLECTION).update_one(doc! {"_id": &claim.id, "generation": claim.generation, "claim_id": &claim.claim_id, "phase": "processing"},
        vec![doc! {"$set": {"phase": if success {"completed"} else {"recoverable"}, "claim_id": bson::Bson::Null,
            "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 30_i64}}}}]).await?;
    Ok(())
}

pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    let service = super::telegram_new_service::TelegramNewService {
        manager_id: None,
        db: &state.db,
        keys: &state.encryption_keys,
        config: &state.config,
        api: super::telegram_new_api::TelegramApi::new(&state.http_client),
    };
    sweep_with_service(state, &service).await
}

pub(crate) async fn claim_recovery(db: &Database) -> AppResult<Option<Document>> {
    let claim = uuid::Uuid::new_v4().to_string();
    Ok(db.collection::<Document>(COLLECTION).find_one_and_update(doc! {"phase": {"$in": ["processing", "recoverable", "recovering"]},
            "attempts": {"$lt": 5_i64}, "$expr": {"$lte": ["$retry_at", "$$NOW"]}},
            vec![doc! {"$set": {"phase": "recovering", "claim_id": &claim, "generation": {"$add": ["$generation", 1_i64]},
                "attempts": {"$add": ["$attempts", 1_i64]}, "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 60_i64}}}}])
            .sort(doc! {"retry_at": 1}).return_document(ReturnDocument::After).await?)
}

pub(crate) async fn sweep_with_service(
    state: &AppState,
    service: &super::telegram_new_service::TelegramNewService<'_>,
) -> AppResult<()> {
    for _ in 0..8 {
        let Some(row) = claim_recovery(&state.db).await? else {
            break;
        };
        let claim = row.get_str("claim_id").map_err(|_| closed())?;
        let result = recover_notice(state, service, &row).await;
        state.db.collection::<Document>(COLLECTION).update_one(doc! {"_id": row.get_str("_id").map_err(|_| closed())?, "generation": row.get_i64("generation").map_err(|_| closed())?,
            "claim_id": &claim, "phase": "recovering", "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
            vec![doc! {"$set": {"phase": if result.is_ok() {"completed"} else if row.get_i64("attempts").map_err(|_| closed())? >= 5 {"interrupted"} else {"recoverable"}, "claim_id": bson::Bson::Null,
                "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 60_i64}}}}]).await?;
    }
    Ok(())
}

async fn recover_notice(
    state: &AppState,
    service: &super::telegram_new_service::TelegramNewService<'_>,
    row: &Document,
) -> AppResult<()> {
    use crate::models::{
        platform_binding::PlatformBinding, platform_bot_config::PlatformBotConfig,
    };
    let profile_id = row.get_str("profile_id").map_err(|_| closed())?;
    let Some(profile) = state.db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME).find_one(doc! {"_id": profile_id,
        "generation": row.get_i64("profile_generation").map_err(|_| closed())?, "enabled": true, "deleted_at": bson::Bson::Null}).await? else {return Ok(())};
    if row.get_str("kind").ok() == Some("customer") {
        let Some(installation) = state.db.collection::<crate::models::platform_installation::PlatformInstallation>(crate::models::platform_installation::COLLECTION_NAME).find_one(doc! {
            "_id": row.get_str("installation_id").map_err(|_| closed())?, "generation": row.get_i64("installation_generation").map_err(|_| closed())?,
            "state": {"$in": ["active", "ready_for_test"]}}).await? else {return Ok(())};
        let sender: i64 = row
            .get_str("sender")
            .map_err(|_| closed())?
            .parse()
            .map_err(|_| closed())?;
        let a = match super::platform_runtime_service::capture(
            state,
            &installation.channel_bot_id,
            sender,
            row.get_i64("update_id").map_err(|_| closed())?,
            installation.state == "ready_for_test",
        )
        .await
        {
            Ok(a) => a,
            Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let bot =
            super::channel_bot_service::get_bot(&state.db, &installation.channel_bot_id).await?;
        let token = zeroize::Zeroizing::new(
            super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?,
        );
        let db = state.db.clone();
        let captured = row.clone();
        let mut session = db.client().start_session().await?;
        let admitted = session.start_transaction().and_run2(async move |session| {
            let result: AppResult<bool> = async {
                super::platform_inference_service::fence_public(&db, &a, session).await?;
                if db.collection::<Document>(COLLECTION).update_one(doc! {"_id": captured.get_str("_id").map_err(|_| closed())?, "generation": captured.get_i64("generation").map_err(|_| closed())?,
                    "claim_id": captured.get_str("claim_id").map_err(|_| closed())?, "phase": "recovering", "$expr": {"$gt": ["$retry_at", "$$NOW"]}}, doc! {"$inc": {"effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
                let kind = recovery_kind(captured.get_i64("attempts").map_err(|_| closed())?);
                super::platform_binding_service::admit_native_reply_in_session(db.clone(), a.profile.clone(), if installation.state == "active" {None} else {a.binding.clone()},
                    captured.get_str("source_id").map_err(|_| closed())?.to_owned(), sender.to_string(), captured.get_i64("update_id").map_err(|_| closed())?, kind, false, session).await
            }.await; transaction_result(result)
        }).await.map_err(map_transaction_error)?;
        if admitted {
            service.api.call(&token, "sendMessage", serde_json::json!({"chat_id": sender, "text": "Your message did not finish delivery. Please send it again to continue.", "protect_content": true})).await?;
        }
        return Ok(());
    }
    // A changed/revoked binding terminates recovery instead of messaging the new client.
    let binding = if let Ok(id) = row.get_str("binding_id") {
        let binding = state.db.collection::<PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME).find_one(doc! {"_id": id,
            "generation": row.get_i64("binding_generation").map_err(|_| closed())?, "is_active": true, "$expr": {"$gt": ["$expires_at", "$$NOW"]}}).await?;
        if binding.is_none() {
            return Ok(());
        }
        binding
    } else {
        None
    };
    let service = service
        .for_manager(profile.manager_bot_id.parse().map_err(|_| closed())?)
        .await?;
    let (manager, _, values) = service.manager().await?;
    if manager.to_string() != profile.manager_bot_id
        || values.get("observation_id") != Some(profile.observation_id.as_str())
    {
        return Ok(());
    }
    let token = values
        .get(super::channel_adapters::telegram_new::MANAGER_TOKEN)
        .ok_or_else(closed)?;
    let db = state.db.clone();
    let captured = row.clone();
    let profile = profile.clone();
    let binding = binding.clone();
    let mut session = db.client().start_session().await?;
    let admitted = session.start_transaction().and_run2(async move |session| {
        let result: AppResult<bool> = async {
            if db.collection::<Document>(COLLECTION).update_one(doc! {"_id": captured.get_str("_id").map_err(|_| closed())?, "generation": captured.get_i64("generation").map_err(|_| closed())?,
                "claim_id": captured.get_str("claim_id").map_err(|_| closed())?, "phase": "recovering", "$expr": {"$gt": ["$retry_at", "$$NOW"]}}, doc! {"$inc": {"effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            let kind = match captured.get_i64("attempts").map_err(|_| closed())? {1 => "recovery_1", 2 => "recovery_2", 3 => "recovery_3", 4 => "recovery_4", _ => "recovery_5"};
            super::platform_binding_service::admit_native_reply_in_session(db.clone(), profile.clone(), binding.clone(), captured.get_str("source_id").map_err(|_| closed())?.to_owned(),
                captured.get_str("sender").map_err(|_| closed())?.to_owned(), captured.get_i64("update_id").map_err(|_| closed())?, kind, false, session).await
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)?;
    if admitted {
        service.api.call(token, "sendMessage", serde_json::json!({"chat_id": row.get_str("sender").map_err(|_| closed())?,
        "text": "Your last guide message did not finish delivery. A draft may already have been saved. Review My bot setups on NyxID, or resend the message to continue.", "protect_content": true})).await?;
    }
    Ok(())
}

fn recovery_kind(attempt: i64) -> &'static str {
    match attempt {
        1 => "recovery_1",
        2 => "recovery_2",
        3 => "recovery_3",
        4 => "recovery_4",
        _ => "recovery_5",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn platform_onboarding_proof_dedup_aborts_with_decision_and_is_child_scoped() {
        let db =
            crate::test_utils::connect_transaction_test_database("platform_proof_atomic").await;
        ensure_indexes(&db).await.unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            admit_proof_in_session(&db, "child-a", 800, 10, &mut session)
                .await
                .unwrap()
        );
        session.abort_transaction().await.unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        session.start_transaction().await.unwrap();
        assert!(
            admit_proof_in_session(&db, "child-a", 800, 10, &mut session)
                .await
                .unwrap()
        );
        assert!(
            !admit_proof_in_session(&db, "child-a", 800, 10, &mut session)
                .await
                .unwrap()
        );
        assert!(
            admit_proof_in_session(&db, "child-b", 800, 10, &mut session)
                .await
                .unwrap()
        );
        session.commit_transaction().await.unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION)
                .count_documents(doc! {"phase": "completed"})
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        drop(session);
        db.drop().await.unwrap();
        db.client().clone().shutdown().await;
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[tokio::test]
    async fn platform_onboarding_reclaimed_receipt_blocks_late_draft_and_redelivery_never_replays_model()
     {
        let f = super::super::platform_onboarding_test_support::fixture().await;
        let mut authority = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 11000,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let claim = admit(&f.state, &authority).await.unwrap().unwrap();
        authority.receipt = Some(claim.clone());
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .update_one(
                doc! {"_id": &claim.id},
                vec![doc! {"$set": {"retry_at": {"$subtract": ["$$NOW", 1_i64]}}}],
            )
            .await
            .unwrap();
        let recovered = claim_recovery(&f.state.db).await.unwrap().unwrap();
        assert!(recovered.get_i64("generation").unwrap() > claim.generation);
        let facts =
            super::super::platform_setup_service::decrypt_facts(&f.state.encryption_keys, &f.setup)
                .await
                .unwrap();
        assert!(
            super::super::platform_setup_service::guide_draft(
                &f.state,
                &authority,
                f.setup.template,
                &facts
            )
            .await
            .is_err()
        );
        authority.receipt = None;
        assert!(admit(&f.state, &authority).await.unwrap().is_none());
        assert_eq!(
            f.state
                .db
                .collection::<Document>("platform_public_turns")
                .count_documents(doc! {"update_id": 11000_i64})
                .await
                .unwrap(),
            0
        );
        // A crashed admitted update becomes a fixed recovery notice, not another
        // paid inference or a replay of its transient user message.
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .update_one(
                doc! {"_id": &claim.id},
                vec![doc! {"$set": {"retry_at": {"$subtract": ["$$NOW", 1_i64]}}}],
            )
            .await
            .unwrap();
        let base = f.server.uri();
        let service = super::super::telegram_new_tests::service(&f.state, &base);
        sweep_with_service(&f.state, &service).await.unwrap();
        let done = f
            .state
            .db
            .collection::<Document>(COLLECTION)
            .find_one(doc! {"_id": &claim.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.get_str("phase").unwrap(), "completed");
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| !request.url.path().ends_with("/chat/completions"))
        );
        assert!(admit(&f.state, &authority).await.unwrap().is_none());
    }
}
