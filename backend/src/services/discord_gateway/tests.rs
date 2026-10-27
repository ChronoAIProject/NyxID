use super::*;
use crate::models::{
    channel_message::ChannelMessage,
    channel_thread::{ThreadAddress, ThreadKind},
    coordination::CoordinationHolder,
};
use crate::services::channel_platform::PlatformAdapter;
use serde_json::json;

fn bot() -> ChannelBot {
    bson::from_document(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":uuid::Uuid::new_v4().to_string(),
        "platform":"discord","label":"fixture","platform_bot_id":"100","platform_bot_username":"helper",
        "bot_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2,3]},
        "webhook_registered":true,"webhook_secret_hash":"hash","status":"active","is_active":true,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).unwrap()
}
fn event(id: &str) -> Value {
    json!({"op":0,"t":"MESSAGE_CREATE","s":2,"d":{
        "id":id,"type":0,"channel_id":"200","guild_id":"300","author":{"id":"400","username":"private name"},
        "content":"private content","mentions":[{"id":"100"}],"mention_everyone":false,"mention_roles":[],
    }})
}
fn channel() -> Value {
    json!({"id":"200","type":11,"parent_id":"201","guild_id":"300"})
}
fn normalize(
    bot: &ChannelBot,
    id: &str,
) -> (
    super::super::channel_platform::InboundMessage,
    crate::models::channel_thread::ChannelThreadFacts,
) {
    facts::normalize(bot, &event(id), &channel())
        .unwrap()
        .unwrap()
}

