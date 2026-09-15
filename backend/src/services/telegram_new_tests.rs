use axum::http::HeaderMap;
use bson::doc;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use zeroize::Zeroizing;

use super::{
    platform_credential_service as credentials,
    telegram_new_api::TelegramApi,
    telegram_new_service::{self, TelegramNewService},
};
use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
use crate::models::platform_credential::{COLLECTION_NAME as CREDENTIALS, PlatformCredential};
use crate::models::telegram_bot_request::{
    COLLECTION_NAME as REQUESTS, TelegramBotRequest, TelegramRequestStatus as Status,
};
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};

const MANAGER: &str = "100:manager-test-secret";
const CHILD: &str = "900:child-test-secret";

async fn fixture() -> (crate::AppState, String, MockServer) {
    let db = connect_transaction_test_database("telegram_new").await;
    telegram_new_service::ensure_indexes(&db).await.unwrap();
    super::coordination_service::ensure_indexes(&db)
        .await
        .unwrap();
    let mut state = test_app_state(db);
    state.config.base_url = "https://api.nyxid.test".into();
    state.config.frontend_url = "https://app.nyxid.test".into();
    let actor = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&actor, crate::models::user::UserType::Person))
        .await
        .unwrap();
    credentials::update(
        &state.db,
        &state.encryption_keys,
        &super::channel_adapters::telegram_new::credential_descriptor(),
        &actor,
        &[(
            "manager_bot_token".into(),
            Some(Zeroizing::new(MANAGER.into())),
        )]
        .into(),
        false,
    )
    .await
    .unwrap();
    state.db.collection::<PlatformCredential>(CREDENTIALS).update_one(doc! {"provider": "telegram-new"}, doc! {"$set": {"fields.manager_bot_id": "100", "fields.manager_username": "NyxSetupBot", "fields.webhook_ready": "true", "fields.observation_id": "test-observation", "fields.observation_started_at": (Utc::now() - Duration::hours(1)).timestamp().to_string()}}).await.unwrap();
    let server = MockServer::start().await;
    for endpoint in ["sendMessage", "answerCallbackQuery", "deleteWebhook"] {
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/{endpoint}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok": true, "result": {"message_id": 50}})),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("POST")).and(path(format!("/bot{MANAGER}/getWebhookInfo"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": {"url": "https://api.nyxid.test/api/v1/webhooks/channel/telegram-new/manager", "pending_update_count": 0}}))).with_priority(10).mount(&server).await;
    (state, actor, server)
}

fn service<'a>(state: &'a crate::AppState, base: &'a str) -> TelegramNewService<'a> {
    TelegramNewService {
        db: &state.db,
        keys: &state.encryption_keys,
        config: &state.config,
        api: TelegramApi {
            http: &state.http_client,
            base_url: base,
        },
    }
}

async fn headers(state: &crate::AppState) -> HeaderMap {
    let values = credentials::load_decrypted(
        &state.db,
        &state.encryption_keys,
        &super::channel_adapters::telegram_new::credential_descriptor(),
    )
    .await
    .unwrap();
    HeaderMap::from_iter([(
        "x-telegram-bot-api-secret-token".parse().unwrap(),
        values
            .get(credentials::VERIFY_TOKEN_FIELD)
            .unwrap()
            .parse()
            .unwrap(),
    )])
}

fn message(body: Value) -> Value {
    let mut value = json!({"from": {"id": 700, "is_bot": false}, "chat": {"id": 700, "type": "private"}, "message_id": 10, "date": Utc::now().timestamp()});
    for (key, val) in body.as_object().unwrap() {
        value[key] = val.clone();
    }
    json!({"update_id": 1, "message": value})
}

fn bot() -> Value {
    json!({"id": 900, "is_bot": true, "username": "CustomerBot"})
}

async fn webhook(service: &TelegramNewService<'_>, headers: &HeaderMap, update: Value) {
    service
        .webhook(headers, &serde_json::to_vec(&update).unwrap())
        .await
        .unwrap();
}

async fn waiting_consent(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
) -> TelegramBotRequest {
    let base = server.uri();
    let service = service(state, &base);
    let headers = headers(state).await;
    let (request, link) = service.begin(actor, actor, "Support", false).await.unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    webhook(
        &service,
        &headers,
        message(json!({"text": format!("/start {challenge}")})),
    )
    .await;
    webhook(&service, &headers, json!({"update_id": 2, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}})).await;
    webhook(
        &service,
        &headers,
        message(json!({"managed_bot_created": {"bot": bot()}})),
    )
    .await;
    service.get(actor, &request.id).await.unwrap()
}

