use super::*;
use crate::{
    models::{
        channel_bot::MetaCredentialBinding,
        user::{User, UserType},
    },
    services::{channel_admission_service, channel_relay_service},
    test_utils::*,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn event(asset: &str, sender: &str, mid: &str) -> Value {
    json!({"sender":{"id":sender},"recipient":{"id":asset},"timestamp":Utc::now().timestamp_millis(),"message":{"mid":mid,"text":"private fixture text"}})
}
fn payload(inbox: Inbox, entries: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({"object":inbox.object(),"entry":entries})).unwrap()
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

pub(crate) async fn fixture(
    platform: &str,
) -> (crate::AppState, MetaAdapter, MockServer, ChannelBot) {
    let db = connect_transaction_test_database("meta_inbox").await;
    let state = test_app_state(db);
    let owner = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let inbox = if platform == "messenger" {
        Inbox::Messenger
    } else {
        Inbox::Instagram
    };
    let server = MockServer::start().await;
    let mut adapter = MetaAdapter::new(inbox);
    adapter.graph_base = server.uri();
    let binding = MetaCredentialBinding {
        operator_suspended: false,
        grant_reconnect_required: false,
        provider_revoked: false,
        erasure_requested: false,
        grant_id: uuid::Uuid::new_v4().to_string(),
        authorized_at: Some(Utc::now()),
        events_since: None,
        generation: 1,
        token_fingerprint: token_fingerprint("fixture-token"),
        grant_actor_id: owner.clone(),
        grant_subject_id: "900".into(),
        app_id: "111".into(),
        exclusive_routing_verified: true,
        subscription: "active".into(),
        next_reconcile_at: None,
        expires_at: Some(Utc::now() + Duration::days(60)),
    };
    let doc = doc! {"_id":uuid::Uuid::new_v4().to_string(),
    "user_id":&owner,
    "platform":platform,
    "label":"Fixture inbox",
    "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,
    bytes:state.encryption_keys.encrypt(b"fixture-token").await.unwrap()},
    "platform_bot_id":"222",
    "platform_bot_username":"Fixture",
    "webhook_registered":true,
    "webhook_secret_hash":"hash",
    "app_id":"111",
    "status":"active",
    "is_active":true,
    "created_at":bson::DateTime::now(),
    "updated_at":bson::DateTime::now(),
    "meta_binding":bson::to_bson(&binding).unwrap()};

    let bot: ChannelBot = bson::from_document(doc).unwrap();
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    (state, adapter, server, bot)
}

async fn original(state: &crate::AppState, bot: &ChannelBot) -> ChannelMessage {
    let inbound = parse_event(
        if bot.platform == "messenger" {
            Inbox::Messenger
        } else {
            Inbox::Instagram
        },
        &bot.id,
        "222",
        &event("222", "333", "mid.1"),
    )
    .unwrap();
    let mut message = channel_relay_service::inbound_metadata(
        &bot.id,
        "route",
        &bot.user_id,
        &bot.platform,
        &inbound,
        "route-key",
        &uuid::Uuid::new_v4().to_string(),
    );
    message.meta_authority = Some(authority(bot, &inbound, 1800).unwrap());
    state
        .db
        .collection::<bson::Document>(CONTROLS)
        .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),
        "identity_key":format!("{}:{}",
        bot.id,
        inbound.conversation_id),
        "bot_id":&bot.id,
        "controlled":true,
        "generation": bot.meta_binding.as_ref().unwrap().generation})
        .await
        .unwrap();
    message
}