#[test]
fn gateway_facts_cover_dm_guild_threads_mentions_broadcasts_and_correlated_replies() {
    let bot = bot();
    let (inbound, facts) = normalize(&bot, "500");
    assert_eq!(inbound.conversation_type, "group");
    assert_eq!(inbound.thread_id.as_deref(), Some("200"));
    assert_eq!(facts.parent_chat_id.as_deref(), Some("201"));
    assert_eq!(facts.kind, ThreadKind::Native);
    assert_eq!(facts.address, ThreadAddress::Mention);
    assert!(!serde_json::to_string(&facts).unwrap().contains("private"));
    let mut e = event("501");
    e["d"]["mentions"] = json!([{"id":"401","username":"helper"}]);
    e["d"]["content"] = json!("@helper <@100>");
    let (_, f) = facts::normalize(&bot, &e, &channel()).unwrap().unwrap();
    assert_eq!(f.address, ThreadAddress::NotAddressed);
    assert!(f.mentions_others);
    for (field, value) in [
        ("mention_everyone", json!(true)),
        ("mention_roles", json!(["999"])),
    ] {
        let mut broadcast = e.clone();
        broadcast["d"][field] = value;
        let (_, f) = facts::normalize(&bot, &broadcast, &channel())
            .unwrap()
            .unwrap();
        assert!(!f.mentions_others);
        assert_eq!(f.address, ThreadAddress::NotAddressed);
        let d = f.discord_gateway.unwrap();
        assert!(d.broadcast_everyone_or_here || d.broadcast_roles);
    }
    e["d"]["message_reference"] = json!({"message_id":"499","channel_id":"200","type":0});
    e["d"]["referenced_message"] = json!({"id":"499","channel_id":"200","author":{"id":"100"}});
    assert_eq!(
        facts::normalize(&bot, &e, &channel())
            .unwrap()
            .unwrap()
            .1
            .address,
        ThreadAddress::ReplyToBot
    );
    e["d"]["referenced_message"]["channel_id"] = json!("999");
    let (m, f) = facts::normalize(&bot, &e, &channel()).unwrap().unwrap();
    assert_eq!(f.address, ThreadAddress::NotAddressed);
    assert!(m.raw_data.get("referenced_message").is_none());
    e["d"]["referenced_message"]["channel_id"] = json!("200");
    e["d"]["message_reference"]["type"] = json!(1);
    let (m, f) = facts::normalize(&bot, &e, &channel()).unwrap().unwrap();
    assert_eq!(f.address, ThreadAddress::NotAddressed);
    assert!(m.reply_to_platform_message_id.is_none());
    assert!(m.raw_data.get("referenced_message").is_none());
    let mut dm = event("502");
    dm["d"].as_object_mut().unwrap().remove("guild_id");
    let (m, f) = facts::normalize(&bot, &dm, &json!({"id":"200","type":1}))
        .unwrap()
        .unwrap();
    assert_eq!(m.conversation_type, "private");
    assert!(m.thread_id.is_none());
    assert!(f.root_id.is_none());
    // Voice/stage text chat must not stall the bot's entire Gateway stream.
    // It is ordinary group chat without a supported follow partition.
    for kind in [2, 13] {
        let (m, f) = facts::normalize(
            &bot,
            &event("503"),
            &json!({"id":"200","type":kind,"guild_id":"300"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(m.conversation_type, "group");
        assert_eq!(f.kind, ThreadKind::Unknown);
        assert!(f.root_id.is_none());
    }
    assert_eq!(
        facts::normalize(
            &bot,
            &event("503"),
            &json!({"id":"200","type":0,"guild_id":"300"})
        )
        .unwrap()
        .unwrap()
        .1
        .kind,
        ThreadKind::ReplyChain
    );
}

#[test]
fn bot_webhook_system_and_uncorrelated_channel_events_never_enter_chat() {
    let bot = bot();
    for alteration in [
        json!({"author":{"id":"100"}}),
        json!({"author":{"id":"401","bot":true}}),
        json!({"webhook_id":"600"}),
        json!({"type":7}),
    ] {
        let mut e = event("500");
        for (k, v) in alteration.as_object().unwrap() {
            e["d"][k] = v.clone();
        }
        assert!(facts::normalize(&bot, &e, &channel()).unwrap().is_none());
    }
    for bad in [
        json!({"id":"999","type":11,"parent_id":"201","guild_id":"300"}),
        json!({"id":"200","type":11,"guild_id":"300"}),
        json!({"id":"200","type":11,"parent_id":"201","guild_id":"999"}),
    ] {
        assert!(facts::normalize(&bot, &event("500"), &bad).is_err());
    }
}

async fn fixture(name: &str) -> (AppState, Fence) {
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    let state = crate::test_utils::test_app_state(db);
    let mut bot = bot();
    bot.bot_token_encrypted = state
        .encryption_keys
        .encrypt(b"fixture-token")
        .await
        .unwrap();
    state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &bot.user_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    state
        .db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    let lease = LeaseStore::acquire(
        &state.db,
        &format!("discord-gateway:{}", bot.id),
        &holder("first"),
        Duration::from_secs(30),
    )
    .await
    .unwrap()
    .unwrap();
    (state, Fence { lease, bot })
}
fn holder(id: &str) -> CoordinationHolder {
    CoordinationHolder {
        instance_id: id.into(),
        generation_id: uuid::Uuid::new_v4().to_string(),
    }
}
fn message(fence: &Fence, id: &str) -> ChannelMessage {
    let (inbound, facts) = normalize(&fence.bot, id);
    let mut message = super::super::channel_relay_service::inbound_metadata(
        &fence.bot.id,
        "route",
        &fence.bot.user_id,
        "discord",
        &inbound,
        "agent",
        &uuid::Uuid::new_v4().to_string(),
    );
    message.thread_context = Some(facts);
    message
}

#[tokio::test]
async fn lease_handover_fences_stale_admission_status_and_checkpoint_and_deduplicates_replay() {
    let (state, old) = fixture("discord_gateway_handover").await;
    assert!(
        LeaseStore::acquire(
            &state.db,
            &old.lease.name,
            &holder("second"),
            Duration::from_secs(30)
        )
        .await
        .unwrap()
        .is_none()
    );
    let resume = Resume {
        session_id: "session".into(),
        url: "wss://gateway.discord.gg/".into(),
        sequence: Some(12),
    };
    checkpoint(&state.db, &old, &resume).await.unwrap();
    let first = message(&old, "500");
    assert!(
        super::super::channel_admission_service::store_discord(&state.db, first.clone(), &old)
            .await
            .unwrap()
            .is_some()
    );
    state
        .db
        .collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&old.lease.name},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(!old.current(&state.db).await.unwrap());
    let lease = LeaseStore::acquire(
        &state.db,
        &old.lease.name,
        &holder("second"),
        Duration::from_secs(30),
    )
    .await
    .unwrap()
    .unwrap();
    let new = Fence {
        lease,
        bot: old.bot.clone(),
    };
    assert!(new.current(&state.db).await.unwrap());
    assert!(status(&state.db, &old, "connected", None).await.is_err());
    assert!(
        checkpoint(&state.db, &old, &Resume::default())
            .await
            .is_err()
    );
    assert!(!LeaseStore::release(&state.db, &old.lease).await.unwrap());
    let restored: Checkpoint = bson::from_bson(
        LeaseStore::load_checkpoint(&state.db, &new.lease)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(restored.resume.sequence, Some(12));
    let mut replay = first;
    replay.id = uuid::Uuid::new_v4().to_string();
    assert!(
        super::super::channel_admission_service::store_discord(&state.db, replay, &new)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        super::super::channel_admission_service::store_discord(
            &state.db,
            message(&old, "501"),
            &old
        )
        .await
        .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    // The failed stale attempt committed no metadata; its claim is bounded.
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn token_rotation_deletion_and_owner_change_stop_live_lease_work() {
    for update in [
        doc! {"is_active":false},
        doc! {"ownership_version":1_i64},
        doc! {"bot_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![9]}},
    ] {
        let (state, fence) = fixture("discord_gateway_lifecycle").await;
        state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .update_one(doc! {"_id":&fence.bot.id}, doc! {"$set":update})
            .await
            .unwrap();
        assert!(!fence.current(&state.db).await.unwrap());
        assert!(status(&state.db, &fence, "connected", None).await.is_err());
        assert!(
            super::super::channel_admission_service::store_discord(
                &state.db,
                message(&fence, "500"),
                &fence
            )
            .await
            .is_err()
        );
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn shared_pipeline_sends_one_signed_callback_with_versioned_facts_and_no_persisted_content() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let (state, fence) = fixture("discord_gateway_pipeline").await;
    let server = MockServer::start().await;
    let agent = uuid::Uuid::new_v4().to_string();
    state.db.collection::<Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
        "_id":&agent,"user_id":&fence.bot.user_id,"name":"agent","key_prefix":"nyxid_ag","key_hash":"deadbeef".repeat(8),
        "scopes":"read write","is_active":true,"callback_url":format!("{}/callback",server.uri()),"created_at":bson::DateTime::now(),
    }).await.unwrap();
    state.db.collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":&fence.bot.user_id,"channel_bot_id":&fence.bot.id,"platform":"discord",
        "platform_conversation_id":"*","platform_conversation_type":"private","agent_api_key_id":agent,"default_agent":true,"is_active":true,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = super::super::channel_adapters::discord::DiscordAdapter::default();
    let (inbound, facts) = normalize(&fence.bot, "500");
    let dispatch = || {
        super::super::channel_inbound_service::process_discord_gateway_message(
            &state, &fence.bot, &adapter, &inbound, &fence, &facts,
        )
    };
    let (a, b) = tokio::join!(dispatch(), dispatch());
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(a || b);
    assert!(dispatch().await.unwrap());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.contains_key("x-nyxid-callback-token"));
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["conversation"]["platform_id"], "200");
    assert_eq!(body["conversation"]["type"], "group");
    assert_eq!(body["thread_context"]["discord_gateway"]["version"], 1);
    assert_eq!(
        body["thread_context"]["discord_gateway"]["bot_mentioned"],
        true
    );
    assert_eq!(body["thread_context"]["native_thread_id"], "200");
    let stored = state
        .db
        .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.to_string().contains("private content"));
    assert!(!stored.contains_key("raw_platform_data"));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn event_webhooks_ping_is_refused_only_after_signature_verification() {
    use axum::{
        body::Bytes,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
    };
    use ed25519_dalek::Signer;
    let (state, fence) = fixture("discord_gateway_ping").await;
    let signing = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
    state
        .db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&fence.bot.id},
            doc! {"$set":{"public_key":hex::encode(signing.verifying_key().as_bytes())}},
        )
        .await
        .unwrap();
    for (kind, signed, status) in [
        (0, false, StatusCode::UNAUTHORIZED),
        (0, true, StatusCode::BAD_REQUEST),
        (1, true, StatusCode::OK),
    ] {
        let body = format!("{{\"type\":{kind}}}");
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let mut headers = HeaderMap::new();
        if signed {
            headers.insert("x-signature-timestamp", timestamp.parse().unwrap());
            headers.insert(
                "x-signature-ed25519",
                hex::encode(
                    signing
                        .sign(format!("{timestamp}{body}").as_bytes())
                        .to_bytes(),
                )
                .parse()
                .unwrap(),
            );
        }
        let response = crate::handlers::channel_webhooks::channel_webhook(
            State(state.clone()),
            Path(("discord".into(), fence.bot.id.clone())),
            "/".parse().unwrap(),
            headers,
            Bytes::from(body),
        )
        .await;
        assert_eq!(response.status(), status);
        assert_eq!(
            super::super::channel_bot_service::get_bot(&state.db, &fence.bot.id)
                .await
                .unwrap()
                .diagnostics
                .failure_reason
                .as_deref(),
            (signed && kind == 0).then_some(EVENT_WEBHOOK_NOTICE)
        );
    }
    assert_eq!(
        super::super::channel_bot_service::get_bot(&state.db, &fence.bot.id)
            .await
            .unwrap()
            .diagnostics
            .failure_reason
            .as_deref(),
        None
    );
    state.db.drop().await.unwrap();
}

#[test]
fn default_off_flag_and_setup_distinguish_discord_endpoints() {
    assert!(
        !super::super::feature_flag_service::find_flag(
            super::super::feature_flag_service::DISCORD_GATEWAY_FLAG_KEY
        )
        .unwrap()
        .default_enabled
    );
    let adapter = super::super::channel_adapters::discord::DiscordAdapter::default();
    let instructions = adapter.registration().setup_instructions.join(" ");
    for text in [
        "Interactions Endpoint URL",
        "Event Webhooks",
        "Message Content Intent",
        "Send Messages in Threads",
        "does not create slash commands",
    ] {
        assert!(instructions.contains(text));
    }
}

#[tokio::test]
async fn websocket_resume_after_lease_handover_deduplicates_replayed_message() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let (state, old) = fixture("discord_gateway_ws_handover").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let server_url = url.clone();
    let peer = tokio::spawn(async move {
        for pass in 0..2 {
            let mut ws = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            ws.send(Message::Text(
                json!({"op":10,"d":{"heartbeat_interval":1000}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            let auth = ws.next().await.unwrap().unwrap().into_text().unwrap();
            let auth: Value = serde_json::from_str(&auth).unwrap();
            assert_eq!(auth["op"], if pass == 0 { 2 } else { 6 });
            let events = if pass == 0 {
                vec![
                    json!({"op":0,"t":"READY","s":1,"d":{"user":{"id":"100"},"session_id":"handover-session","resume_gateway_url":server_url}}),
                    event("500"),
                ]
            } else {
                assert_eq!(auth["d"]["session_id"], "handover-session");
                assert_eq!(auth["d"]["seq"], 2);
                let mut next = event("501");
                next["s"] = json!(4);
                vec![
                    event("500"),
                    json!({"op":0,"t":"RESUMED","s":3,"d":{}}),
                    next,
                ]
            };
            for value in events.into_iter().chain([json!({"op":7})]) {
                ws.send(Message::Text(value.to_string().into()))
                    .await
                    .unwrap();
            }
            while let Some(Ok(frame)) = ws.next().await {
                if matches!(frame, Message::Close(_)) {
                    break;
                }
                if matches!(frame, Message::Text(_)) {
                    let _ = ws
                        .send(Message::Text(json!({"op":11}).to_string().into()))
                        .await;
                }
            }
        }
    });
    async fn dispatch(
        state: &AppState,
        fence: &Fence,
        cursor: Resume,
        value: Value,
    ) -> Result<(), Failure> {
        if value["t"] == "MESSAGE_CREATE" {
            let (inbound, facts) = facts::normalize(&fence.bot, &value, &channel())?.unwrap();
            let mut message = super::super::channel_relay_service::inbound_metadata(
                &fence.bot.id,
                "route",
                &fence.bot.user_id,
                "discord",
                &inbound,
                "agent",
                &uuid::Uuid::new_v4().to_string(),
            );
            message.thread_context = Some(facts);
            super::super::channel_admission_service::store_discord(&state.db, message, fence)
                .await
                .map_err(|_| Failure::Ingress)?;
        }
        checkpoint(&state.db, fence, &cursor).await
    }
    let mut resume = Resume::default();
    assert_eq!(
        protocol::connection(
            &url,
            "fixture-token",
            "100",
            &mut resume,
            |cursor, event| dispatch(&state, &old, cursor, event)
        )
        .await,
        Failure::Reconnect
    );
    assert!(LeaseStore::release(&state.db, &old.lease).await.unwrap());
    let lease = LeaseStore::acquire(
        &state.db,
        &old.lease.name,
        &holder("new-replica"),
        Duration::from_secs(30),
    )
    .await
    .unwrap()
    .unwrap();
    let new = Fence {
        lease,
        bot: old.bot.clone(),
    };
    let stored: Checkpoint = bson::from_bson(
        LeaseStore::load_checkpoint(&state.db, &new.lease)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(stored.matches_bot(&new.bot));
    for mutation in [
        "platform_bot_id",
        "user_id",
        "ownership_version",
        "bot_token_encrypted",
    ] {
        let mut changed = new.bot.clone();
        match mutation {
            "platform_bot_id" => changed.platform_bot_id = "999".into(),
            "user_id" => changed.user_id = uuid::Uuid::new_v4().to_string(),
            "ownership_version" => changed.ownership_version += 1,
            _ => changed.bot_token_encrypted.push(0),
        }
        assert!(!stored.matches_bot(&changed), "{mutation}");
    }
    let mut resume = stored.resume;
    assert_eq!(
        protocol::connection(
            &url,
            "fixture-token",
            "100",
            &mut resume,
            |cursor, event| dispatch(&state, &new, cursor, event)
        )
        .await,
        Failure::Reconnect
    );
    peer.await.unwrap();
    assert_eq!(resume.sequence, Some(4));
    assert_eq!(
        state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shutdown_drops_owned_work_before_releasing_lease() {
    let (state, fence) = fixture("discord_gateway_shutdown").await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    run_owned(&state, fence.clone(), cancel).await;
    assert!(!fence.current(&state.db).await.unwrap());
    assert!(
        LeaseStore::acquire(
            &state.db,
            &fence.lease.name,
            &holder("replacement"),
            Duration::from_secs(30)
        )
        .await
        .unwrap()
        .is_some()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn disabled_ingress_never_opens_a_session_and_expired_local_deadline_stops_guard() {
    let (state, fence) = fixture("discord_gateway_start_guard").await;
    // The default-off flag is checked before decrypting or connecting.
    run_owned(&state, fence.clone(), CancellationToken::new()).await;
    assert!(!fence.current(&state.db).await.unwrap());
    let bot = super::super::channel_bot_service::get_bot(&state.db, &fence.bot.id)
        .await
        .unwrap();
    assert_eq!(bot.diagnostics.discord_gateway.unwrap().state, "stopped");
    assert!(bot.diagnostics.failure_reason.is_none());
    // A delayed renewal cannot add a whole renewal interval past the deadline.
    tokio::time::timeout(
        Duration::from_millis(100),
        guard(&state, &fence, tokio::time::Instant::now()),
    )
    .await
    .unwrap();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn abandoned_admission_claim_is_not_mistaken_for_a_delivered_duplicate() {
    use super::super::{
        channel_admission_service as admission,
        coordination_service::{EventDedupClaimResult, EventDedupStore},
    };
    let (state, fence) = fixture("discord_gateway_pending_claim").await;
    let message = message(&fence, "500");
    let EventDedupClaimResult::Claimed(claim) =
        admission::claim(&state.db, &message).await.unwrap()
    else {
        panic!("fresh claim")
    };
    assert!(
        admission::store_discord(&state.db, message.clone(), &fence)
            .await
            .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(EventDedupStore::release(&state.db, &claim).await.unwrap());
    assert!(
        admission::store_discord(&state.db, message.clone(), &fence)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        admission::store_discord(&state.db, message, &fence)
            .await
            .unwrap()
            .is_none()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn admission_capability_is_bound_to_its_bot_owner_and_platform() {
    let (state, fence) = fixture("discord_gateway_scope").await;
    for (field, value) in [
        ("user_id", "another-owner"),
        ("channel_bot_id", "another-bot"),
        ("platform", "whatsapp"),
    ] {
        let mut raw = bson::to_document(&message(&fence, "500")).unwrap();
        raw.insert(field, value);
        let message = bson::from_document(raw).unwrap();
        assert!(
            super::super::channel_admission_service::store_discord(&state.db, message, &fence)
                .await
                .is_err()
        );
    }
    assert_eq!(
        state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn transport_reservation_stops_relay_until_remote_release_is_proven() {
    let (state, fence) = fixture("discord_transport_fence").await;
    let channels = state
        .db
        .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME);
    let releases = state
        .db
        .collection::<Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME);
    let id = uuid::Uuid::new_v4().to_string();
    channels
        .insert_one(
            doc! {"_id":&id,"channel_bot_id":&fence.bot.id,"status":"active","transport":"direct"},
        )
        .await
        .unwrap();
    assert!(
        transport::relay_allowed(&state.db, &fence.bot.id)
            .await
            .unwrap()
    );
    assert!(fence.current(&state.db).await.unwrap());
    channels
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"pending_agent_api_key_id":"pending"}},
        )
        .await
        .unwrap();
    assert!(
        !transport::relay_allowed(&state.db, &fence.bot.id)
            .await
            .unwrap()
    );
    assert!(!fence.current(&state.db).await.unwrap());
    assert!(
        checkpoint(&state.db, &fence, &Resume::default())
            .await
            .is_err()
    );
    let waiter = tokio::spawn({
        let db = state.db.clone();
        let bot = fence.bot.id.clone();
        async move { transport::wait_relay_stopped(&db, &bot).await }
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        !waiter.is_finished(),
        "remote creation cannot race a live relay session"
    );
    assert!(LeaseStore::release(&state.db, &fence.lease).await.unwrap());
    tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    channels
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"transport":"gateway"},"$unset":{"pending_agent_api_key_id":""}},
        )
        .await
        .unwrap();
    assert!(
        !transport::relay_allowed(&state.db, &fence.bot.id)
            .await
            .unwrap()
    );
    releases
        .insert_one(doc! {"_id":"cleanup","channel_bot_id":&fence.bot.id,"state":"due"})
        .await
        .unwrap();
    channels
        .update_one(doc! {"_id":&id}, doc! {"$set":{"transport":"direct"}})
        .await
        .unwrap();
    assert!(
        !transport::relay_allowed(&state.db, &fence.bot.id)
            .await
            .unwrap()
    );
    for resolution in ["preserved", "released", "not_created"] {
        releases
            .update_one(
                doc! {"_id":"cleanup"},
                doc! {"$set":{"state":"settled","resolution":resolution}},
            )
            .await
            .unwrap();
        assert_eq!(
            transport::relay_allowed(&state.db, &fence.bot.id)
                .await
                .unwrap(),
            resolution != "preserved"
        );
    }
    let next = Fence {
        bot: fence.bot,
        lease: LeaseStore::acquire(
            &state.db,
            &fence.lease.name,
            &holder("returned"),
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap(),
    };
    assert!(next.current(&state.db).await.unwrap());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mixed_replica_readiness_prevents_discord_socket_activation() {
    use crate::services::{feature_flag_service as flags, security_readiness_service as readiness};
    let (state, fence) = fixture("discord_mixed_readiness").await;
    readiness::ready_test_replica(&state.db).await;
    flags::set_platform_override(
        &state.db,
        flags::DISCORD_GATEWAY_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &fence.bot.user_id,
    )
    .await
    .unwrap();
    assert!(enabled(&state.db, &fence.bot).await.unwrap());
    state
        .db
        .collection::<Document>(crate::models::security_replica::COLLECTION_NAME)
        .update_one(
            doc! {"_id":"test-process"},
            doc! {"$pull":{"capabilities":flags::DISCORD_GATEWAY_FLAG_KEY}},
        )
        .await
        .unwrap();
    assert!(enabled(&state.db, &fence.bot).await.is_err());
    tokio::time::timeout(
        Duration::from_secs(2),
        run_owned(&state, fence.clone(), CancellationToken::new()),
    )
    .await
    .unwrap();
    assert!(
        LeaseStore::load_checkpoint(&state.db, &fence.lease)
            .await
            .unwrap()
            .is_none()
    );
    state.db.drop().await.unwrap();
}
