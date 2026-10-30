use super::*;
use crate::models::{
    audit_log::COLLECTION_NAME as AUDIT,
    channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
    channel_whatsapp_native::COLLECTION_NAME as NATIVE,
};
use axum::{body::to_bytes, extract::Query, http::StatusCode};
use mongodb::bson::{self, doc};
use serde_json::Value;
use sha2::{Digest, Sha256};

const OLD: &str = "original-relay-verify-token";

async fn setup() -> (AppState, ChannelBot) {
    let db = crate::test_utils::connect_transaction_test_database("wa_rollback_token").await;
    let bot: ChannelBot = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": uuid::Uuid::new_v4().to_string(),
        "platform": "whatsapp", "label": "test", "platform_bot_id": "333", "platform_bot_username": "+15551234567",
        "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1] },
        "app_secret_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![2] },
        "app_id":"222", "webhook_secret_hash":hex::encode(Sha256::digest(OLD.as_bytes())),
        "webhook_registered":true, "status":"active", "is_active":true,
        "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
    }).unwrap();
    db.collection::<ChannelBot>(BOTS)
        .insert_one(&bot)
        .await
        .unwrap();
    native::configure(&db, &bot, "111", "222", true, 0)
        .await
        .unwrap();
    db.collection::<bson::Document>(NATIVE)
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"callback.phase":"awaiting_relay"}},
        )
        .await
        .unwrap();
    (crate::test_utils::test_app_state(db), bot)
}

async fn issue(
    state: &AppState,
    bot: &ChannelBot,
    revision: i64,
) -> AppResult<axum::response::Response> {
    Box::pin(replace(
        State(state.clone()),
        crate::test_utils::test_auth_user(&bot.user_id),
        Path(bot.id.clone()),
        Json(ReplaceRequest { revision }),
    ))
    .await
}

async fn body(response: axum::response::Response) -> Value {
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap()
}

async fn verifies(state: &AppState, bot: &ChannelBot, token: &str) -> StatusCode {
    let response = crate::handlers::channel_webhooks::channel_subscription(
        State(state.clone()),
        Path(("whatsapp".into(), bot.id.clone())),
        Query(std::collections::HashMap::from([
            ("hub.mode".into(), "subscribe".into()),
            ("hub.verify_token".into(), token.into()),
            ("hub.challenge".into(), "000123".into()),
        ])),
    )
    .await;
    response.status()
}