#[tokio::test]
async fn raw_signature_and_asset_split_never_leak_neighbor_events() {
    let (state, a, _, bot) = fixture("instagram").await;
    let body = payload(
        a.inbox,
        vec![
            json!({"id":"222","messaging":[event("222","333","same-mid")]}),
            json!({"id":"444","messaging":[event("444","333","same-mid")]}),
        ],
    );
    let headers = signed(&body);
    let secrets = PlatformVerifySecrets::from([("app_secret", "fixture-signing-secret")]);
    let prepared = a
        .prepare_webhook(&bot, Some(&secrets), &headers, &body)
        .await
        .unwrap();
    let messages = a.parse_inbound(&prepared.body).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].conversation_id,
        format!("instagram:{}:222:333", bot.id)
    );
    assert!(
        !serde_json::to_string(&messages[0].raw_data)
            .unwrap()
            .contains("444")
    );
    let mut changed = body.clone();
    changed.push(b' ');
    assert!(
        a.prepare_webhook(&bot, Some(&secrets), &headers, &changed)
            .await
            .is_err()
    );
    assert!(
        state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap()
            == 0
    );
}

#[test]
fn quote_text_is_supported_and_stories_echoes_groups_receipts_are_filtered() {
    let mut quoted = event("222", "333", "mid.1");
    quoted["message"]["reply_to"] = json!({"mid":"mid.previous"});
    assert_eq!(
        parse_event(Inbox::Instagram, "fixture-bot", "222", &quoted)
            .unwrap()
            .reply_to_platform_message_id
            .as_deref(),
        Some("mid.previous")
    );
    quoted["message"]["reply_to"] = json!({"story":{"id":"story"}});
    assert!(parse_event(Inbox::Instagram, "fixture-bot", "222", &quoted).is_none());
    for key in [
        "delivery",
        "read",
        "reaction",
        "postback",
        "group_id",
        "pass_thread_control",
    ] {
        let mut e = event("222", "333", "mid");
        e[key] = json!({});
        assert!(
            parse_event(Inbox::Messenger, "fixture-bot", "222", &e).is_none(),
            "{key}"
        );
    }
    let mut echo = event("222", "333", "mid");
    echo["message"]["is_echo"] = json!(true);
    assert!(parse_event(Inbox::Messenger, "fixture-bot", "222", &echo).is_none());
    assert!(
        parse_event(
            Inbox::Messenger,
            "fixture-bot",
            "222",
            &event("999", "333", "mid")
        )
        .is_none()
    );
    assert!(
        parse_event(
            Inbox::Messenger,
            "fixture-bot",
            "222",
            &event("222", "222", "mid")
        )
        .is_none()
    );
}

#[test]
fn future_provider_timestamps_are_rejected_without_clock_skew_tolerance() {
    let mut future = event("222", "333", "future");
    future["timestamp"] = json!((Utc::now() + Duration::seconds(61)).timestamp_millis());
    assert!(parse_event(Inbox::Messenger, "fixture-bot", "222", &future).is_none());
    // The launch policy deliberately rejects all future timestamps. Operators
    // repair clock synchronization rather than accepting an unauthenticated
    // future event or adding a provider clock-skew window.
}

