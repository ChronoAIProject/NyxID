use axum::response::IntoResponse;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use bson::doc;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path, query_param},
};

use super::{admin_platform_credentials as admin, channel_managed as managed};
use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
use crate::models::platform_credential::{COLLECTION_NAME as CREDENTIALS, PlatformCredential};
use crate::services::{
    channel_adapters::resolve_adapter,
    channel_managed::{ManagedProgress, build_verify_secrets},
    platform_credential_service as credentials,
    provider_token_exchange_service::TokenExchangeCache,
};
use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};

async fn fixture() -> (crate::AppState, crate::mw::auth::AuthUser, MockServer) {
    let db = connect_test_database("managed_channel")
        .await
        .expect("test MongoDB");
    let state = test_app_state(db);
    let id = uuid::Uuid::new_v4().to_string();
    let mut user = test_user(&id, crate::models::user::UserType::Person);
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    user.role_ids.push(
        crate::services::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap()
            .admin,
    );
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(user)
        .await
        .unwrap();
    let auth = test_auth_user(&id);
    let adapter = resolve_adapter("whatsapp", &state.token_exchange_cache).unwrap();
    credentials::update(
        &state.db,
        &state.encryption_keys,
        &adapter.platform_credentials().unwrap(),
        &id,
        &[
            (
                "app_id".to_string(),
                Some(zeroize::Zeroizing::new("111".to_string())),
            ),
            (
                "app_secret".to_string(),
                Some(zeroize::Zeroizing::new("secret-for-test".to_string())),
            ),
            (
                "embedded_signup_config_id".to_string(),
                Some(zeroize::Zeroizing::new("222".to_string())),
            ),
        ]
        .into(),
        false,
    )
    .await
    .unwrap();
    let server = MockServer::start().await;
    state
        .db
        .collection::<PlatformCredential>(CREDENTIALS)
        .update_one(
            doc! { "provider": "meta" },
            doc! { "$set": { "fields.test_graph_base": server.uri() } },
        )
        .await
        .unwrap();
    (state, auth, server)
}

fn input() -> managed::CompleteRequest {
    serde_json::from_value(json!({ "label": "Support", "code": "one-use-code", "phone_number_id": "333", "waba_id": "444" })).unwrap()
}

fn token_proof() -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"secret-for-test").unwrap();
    mac.update(b"business-token");
    hex::encode(mac.finalize().into_bytes())
}