#[tokio::test]
async fn whatsapp_rollback_issues_once_invalidates_old_token_and_audits_metadata() {
    let (state, bot) = setup().await;
    assert_eq!(verifies(&state, &bot, OLD).await, StatusCode::OK);
    // No flag override: rollback recovery remains usable with gateway disabled.
    let result = body(issue(&state, &bot, 1).await.unwrap()).await;
    let token = result["verify_token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(result["revision"], 2);
    assert_eq!(result["previous_token_invalidated_immediately"], true);
    assert_eq!(
        result["relay_webhook_url"],
        crate::services::channel_bot_service::webhook_url(&state.config.base_url, &bot)
    );
    assert_eq!(verifies(&state, &bot, OLD).await, StatusCode::FORBIDDEN);
    assert_eq!(verifies(&state, &bot, token).await, StatusCode::OK);
    let stored = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id":&bot.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.webhook_secret_hash,
        hex::encode(Sha256::digest(token.as_bytes()))
    );
    assert!(stored.webhook_secret_encrypted.is_none());
    let config = native::load(&state.db, &bot).await.unwrap().unwrap();
    assert_eq!(config.callback.phase, CallbackPhase::AwaitingRelay);
    assert!(
        !bson::to_document(&config)
            .unwrap()
            .to_string()
            .contains(token)
    );
    let settings = super::super::get(
        State(state.clone()),
        crate::test_utils::test_auth_user(&bot.user_id),
        Path(bot.id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert!(!serde_json::to_string(&settings).unwrap().contains(token));
    let audit = state
        .db
        .collection::<bson::Document>(AUDIT)
        .find_one(doc! {
            "event_type":"channel_whatsapp_relay_verify_token_replaced"
        })
        .await
        .unwrap()
        .unwrap();
    let metadata = audit.get_document("event_data").unwrap();
    assert!(
        metadata
            .get_bool("previous_token_invalidated_immediately")
            .unwrap()
    );
    assert_eq!(
        metadata.get_str("purpose").unwrap(),
        "native_to_relay_rollback"
    );
    assert!(!audit.to_string().contains(token));
    assert!(!audit.to_string().contains(&stored.webhook_secret_hash));
    assert!(issue(&state, &bot, 1).await.is_err()); // a retried request cannot rotate again
    let second = body(issue(&state, &bot, 2).await.unwrap()).await;
    let replacement = second["verify_token"].as_str().unwrap();
    assert_ne!(replacement, token);
    assert_eq!(verifies(&state, &bot, token).await, StatusCode::FORBIDDEN);
    assert_eq!(verifies(&state, &bot, replacement).await, StatusCode::OK);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn whatsapp_rollback_refuses_non_owner_and_non_first_party_credentials() {
    let (state, bot) = setup().await;
    let stranger = uuid::Uuid::new_v4().to_string();
    let err = Box::pin(replace(
        State(state.clone()),
        crate::test_utils::test_auth_user(&stranger),
        Path(bot.id.clone()),
        Json(ReplaceRequest { revision: 1 }),
    ))
    .await
    .err()
    .unwrap();
    assert!(matches!(err, AppError::ChannelBotNotFound(_)));
    for method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::Delegated,
        crate::mw::auth::AuthMethod::ServiceAccount,
    ] {
        let mut auth = crate::test_utils::test_auth_user(&bot.user_id);
        auth.auth_method = method;
        assert!(
            Box::pin(replace(
                State(state.clone()),
                auth,
                Path(bot.id.clone()),
                Json(ReplaceRequest { revision: 1 })
            ))
            .await
            .is_err()
        );
    }
    let mut auth = crate::test_utils::test_auth_user(&bot.user_id);
    auth.oauth_client_id = Some("third-party".into());
    assert!(
        Box::pin(replace(
            State(state.clone()),
            auth,
            Path(bot.id.clone()),
            Json(ReplaceRequest { revision: 1 })
        ))
        .await
        .is_err()
    );
    assert_eq!(verifies(&state, &bot, OLD).await, StatusCode::OK);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(AUDIT)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn whatsapp_rollback_requires_pending_restore_and_org_write_access() {
    use crate::models::{
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
        user::{COLLECTION_NAME as USERS, UserType},
    };
    let (state, bot) = setup().await;
    for phase in ["relay", "awaiting_gateway", "gateway", "future_phase"] {
        state
            .db
            .collection::<bson::Document>(NATIVE)
            .update_one(doc! {"_id":&bot.id}, doc! {"$set":{"callback.phase":phase}})
            .await
            .unwrap();
        assert!(issue(&state, &bot, 1).await.is_err());
    }
    state
        .db
        .collection::<bson::Document>(NATIVE)
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"callback.phase":"awaiting_relay"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection(USERS)
        .insert_one(crate::test_utils::test_user(&bot.user_id, UserType::Org))
        .await
        .unwrap();
    let actor = uuid::Uuid::new_v4().to_string();
    let membership =
        crate::test_utils::test_membership(&bot.user_id, &actor, OrgRole::Viewer, None);
    state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    let err = Box::pin(replace(
        State(state.clone()),
        crate::test_utils::test_auth_user(&actor),
        Path(bot.id.clone()),
        Json(ReplaceRequest { revision: 1 }),
    ))
    .await
    .err()
    .unwrap();
    assert!(matches!(err, AppError::OrgRoleInsufficient(_)));
    state
        .db
        .collection::<bson::Document>(MEMBERSHIPS)
        .update_one(doc! {"_id":&membership.id}, doc! {"$set":{"role":"admin"}})
        .await
        .unwrap();
    assert!(
        Box::pin(replace(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor),
            Path(bot.id.clone()),
            Json(ReplaceRequest { revision: 1 })
        ))
        .await
        .is_ok()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn whatsapp_rollback_concurrent_issuance_accepts_one_revision() {
    let (state, bot) = setup().await;
    let (a, b) = tokio::join!(
        Box::pin(issue(&state, &bot, 1)),
        Box::pin(issue(&state, &bot, 1))
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let result = body(a.or(b).unwrap()).await;
    assert_eq!(
        verifies(&state, &bot, result["verify_token"].as_str().unwrap()).await,
        StatusCode::OK
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(AUDIT)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    state.db.drop().await.unwrap();
}