async fn consent_data(server: &MockServer) -> String {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .rev()
        .filter_map(|request| request.body_json::<Value>().ok())
        .find_map(|body| {
            body["reply_markup"]["inline_keyboard"][0][1]["callback_data"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap()
}

async fn approve(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
    id: &str,
    user: i64,
) -> TelegramBotRequest {
    let base = server.uri();
    let service = service(state, &base);
    webhook(&service, &headers(state).await, json!({"update_id": 4, "callback_query": {"id": "callback-test", "from": {"id": user, "is_bot": false}, "message": {"message_id": 50, "chat": {"id": user, "type": "private"}}, "data": consent_data(server).await}})).await;
    service.get(actor, id).await.unwrap()
}

async fn provider_connection(server: &MockServer, request_id: &str, webhook_status: u16) {
    for (endpoint, result) in [("getManagedBotToken", json!(CHILD))] {
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/{endpoint}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": result})),
            )
            .mount(server)
            .await;
    }
    let callback =
        format!("https://api.nyxid.test/api/v1/webhooks/channel/telegram-new/{request_id}");
    for (endpoint, result) in [
        ("getMe", bot()),
        ("getWebhookInfo", json!({"url": callback})),
        ("deleteWebhook", json!(true)),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/bot{CHILD}/{endpoint}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": result})),
            )
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path(format!("/bot{CHILD}/setWebhook")))
        .respond_with(
            ResponseTemplate::new(webhook_status)
                .set_body_json(json!({"ok": webhook_status == 200, "result": true})),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn telegram_new_requires_named_consent_and_never_exposes_tokens() {
    let (state, actor, server) = fixture().await;
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &actor},
            doc! {"$set": {"email": "calvin&team@example.test"}},
        )
        .await
        .unwrap();
    let pending = waiting_consent(&state, &actor, &server).await;
    let sent = server.received_requests().await.unwrap();
    let consent = sent
        .iter()
        .filter_map(|request| request.body_json::<Value>().ok())
        .find(|body| body["reply_markup"]["inline_keyboard"][0][1]["text"] == "Approve this bot")
        .unwrap();
    assert_eq!(consent["parse_mode"], "HTML");
    assert_eq!(consent["link_preview_options"]["is_disabled"], true);
    let text = consent["text"].as_str().unwrap();
    let (visible, references) = text.split_once("<blockquote expandable>").unwrap();
    assert!(visible.contains("@CustomerBot"));
    assert!(visible.contains("Personal account: calvin&amp;team@example.test"));
    assert!(visible.contains("https://app.nyxid.test"));
    assert!(visible.contains("receive messages sent to this bot and send replies"));
    assert!(visible.contains("<b>Connect bot</b>"));
    assert!(!visible.contains(&actor));
    assert!(references.contains(&actor));
    assert!(references.contains("Telegram bot ID: 900"));
    let creation = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|request| request.body_json::<Value>().ok())
        .find_map(|body| {
            body["reply_markup"]["keyboard"][0][0]
                .get("request_managed_bot")
                .cloned()
        })
        .unwrap();
    assert_eq!(creation["suggested_name"], "Support");
    assert_eq!(creation["suggested_username"], "support_bot");
    assert_eq!(pending.status, Status::WaitingConsent);
    let view = serde_json::to_value(crate::handlers::telegram_new::RequestResponse::from(
        pending.clone(),
    ))
    .unwrap();
    assert!(view["telegram_bot_id"].is_null());
    assert!(view["bot_username"].is_null());
    let wrong = approve(&state, &actor, &server, &pending.id, 999).await;
    assert_eq!(wrong.status, Status::WaitingConsent);
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    assert_eq!(ready.status, Status::Ready);
    let sent = server.received_requests().await.unwrap();
    let return_message = sent
        .iter()
        .filter_map(|request| request.body_json::<Value>().ok())
        .find(|body| body["reply_markup"]["inline_keyboard"][0][0]["text"] == "Return to NyxID")
        .unwrap();
    assert_eq!(
        return_message["reply_markup"]["inline_keyboard"][0][0]["url"],
        "https://app.nyxid.test/channel-bots?connect=telegram-new"
    );
    assert!(
        return_message["text"]
            .as_str()
            .unwrap()
            .contains("<b>Connect bot</b>")
    );
    provider_connection(&server, &ready.id, 200).await;
    let base = server.uri();
    let service = service(&state, &base);
    let result = service
        .connect(&actor, &ready.id, 900, ready.revision)
        .await
        .unwrap();
    assert_eq!(result.status, "active");
    assert_eq!(result.platform, "telegram-new");
    assert_eq!(result.credential_source, "platform");
    let provenance = state
        .db
        .collection::<crate::models::telegram_bot_request::ManagedBotEvents>(
            crate::models::telegram_bot_request::MANAGED_BOTS,
        )
        .find_one(doc! {"telegram_bot_id": 900_i64})
        .await
        .unwrap()
        .unwrap();
    assert!(
        provenance.retired,
        "provisioning permanently consumes the creation identity"
    );
    let repeated = service
        .connect(&actor, &ready.id, 900, ready.revision)
        .await
        .unwrap();
    assert_eq!(result.id, repeated.id);
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, &result)
            .await
            .unwrap(),
        CHILD
    );
    let response = serde_json::to_string(&crate::handlers::telegram_new::RequestResponse::from(
        service.get(&actor, &ready.id).await.unwrap(),
    ))
    .unwrap();
    for forbidden in [
        MANAGER,
        CHILD,
        "challenge_hash",
        "consent_hash",
        "encrypted",
    ] {
        assert!(!response.contains(forbidden));
    }
    let calls = server.received_requests().await.unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|request| request.url.path() == format!("/bot{CHILD}/setWebhook"))
            .count(),
        1
    );
    assert!(!format!("{result:?}").contains(CHILD));
}

#[tokio::test]
async fn telegram_new_failed_webhook_resumes_same_saved_bot_after_expiry() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    provider_connection(&server, &ready.id, 503).await;
    let base = server.uri();
    let service = service(&state, &base);
    let error = service
        .connect(&actor, &ready.id, 900, ready.revision)
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(CHILD));
    let saved = super::channel_bot_service::get_bot(&state.db, &ready.id)
        .await
        .unwrap();
    assert_eq!(saved.status, "pending");
    state.db.collection::<TelegramBotRequest>(REQUESTS).update_one(doc! {"_id": &ready.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::hours(1))}}).await.unwrap();
    assert_eq!(
        service.get(&actor, &ready.id).await.unwrap().status,
        Status::Provisioning
    );
    server.reset().await;
    provider_connection(&server, &ready.id, 200).await;
    let repaired = service
        .connect(&actor, &ready.id, 900, ready.revision)
        .await
        .unwrap();
    assert_eq!(repaired.id, saved.id);
    assert_eq!(repaired.bot_token_encrypted, saved.bot_token_encrypted);
    assert_eq!(
        repaired.webhook_secret_encrypted,
        saved.webhook_secret_encrypted
    );
    assert_eq!(repaired.status, "active");
}

#[tokio::test]
async fn telegram_new_expired_request_releases_window_and_orphan_can_be_recovered() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    state.db.collection::<TelegramBotRequest>(REQUESTS).update_one(doc! {"_id": &pending.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(1))}}).await.unwrap();
    let base = server.uri();
    let service = service(&state, &base);
    assert_eq!(
        service.get(&actor, &pending.id).await.unwrap().status,
        Status::Expired
    );
    let (new, link) = service
        .begin(&actor, &actor, "Recovered", false)
        .await
        .unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    webhook(
        &service,
        &headers(&state).await,
        message(json!({"text": format!("/start {challenge}")})),
    )
    .await;
    webhook(
        &service,
        &headers(&state).await,
        message(json!({"text": "/recover @CustomerBot"})),
    )
    .await;
    let recovered = service.get(&actor, &new.id).await.unwrap();
    assert_eq!(recovered.status, Status::WaitingConsent);
    assert_eq!(recovered.telegram_bot_id, Some(900));
    assert!(
        serde_json::to_value(crate::handlers::telegram_new::RequestResponse::from(
            recovered
        ))
        .unwrap()["bot_username"]
            .is_null()
    );
    let approved = approve(&state, &actor, &server, &new.id, 700).await;
    assert_eq!(approved.status, Status::Ready);
}