#[tokio::test]
async fn mixed_standby_only_blocks_its_exact_asset_and_participant() {
    let (state, a, _, bot) = fixture("instagram").await;
    let body = payload(
        a.inbox,
        vec![
            json!({"id":"222","messaging":[event("222","333","a")],"standby":[event("222","444","b"),event("999","555","c")]}),
            json!({"id":"999","standby":[event("999","666","d")]}),
        ],
    );
    a.apply_webhook_state(
        &state.db,
        &bot,
        &body,
        &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
    )
    .await
    .unwrap();
    assert!(
        require_ingress(&state.db, &bot, &format!("instagram:{}:222:333", bot.id))
            .await
            .is_ok()
    );
    assert!(
        require_ingress(&state.db, &bot, &format!("instagram:{}:222:444", bot.id))
            .await
            .is_err()
    );
    assert!(
        require_ingress(&state.db, &bot, &format!("instagram:{}:222:555", bot.id))
            .await
            .is_ok()
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(CONTROLS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn messenger_pass_thread_control_gains_only_when_the_provider_names_this_app() {
    let (state, a, _, bot) = fixture("messenger").await;
    let mut gain = event("222", "333", "gain");
    gain["pass_thread_control"] = json!({"new_owner_app_id":111});
    let body = payload(a.inbox, vec![json!({"id":"222","messaging":[gain]})]);
    a.apply_webhook_state(
        &state.db,
        &bot,
        &body,
        &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
    )
    .await
    .unwrap();
    assert!(
        require_ingress(
            &state.db,
            &bot,
            &conversation("messenger", &bot.id, "222", "333")
        )
        .await
        .is_ok()
    );
    let mut other = event("222", "333", "other");
    other["pass_thread_control"] = json!({"new_owner_app_id":999});
    let body = payload(a.inbox, vec![json!({"id":"222","messaging":[other]})]);
    a.apply_webhook_state(
        &state.db,
        &bot,
        &body,
        &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
    )
    .await
    .unwrap();
    assert!(
        require_ingress(
            &state.db,
            &bot,
            &conversation("messenger", &bot.id, "222", "333")
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn provider_control_events_use_provider_time_and_conservative_ties() {
    let (state, adapter, _, bot) = fixture("messenger").await;
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {"meta_binding.authorized_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(5))}},
        )
        .await
        .unwrap();
    let bot = crate::services::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let newer = Utc::now().timestamp_millis() - 500;
    let older = newer - 500;
    let mut gain = event("222", "333", "gain-newer");
    gain["timestamp"] = json!(newer);
    gain["pass_thread_control"] = json!({"new_owner_app_id":111});
    adapter
        .apply_webhook_state(
            &state.db,
            &bot,
            &payload(
                adapter.inbox,
                vec![json!({"id":"222","messaging":[gain.clone()]})],
            ),
            &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
        )
        .await
        .unwrap();
    let mut loss = event("222", "333", "loss-older");
    loss["timestamp"] = json!(older);
    loss["take_thread_control"] = json!({});
    adapter
        .apply_webhook_state(
            &state.db,
            &bot,
            &payload(adapter.inbox, vec![json!({"id":"222","messaging":[loss]})]),
            &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
        )
        .await
        .unwrap();
    assert!(
        require_ingress(
            &state.db,
            &bot,
            &conversation("messenger", &bot.id, "222", "333")
        )
        .await
        .is_ok()
    );

    let mut loss_newer = event("222", "444", "loss-newer");
    loss_newer["timestamp"] = json!(newer);
    loss_newer["take_thread_control"] = json!({});
    adapter
        .apply_webhook_state(
            &state.db,
            &bot,
            &payload(
                adapter.inbox,
                vec![json!({"id":"222","messaging":[loss_newer]})],
            ),
            &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
        )
        .await
        .unwrap();
    let mut gain_older = event("222", "444", "gain-older");
    gain_older["timestamp"] = json!(older);
    gain_older["pass_thread_control"] = json!({"new_owner_app_id":111});
    adapter
        .apply_webhook_state(
            &state.db,
            &bot,
            &payload(
                adapter.inbox,
                vec![json!({"id":"222","messaging":[gain_older]})],
            ),
            &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
        )
        .await
        .unwrap();
    assert!(
        require_ingress(
            &state.db,
            &bot,
            &conversation("messenger", &bot.id, "222", "444")
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn generation_change_reproves_messenger_and_carries_positive_instagram_control() {
    let (state, messenger, server, bot) = fixture("messenger").await;
    Mock::given(method("GET"))
        .and(path("/v25.0/222/thread_owner"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"thread_owner": {"app_id": "111"}}]
        })))
        .mount(&server)
        .await;
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$inc": {"meta_binding.generation": 1_i64}},
        )
        .await
        .unwrap();
    let current = crate::services::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let inbound = parse_event(
        Inbox::Messenger,
        &current.id,
        &current.platform_bot_id,
        &event("222", "333", "generation-mid"),
    )
    .unwrap();
    messenger
        .authorize_inbound(
            &state.db,
            &reqwest::Client::new(),
            &state.encryption_keys,
            &current,
            &inbound,
        )
        .await
        .unwrap();
    let control = state
        .db
        .collection::<bson::Document>(CONTROLS)
        .find_one(doc! {"identity_key": format!("{}:{}", current.id, inbound.conversation_id)})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(control.get_i64("generation").unwrap(), 2);
    assert_eq!(control.get_bool("controlled").unwrap(), true);

    let (state, instagram, _, bot) = fixture("instagram").await;
    let key = format!(
        "{}:{}",
        bot.id,
        conversation("instagram", &bot.id, &bot.platform_bot_id, "333")
    );
    state
        .db
        .collection::<bson::Document>(CONTROLS)
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "identity_key": &key,
            "bot_id": &bot.id,
            "generation": 1_i64,
            "revision": 1_i64,
            "controlled": true,
        })
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$inc": {"meta_binding.generation": 1_i64}},
        )
        .await
        .unwrap();
    let current = crate::services::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let inbound = parse_event(
        Inbox::Instagram,
        &current.id,
        &current.platform_bot_id,
        &event("222", "333", "generation-mid"),
    )
    .unwrap();
    instagram
        .authorize_inbound(
            &state.db,
            &reqwest::Client::new(),
            &state.encryption_keys,
            &current,
            &inbound,
        )
        .await
        .unwrap();
    let control = state
        .db
        .collection::<bson::Document>(CONTROLS)
        .find_one(doc! {"identity_key": &key})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(control.get_i64("generation").unwrap(), 2);
    assert_eq!(control.get_bool("controlled").unwrap(), true);
}

#[tokio::test]
async fn instagram_control_loss_stays_sticky_across_generation_and_next_dm() {
    let (state, adapter, _, bot) = fixture("instagram").await;
    let key = format!(
        "{}:{}",
        bot.id,
        conversation("instagram", &bot.id, &bot.platform_bot_id, "333")
    );
    state
        .db
        .collection::<bson::Document>(CONTROLS)
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "identity_key": &key,
            "bot_id": &bot.id,
            "generation": 1_i64,
            "revision": 4_i64,
            "controlled": false,
        })
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$inc": {"meta_binding.generation": 1_i64}},
        )
        .await
        .unwrap();
    let current = crate::services::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let inbound = parse_event(
        Inbox::Instagram,
        &current.id,
        &current.platform_bot_id,
        &event("222", "333", "loss-followup"),
    )
    .unwrap();
    let error = adapter
        .authorize_inbound(
            &state.db,
            &reqwest::Client::new(),
            &state.encryption_keys,
            &current,
            &inbound,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("inbox_control_lost"));
    let control = state
        .db
        .collection::<bson::Document>(CONTROLS)
        .find_one(doc! {"identity_key": &key})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(control.get_bool("controlled").unwrap(), false);
    assert_eq!(control.get_i64("generation").unwrap(), 1);
    assert_eq!(
        state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn operator_hold_survives_grant_repair_until_explicit_resume() {
    let (state, _adapter, _, bot) = fixture("instagram").await;
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {
                "meta_binding.operator_suspended": true,
                "meta_binding.subscription": "suspended",
                "status": "suspended",
            }},
        )
        .await
        .unwrap();
    let held = crate::services::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(require_verifiable(&held).is_err());
    assert!(require_resumable(&held).is_ok());
    assert!(
        require_ingress(
            &state.db,
            &held,
            &conversation("instagram", &held.id, &held.platform_bot_id, "333")
        )
        .await
        .is_err()
    );
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &held.id},
            doc! {"$set": {"meta_binding.grant_reconnect_required": true}},
        )
        .await
        .unwrap();
    let expired = crate::services::channel_bot_service::get_bot(&state.db, &held.id)
        .await
        .unwrap();
    assert!(require_resumable(&expired).is_err());
}