async fn graph(
    server: &MockServer,
    scope_waba: &str,
    identity: &str,
    register_status: u16,
    override_status: u16,
    coexistence: bool,
) {
    let version = crate::services::channel_adapters::whatsapp::GRAPH_API_VERSION;
    Mock::given(method("GET"))
        .and(path(format!("/{version}/oauth/access_token")))
        .and(query_param("client_id", "111"))
        .and(query_param("client_secret", "secret-for-test"))
        .and(query_param("code", "one-use-code"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "access_token": "business-token" })),
        )
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path(format!("/{version}/debug_token"))).and(query_param("input_token", "business-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "app_id": "111", "is_valid": true, "granular_scopes": [ { "scope": "whatsapp_business_management", "target_ids": [scope_waba] }, { "scope": "whatsapp_business_messaging", "target_ids": [scope_waba] } ] } }))).mount(server).await;
    for (resource, fields, body) in [
        (
            "444/phone_numbers",
            "id",
            json!({ "data": [{ "id": "333" }] }),
        ),
        (
            "333",
            "id,display_phone_number,verified_name",
            json!({ "id": identity, "display_phone_number": "+1 555 123 4567" }),
        ),
        (
            "333",
            "is_on_biz_app,platform_type",
            json!({ "id": "333", "is_on_biz_app": coexistence, "platform_type": "CLOUD_API" }),
        ),
        (
            "333",
            "id,status",
            json!({ "id": "333", "status": "CONNECTED" }),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/{version}/{resource}")))
            .and(query_param("fields", fields))
            .and(query_param("appsecret_proof", token_proof()))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path(format!("/{version}/444/subscribed_apps")))
        .and(query_param("appsecret_proof", token_proof()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
        .with_priority(10)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/{version}/444/subscribed_apps")))
        .and(|request: &wiremock::Request| {
            request
                .body_json::<Value>()
                .ok()
                .is_some_and(|body| body.get("override_callback_uri").is_some())
        })
        .respond_with(ResponseTemplate::new(override_status).set_body_json(
            if override_status == 200 {
                json!({ "success": true })
            } else {
                json!({ "error": { "message": "UPSTREAM-SECRET", "code": 100 } })
            },
        ))
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/{version}/333/register")))
        .and(query_param("appsecret_proof", token_proof()))
        .and(body_partial_json(
            json!({ "messaging_product": "whatsapp" }),
        ))
        .respond_with(ResponseTemplate::new(register_status).set_body_json(
            if register_status == 200 {
                json!({ "success": true })
            } else {
                json!({ "error": { "message": "already registered UPSTREAM-SECRET", "code": 100 } })
            },
        ))
        .mount(server)
        .await;
}

#[test]
fn managed_registry_and_admin_descriptors_are_adapter_owned() {
    let cache = Arc::new(TokenExchangeCache::new());
    for adapter in crate::services::channel_adapters::registered_adapters(&cache) {
        assert_eq!(
            adapter.managed_onboarding().is_some(),
            adapter.platform_id() == "whatsapp"
        );
        assert_eq!(
            adapter.platform_webhook(),
            adapter.platform_id() == "whatsapp"
        );
    }
    let all = credentials::descriptors(&cache);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].1.provider, "meta");
}

#[tokio::test]
async fn managed_reregister_reuses_stored_pin_and_rechecks_coexistence() {
    let (state, auth, server) = fixture().await;
    graph(&server, "444", "333", 200, 200, false).await;
    let (_, Json(bot)) = managed::complete_inner(
        &state,
        &auth,
        "whatsapp",
        input(),
        &ManagedProgress::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        managed::reregister(State(state), auth, Path(bot.id))
            .await
            .unwrap(),
        StatusCode::NO_CONTENT
    );
    let requests = server.received_requests().await.unwrap();
    let registrations: Vec<Value> = requests
        .iter()
        .filter(|r| r.url.path().ends_with("/register"))
        .map(|r| r.body_json().unwrap())
        .collect();
    assert_eq!(registrations.len(), 2);
    assert_eq!(registrations[0]["pin"], registrations[1]["pin"]);
}

#[tokio::test]
async fn managed_completion_success_encrypts_pin_and_reuses_registration() {
    let (state, auth, server) = fixture().await;
    graph(&server, "444", "333", 200, 200, false).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (status, Json(response)) = managed::complete_inner(
        &state,
        &auth,
        "whatsapp",
        input(),
        &ManagedProgress(Some(tx)),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(response.credential_source, "platform");
    assert!(response.webhook_secret.is_none());
    assert!(response.setup_instructions.is_empty());
    let Json(detail) = super::channel_bots::get_bot(
        State(state.clone()),
        auth.clone(),
        Path(response.id.clone()),
    )
    .await
    .unwrap();
    assert!(detail.setup_instructions.is_empty());
    let bot = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! { "_id": &response.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bot.status, "pending_webhook");
    assert_eq!(bot.app_id.as_deref(), Some("444"));
    assert!(bot.app_secret_encrypted.is_none());
    assert_eq!(
        state
            .encryption_keys
            .decrypt(&bot.bot_token_encrypted)
            .await
            .unwrap(),
        b"business-token"
    );
    let pin = state
        .encryption_keys
        .decrypt(bot.registration_pin_encrypted.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(pin.len(), 6);
    assert!(pin.iter().all(u8::is_ascii_digit));
    assert_eq!(
        bot.managed_setup.as_ref().unwrap().registration,
        "registered"
    );
    let stages = [
        rx.try_recv().unwrap(),
        rx.try_recv().unwrap(),
        rx.try_recv().unwrap(),
    ];
    assert_eq!(
        stages,
        [
            json!({ "stage": "exchanging" }),
            json!({ "stage": "subscribing" }),
            json!({ "stage": "registering" })
        ]
    );
    let requests = server.received_requests().await.unwrap();
    let exchange = requests
        .iter()
        .find(|r| r.url.path().ends_with("oauth/access_token"))
        .unwrap();
    assert!(!exchange.url.query_pairs().any(|(k, _)| k == "redirect_uri"));
    let registration = requests
        .iter()
        .find(|r| r.url.path().ends_with("/register"))
        .unwrap()
        .body_json::<Value>()
        .unwrap();
    assert_eq!(registration["pin"], String::from_utf8(pin).unwrap());
    for request in requests.iter().filter(|r| {
        r.headers
            .get("authorization")
            .is_some_and(|h| h == "Bearer business-token")
    }) {
        assert!(
            request
                .url
                .query_pairs()
                .any(|(key, value)| key == "appsecret_proof" && value == token_proof())
        );
    }
    let count = state
        .db
        .collection::<ChannelBot>(BOTS)
        .count_documents(doc! {})
        .await
        .unwrap();
    assert!(
        managed::complete_inner(
            &state,
            &auth,
            "whatsapp",
            input(),
            &ManagedProgress::default()
        )
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
        count
    );
}

#[tokio::test]
async fn completion_pages_waba_membership_without_following_provider_urls() {
    let (state, auth, server) = fixture().await;
    graph(&server, "444", "333", 200, 200, false).await;
    let endpoint = format!(
        "/{}/444/phone_numbers",
        crate::services::channel_adapters::whatsapp::GRAPH_API_VERSION
    );
    Mock::given(method("GET")).and(path(&endpoint))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "999" }], "paging": { "next": "https://untrusted.invalid/next", "cursors": { "after": "next-page" } } })))
        .with_priority(2).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(&endpoint))
        .and(query_param("after", "next-page"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "333" }] })),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        managed::complete_inner(
            &state,
            &auth,
            "whatsapp",
            input(),
            &ManagedProgress::default()
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn completion_rejects_scope_and_phone_identity_mismatches_without_bot_or_side_effects() {
    for (waba, identity) in [("999", "333"), ("444", "999")] {
        let (state, auth, server) = fixture().await;
        graph(&server, waba, identity, 200, 200, false).await;
        let error = managed::complete_inner(
            &state,
            &auth,
            "whatsapp",
            input(),
            &ManagedProgress::default(),
        )
        .await
        .unwrap_err();
        assert!(!error.to_string().contains("UPSTREAM-SECRET"));
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
                .any(|r| r.method == "POST")
        );
    }
}

