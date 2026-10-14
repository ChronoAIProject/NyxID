//! Durable metadata-only setup notices. Browser status remains authoritative.
use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        platform_binding::PlatformBinding,
        platform_installation::{NOTICES, SetupStepAuthority},
        platform_setup::PlatformSetup,
    },
};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
fn closed() -> AppError {
    AppError::Conflict("Setup notice authority changed".into())
}

pub(crate) async fn queue(
    db: &Database,
    a: &SetupStepAuthority,
    kind: &'static str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(NOTICES).update_one(doc! {"setup_id": &a.setup_id, "revision": a.setup_revision, "kind": kind},
        doc! {"$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "setup_id": &a.setup_id, "revision": a.setup_revision,
            "authority_generation": a.authority_generation, "kind": kind, "binding_id": &a.binding_id, "binding_generation": a.binding_generation,
            "profile_id": &a.platform_config_id, "profile_generation": a.profile_generation, "completed": false, "attempts": 0_i64,
            "claim_generation": 0_i64, "retry_at": bson::DateTime::now(), "expires_at": bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::days(7))}})
        .upsert(true).session(session).await?;
    Ok(())
}

async fn claim_notice(db: &Database) -> AppResult<Option<Document>> {
    let claim = uuid::Uuid::new_v4().to_string();
    Ok(db.collection::<Document>(NOTICES).find_one_and_update(
            doc! {"completed": false, "attempts": {"$lt": 5_i64}, "$expr": {"$and": [{"$lte": ["$retry_at", "$$NOW"]}, {"$gt": ["$expires_at", "$$NOW"]}]}},
            vec![doc! {"$set": {"claim_id": &claim, "claim_generation": {"$add": [{"$ifNull": ["$claim_generation", 0_i64]}, 1_i64]},
                "attempts": {"$add": ["$attempts", 1_i64]}, "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 60_i64}}}}])
            .return_document(mongodb::options::ReturnDocument::After).await?)
}

pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    for _ in 0..16 {
        let Some(row) = claim_notice(&state.db).await? else {
            break;
        };
        let claim = row.get_str("claim_id").map_err(|_| closed())?;
        let result = deliver(state, &row).await;
        let terminal = result.is_ok()
            || matches!(
                result,
                Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_))
            );
        state.db.collection::<Document>(NOTICES).update_one(doc! {"_id": row.get_str("_id").map_err(|_| closed())?, "claim_id": &claim,
            "claim_generation": row.get_i64("claim_generation").map_err(|_| closed())?, "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
            doc! {"$set": {"completed": terminal}}).await?;
        if let Err(error) = result {
            tracing::warn!(
                error_code = error.error_code(),
                "Platform setup notice not delivered"
            );
        }
    }
    Ok(())
}

async fn deliver(state: &AppState, row: &Document) -> AppResult<()> {
    let service = super::telegram_new_service::TelegramNewService {
        db: &state.db,
        keys: &state.encryption_keys,
        config: &state.config,
        api: super::telegram_new_api::TelegramApi::new(&state.http_client),
    };
    deliver_with_service(state, row, &service).await
}

async fn deliver_with_service(
    state: &AppState,
    row: &Document,
    service: &super::telegram_new_service::TelegramNewService<'_>,
) -> AppResult<()> {
    let profile = super::platform_bot_config_service::get(
        &state.db,
        row.get_str("profile_id").map_err(|_| closed())?,
    )
    .await?;
    if profile.generation != row.get_i64("profile_generation").map_err(|_| closed())? {
        return Err(closed());
    }
    let binding = state.db.collection::<PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME)
        .find_one(doc! {"_id": row.get_str("binding_id").map_err(|_| closed())?, "generation": row.get_i64("binding_generation").map_err(|_| closed())?}).await?.ok_or_else(closed)?;
    let (manager, _, values) = service.manager().await?;
    if manager.to_string() != profile.manager_bot_id
        || values.get("observation_id") != Some(profile.observation_id.as_str())
    {
        return Err(closed());
    }
    let token = values
        .get(super::channel_adapters::telegram_new::MANAGER_TOKEN)
        .ok_or_else(closed)?;
    let db = state.db.clone();
    let row = row.clone();
    let captured = row.clone();
    let sender = binding.telegram_sender_id.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            super::platform_binding_service::fence_reply(&db, &profile, Some(&binding), session).await?;
            if db.collection::<Document>(NOTICES).update_one(doc! {"_id": row.get_str("_id").map_err(|_| closed())?, "claim_id": row.get_str("claim_id").map_err(|_| closed())?,
                "claim_generation": row.get_i64("claim_generation").map_err(|_| closed())?, "completed": false, "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
                doc! {"$inc": {"effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            if db.collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME).update_one(doc! {"_id": row.get_str("setup_id").map_err(|_| closed())?,
                "revision": row.get_i64("revision").map_err(|_| closed())?, "authority_generation": row.get_i64("authority_generation").map_err(|_| closed())?,
                "owner_user_id": &binding.user_id, "state": row.get_str("kind").map_err(|_| closed())?}, doc! {"$inc": {"notice_effect_revision": 1_i64}})
                .session(&mut *session).await?.matched_count != 1 { return Err(closed()); }
            Ok(())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)?;
    let text = match captured.get_str("kind").map_err(|_| closed())? {
        "awaiting_child_review" => {
            "Your created Telegram bot is ready for exact identity review in NyxID."
        }
        "ready_for_test" => {
            "Your bot is ready for an independent customer delivery test. Review the instructions in NyxID."
        }
        "active" => {
            "Your reviewed bot is active. You can pause customer replies from your setup in NyxID."
        }
        _ => return Err(closed()),
    };
    service.api.call(token, "sendMessage", serde_json::json!({"chat_id": sender, "text": text, "protect_content": true,
        "reply_markup": {"inline_keyboard": [[{"text": "Open your setup", "url": format!("{}/onboarding/setups/{}", state.config.frontend_url.trim_end_matches('/'), captured.get_str("setup_id").map_err(|_| closed())?)}]]}})).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn platform_onboarding_setup_notice_reclaim_and_unlink_fence_late_delivery() {
        let f = super::super::platform_onboarding_test_support::distinct_fixture().await;
        Box::pin(f.observe()).await;
        assert_eq!(
            f.state
                .db
                .collection::<Document>(NOTICES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let old = claim_notice(&f.state.db).await.unwrap().unwrap();
        f.state
            .db
            .collection::<Document>(NOTICES)
            .update_one(
                doc! {"_id": old.get_str("_id").unwrap()},
                vec![doc! {"$set": {"retry_at": {"$subtract": ["$$NOW", 1_i64]}}}],
            )
            .await
            .unwrap();
        let current = claim_notice(&f.state.db).await.unwrap().unwrap();
        assert!(
            current.get_i64("claim_generation").unwrap() > old.get_i64("claim_generation").unwrap()
        );
        let base = f.server.uri();
        let service = super::super::telegram_new_tests::service(&f.state, &base);
        let before = f.server.received_requests().await.unwrap().len();
        assert!(
            deliver_with_service(&f.state, &old, &service)
                .await
                .is_err()
        );
        super::super::platform_binding_service::revoke_owned(
            &f.state.db,
            &f.owner,
            &f.binding.id,
            f.binding.generation,
        )
        .await
        .unwrap();
        assert!(
            deliver_with_service(&f.state, &current, &service)
                .await
                .is_err()
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), before);
    }
}
