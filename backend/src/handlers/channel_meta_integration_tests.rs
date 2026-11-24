//! Full relay contracts with raw signed Meta batches and real reply-token auth.
use super::*;
use crate::services::channel_adapters::meta::{self, tests::fixture};
use crate::services::channel_platform::PlatformAdapter;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tower::ServiceExt;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path, query_param},
};

async fn configure(state: &AppState, bot: &ChannelBot) {
    let descriptor = crate::services::channel_meta_service::adapter(&bot.platform)
        .unwrap()
        .platform_credentials()
        .unwrap();
    let fields = [
        ("app_id", "111"),
        ("client_secret", "login-secret"),
        ("webhook_signing_secret", "fixture-signing-secret"),
        ("verify_token", "verify-secret"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), Some(zeroize::Zeroizing::new(v.into()))))
    .collect();
    crate::services::platform_credential_service::update(
        &state.db,
        &state.encryption_keys,
        &descriptor,
        &bot.user_id,
        &fields,
        false,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"credential_source":"platform"}},
        )
        .await
        .unwrap();
}

fn signed(body: &[u8]) -> HeaderMap {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-signing-secret").unwrap();
    mac.update(body);
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-hub-signature-256",
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
            .parse()
            .unwrap(),
    );
    headers
}
fn event(asset: &str, sender: &str, mid: &str) -> serde_json::Value {
    serde_json::json!({"sender":{"id":sender},
        "recipient":{"id":asset},
        "timestamp":Utc::now().timestamp_millis(),
        "message":{"mid":mid,
            "text":"Synthetic customer question"}})
}
async fn route(state: &AppState, bot: &ChannelBot, callback: &str) {
    let agent = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<bson::Document>(API_KEYS)
        .insert_one(doc! {"_id":&agent,
        "user_id":&bot.user_id,
        "name":"fixture-agent",
        "key_prefix":"nyxid_ag",
        "key_hash":"deadbeef".repeat(8),
        "scopes":"read write",
        "is_active":true,
        "callback_url":callback,
        "created_at":bson::DateTime::now()})
        .await
        .unwrap();

    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),
        "user_id":&bot.user_id,
        "channel_bot_id":&bot.id,
        "platform":&bot.platform,
        "platform_conversation_id":"*",
        "platform_conversation_type":"private",
        "agent_api_key_id":&agent,
        "default_agent":true,
        "is_active":true,
        "created_at":bson::DateTime::now(),
        "updated_at":bson::DateTime::now()})
        .await
        .unwrap();
}