#[tokio::test]
async fn completion_override_failure_is_nonfatal_and_already_registered_is_confirmed() {
    let (state, auth, server) = fixture().await;
    graph(&server, "444", "333", 400, 400, false).await;
    let (_, Json(response)) = managed::complete_inner(
        &state,
        &auth,
        "whatsapp",
        input(),
        &ManagedProgress::default(),
    )
    .await
    .unwrap();
    let setup = response.managed_setup.unwrap();
    assert_eq!(setup.subscription, "subscribed");
    assert_eq!(setup.webhook_override, "failed");
    assert_eq!(setup.registration, "already_registered");
}

#[tokio::test]
async fn coexistence_and_finish_only_waba_discover_identity_and_skip_register() {
    let (state, auth, server) = fixture().await;
    graph(&server, "444", "333", 200, 200, true).await;
    let mut request = input();
    request.input.remove("phone_number_id");
    let (_, Json(response)) = managed::complete_inner(
        &state,
        &auth,
        "whatsapp",
        request,
        &ManagedProgress::default(),
    )
    .await
    .unwrap();
    assert_eq!(response.managed_setup.unwrap().registration, "coexistence");
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/register"))
    );
}

#[tokio::test]
async fn platform_credentials_mask_rotate_clear_and_fallback_on_demand() {
    let (state, auth, _) = fixture().await;
    let (_, Json(list)) = admin::list(State(state.clone()), auth.clone())
        .await
        .unwrap();
    let serialized = serde_json::to_value(&list).unwrap();
    assert!(!serialized.to_string().contains("secret-for-test"));
    assert!(!format!("{list:?}").contains(list[0].webhook_verify_token.as_ref().unwrap()));
    assert!(!serialized.to_string().contains("test_graph_base"));
    let original = credentials::load(&state.db, "meta").await.unwrap().unwrap();
    assert!(
        uuid::Uuid::parse_str(&original.id)
            .unwrap()
            .get_version_num()
            == 4
    );
    let document = bson::to_document(&original).unwrap();
    assert!(document.get_datetime("updated_at").is_ok());
    assert!(!format!("{original:?}").contains("secret-for-test"));
    let adapter = resolve_adapter("whatsapp", &state.token_exchange_cache).unwrap();
    let bot: ChannelBot = bson::from_document(doc! { "_id": "bot", "user_id": auth.user_id.to_string(), "platform": "whatsapp", "label": "test", "credential_source": "platform", "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] }, "platform_bot_id": "333", "platform_bot_username": "number", "webhook_registered": false, "webhook_secret_hash": "hash", "status": "pending_webhook", "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now() }).unwrap();
    assert_eq!(
        build_verify_secrets(&state.db, &state.encryption_keys, adapter.as_ref(), &bot)
            .await
            .unwrap()
            .get("app_secret"),
        Some("secret-for-test")
    );
    let body: admin::UpdatePlatformCredentialsRequest =
        serde_json::from_value(json!({ "fields": { "app_secret": "rotated-secret" } })).unwrap();
    assert!(!format!("{body:?}").contains("rotated-secret"));
    let (_, Json(rotated)) = admin::update(
        State(state.clone()),
        auth.clone(),
        Path("meta".to_string()),
        Json(body),
    )
    .await
    .unwrap();
    assert_eq!(list[0].webhook_verify_token, rotated.webhook_verify_token);
    assert_eq!(
        build_verify_secrets(&state.db, &state.encryption_keys, adapter.as_ref(), &bot)
            .await
            .unwrap()
            .get("app_secret"),
        Some("rotated-secret")
    );
    let mut byo = bot.clone();
    byo.credential_source = "user".to_string();
    assert!(
        build_verify_secrets(&state.db, &state.encryption_keys, adapter.as_ref(), &byo)
            .await
            .unwrap()
            .get("app_secret")
            .is_none()
    );
    let clear = serde_json::from_value(json!({ "fields": { "app_secret": null } })).unwrap();
    let (_, Json(cleared)) = admin::update(
        State(state.clone()),
        auth.clone(),
        Path("meta".to_string()),
        Json(clear),
    )
    .await
    .unwrap();
    assert!(!cleared.available);
    let (_, Json(bootstrap)) = managed::bootstrap(
        State(state.clone()),
        auth.clone(),
        Path("whatsapp".to_string()),
    )
    .await
    .unwrap();
    assert!(!bootstrap.available);
    assert!(bootstrap.fields.is_empty());
    admin::delete(State(state.clone()), auth, Path("meta".to_string()))
        .await
        .unwrap();
    assert!(
        credentials::load(&state.db, "meta")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn admin_endpoints_reject_non_admin_and_onboarding_rejects_non_humans() {
    let (state, auth, _) = fixture().await;
    let id = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&id, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let user = test_auth_user(&id);
    let error = admin::list(State(state.clone()), user.clone())
        .await
        .unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::FORBIDDEN);
    let error = admin::update(
        State(state.clone()),
        user.clone(),
        Path("meta".into()),
        Json(serde_json::from_value(json!({ "fields": {} })).unwrap()),
    )
    .await
    .unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::FORBIDDEN);
    assert_eq!(
        admin::delete(State(state.clone()), user, Path("meta".into()))
            .await
            .unwrap_err()
            .into_response()
            .status(),
        StatusCode::FORBIDDEN
    );
    for method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::ServiceAccount,
        crate::mw::auth::AuthMethod::Delegated,
        crate::mw::auth::AuthMethod::Relay,
    ] {
        let mut denied = auth.clone();
        denied.auth_method = method;
        assert_eq!(
            managed::bootstrap(State(state.clone()), denied, Path("whatsapp".into()))
                .await
                .unwrap_err()
                .into_response()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
}

pub(super) fn signed(body: &[u8]) -> HeaderMap {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"secret-for-test").unwrap();
    mac.update(body);
    HeaderMap::from_iter([(
        "x-hub-signature-256".parse().unwrap(),
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
            .parse()
            .unwrap(),
    )])
}