#[tokio::test]
async fn professional_account_wire_casing_and_own_account_id_are_verified() {
    let (_, a, server, _) = fixture("instagram").await;
    for kind in [
        "Business",
        "Media_Creator",
        "BUSINESS",
        "MEDIA_CREATOR",
        "PERSONAL",
    ] {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/v25.0/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data":[{"id":"999","user_id":"222","username":"fixture","account_type":kind}]}),
            ))
            .mount(&server)
            .await;
        let result = a
            .verify_bot_token(
                &reqwest::Client::new(),
                &BotCredentials::from("fixture-token"),
            )
            .await;
        assert_eq!(result.is_ok(), kind != "PERSONAL");
        if let Ok(identity) = result {
            assert_eq!(identity.platform_bot_id, "222");
        }
    }
}

#[tokio::test]
async fn atomic_admission_has_one_winner_and_stores_only_authority_metadata() {
    let (state, _, _, bot) = fixture("instagram").await;
    let first = original(&state, &bot).await;
    let mut duplicate = first.clone();
    duplicate.id = uuid::Uuid::new_v4().to_string();
    let (a, b) = tokio::join!(
        channel_admission_service::store(&state.db, first),
        channel_admission_service::store(&state.db, duplicate)
    );
    assert_eq!(
        usize::from(a.unwrap().is_some()) + usize::from(b.unwrap().is_some()),
        1
    );
    let stored = state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let encoded = serde_json::to_string(&stored).unwrap();
    assert!(!encoded.contains("private fixture text"));
    assert!(!encoded.contains("fixture-token"));
    assert!(
        stored
            .get_document("meta_authority")
            .unwrap()
            .get_datetime("event_at")
            .is_ok()
    );
}