#[tokio::test]
async fn telegram_new_duplicate_initial_event_is_safe_but_lifecycle_changes_suspend() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    provider_connection(&server, &ready.id, 200).await;
    let base = server.uri();
    let service = service(&state, &base);
    service
        .connect(&actor, &ready.id, 900, ready.revision)
        .await
        .unwrap();
    let event = |update_id| json!({"update_id": update_id, "managed_bot": {"user": {"id": 700}, "bot": bot()}});
    webhook(&service, &headers(&state).await, event(2)).await;
    assert_eq!(
        super::channel_bot_service::get_bot(&state.db, &ready.id)
            .await
            .unwrap()
            .status,
        "active"
    );
    webhook(&service, &headers(&state).await, event(55)).await;
    let saved = super::channel_bot_service::get_bot(&state.db, &ready.id)
        .await
        .unwrap();
    assert!(saved.is_active);
    assert_eq!(saved.status, "suspended");
    assert!(
        service
            .connect(&actor, &ready.id, 900, ready.revision)
            .await
            .is_err()
    );
    webhook(&service, &headers(&state).await, event(55)).await;
    assert_eq!(
        super::channel_bot_service::get_bot(&state.db, &ready.id)
            .await
            .unwrap()
            .status,
        "suspended"
    );
}

#[tokio::test]
async fn telegram_new_rejects_bad_secret_and_other_actors_and_cancelled_consent() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    assert!(service.webhook(&HeaderMap::new(), b"{}").await.is_err());
    assert!(
        service
            .get(&uuid::Uuid::new_v4().to_string(), &pending.id)
            .await
            .is_err()
    );
    assert!(
        service
            .begin(
                &actor,
                &uuid::Uuid::new_v4().to_string(),
                "Wrong owner",
                false
            )
            .await
            .is_err()
    );
    service.cancel(&actor, &pending.id).await.unwrap();
    let request = approve(&state, &actor, &server, &pending.id, 700).await;
    assert_eq!(request.status, Status::Cancelled);
    assert!(
        service
            .connect(&actor, &pending.id, 900, pending.revision)
            .await
            .is_err()
    );
}

#[test]
fn telegram_new_private_creation_proof_rejects_forwarded_and_relayed_messages() {
    let valid = message(json!({}))["message"].clone();
    assert_eq!(telegram_new_service::private_sender(&valid), Some(700));
    for key in ["forward_origin", "forward_date", "sender_chat", "via_bot"] {
        let mut input = valid.clone();
        input[key] = json!({});
        assert_eq!(telegram_new_service::private_sender(&input), None);
    }
    let mut input = valid.clone();
    input["chat"]["id"] = json!(701);
    assert_eq!(telegram_new_service::private_sender(&input), None);
    input = valid;
    input["chat"]["type"] = json!("group");
    assert_eq!(telegram_new_service::private_sender(&input), None);
}

#[tokio::test]
async fn telegram_new_same_telegram_account_conflict_is_acknowledged() {
    let (state, actor, server) = fixture().await;
    let original = waiting_consent(&state, &actor, &server).await;
    let other = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&other, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let base = server.uri();
    let service = service(&state, &base);
    let (second, link) = service
        .begin(&other, &other, "Second account", false)
        .await
        .unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    webhook(
        &service,
        &headers(&state).await,
        message(json!({"text": format!("/start {challenge}")})),
    )
    .await;
    assert_eq!(
        service.get(&other, &second.id).await.unwrap().status,
        Status::WaitingTelegram
    );
    assert_eq!(
        service.get(&actor, &original.id).await.unwrap().status,
        Status::WaitingConsent
    );
    assert!(server.received_requests().await.unwrap().iter().any(|r| {
        r.body_json::<Value>().unwrap()["text"]
            .as_str()
            .is_some_and(|text| text.contains("already has an active"))
    }));
}