#[tokio::test]
async fn platform_dispatcher_handshake_signature_and_multinumber_targets() {
    let (state, auth, server) = fixture().await;
    let (_, Json(rows)) = admin::list(State(state.clone()), auth).await.unwrap();
    let adapter = resolve_adapter("whatsapp", &state.token_exchange_cache).unwrap();
    let secrets = credentials::load_decrypted(&state.db, &state.encryption_keys, "meta")
        .await
        .unwrap();
    let mut query = [
        ("hub.mode".to_string(), "subscribe".to_string()),
        (
            "hub.verify_token".to_string(),
            rows[0].webhook_verify_token.clone().unwrap(),
        ),
        ("hub.challenge".to_string(), "000123".to_string()),
    ]
    .into();
    assert_eq!(
        adapter
            .platform_subscription_handshake(&secrets, &query)
            .unwrap(),
        "000123"
    );
    query.insert("hub.verify_token".to_string(), "bad".to_string());
    assert!(
        adapter
            .platform_subscription_handshake(&secrets, &query)
            .is_err()
    );
    let body = serde_json::to_vec(&json!({ "object": "whatsapp_business_account", "entry": [{ "changes": [ { "value": { "metadata": { "phone_number_id": "333" } } }, { "value": { "metadata": { "phone_number_id": "555" } } }, { "value": { "metadata": { "phone_number_id": "333" } } } ] }] })).unwrap();
    assert_eq!(
        adapter
            .platform_webhook_targets(&secrets, &signed(&body), &body)
            .await
            .unwrap(),
        vec!["333", "555"]
    );
    assert!(
        adapter
            .platform_webhook_targets(&secrets, &signed(b"wrong"), &body)
            .await
            .is_err()
    );
    assert!(
        adapter
            .platform_webhook_targets(&secrets, &HeaderMap::new(), &body)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn platform_dispatcher_routes_two_bots_drops_unknown_and_preserves_dedup() {
    let (state, auth, _) = fixture().await;
    let callback = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(202))
        .expect(2)
        .mount(&callback)
        .await;
    let owner = auth.user_id.to_string();
    for phone in ["333", "555"] {
        let bot_id = uuid::Uuid::new_v4().to_string();
        let agent_id = uuid::Uuid::new_v4().to_string();
        state.db.collection::<bson::Document>(BOTS).insert_one(doc! {
            "_id": &bot_id, "user_id": &owner, "platform": "whatsapp", "label": "Support", "credential_source": "platform",
            "platform_bot_id": phone, "platform_bot_username": phone, "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1] },
            "webhook_registered": false, "webhook_secret_hash": "unused", "status": "pending_webhook", "is_active": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        }).await.unwrap();
        state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
            "_id": &agent_id, "user_id": &owner, "name": "agent", "key_prefix": "nyxid_ag", "key_hash": "hash", "scopes": "read write",
            "is_active": true, "callback_url": callback.uri(), "created_at": bson::DateTime::now(),
        }).await.unwrap();
        state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "user_id": &owner, "channel_bot_id": &bot_id, "platform": "whatsapp",
            "platform_conversation_id": "15551234567", "platform_conversation_type": "private", "agent_api_key_id": &agent_id,
            "default_agent": false, "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        }).await.unwrap();
    }
    let entries: Vec<Value> = ["333", "555", "777"].iter().map(|phone| json!({ "changes": [{ "field": "messages", "value": {
        "messaging_product": "whatsapp", "metadata": { "phone_number_id": phone },
        "messages": [{ "from": "15551234567", "id": "wamid.same-across-bots", "type": "text", "text": { "body": "Hello" } }]
    } }] })).collect();
    let body =
        serde_json::to_vec(&json!({ "object": "whatsapp_business_account", "entry": entries }))
            .unwrap();
    assert!(
        super::channel_webhooks::dispatch_platform_webhook(
            &state,
            "whatsapp",
            &signed(b"wrong"),
            &body
        )
        .await
        .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    for _ in 0..2 {
        super::channel_webhooks::dispatch_platform_webhook(
            &state,
            "whatsapp",
            &signed(&body),
            &body,
        )
        .await
        .unwrap();
    }
    let count = state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .count_documents(doc! { "direction": "inbound" })
        .await
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! { "status": "active" })
            .await
            .unwrap(),
        2
    );
    assert_eq!(callback.received_requests().await.unwrap().len(), 2);
}