async fn reply(
    state: &AppState,
    payload: &serde_json::Value,
    text: &str,
) -> axum::response::Response {
    axum::Router::new()
        .route(
            "/reply",
            axum::routing::post(crate::handlers::channel_relay_request::reply),
        )
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reply")
                .header(
                    "authorization",
                    format!("Bearer {}", payload["reply_token"].as_str().unwrap()),
                )
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"message_id":payload["message_id"],"reply":{"text":text}})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn full_path(platform: &str) {
    let (state, _, server, bot) = fixture(platform).await;
    configure(&state, &bot).await;
    route(&state, &bot, &format!("{}/callback", server.uri())).await;
    let mut second_bot = bot.clone();
    second_bot.id = uuid::Uuid::new_v4().to_string();
    second_bot.platform_bot_id = "444".into();
    second_bot.credential_source = "platform".into();
    second_bot.meta_binding.as_mut().unwrap().grant_id = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&second_bot)
        .await
        .unwrap();
    route(&state, &second_bot, &format!("{}/callback", server.uri())).await;
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(202))
        .expect(3)
        .mount(&server)
        .await;
    if platform == "messenger" {
        Mock::given(method("GET"))
            .and(path("/v25.0/222/thread_owner"))
            .and(query_param("recipient", "333"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":[{"thread_owner":{"app_id":111,
                    "expiration":Utc::now().timestamp()+3600}}]})),
            )
            .mount(&server)
            .await;
    }
    if platform == "messenger" {
        Mock::given(method("GET"))
            .and(path("/v25.0/444/thread_owner"))
            .and(query_param("recipient", "333"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":[{"thread_owner":{"app_id":111,
                    "expiration":Utc::now().timestamp()+3600}}]})),
            )
            .mount(&server)
            .await;
    }
    let body=serde_json::to_vec(&serde_json::json!({"object":if platform=="messenger" {"page"}else{"instagram"},"entry":[
        {"id":"222","messaging":[event("222","333","mid.first"),event("222","333","mid.second")]},
        {"id":"444","messaging":[event("444","333","mid.first")]},
        {"id":"999","messaging":[event("999","333","mid.first")]}
    ]})).unwrap();
    meta::with_test_graph_base(server.uri(), async {
        let mut forged = signed(&body);
        forged.insert(
            "x-hub-signature-256",
            format!("sha256={}", "00".repeat(32)).parse().unwrap(),
        );
        assert!(
            crate::handlers::channel_webhooks::dispatch_platform_webhook(
                &state, platform, &forged, &body
            )
            .await
            .is_err()
        );
        crate::handlers::channel_webhooks::dispatch_platform_webhook(
            &state,
            platform,
            &signed(&body),
            &body,
        )
        .await
        .unwrap();
        crate::handlers::channel_webhooks::dispatch_platform_webhook(
            &state,
            platform,
            &signed(&body),
            &body,
        )
        .await
        .unwrap();
        let callbacks: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == "/callback")
            .collect();
        assert_eq!(callbacks.len(), 3);

        let first: serde_json::Value = serde_json::from_slice(&callbacks[0].body).unwrap();
        for callback in &callbacks {
            assert!(callback.headers.get("x-nyxid-user-token").is_none());
            assert!(callback.headers.get("x-nyxid-callback-token").is_some());
            assert!(callback.headers.get("x-nyxid-signature").is_some());
            let payload: serde_json::Value = serde_json::from_slice(&callback.body).unwrap();
            let asset = payload["raw_platform_data"]["asset_id"].as_str().unwrap();
            let scoped_bot = if asset == "222" {
                &bot.id
            } else {
                &second_bot.id
            };
            assert_eq!(
                payload["sender"]["platform_id"],
                format!("{platform}:{scoped_bot}:{asset}:333")
            );
            assert!(payload["raw_platform_data"].get("entry").is_none());
        }
        let sequence = std::sync::atomic::AtomicUsize::new(0);
        Mock::given(method("POST"))
            .and(path("/v25.0/222/messages"))
            .respond_with(move |_: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"message_id":format!("out.{}",
                sequence.fetch_add(1,
                std::sync::atomic::Ordering::SeqCst)),
                "recipient_id":"333"}),
                )
            })
            .mount(&server)
            .await;

        let text = "Complete reply with emoji 🦉界\n\n".repeat(180);
        assert_eq!(reply(&state, &first, &text).await.status(), StatusCode::OK);
        let sends: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == "/v25.0/222/messages")
            .collect();
        assert!(sends.len() > 1);
        let limit = crate::services::channel_text::TextLimit::for_platform(platform);
        let mut reconstructed = String::new();
        for request in sends {
            let sent: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(sent["recipient"]["id"], "333");
            if platform == "messenger" {
                assert_eq!(sent["messaging_type"], "RESPONSE");
            }
            let part = sent["message"]["text"].as_str().unwrap();
            assert!(limit.measure(part) <= limit.budget());
            reconstructed.push_str(part);
        }
        assert_eq!(reconstructed, text);
        assert!(
            reply(&state, &first, "Do not repeat")
                .await
                .status()
                .is_client_error()
        );
        let stored: Vec<bson::Document> = state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();

        assert_eq!(
            stored
                .iter()
                .filter(|m| m.get_str("direction").ok() == Some("inbound"))
                .count(),
            3
        );
        for message in stored {
            let json = serde_json::to_string(&message).unwrap();
            assert!(!json.contains("Synthetic customer question"));
            assert!(!json.contains("Complete reply"));
            assert!(!json.contains("fixture-token"));
        }
        let bot = channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert_eq!(
            bot.diagnostics.inbound_outcomes["callback_accepted"].count,
            2
        );
        assert!(bot.diagnostics.inbound_outcomes["inbound_duplicate"].count >= 2);
    })
    .await;
    server.verify().await;
}

#[tokio::test]
async fn messenger_signed_webhook_dispatcher_callback_and_bound_reply_handler_full_path() {
    full_path("messenger").await;
}
#[tokio::test]
async fn instagram_signed_webhook_dispatcher_callback_and_bound_reply_handler_full_path() {
    full_path("instagram").await;
}