#[tokio::test]
async fn telegram_new_revoked_destination_releases_request_and_webhook_acks() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    let missing_owner = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .update_one(
            doc! {"_id": &pending.id},
            doc! {"$set": {"owner_user_id": missing_owner}},
        )
        .await
        .unwrap();
    // Telegram delivery must ACK this permanent access failure without approving.
    webhook(&service, &headers(&state).await, json!({"callback_query": {"id": "revoked", "from": {"id": 700, "is_bot": false}, "message": {"chat": {"id": 700, "type": "private"}}, "data": consent_data(&server).await}})).await;
    assert!(service.current(&actor).await.unwrap().is_none());
    let cancelled = state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .find_one(doc! {"_id": &pending.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert!(!cancelled.active);
    let (new, _) = service
        .begin(&actor, &actor, "New request", false)
        .await
        .unwrap();
    state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .update_one(
            doc! {"_id": &new.id},
            doc! {"$set": {"owner_user_id": uuid::Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    service.cancel(&actor, &new.id).await.unwrap();
    assert!(service.current(&actor).await.unwrap().is_none());
    let (new, _) = service
        .begin(&actor, &actor, "Read cleanup", false)
        .await
        .unwrap();
    state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .update_one(
            doc! {"_id": &new.id},
            doc! {"$set": {"owner_user_id": uuid::Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(service.current(&actor).await.unwrap().is_none());
    service
        .begin(&actor, &actor, "Recovered", false)
        .await
        .unwrap();
}

#[tokio::test]
async fn telegram_new_management_change_before_consent_cannot_be_recovered_by_old_creator() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    webhook(
        &service,
        &headers(&state).await,
        json!({"update_id": 42, "managed_bot": {"user": {"id": 700}, "bot": bot()}}),
    )
    .await;
    let result = approve(&state, &actor, &server, &pending.id, 700).await;
    assert_eq!(result.status, Status::Cancelled);
    assert!(
        service
            .connect(&actor, &pending.id, 900, result.revision)
            .await
            .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn telegram_new_manual_identity_and_quota_writers_are_serialized() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    provider_connection(&server, &ready.id, 200).await;
    let base = server.uri();
    let service = service(&state, &base);
    let registered = service
        .connect(&actor, &ready.id, 900, ready.revision)
        .await
        .unwrap();
    let mut manual = registered.clone();
    manual.id = uuid::Uuid::new_v4().to_string();
    manual.platform = "telegram".into();
    assert!(
        super::channel_bot_service::insert_registered_bot(&state.db, &manual, 20, None)
            .await
            .is_err()
    );
    manual.platform_bot_id = "100".into();
    assert!(
        super::channel_bot_service::insert_registered_bot(&state.db, &manual, 20, None)
            .await
            .is_err()
    );
    // Exercise the transaction fence (non-Telegram writers do not take the manager lease).
    let mut left = manual.clone();
    left.platform = "discord".into();
    left.platform_bot_id = "901".into();
    let mut right = left.clone();
    right.id = uuid::Uuid::new_v4().to_string();
    right.platform_bot_id = "902".into();
    let (a, b) = tokio::join!(
        super::channel_bot_service::insert_registered_bot(&state.db, &left, 2, None),
        super::channel_bot_service::insert_registered_bot(&state.db, &right, 2, None)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert!(matches!(
        a.err().or(b.err()).unwrap(),
        crate::errors::AppError::ChannelBotLimitReached(_)
    ));
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {"is_active": true})
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn telegram_new_clear_manager_survives_webhook_failures() {
    for endpoint in ["getWebhookInfo", "deleteWebhook"] {
        for status in [401, 404, 503] {
            let (state, _, server) = fixture().await;
            let base = server.uri();
            let service = service(&state, &base);
            Mock::given(method("POST"))
                .and(path(format!("/bot{MANAGER}/{endpoint}")))
                .respond_with(ResponseTemplate::new(status))
                .with_priority(1)
                .expect(1)
                .mount(&server)
                .await;

            service.clear_manager().await.unwrap();

            assert!(
                credentials::load(
                    &state.db,
                    &super::channel_adapters::telegram_new::credential_descriptor(),
                )
                .await
                .unwrap()
                .is_none(),
                "saved credentials must be deleted when {endpoint} returns {status}"
            );
            assert!(service.manager().await.is_err());
        }
    }
}

#[tokio::test]
async fn telegram_new_clear_manager_removes_only_its_own_webhook() {
    for owned in [true, false] {
        let (state, _, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let callback = if owned {
            service.manager_callback()
        } else {
            "https://another.example/webhook".into()
        };
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/getWebhookInfo")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok": true, "result": {"url": callback}})),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/deleteWebhook")))
            .and(wiremock::matchers::body_json(
                json!({"drop_pending_updates": false}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true})),
            )
            .with_priority(1)
            .expect(u64::from(owned))
            .mount(&server)
            .await;

        service.clear_manager().await.unwrap();
        service.clear_manager().await.unwrap();
        assert!(service.manager().await.is_err());
    }
}

#[tokio::test]
async fn telegram_new_clear_deleted_manager_allows_recreated_username() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let old_headers = headers(&state).await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/getWebhookInfo")))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(1)
        .mount(&server)
        .await;
    service.clear_manager().await.unwrap();

    let replacement = "200:replacement-manager-test-secret";
    Mock::given(method("POST"))
        .and(path(format!("/bot{replacement}/getMe")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": {"id": 200, "username": "NyxSetupBot", "is_bot": true, "can_manage_bots": true}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{replacement}/getWebhookInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": {"url": service.manager_callback(), "allowed_updates": ["message", "callback_query", "managed_bot"]}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{replacement}/setWebhook")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true})))
        .expect(1)
        .mount(&server)
        .await;
    service
        .configure_manager(
            &actor,
            &[(
                "manager_bot_token".into(),
                Some(Zeroizing::new(replacement.into())),
            )]
            .into(),
            false,
        )
        .await
        .unwrap();

    let (request, launch_url) = service
        .begin(&actor, &actor, "Replacement", false)
        .await
        .unwrap();
    assert_eq!(request.manager_bot_id, 200);
    assert_ne!(request.observation_id, "test-observation");
    assert!(launch_url.starts_with("https://t.me/NyxSetupBot?start="));
    assert_ne!(old_headers, headers(&state).await);
    assert!(service.webhook(&old_headers, br#"{}"#).await.is_err());
}

#[tokio::test]
async fn telegram_new_clear_manager_rejects_saved_managed_bots() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    provider_connection(&server, &ready.id, 200).await;
    let base = server.uri();
    let service = service(&state, &base);
    service
        .connect(&actor, &ready.id, 900, ready.revision)
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .count_documents(doc! {"active": true})
            .await
            .unwrap(),
        0
    );
    server.reset().await;

    assert!(matches!(
        service.clear_manager().await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    assert!(service.manager().await.is_ok());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn telegram_new_manager_configuration_preserves_secret_and_validates_webhook() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let secret_before = headers(&state).await;
    Mock::given(method("POST")).and(path(format!("/bot{MANAGER}/getMe")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": {"id": 100, "username": "NyxSetupBot", "is_bot": true, "can_manage_bots": true}}))).mount(&server).await;
    Mock::given(method("POST")).and(path(format!("/bot{MANAGER}/getWebhookInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": {"url": service.manager_callback(), "allowed_updates": ["message", "callback_query", "managed_bot"]}}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/setWebhook")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true})))
        .mount(&server)
        .await;
    service
        .configure_manager(&actor, &Default::default(), false)
        .await
        .unwrap();
    assert_eq!(secret_before, headers(&state).await);
    let calls = server.received_requests().await.unwrap();
    let setup = calls
        .iter()
        .find(|r| r.url.path().ends_with("/setWebhook"))
        .unwrap()
        .body_json::<Value>()
        .unwrap();
    assert_eq!(setup["max_connections"], 1);
    assert_eq!(
        setup["allowed_updates"],
        json!(["message", "callback_query", "managed_bot"])
    );
    assert!(setup.get("drop_pending_updates").is_none());
    service
        .begin(&actor, &actor, "Active request", false)
        .await
        .unwrap();
    server.reset().await;
    assert!(service.clear_manager().await.is_err());
    assert!(service.manager().await.is_ok());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(
        service
            .configure_manager(&actor, &Default::default(), true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn telegram_new_webhook_cleanup_preserves_another_app_and_cleans_pending_setup() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    provider_connection(&server, &ready.id, 503).await;
    let base = server.uri();
    let service = service(&state, &base);
    assert!(
        service
            .connect(&actor, &ready.id, 900, ready.revision)
            .await
            .is_err()
    );
    let saved = super::channel_bot_service::get_bot(&state.db, &ready.id)
        .await
        .unwrap();
    assert!(!saved.webhook_registered);
    // A timeout can leave the provider webhook live even with a pending local row.
    assert_eq!(
        service.remove_owned_webhook(&saved).await.unwrap(),
        "removed"
    );
    server.reset().await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{CHILD}/getWebhookInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"ok": true, "result": {"url": "https://another.example/webhook"}}),
        ))
        .mount(&server)
        .await;
    assert_eq!(
        service.remove_owned_webhook(&saved).await.unwrap(),
        "retained_external"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn telegram_new_pending_or_failed_manager_delivery_blocks_token_acquisition() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    let base = server.uri();
    let service = service(&state, &base);
    for health in [
        json!({"pending_update_count": 1}),
        json!({"pending_update_count": 0, "last_error_date": Utc::now().timestamp()}),
        json!({"pending_update_count": 0, "url": "https://changed.example/hook"}),
    ] {
        server.reset().await;
        let mut info = json!({"url": service.manager_callback()});
        for (key, value) in health.as_object().unwrap() {
            info[key] = value.clone();
        }
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/getWebhookInfo")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": info})),
            )
            .mount(&server)
            .await;
        assert!(
            service
                .connect(&actor, &ready.id, 900, ready.revision)
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            state
                .db
                .collection::<ChannelBot>(BOTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn telegram_new_provenance_is_immutable_and_recovery_expires() {
    use crate::models::telegram_bot_request::{MANAGED_BOTS, ManagedBotEvents};
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let events = state.db.collection::<ManagedBotEvents>(MANAGED_BOTS);
    let original = events
        .find_one(doc! {"telegram_bot_id": 900_i64})
        .await
        .unwrap()
        .unwrap();
    let base = server.uri();
    let service = service(&state, &base);
    // A late creation service message cannot rewrite the original owner/time.
    webhook(&service, &headers(&state).await, message(json!({"from": {"id": 999, "is_bot": false}, "chat": {"id": 999, "type": "private"}, "managed_bot_created": {"bot": bot()}}))).await;
    let unchanged = events
        .find_one(doc! {"telegram_bot_id": 900_i64})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.created_at, original.created_at);
    assert_eq!(unchanged.created_by, Some(700));
    events.update_one(doc! {"telegram_bot_id": 900_i64}, doc! {"$set": {"created_at": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(61))}}).await.unwrap();
    let cancelled = approve(&state, &actor, &server, &pending.id, 700).await;
    assert_eq!(cancelled.status, Status::Cancelled);
}

#[tokio::test]
async fn telegram_new_rejects_pre_observation_creation_without_stamping_provenance() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let (_, link) = service
        .begin(&actor, &actor, "Support", false)
        .await
        .unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    webhook(
        &service,
        &headers(&state).await,
        message(json!({"text": format!("/start {challenge}")})),
    )
    .await;
    webhook(&service, &headers(&state).await, message(json!({"date": (Utc::now() - Duration::hours(2)).timestamp(), "managed_bot_created": {"bot": bot()}}))).await;
    assert_eq!(
        service.current(&actor).await.unwrap().unwrap().status,
        Status::WaitingBot
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::telegram_bot_request::MANAGED_BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

async fn automatic_request(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
    management_first: bool,
) -> TelegramBotRequest {
    let base = server.uri();
    let service = service(state, &base);
    let headers = headers(state).await;
    let (request, link) = service.begin(actor, actor, "Support", true).await.unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    webhook(
        &service,
        &headers,
        message(json!({"text": format!("/start {challenge}")})),
    )
    .await;
    if management_first {
        webhook(&service, &headers, json!({"update_id": 2, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}})).await;
    }
    let mut created = message(json!({"managed_bot_created": {"bot": bot()}}));
    created["update_id"] = json!(3);
    webhook(&service, &headers, created).await;
    service.get(actor, &request.id).await.unwrap()
}

#[tokio::test]
async fn telegram_new_creation_webhook_completes_without_consent_or_browser_connect() {
    let (state, actor, server) = fixture().await;
    let pending = automatic_request(&state, &actor, &server, true).await;
    assert_eq!(pending.status, Status::Ready);
    assert!(pending.consent_hash.is_none());
    assert_eq!(pending.start_update_id, Some(1));
    provider_connection(&server, &pending.id, 200).await;
    let base = server.uri();
    let service = service(&state, &base);
    let (first, second) = tokio::join!(
        service.complete_pending_creations(),
        service.complete_pending_creations()
    );
    first.unwrap();
    second.unwrap();
    let connected = service.get(&actor, &pending.id).await.unwrap();
    assert_eq!(connected.status, Status::Connected);
    let saved = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id": &pending.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.user_id, actor);
    assert_eq!(saved.status, "active");
    assert!(saved.webhook_registered);
    assert!(
        !saved
            .bot_token_encrypted
            .windows(CHILD.len())
            .any(|bytes| bytes == CHILD.as_bytes())
    );
    let mut repeated = message(json!({"managed_bot_created": {"bot": bot()}}));
    repeated["update_id"] = json!(3);
    webhook(&service, &headers(&state).await, repeated).await;
    service.complete_pending_creations().await.unwrap();
    let sent = server.received_requests().await.unwrap();
    assert_eq!(
        sent.iter()
            .filter(|request| request.url.path().ends_with("getManagedBotToken"))
            .count(),
        1
    );
    assert_eq!(
        sent.iter()
            .filter(|request| request.url.path().ends_with("setWebhook"))
            .count(),
        1
    );
    assert!(!sent.iter().any(|request| {
        request
            .body_json::<Value>()
            .ok()
            .is_some_and(|body| body.to_string().contains("Approve this bot"))
    }));
    let completion = sent
        .iter()
        .filter_map(|request| request.body_json::<Value>().ok())
        .find(|body| {
            body["text"]
                .as_str()
                .is_some_and(|text| text.contains("Setup is complete"))
        })
        .expect("automatic connection should direct the creator to the new bot");
    assert_eq!(completion["chat_id"], 700);
    assert_eq!(
        completion["reply_markup"]["inline_keyboard"][0][0],
        json!({
            "text": "Open your bot", "url": "https://t.me/CustomerBot",
        })
    );
    assert_eq!(
        completion["reply_markup"]["inline_keyboard"][1][0],
        json!({
            "text": "Bot settings", "url": format!("https://app.nyxid.test/channel-bots/{}", pending.id),
        })
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_automatic_connection_waits_for_management_delivery() {
    let (state, actor, server) = fixture().await;
    let pending = automatic_request(&state, &actor, &server, false).await;
    provider_connection(&server, &pending.id, 200).await;
    let base = server.uri();
    let service = service(&state, &base);
    service.complete_pending_creations().await.unwrap();
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| request.url.path().ends_with("getManagedBotToken"))
    );
    webhook(&service, &headers(&state).await, json!({"update_id": 4, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}})).await;
    state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .update_one(
            doc! {"_id": &pending.id},
            doc! {"$unset": {"next_connection_attempt_at": ""}},
        )
        .await
        .unwrap();
    service.complete_pending_creations().await.unwrap();
    assert_eq!(
        service.get(&actor, &pending.id).await.unwrap().status,
        Status::Connected
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_automatic_retry_survives_browser_absence_and_request_expiry() {
    let (state, actor, server) = fixture().await;
    let pending = automatic_request(&state, &actor, &server, true).await;
    provider_connection(&server, &pending.id, 500).await;
    let base = server.uri();
    service(&state, &base)
        .complete_pending_creations()
        .await
        .unwrap();
    let saved = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id": &pending.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        service(&state, &base)
            .get(&actor, &pending.id)
            .await
            .unwrap()
            .status,
        Status::Provisioning
    );
    Mock::given(method("POST"))
        .and(path(format!("/bot{CHILD}/setWebhook")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true})))
        .with_priority(1)
        .mount(&server)
        .await;
    state.db.collection::<TelegramBotRequest>(REQUESTS).update_one(doc! {"_id": &pending.id}, doc! {
        "$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(1))},
        "$unset": {"next_connection_attempt_at": ""},
    }).await.unwrap();
    service(&state, &base)
        .complete_pending_creations()
        .await
        .unwrap();
    let result = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id": &pending.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, "active");
    assert_eq!(result.bot_token_encrypted, saved.bot_token_encrypted);
    assert_eq!(result.webhook_secret_hash, saved.webhook_secret_hash);
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        service(&state, &base)
            .get(&actor, &pending.id)
            .await
            .unwrap()
            .connection_error
            .is_none()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_automatic_connection_rechecks_destination_access() {
    let (state, actor, server) = fixture().await;
    let pending = automatic_request(&state, &actor, &server, true).await;
    provider_connection(&server, &pending.id, 200).await;
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": &actor}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    let base = server.uri();
    service(&state, &base)
        .complete_pending_creations()
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| request.url.path().ends_with("getManagedBotToken"))
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_automatic_creation_rejects_unbound_senders_and_earlier_updates() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let (pending, link) = service
        .begin(&actor, &actor, "Support", true)
        .await
        .unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    let mut start = message(json!({"text": format!("/start {challenge}")}));
    start["update_id"] = json!(10);
    webhook(&service, &headers(&state).await, start).await;
    let mut earlier = message(json!({"managed_bot_created": {"bot": bot()}}));
    earlier["update_id"] = json!(9);
    webhook(&service, &headers(&state).await, earlier).await;
    assert_eq!(
        service.get(&actor, &pending.id).await.unwrap().status,
        Status::WaitingBot
    );
    let mut old_date = message(json!({"managed_bot_created": {"bot": bot()}}));
    old_date["update_id"] = json!(11);
    old_date["message"]["date"] = json!(pending.created_at.timestamp() - 10);
    webhook(&service, &headers(&state).await, old_date).await;
    let mut recovery = message(json!({"text": "/recover @CustomerBot"}));
    recovery["update_id"] = json!(12);
    webhook(&service, &headers(&state).await, recovery).await;
    assert_eq!(
        service.get(&actor, &pending.id).await.unwrap().status,
        Status::WaitingBot
    );
    let mut stranger = message(json!({"managed_bot_created": {"bot": bot()}}));
    stranger["update_id"] = json!(11);
    stranger["message"]["from"]["id"] = json!(701);
    stranger["message"]["chat"]["id"] = json!(701);
    webhook(&service, &headers(&state).await, stranger).await;
    assert_eq!(
        service.get(&actor, &pending.id).await.unwrap().status,
        Status::WaitingBot
    );
    assert!(
        service
            .get(&uuid::Uuid::new_v4().to_string(), &pending.id)
            .await
            .is_err()
    );
    service.cancel(&actor, &pending.id).await.unwrap();
    let mut late = message(json!({"managed_bot_created": {"bot": bot()}}));
    late["update_id"] = json!(12);
    webhook(&service, &headers(&state).await, late).await;
    service.complete_pending_creations().await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_worker_never_upgrades_legacy_authorization() {
    let (state, actor, server) = fixture().await;
    let pending = waiting_consent(&state, &actor, &server).await;
    let ready = approve(&state, &actor, &server, &pending.id, 700).await;
    state.db.collection::<TelegramBotRequest>(REQUESTS).update_one(
        doc! {"_id": &ready.id},
        doc! {"$unset": {"auto_connect": "", "start_update_id": "", "connection_attempts": "", "next_connection_attempt_at": "", "connection_error": ""}},
    ).await.unwrap();
    let base = server.uri();
    let service = service(&state, &base);
    service.complete_pending_creations().await.unwrap();
    let saved = service.get(&actor, &ready.id).await.unwrap();
    assert_eq!(saved.status, Status::Ready);
    assert!(!saved.auto_connect);
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| request.url.path().ends_with("getManagedBotToken"))
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_worker_does_not_resume_suspended_provisioning() {
    let (state, actor, server) = fixture().await;
    let pending = automatic_request(&state, &actor, &server, true).await;
    provider_connection(&server, &pending.id, 500).await;
    let base = server.uri();
    let service = service(&state, &base);
    service.complete_pending_creations().await.unwrap();
    webhook(&service, &headers(&state).await, json!({"update_id": 20, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}})).await;
    service.complete_pending_creations().await.unwrap();
    assert_eq!(
        service.get(&actor, &pending.id).await.unwrap().status,
        Status::Suspended
    );
    let saved = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id": &pending.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, "suspended");
    assert!(!saved.webhook_registered);
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path().ends_with("setWebhook"))
            .count(),
        1
    );
    state.db.drop().await.unwrap();
}

mod claims {
    use super::*;
    use crate::errors::AppError;
    use crate::models::telegram_bot_claim::{
        COLLECTION_NAME as CLAIMS, TelegramBotClaim, TelegramClaimStatus,
    };
    use crate::models::telegram_bot_request::MANAGED_BOTS;

    async fn claim_code(server: &MockServer) -> String {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .rev()
            .filter_map(|request| request.body_json::<Value>().ok())
            .find_map(|body| {
                let url = reqwest::Url::parse(
                    body["reply_markup"]["inline_keyboard"][0][0]["url"].as_str()?,
                )
                .ok()?;
                url.query_pairs()
                    .find(|(key, _)| key == "claim")
                    .map(|(_, value)| value.into_owned())
            })
            .expect("private manager message must contain a claim link")
    }

    async fn mint(state: &crate::AppState, server: &MockServer, confirmed: bool) -> String {
        let base = server.uri();
        let service = service(state, &base);
        let headers = headers(state).await;
        if confirmed {
            webhook(&service, &headers, json!({"update_id": 2, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}})).await;
        }
        webhook(
            &service,
            &headers,
            message(json!({"managed_bot_created": {"bot": bot()}})),
        )
        .await;
        claim_code(server).await
    }

    async fn saved_claim(state: &crate::AppState) -> TelegramBotClaim {
        state
            .db
            .collection::<TelegramBotClaim>(CLAIMS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn telegram_new_claim_starts_in_private_telegram_and_mints_before_confirmation() {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        webhook(
            &service,
            &headers,
            message(json!({"chat": {"id": 700, "type": "group"}, "text": "/start"})),
        )
        .await;
        assert!(server.received_requests().await.unwrap().is_empty());
        webhook(&service, &headers, message(json!({"text": "/start"}))).await;
        assert!(
            server.received_requests().await.unwrap()[0]
                .body_json::<Value>()
                .unwrap()["reply_markup"]["keyboard"][0][0]["request_managed_bot"]
                .is_object()
        );
        let code = mint(&state, &server, false).await;
        let claim = saved_claim(&state).await;
        assert_eq!(claim.status, TelegramClaimStatus::Pending);
        assert!(!format!("{claim:?}").contains(&claim.code_hash));
        assert!(
            !bson::to_document(&claim)
                .unwrap()
                .to_string()
                .contains(&code)
        );
        let before = server.received_requests().await.unwrap().len();
        assert!(matches!(
            service.preview_claim(&actor, &code).await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            service.redeem_claim(&actor, &actor, &code, "Support").await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), before);
        assert_eq!(
            saved_claim(&state).await.status,
            TelegramClaimStatus::Pending
        );
        webhook(&service, &headers, json!({"update_id": 2, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}})).await;
        let preview = service
            .preview_claim(&actor, &code.to_lowercase().replace('-', " "))
            .await
            .unwrap();
        let response = crate::handlers::telegram_new::ClaimPreviewResponse {
            bot_username: preview.bot_username,
            expires_at: preview.expires_at.to_rfc3339(),
        };
        let response = serde_json::to_string(&response).unwrap();
        for secret in [&code, &claim.code_hash, MANAGER, CHILD] {
            assert!(!response.contains(secret));
        }
        assert_eq!(server.received_requests().await.unwrap().len(), before);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_bearer_selects_own_account_and_is_permanently_single_use() {
        let (state, other, server) = fixture().await;
        let actor = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&actor, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let code = mint(&state, &server, true).await;
        let base = server.uri();
        let service = service(&state, &base);
        for invalid in ["", "AAAAA-BBBBB-CCCCC-DDDDD", "900:child-test-secret"] {
            assert!(matches!(
                service
                    .redeem_claim(&actor, &actor, invalid, "Support")
                    .await,
                Err(AppError::NotFound(_))
            ));
        }
        let ready = service
            .redeem_claim(&actor, &actor, &code, "Support")
            .await
            .unwrap();
        assert_eq!(ready.status, Status::Ready);
        assert_eq!(ready.actor_user_id, actor);
        assert_eq!(ready.owner_user_id, actor);
        assert!(ready.auto_connect);
        assert_eq!(
            service
                .redeem_claim(&actor, &actor, &code, "Support")
                .await
                .unwrap()
                .id,
            ready.id
        );
        assert!(matches!(
            service
                .redeem_claim(&actor, &actor, &code, "Different")
                .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            service.redeem_claim(&other, &other, &code, "Support").await,
            Err(AppError::NotFound(_))
        ));
        assert!(
            !server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path().ends_with("getManagedBotToken"))
        );
        provider_connection(&server, &ready.id, 200).await;
        service.complete_pending_creations().await.unwrap();
        let connected = service.get(&actor, &ready.id).await.unwrap();
        assert_eq!(connected.status, Status::Connected);
        let response = serde_json::to_string(
            &crate::handlers::telegram_new::RequestResponse::from(connected),
        )
        .unwrap();
        for secret in [&code, MANAGER, CHILD] {
            assert!(!response.contains(secret));
        }
        assert!(matches!(
            service.redeem_claim(&actor, &actor, &code, "Support").await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            service.redeem_claim(&other, &other, &code, "Support").await,
            Err(AppError::NotFound(_))
        ));
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(BOTS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .get_str("user_id")
                .unwrap(),
            actor
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_existing_setup_conflicts_without_consuming_code() {
        let (state, actor, server) = fixture().await;
        let code = mint(&state, &server, true).await;
        let base = server.uri();
        let service = service(&state, &base);
        let (existing, _) = service
            .begin(&actor, &actor, "Earlier", true)
            .await
            .unwrap();
        for status in [
            "waiting_telegram",
            "waiting_bot",
            "waiting_consent",
            "ready",
            "provisioning",
        ] {
            state
                .db
                .collection::<TelegramBotRequest>(REQUESTS)
                .update_one(
                    doc! {"_id": &existing.id},
                    doc! {"$set": {"status": status}},
                )
                .await
                .unwrap();
            let err = service
                .redeem_claim(&actor, &actor, &code, "New")
                .await
                .unwrap_err();
            assert!(
                matches!(err, AppError::Conflict(ref message) if message == "Finish or cancel your existing Telegram creation request first")
            );
            let claim = saved_claim(&state).await;
            assert_eq!(claim.status, TelegramClaimStatus::Pending);
            assert!(claim.actor_user_id.is_none());
        }
        state
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .update_one(
                doc! {"_id": &existing.id},
                doc! {"$set": {"status": "waiting_telegram"}},
            )
            .await
            .unwrap();
        service.cancel(&actor, &existing.id).await.unwrap();
        assert!(
            service
                .redeem_claim(&actor, &actor, &code, "New")
                .await
                .is_ok()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_expiry_lifecycle_and_observation_changes_fail_closed() {
        let (state, actor, server) = fixture().await;
        let code = mint(&state, &server, true).await;
        let base = server.uri();
        let service = service(&state, &base);
        let claims = state.db.collection::<TelegramBotClaim>(CLAIMS);
        let future = saved_claim(&state).await.expires_at;
        claims.update_one(doc! {}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        assert!(matches!(
            service.redeem_claim(&actor, &actor, &code, "Support").await,
            Err(AppError::NotFound(_))
        ));
        claims
            .update_one(
                doc! {},
                doc! {"$set": {"expires_at": bson::DateTime::from_chrono(future)}},
            )
            .await
            .unwrap();
        for change in [
            doc! {"revision": 2_i64},
            doc! {"revision": 1_i64, "retired": true},
            doc! {"retired": false, "observation_id": "another"},
        ] {
            state
                .db
                .collection::<bson::Document>(MANAGED_BOTS)
                .update_one(doc! {}, doc! {"$set": change})
                .await
                .unwrap();
            assert!(matches!(
                service.preview_claim(&actor, &code).await,
                Err(AppError::NotFound(_))
            ));
            assert!(matches!(
                service.redeem_claim(&actor, &actor, &code, "Support").await,
                Err(AppError::NotFound(_))
            ));
        }
        assert_eq!(
            saved_claim(&state).await.status,
            TelegramClaimStatus::Pending
        );
        assert!(
            !server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path().ends_with("getManagedBotToken"))
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_concurrent_redeemers_have_one_winner() {
        let (state, actor, server) = fixture().await;
        let other = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&other, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let code = mint(&state, &server, true).await;
        let base = server.uri();
        let service = service(&state, &base);
        let (a, b) = tokio::join!(
            service.redeem_claim(&actor, &actor, &code, "A"),
            service.redeem_claim(&other, &other, &code, "B")
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let winner = a.or(b).unwrap();
        assert_eq!(
            state
                .db
                .collection::<TelegramBotRequest>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            saved_claim(&state).await.request_id.as_deref(),
            Some(winner.id.as_str())
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_recovery_rotates_only_on_creator_request_before_provisioning() {
        let (state, actor, server) = fixture().await;
        let code = mint(&state, &server, true).await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        webhook(
            &service,
            &headers,
            message(json!({"managed_bot_created": {"bot": bot()}})),
        )
        .await;
        assert_eq!(claim_code(&server).await, code);
        webhook(
            &service,
            &headers,
            message(json!({"text": "/recover @CustomerBot"})),
        )
        .await;
        let replacement = claim_code(&server).await;
        assert_ne!(code, replacement);
        assert!(matches!(
            service.preview_claim(&actor, &code).await,
            Err(AppError::NotFound(_))
        ));
        let ready = service
            .redeem_claim(&actor, &actor, &replacement, "Support")
            .await
            .unwrap();
        service.cancel(&actor, &ready.id).await.unwrap();
        webhook(
            &service,
            &headers,
            message(json!({"text": "/recover @CustomerBot"})),
        )
        .await;
        let renewed = claim_code(&server).await;
        assert_ne!(renewed, replacement);
        let ready = service
            .redeem_claim(&actor, &actor, &renewed, "Support")
            .await
            .unwrap();
        provider_connection(&server, &ready.id, 500).await;
        service.complete_pending_creations().await.unwrap();
        let saved = state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id": &ready.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            service.get(&actor, &ready.id).await.unwrap().status,
            Status::Provisioning
        );
        assert!(
            state
                .db
                .collection::<bson::Document>(MANAGED_BOTS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .get_bool("retired")
                .unwrap()
        );
        webhook(
            &service,
            &headers,
            message(json!({"text": "/recover @CustomerBot"})),
        )
        .await;
        assert_eq!(claim_code(&server).await, renewed);
        Mock::given(method("POST"))
            .and(path(format!("/bot{CHILD}/setWebhook")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true})),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        state.db.collection::<TelegramBotRequest>(REQUESTS).update_one(doc! {"_id": &ready.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}, "$unset": {"next_connection_attempt_at": ""}}).await.unwrap();
        state.db.collection::<TelegramBotClaim>(CLAIMS).update_one(doc! {}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        service.complete_pending_creations().await.unwrap();
        let connected = state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id": &ready.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connected.bot_token_encrypted, saved.bot_token_encrypted);
        assert_eq!(connected.status, "active");
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path().ends_with("getManagedBotToken"))
                .count(),
            1
        );
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .delete_one(doc! {"_id": &ready.id})
            .await
            .unwrap();
        webhook(
            &service,
            &headers,
            message(json!({"text": "/recover @CustomerBot"})),
        )
        .await;
        assert_eq!(claim_code(&server).await, renewed);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_org_destination_requires_live_write_access() {
        use crate::models::org_membership::{MemberScopeSource, OrgRole};
        let (state, actor, server) = fixture().await;
        let org =
            crate::services::org_service::create_org_user(&state.db, "Support team", None, None)
                .await
                .unwrap();
        let code = mint(&state, &server, true).await;
        let base = server.uri();
        let service = service(&state, &base);
        assert!(
            service
                .redeem_claim(&actor, &org.id, &code, "Support")
                .await
                .is_err()
        );
        let member = crate::services::org_service::create_membership(
            &state.db,
            &org.id,
            &actor,
            OrgRole::Viewer,
            MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            service
                .redeem_claim(&actor, &org.id, &code, "Support")
                .await,
            Err(AppError::Forbidden(_))
        ));
        assert_eq!(
            saved_claim(&state).await.status,
            TelegramClaimStatus::Pending
        );
        state
            .db
            .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(doc! {"_id": &member.id}, doc! {"$set": {"role": "admin"}})
            .await
            .unwrap();
        let ready = service
            .redeem_claim(&actor, &org.id, &code, "Support")
            .await
            .unwrap();
        assert_eq!(ready.owner_user_id, org.id);
        state
            .db
            .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &member.id},
                doc! {"$set": {"revoked_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
        provider_connection(&server, &ready.id, 200).await;
        service.complete_pending_creations().await.unwrap();
        assert!(
            !server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path().ends_with("getManagedBotToken"))
        );
        assert_eq!(
            saved_claim(&state).await.status,
            TelegramClaimStatus::Redeemed
        );
        assert!(
            service
                .redeem_claim(&actor, &org.id, &code, "Support")
                .await
                .is_err()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_creator_conflict_does_not_name_another_actors_setup_as_yours() {
        let (state, actor, server) = fixture().await;
        let other = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&other, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let code = mint(&state, &server, true).await;
        let base = server.uri();
        let service = service(&state, &base);
        let (_, link) = service
            .begin(&other, &other, "Other account setup", true)
            .await
            .unwrap();
        let challenge = reqwest::Url::parse(&link)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "start")
            .unwrap()
            .1
            .to_string();
        webhook(
            &service,
            &headers(&state).await,
            message(json!({"text": format!("/start {challenge}")})),
        )
        .await;
        assert!(
            matches!(service.redeem_claim(&actor, &actor, &code, "Support").await, Err(AppError::Conflict(ref text)) if text.starts_with("The bot creator has another Telegram setup"))
        );
        assert!(service.current(&actor).await.unwrap().is_none());
        assert_eq!(
            saved_claim(&state).await.status,
            TelegramClaimStatus::Pending
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_redemption_gives_ready_request_time_to_retry_after_claim_expiry() {
        let (state, actor, server) = fixture().await;
        let code = mint(&state, &server, true).await;
        state.db.collection::<TelegramBotClaim>(CLAIMS).update_one(doc! {}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(5))}}).await.unwrap();
        let base = server.uri();
        let service = service(&state, &base);
        let ready = service
            .redeem_claim(&actor, &actor, &code, "Support")
            .await
            .unwrap();
        assert!(ready.expires_at > Utc::now() + Duration::minutes(14));
        provider_connection(&server, &ready.id, 200).await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/getManagedBotToken")))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        service.complete_pending_creations().await.unwrap();
        assert_eq!(
            service.get(&actor, &ready.id).await.unwrap().status,
            Status::Ready
        );
        assert_eq!(
            state
                .db
                .collection::<ChannelBot>(BOTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        state.db.collection::<TelegramBotClaim>(CLAIMS).update_one(doc! {}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        state
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .update_one(
                doc! {"_id": &ready.id},
                doc! {"$unset": {"next_connection_attempt_at": ""}},
            )
            .await
            .unwrap();
        service.complete_pending_creations().await.unwrap();
        assert_eq!(
            service.get(&actor, &ready.id).await.unwrap().status,
            Status::Connected
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn telegram_new_claim_never_minted_for_bound_website_request() {
        let (state, actor, server) = fixture().await;
        automatic_request(&state, &actor, &server, true).await;
        assert_eq!(
            state
                .db
                .collection::<TelegramBotClaim>(CLAIMS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }
}
