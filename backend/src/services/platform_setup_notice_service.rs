//! Durable metadata-only setup notices. Browser status remains authoritative.
//! Delivery remains at-least-once: accepted sends can repeat after a crash before
//! completion. Bound dispatch to 30s inside a freshly checked 60s claim; do not
//! suppress retries of an unknown provider outcome.
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

const NOTICE_CLAIM_LIFETIME: std::time::Duration = std::time::Duration::from_secs(60);
const NOTICE_DISPATCH_BOUND: std::time::Duration =
    std::time::Duration::from_secs(NOTICE_CLAIM_LIFETIME.as_secs() / 2);

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
                "attempts": {"$add": ["$attempts", 1_i64]}, "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": NOTICE_CLAIM_LIFETIME.as_secs() as i64}}}}])
            .return_document(mongodb::options::ReturnDocument::After).await?)
}

pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    for _ in 0..16 {
        let Some(row) = claim_notice(&state.db).await? else {
            break;
        };
        let result = deliver(state, &row).await;
        complete_notice(&state.db, &row, &result).await?;
        if let Err(error) = result {
            tracing::warn!(
                error_code = error.error_code(),
                "Platform setup notice not delivered"
            );
        }
    }
    Ok(())
}

async fn complete_notice(db: &Database, row: &Document, result: &AppResult<()>) -> AppResult<()> {
    let terminal = result.is_ok()
        || matches!(
            result,
            Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_))
        );
    db.collection::<Document>(NOTICES).update_one(doc! {"_id": row.get_str("_id").map_err(|_| closed())?,
        "claim_id": row.get_str("claim_id").map_err(|_| closed())?,
        "claim_generation": row.get_i64("claim_generation").map_err(|_| closed())?, "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
        doc! {"$set": {"completed": terminal}}).await?;
    Ok(())
}

async fn deliver(state: &AppState, row: &Document) -> AppResult<()> {
    let service = super::telegram_new_service::TelegramNewService {
        manager_id: None,
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
    let service = service
        .for_manager(profile.manager_bot_id.parse().map_err(|_| closed())?)
        .await?;
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
    #[cfg(test)]
    tests::take_over_before_dispatch(&state.db, &captured).await;
    let deadline = tokio::time::Instant::now() + NOTICE_DISPATCH_BOUND;
    tokio::time::timeout_at(deadline, async {
        let renewed = state.db.collection::<Document>(NOTICES).update_one(
            doc! {"_id": captured.get_str("_id").map_err(|_| closed())?,
                "claim_id": captured.get_str("claim_id").map_err(|_| closed())?,
                "claim_generation": captured.get_i64("claim_generation").map_err(|_| closed())?,
                "completed": false, "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
            vec![doc! {"$set": {"retry_at": {"$add": ["$$NOW", NOTICE_CLAIM_LIFETIME.as_millis() as i64]}}}],
        ).await?;
        if renewed.matched_count != 1 || tokio::time::Instant::now() >= deadline {
            return Err(closed());
        }
    service.api.call(token, "sendMessage", serde_json::json!({"chat_id": sender, "text": text, "protect_content": true,
        "reply_markup": {"inline_keyboard": [[{"text": "Open your setup", "url": format!("{}/onboarding/setups/{}", state.config.frontend_url.trim_end_matches('/'), captured.get_str("setup_id").map_err(|_| closed())?)}]]}})).await?;
        Ok(())
    }).await.map_err(|_| AppError::ChannelPlatformError("Setup notice delivery timed out".into()))??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    tokio::task_local! {
        static TAKEOVER: std::sync::Arc<std::sync::Mutex<Option<Document>>>;
    }
    pub(super) async fn take_over_before_dispatch(db: &Database, row: &Document) {
        let Ok(winner) = TAKEOVER.try_with(Clone::clone) else {
            return;
        };
        db.collection::<Document>(NOTICES)
            .update_one(
                doc! {"_id":row.get_str("_id").unwrap()},
                vec![doc! {"$set":{"retry_at":{"$subtract":["$$NOW",1_i64]}}}],
            )
            .await
            .unwrap();
        let successor = claim_notice(db).await.unwrap();
        *winner.lock().unwrap() = successor;
    }

    #[tokio::test]
    async fn round2_setup_lost_claim_never_sends_or_changes_successor() {
        Box::pin(async {
            let f = super::super::platform_onboarding_test_support::distinct_fixture().await;
            Box::pin(f.observe()).await;
            let before_dispatch = f.server.received_requests().await.unwrap().len();
            let old = claim_notice(&f.state.db).await.unwrap().unwrap();
            let winner = std::sync::Arc::new(std::sync::Mutex::new(None));
            let base = f.server.uri();
            let service = super::super::telegram_new_tests::service(&f.state, &base);
            let result = TAKEOVER
                .scope(
                    winner.clone(),
                    deliver_with_service(&f.state, &old, &service),
                )
                .await;
            assert!(matches!(result, Err(AppError::Conflict(_))));
            let successor = winner.lock().unwrap().clone().unwrap();
            complete_notice(&f.state.db, &old, &result).await.unwrap();
            let after = f
                .state
                .db
                .collection::<Document>(NOTICES)
                .find_one(doc! {"_id":old.get_str("_id").unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                bson::to_vec(&after).unwrap(),
                bson::to_vec(&successor).unwrap(),
                "the loser must leave the successor byte-identical, including attempts and backoff"
            );
            assert_eq!(
                f.server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .skip(before_dispatch)
                    .filter(|request| request.url.path().ends_with("/sendMessage"))
                    .count(),
                0
            );
            f.state.db.drop().await.unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn setup_notice_send_outlives_original_claim_without_a_second_sender() {
        Box::pin(async {
            use std::sync::Arc;
            use std::time::Duration;
            use wiremock::{Mock, ResponseTemplate, matchers::path_regex};
            let f = super::super::platform_onboarding_test_support::distinct_fixture().await;
            Box::pin(f.observe()).await;
            let claimed = claim_notice(&f.state.db).await.unwrap().unwrap();
            // Preparation has used all but one second of the original lease.
            f.state
                .db
                .collection::<Document>(NOTICES)
                .update_one(
                    doc! {"_id":claimed.get_str("_id").unwrap()},
                    vec![doc! {"$set":{"retry_at":{"$add":["$$NOW",1000_i64]}}}],
                )
                .await
                .unwrap();
            let started = Arc::new(tokio::sync::Notify::new());
            let seen = started.clone();
            let sends = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counted = sends.clone();
            Mock::given(path_regex("/sendMessage$"))
                .respond_with(move |_: &wiremock::Request| {
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    seen.notify_one();
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok":true,"result":{"message_id":99}}))
                        .set_delay(Duration::from_millis(1800))
                })
                .with_priority(1)
                .mount(&f.server)
                .await;
            let base = f.server.uri();
            let service = super::super::telegram_new_tests::service(&f.state, &base);
            let (first, second) =
                tokio::join!(deliver_with_service(&f.state, &claimed, &service), async {
                    started.notified().await;
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                    if let Some(other) = claim_notice(&f.state.db).await.unwrap() {
                        deliver_with_service(&f.state, &other, &service)
                            .await
                            .unwrap();
                        true
                    } else {
                        false
                    }
                });
            first.unwrap();
            assert_eq!(
                sends.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "a live setup sender must retain its claim past the old expiry"
            );
            assert!(!second);
            f.state.db.drop().await.unwrap();
        })
        .await;
    }

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