#[tokio::test]
async fn signed_held_instagram_control_events_project_without_admitting_dms() {
    let (state, adapter, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {
                "status": "suspended",
                "meta_binding.subscription": "suspended",
                "meta_binding.operator_suspended": true,
                "meta_binding.events_since": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1)),
            }},
        )
        .await
        .unwrap();
    let mut standby = event("222", "333", "standby");
    standby["timestamp"] = serde_json::json!(Utc::now().timestamp_millis());
    let body = serde_json::to_vec(&serde_json::json!({
        "object": "instagram",
        "entry": [{
            "id": "222",
            "messaging": [event("222", "333", "held-dm")],
            "standby": [standby]
        }]
    }))
    .unwrap();
    crate::handlers::channel_webhooks::dispatch_platform_webhook(
        &state,
        "instagram",
        &signed(&body),
        &body,
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {"channel_bot_id": &bot.id})
            .await
            .unwrap(),
        0
    );
    let control = state
        .db
        .collection::<bson::Document>(meta::CONTROLS)
        .find_one(doc! {"bot_id": &bot.id, "identity_key": {"$regex": "333$"}})
        .await
        .unwrap()
        .expect("held standby control should be projected after signature verification");
    assert!(!control.get_bool("controlled").unwrap());
    let get_bot = || channel_bot_service::get_bot(&state.db, &bot.id);
    // Capture the signed callback's bot and verification authority while the
    // delivery hold is still active. NyxID verifies the callback while held
    // and records its control loss after resume; the same grant must remain
    // authoritative for that delayed observation.
    let held_bot = get_bot().await.unwrap();
    let verification_snapshot =
        crate::services::channel_verification_service::CredentialSnapshot::load(
            &state.db, &adapter, &held_bot,
        )
        .await
        .unwrap();
    let verification_secrets = verification_snapshot
        .secrets(&state.encryption_keys, &adapter)
        .await
        .unwrap();
    let second_loss = serde_json::to_vec(&serde_json::json!({
        "object": "instagram",
        "entry": [{
            "id": "222",
            "messaging": [event("222", "444", "held-second-dm")],
            "standby": [event("222", "444", "held-second-loss")]
        }]
    }))
    .unwrap();
    let prepared_second_loss = adapter
        .prepare_webhook(
            &held_bot,
            Some(&verification_secrets),
            &signed(&second_loss),
            &second_loss,
        )
        .await
        .unwrap();
    crate::services::channel_meta_service::set_bot_delivery(
        &state.db,
        &held_bot,
        &held_bot.user_id,
        true,
    )
    .await
    .unwrap();
    let resumed = get_bot().await.unwrap();
    assert!(!resumed.meta_binding.as_ref().unwrap().operator_suspended);
    // Apply the accepted control observation with the pre-resume bot and
    // signer snapshot.  Resume changes the delivery generation, but does not
    // replace the grant, owner, asset, or signing authority.
    adapter
        .apply_webhook_state(
            &state.db,
            &held_bot,
            &prepared_second_loss.body,
            &verification_snapshot,
        )
        .await
        .unwrap();
    let second_control = state
        .db
        .collection::<bson::Document>(meta::CONTROLS)
        .find_one(doc! {"bot_id": &bot.id, "identity_key": {"$regex": "444$"}})
        .await
        .unwrap()
        .expect("the delayed signed control observation should be retained");
    assert!(!second_control.get_bool("controlled").unwrap());
    assert_eq!(
        second_control.get_i64("generation").unwrap(),
        resumed.meta_binding.as_ref().unwrap().generation
    );
    assert!(matches!(
        adapter.prove_control(&state.db, &state.http_client, &resumed, "333", "fixture-token").await,
        Err(crate::errors::AppError::ChannelConversationNotReachable(reason)) if reason == "meta:inbox_control_lost"
    ));
    assert!(matches!(
        adapter.prove_control(&state.db, &state.http_client, &resumed, "444", "fixture-token").await,
        Err(crate::errors::AppError::ChannelConversationNotReachable(reason)) if reason == "meta:inbox_control_lost"
    ));
    // A successor grant cannot reuse an observation signed under the prior
    // grant, even when it keeps the same owner, asset, and app credentials.
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {"meta_binding.grant_id": "successor-grant"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        adapter
            .apply_webhook_state(
                &state.db,
                &resumed,
                &prepared_second_loss.body,
                &verification_snapshot,
            )
            .await,
        Err(crate::errors::AppError::ChannelConversationNotReachable(reason)) if reason == "meta:binding_changed"
    ));
    server.verify().await;
}