#[tokio::test]
async fn windows_generation_and_actual_credential_provenance_fail_before_dispatch() {
    let (state, a, server, bot) = fixture("instagram").await;
    let original = original(&state, &bot).await;
    let chat = &format!("instagram:{}:222:333", bot.id);
    let credentials = BotCredentials::from("fixture-token");
    assert!(
        require_reply(&state.db, &bot, &original, chat, &credentials)
            .await
            .is_ok()
    );
    let mut expired = original.clone();
    expired.meta_authority.as_mut().unwrap().event_at = Utc::now() - Duration::hours(24);
    assert!(matches!(require_reply(&state.db,
    &bot,
    &expired,
    chat,
    &credentials).await,
    Err(AppError::ChannelConversationNotReachable(reason)) if reason=="meta:window_closed"));
    let mut token_expired = original.clone();
    token_expired
        .meta_authority
        .as_mut()
        .unwrap()
        .reply_expires_at = Utc::now();
    assert!(matches!(require_reply(&state.db,
    &bot,
    &token_expired,
    chat,
    &credentials).await,
    Err(AppError::ChannelConversationNotReachable(reason)) if reason=="meta:reply_token_expired"));
    assert!(
        require_reply(
            &state.db,
            &bot,
            &original,
            chat,
            &BotCredentials::from("other-token")
        )
        .await
        .is_err()
    );
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$inc":{"meta_binding.generation":1}},
        )
        .await
        .unwrap();
    assert!(
        require_reply(&state.db, &bot, &original, chat, &credentials)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id":&bot.user_id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        require_reply(&state.db, &bot, &original, chat, &credentials)
            .await
            .is_err()
    );
    assert!(
        a.send_reply(
            &state.http_client,
            &credentials,
            chat,
            &OutboundReply {
                attachments: vec![],
                text: Some("unsafe".into()),
                reply_to_platform_message_id: None,
                metadata: None
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn typed_provider_outcomes_preserve_refusal_throttle_and_uncertainty() {
    let (state, a, server, bot) = fixture("instagram").await;
    let original = original(&state, &bot).await;
    let reply = OutboundReply {
        attachments: vec![],
        text: Some("fixture reply".into()),
        reply_to_platform_message_id: None,
        metadata: None,
    };
    for (status, body, uncertain, accepted) in [
        (200, json!({"message_id":"accepted"}), false, true),
        (
            400,
            json!({"error":{"code":200,"message":"private provider text"}}),
            false,
            false,
        ),
        (429, json!({"error":{"code":4}}), false, false),
        (503, json!({"error":{"code":2}}), true, false),
        (200, json!({"success":true}), true, false),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/v25.0/222/messages"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let result = a
            .send_bound_reply_outcome(
                &state.db,
                &state.http_client,
                &bot,
                &original,
                &BotCredentials::from("fixture-token"),
                &format!("instagram:{}:222:333", bot.id),
                &reply,
                None,
            )
            .await
            .unwrap();
        let record = result.observation.unwrap();
        assert_eq!(record.uncertain, uncertain);
        assert_eq!(record.complete, accepted);
        assert_eq!(result.final_id.is_some(), accepted);
        assert_eq!(result.error.is_none(), accepted);
        if let Some(error) = result.error {
            assert!(!error.to_string().contains("private provider text"));
            if uncertain {
                assert!(error.to_string().contains("acceptance is unknown"));
            }
        }
        server.verify().await;
    }
}

#[test]
fn instagram_multipart_is_lossless_and_never_exceeds_utf8_budget() {
    let text = "🦉界 text\n\n".repeat(1500);
    let limit = crate::services::channel_text::TextLimit::for_platform("instagram");
    let parts: Vec<_> = limit.chunks(&text).collect();
    assert_eq!(parts.concat(), text);
    assert!(parts.iter().all(|p| p.len() <= 1000));
}

#[test]
fn instagram_account_envelopes_are_unambiguous_and_numeric_subjects_are_lossless() {
    for value in [
        json!({}),
        json!({"data":[]}),
        json!({"data":[{},{}]}),
        json!({"data":[null]}),
        json!({"access_token":"flat"}),
    ] {
        assert!(instagram_record(&value).is_err(), "{value}");
    }
    let response = json!({"data":[{"user_id":18446744073709551615_u64}]});
    assert_eq!(
        identity_value(&instagram_record(&response).unwrap()["user_id"]).as_deref(),
        Some("18446744073709551615")
    );
    assert!(identity_value(&json!(900.5)).is_none());
    assert_eq!(identity_value(&json!("900")).as_deref(), Some("900"));
}

#[tokio::test]
async fn signed_control_loss_stops_a_queued_multipart_reply_after_its_accepted_prefix() {
    multipart_fence("control").await;
}

#[tokio::test]
async fn multipart_reply_stops_when_the_nyxid_token_expires_after_the_accepted_prefix() {
    multipart_fence("reply_token").await;
}

#[tokio::test]
async fn multipart_reply_stops_when_the_meta_window_closes_after_the_accepted_prefix() {
    multipart_fence("window").await;
}

#[tokio::test]
async fn multipart_reply_cannot_adopt_a_rotated_credential_after_the_accepted_prefix() {
    multipart_fence("credential").await;
}

async fn multipart_fence(scenario: &str) {
    use crate::models::channel_reply_delivery::{
        COLLECTION_NAME, ChannelReplyDelivery, PartStatus,
    };
    let (state, a, server, bot) = fixture("instagram").await;
    let mut original = original(&state, &bot).await;
    if scenario == "reply_token" {
        original.meta_authority.as_mut().unwrap().reply_expires_at =
            Utc::now() + Duration::milliseconds(500);
    } else if scenario == "window" {
        original.meta_authority.as_mut().unwrap().event_at =
            Utc::now() - Duration::hours(24) + Duration::milliseconds(500);
    }
    let key = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .insert_one(doc! {"_id":&key,"is_active":true})
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/v25.0/222/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(750))
                .set_body_json(json!({"message_id":"prefix-accepted"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let chat = format!("instagram:{}:222:333", bot.id);
    let reply = OutboundReply {
        text: Some("part".into()),
        attachments: vec![],
        reply_to_platform_message_id: None,
        metadata: None,
    };
    let credentials = BotCredentials::from("fixture-token");
    let send = crate::services::channel_reply_delivery::send_parts(
        &state.db,
        &state.http_client,
        &a,
        &bot,
        &original,
        &key,
        &chat,
        &credentials,
        vec![reply.clone(), reply],
    );
    let control_loss = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while server.received_requests().await.unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if scenario == "credential" {
            state.db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME).update_one(
                doc! {"_id":&bot.id}, doc! {"$inc":{"meta_binding.generation":1_i64}, "$set":{"meta_binding.token_fingerprint":token_fingerprint("successor-token")}},
            ).await.unwrap();
            return;
        }
        if scenario != "control" {
            return;
        }
        let body = payload(
            Inbox::Instagram,
            vec![json!({"id":"222","standby":[event("222","333","standby")]})],
        );
        a.prepare_webhook(
            &bot,
            Some(&PlatformVerifySecrets::from([(
                "app_secret",
                "fixture-signing-secret",
            )])),
            &signed(&body),
            &body,
        )
        .await
        .unwrap();
        a.apply_webhook_state(
            &state.db,
            &bot,
            &body,
            &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
        )
        .await
        .unwrap();
        assert!(require_ingress(&state.db, &bot, &chat).await.is_err());
    };
    let (result, ()) = tokio::join!(send, control_loss);
    let reason = match scenario {
        "control" => "meta:inbox_control_lost",
        "reply_token" => "meta:reply_token_expired",
        "window" => "meta:window_closed",
        _ => "meta:credential_authority_changed",
    };
    assert!(
        matches!(result, Err(AppError::ChannelConversationNotReachable(actual)) if actual == reason)
    );
    let journal = state
        .db
        .collection::<ChannelReplyDelivery>(COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(!journal.complete);
    assert!(journal.parts[0].status == PartStatus::Accepted);
    assert_eq!(journal.parts[0].platform_message_ids, ["prefix-accepted"]);
    assert!(journal.parts[1].status == PartStatus::Failed);
    server.verify().await;
}

#[test]
fn instagram_direct_and_array_shapes_require_one_consistent_account() {
    let direct = json!({"access_token":"token", "user_id":"900", "permissions":"instagram_business_basic,instagram_business_manage_messages"});
    assert_eq!(instagram_record(&direct).unwrap(), &direct);
    let identity = json!({"user_id":777, "account_type":"Media_Creator", "username":"fixture"});
    assert_eq!(instagram_record(&identity).unwrap(), &identity);
    let wrapped = json!({"data":[direct.clone()],"user_id":"900"});
    assert_eq!(instagram_record(&wrapped).unwrap(), &direct);
    for conflict in [
        json!({"data":[direct.clone()],"user_id":"901"}),
        json!({"data":[direct],"access_token":"other"}),
        json!({"data":[identity.clone(),identity.clone()]}),
        json!({"data":[],"user_id":777,"account_type":"Business"}),
        json!({"access_token":"token","user_id":"900"}),
        json!({"user_id":"777"}),
    ] {
        assert!(instagram_record(&conflict).is_err(), "{conflict}");
    }
}

#[tokio::test]
async fn response_disconnect_keeps_unknown_acceptance_in_the_journal_without_sending_the_suffix() {
    use crate::models::channel_reply_delivery::{
        COLLECTION_NAME, ChannelReplyDelivery, PartStatus,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (state, mut a, _, bot) = fixture("instagram").await;
    let original = original(&state, &bot).await;
    let key = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .insert_one(doc! {"_id":&key,"is_active":true})
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    a.graph_base = format!("http://{}", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        let (mut connection, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 4096];
        let count = connection.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        // The provider may have accepted the request; the body is truncated.
        connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{\"message_id\":\"").await.unwrap();
        connection.shutdown().await.unwrap();
    });
    let reply = OutboundReply {
        text: Some("fixture".into()),
        attachments: vec![],
        reply_to_platform_message_id: None,
        metadata: None,
    };
    let result = crate::services::channel_reply_delivery::send_parts(
        &state.db,
        &state.http_client,
        &a,
        &bot,
        &original,
        &key,
        &format!("instagram:{}:222:333", bot.id),
        &BotCredentials::from("fixture-token"),
        vec![reply.clone(), reply],
    )
    .await;
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("acceptance is unknown")
    );
    provider.await.unwrap();
    let journal = state
        .db
        .collection::<ChannelReplyDelivery>(COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(!journal.complete);
    assert!(journal.parts[0].status == PartStatus::Uncertain);
    assert!(journal.parts[1].status == PartStatus::NotAttempted);
}

#[tokio::test]
async fn idempotent_control_reads_retry_transient_graph_failures_with_actionable_bounds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v25.0/read"))
        .respond_with(ResponseTemplate::new(503).set_body_string("temporarily unavailable"))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v25.0/read"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    let value = graph_json_read(reqwest::Client::new().get(format!("{}/v25.0/read", server.uri())))
        .await
        .unwrap();
    assert_eq!(value["ok"], true);
    Mock::given(method("GET"))
        .and(path("/v25.0/throttled"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        graph_json_read(reqwest::Client::new().get(format!("{}/v25.0/throttled", server.uri())))
            .await,
        Err(AppError::RateLimitedWithRetry { .. })
    ));
    server.verify().await;
}

#[tokio::test]
async fn stale_standby_after_reconnect_cannot_close_instagram_control() {
    let (state, adapter, _, mut bot) = fixture("instagram").await;
    let installed = Utc::now() - Duration::seconds(5);
    let started = installed - Duration::minutes(3);
    let before_install = installed - Duration::seconds(1);
    let binding = bot.meta_binding.as_mut().unwrap();
    binding.authorized_at = Some(started);
    binding.events_since = Some(installed);
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set":{
                "meta_binding.authorized_at": bson::DateTime::from_chrono(started),
                "meta_binding.events_since": bson::DateTime::from_chrono(installed),
            }},
        )
        .await
        .unwrap();
    for timestamp in [
        Value::Null,
        json!(started.timestamp_millis() - 1000),
        json!(before_install.timestamp_millis()),
        json!((Utc::now() + Duration::days(1)).timestamp_millis()),
    ] {
        let mut loss = event("222", "333", "old");
        loss["timestamp"] = timestamp;
        adapter
            .apply_webhook_state(
                &state.db,
                &bot,
                &payload(adapter.inbox, vec![json!({"id":"222","standby":[loss]})]),
                &crate::services::channel_verification_service::CredentialSnapshot::local(&bot),
            )
            .await
            .unwrap();
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(CONTROLS)
            .count_documents(doc! {"bot_id": &bot.id})
            .await
            .unwrap(),
        0
    );
    let mut dm = event("222", "333", "before-install");
    dm["timestamp"] = json!(before_install.timestamp_millis());
    let inbound = parse_event(adapter.inbox, &bot.id, "222", &dm).unwrap();
    assert!(matches!(
        adapter.authorize_inbound(&state.db, &state.http_client, &state.encryption_keys, &bot, &inbound).await,
        Err(AppError::ChannelConversationNotReachable(reason)) if reason == "meta:credential_authority_changed"
    ));
}

#[tokio::test]
async fn meta_erasure_fences_journal_creation_but_disconnect_keeps_an_accepted_receipt() {
    let (state, _, _, bot) = fixture("instagram").await;
    let inbound = original(&state, &bot).await;
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(doc! {"_id": &bot.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let store = || {
        crate::services::channel_relay_service::store_outbound_message(
            &state.db,
            &bot.id,
            "conversation",
            &bot.user_id,
            &bot.platform,
            "agent",
            Some(&inbound.id),
            Some("confirmed-id"),
            Some("participant"),
            "text",
            None,
        )
    };
    assert_eq!(
        store().await.unwrap().platform_message_id.as_deref(),
        Some("confirmed-id")
    );
    state
        .db
        .collection::<bson::Document>("channel_meta_erasure_fences")
        .insert_one(doc! {"_id":&bot.id})
        .await
        .unwrap();
    assert!(store().await.is_err());
    assert!(
        crate::services::channel_reply_delivery::Delivery::begin(&state.db, &bot, &inbound, 1)
            .await
            .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
            .count_documents(doc! {"channel_bot_id":&bot.id})
            .await
            .unwrap(),
        0
    );
}