#[tokio::test]
async fn managed_shared_dispatch_preserves_lookup_hold_through_reload() {
    let (state, adapter, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    route(&state, &bot, &format!("{}/callback", server.uri())).await;

    // Participant B already has provider control before the delivery hold.
    adapter
        .prove_control(&state.db, &state.http_client, &bot, "444", "fixture-token")
        .await
        .unwrap();
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {
                "status": "suspended",
                "meta_binding.subscription": "suspended",
                "meta_binding.operator_suspended": true,
                "meta_binding.events_since": bson::DateTime::from_chrono(
                    Utc::now() - chrono::Duration::seconds(1)
                ),
            }},
        )
        .await
        .unwrap();

    let body = serde_json::to_vec(&serde_json::json!({
        "object": "instagram",
        "entry": [{
            "id": "222",
            "messaging": [event("222", "444", "held-b-still-controlled")],
            "standby": [event("222", "333", "held-a-control-loss")]
        }]
    }))
    .unwrap();
    let pause = crate::handlers::channel_webhooks::new_shared_dispatch_pause(&bot.id);
    let reached = pause.reached.clone();
    let release = pause.release.clone();
    let dispatch_state = state.clone();
    let dispatch_body = body.clone();
    let dispatch_headers = signed(&dispatch_body);
    let dispatch = tokio::spawn(async move {
        crate::handlers::channel_webhooks::dispatch_platform_webhook_with_pause(
            &dispatch_state,
            "instagram",
            &dispatch_headers,
            &dispatch_body,
            pause,
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), reached.notified())
        .await
        .expect("shared dispatch should pause after its held target lookup");

    // Commit resume and subscription activation while the dispatcher is
    // between target lookup and its per-bot handler reload.
    let held = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    crate::services::channel_meta_service::set_bot_delivery(&state.db, &held, &held.user_id, true)
        .await
        .unwrap();
    let resumed = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": &bot.id,
                "meta_binding.generation": resumed.meta_binding.as_ref().unwrap().generation,
            },
            doc! {"$set": {
                "status": "active",
                "meta_binding.subscription": "active",
                "webhook_registered": true,
            }},
        )
        .await
        .unwrap();
    release.notify_one();
    dispatch.await.unwrap().unwrap();

    // The held batch was authenticated and its control observation was
    // retained, but neither participant's DM was admitted from that batch.
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(
        meta::require_ingress(
            &state.db,
            &current,
            &meta::conversation("instagram", &bot.id, "222", "444"),
        )
        .await
        .is_ok()
    );
    assert!(
        meta::require_ingress(
            &state.db,
            &current,
            &meta::conversation("instagram", &bot.id, "222", "333"),
        )
        .await
        .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME,)
            .count_documents(doc! {"channel_bot_id": &bot.id})
            .await
            .unwrap(),
        0
    );

    // A fresh delivery after the hold is gone can admit B, proving the held
    // batch was suppressed independently of B's still-valid control row.
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    let fresh = serde_json::to_vec(&serde_json::json!({
        "object": "instagram",
        "entry": [{
            "id": "222",
            "messaging": [event("222", "444", "fresh-b-still-controlled")]
        }]
    }))
    .unwrap();
    crate::handlers::channel_webhooks::dispatch_platform_webhook(
        &state,
        "instagram",
        &signed(&fresh),
        &fresh,
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME,)
            .count_documents(doc! {"channel_bot_id": &bot.id})
            .await
            .unwrap(),
        1
    );
    server.verify().await;
}

#[tokio::test]
async fn meta_callback_failure_is_not_replayed_and_does_not_advance_last_success() {
    let (state, a, server, bot) = fixture("instagram").await;
    route(&state, &bot, &format!("{}/callback", server.uri())).await;
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let body=serde_json::to_vec(&serde_json::json!({"object":"instagram","entry":[{"id":"222","messaging":[event("222","333","mid.failure")]}]})).unwrap();
    let secrets = crate::services::channel_platform::PlatformVerifySecrets::from([(
        "app_secret",
        "fixture-signing-secret",
    )]);
    let prepared = a
        .prepare_webhook(&bot, Some(&secrets), &signed(&body), &body)
        .await
        .unwrap();
    let messages = a.parse_inbound(&prepared.body).await.unwrap();
    let process = || {
        crate::services::channel_inbound_service::process_inbound_messages(
            crate::services::channel_inbound_service::InboundSource::Webhook,
            crate::services::channel_inbound_service::InboundDeps::from(&state),
            &bot,
            &a,
            &messages,
        )
    };
    process().await.unwrap();
    process().await.unwrap();
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        current.diagnostics.inbound_outcomes["callback_failed"].count,
        1
    );
    assert!(
        !current
            .diagnostics
            .inbound_outcomes
            .contains_key("callback_accepted")
    );
    server.verify().await;
}
