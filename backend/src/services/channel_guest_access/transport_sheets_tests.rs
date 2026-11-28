//! Signed Slack events and authenticated X polling feed real hosted MCP calls.
use super::{VerifiedGuestInput, test_support::*, transport};
use crate::{
    models::{channel_bot::ChannelBot, channel_guest_policy::GuestDataPolicy},
    services::{
        assistant_agent_credential_service as credentials,
        assistant_authority_tests::Fixture,
        channel_adapters::{slack::SlackAdapter, x::XAdapter},
        channel_platform::{
            BotCredentials, InboundMessage, PlatformAdapter, PlatformVerifySecrets,
        },
        destination_routing::tests as google,
    },
};
use bson::{Document, doc};
use serde_json::{Value, json};

fn bot(f: &Fixture, platform: &str, id: &str) -> ChannelBot {
    bson::from_document(doc!{"_id":id,"user_id":&f.owner,"platform":platform,"label":"Transport fixture",
        "platform_bot_id":if platform=="slack" {"Ubot"}else{"999"},"platform_bot_username":"fixture",
        "status":"active","is_active":true,"guest_access_v1":true,"guest_access_v1_revision":1_i64,
        "webhook_registered":true,"webhook_secret_hash":"fixture",
        "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap()
}

async fn slack_message(bot: &ChannelBot, event: Value) -> InboundMessage {
    use hmac::{Hmac, Mac};
    let body = serde_json::to_vec(&json!({"type":"event_callback","event":event})).unwrap();
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let secret = "ws6-slack-fixture";
    let mut signature = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    signature.update(format!("v0:{timestamp}:").as_bytes());
    signature.update(&body);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
    headers.insert(
        "x-slack-signature",
        format!("v0={}", hex::encode(signature.finalize().into_bytes()))
            .parse()
            .unwrap(),
    );
    let adapter = SlackAdapter::new();
    adapter
        .verify_webhook(
            bot,
            Some(&PlatformVerifySecrets::from([("app_secret", secret)])),
            &headers,
            &body,
        )
        .await
        .unwrap();
    adapter.parse_inbound(&body).await.unwrap().remove(0)
}

pub(super) async fn exercise(f: &Fixture) {
    let mut cases = Vec::new();
    // C-prefixed private channels use channel_type=group. Both message and
    // app_mention events also legitimately omit channel_type.
    for (index, (kind, channel, channel_type, allowed)) in [
        ("app_mention", "C123", None, true),
        ("app_mention", "G123", None, true),
        ("message", "C123", Some("group"), true),
        ("message", "G123", None, true),
        ("message", "C123", None, true),
        ("message", "D123", Some("im"), false),
        ("message", "D123", None, false),
        ("message", "C123", Some("im"), false),
        ("message", "Q123", None, false),
        ("message", "G123", Some("unknown"), false),
    ]
    .into_iter()
    .enumerate()
    {
        let bot = bot(f, "slack", &format!("ws6-slack-{index}"));
        let mut event = json!({"type":kind,"channel":channel,"user":"Uguest","ts":format!("1700000000.{index:06}"),"text":"A question for the host"});
        if let Some(channel_type) = channel_type {
            event["channel_type"] = json!(channel_type);
        }
        let message = slack_message(&bot, event).await;
        cases.push((bot, message, allowed));
    }
    // Exercise the same legacy poll entry point and token/account binding as
    // production, rather than constructing a normalized X message by hand.
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };
    let server = MockServer::start().await;
    let event = |id: &str, conversation: &str, kind: Option<&str>| {
        let mut value = json!({"id":id,"sender_id":"666","dm_conversation_id":conversation,"text":"A question for the host"});
        if let Some(kind) = kind {
            value["event_type"] = json!(kind);
        }
        value
    };
    Mock::given(method("GET"))
        .and(path("/2/dm_events"))
        .and(header("authorization", "Bearer ws6-x-fixture"))
        .and(query_param("event_types", "MessageCreate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
            event("110","779",Some("ParticipantsJoin")),
            event("109","666-998",Some("MessageCreate")),
            event("108","unknown",Some("MessageCreate")),
            event("107","666-999",Some("MessageCreate")),
            event("106","778",None),event("105","777",Some("MessageCreate"))
        ],"meta":{"result_count":6}})))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = XAdapter {
        api_base: Some(server.uri()),
    };
    let polled = adapter
        .poll_inbound(
            &reqwest::Client::new(),
            &BotCredentials {
                billing: None,
                token: "ws6-x-fixture",
                platform_bot_id: Some("999"),
                platform_secrets: None,
            },
            Some("100"),
        )
        .await
        .unwrap();
    assert_eq!(polled.messages.len(), 6);
    for message in polled.messages {
        let id = message.platform_message_id.clone();
        let bot = bot(f, "x", &format!("ws6-x-{id}"));
        cases.push((bot, message, matches!(id.as_str(), "105" | "106")));
    }
    server.verify().await;
    let policy = f
        .state
        .db
        .collection::<GuestDataPolicy>(crate::models::channel_guest_policy::COLLECTION_NAME)
        .find_one(doc! {"_id":"bot-1"})
        .await
        .unwrap()
        .unwrap();
    let echo = google::Echo::start().await;
    let mut state = f.state.clone();
    state.http_client = echo.client.clone();
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let router = router.with_state(state);
    let preferences = f
        .state
        .db
        .collection::<Document>(crate::models::notification_channel::COLLECTION_NAME);
    assert_eq!(
        preferences
            .update_one(
                doc! {"user_id":&f.owner},
                doc! {"$set":{"approval_required":false}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .scope(echo.client_builder.clone(), async {
            for (bot, message, allowed) in cases {
                Box::pin(check_case(
                    f, &router, &echo, &policy, bot, message, allowed,
                ))
                .await;
            }
        })
        .await;
    assert_eq!(
        preferences
            .update_one(
                doc! {"user_id":&f.owner},
                doc! {"$unset":{"approval_required":""}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
}

async fn check_case(
    f: &Fixture,
    router: &axum::Router,
    echo: &google::Echo,
    policy: &GuestDataPolicy,
    bot: ChannelBot,
    message: InboundMessage,
    allowed: bool,
) {
    let db = &f.state.db;
    let facts = transport::inbound_facts(&bot, &message, None);
    assert_eq!(
        facts.shared_room_verified, allowed,
        "{} {}",
        bot.platform, message.platform_message_id
    );
    let mut wrong = message.clone();
    wrong.sender_platform_id = "other-sender".into();
    assert!(!transport::inbound_facts(&bot, &wrong, None).shared_room_verified);
    wrong = message.clone();
    wrong.conversation_id = "other-destination".into();
    assert!(!transport::inbound_facts(&bot, &wrong, None).shared_room_verified);
    db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    let mut granted = policy.clone();
    granted.id = bot.id.clone();
    granted.bot_id = bot.id.clone();
    db.collection::<GuestDataPolicy>(crate::models::channel_guest_policy::COLLECTION_NAME)
        .insert_one(granted)
        .await
        .unwrap();
    let event = format!("{}-{}", bot.id, message.platform_message_id);
    let destination =
        transport::destination(&message.conversation_id, message.thread_id.as_deref());
    let chat = Box::pin(start_guest_with_input(
        f,
        VerifiedGuestInput {
            bot_id: &bot.id,
            binding_id: &bot.id,
            platform: &bot.platform,
            conversation_id: &destination,
            source_event_id: &event,
            sender_id: facts.sender_id,
            audience: facts.audience,
            shared_room_verified: facts.shared_room_verified,
            meta_grant_id: None,
        },
    ))
    .await;
    let turn = chat.channel_guest.as_ref().unwrap();
    assert_eq!(turn.platform, bot.platform);
    assert_eq!(turn.conversation_id, destination);
    assert_eq!(turn.source_event_id, event);
    if facts.sender_id.is_some() {
        assert_eq!(
            turn.actor.assurance,
            super::GuestAssurance::ProviderVerified
        );
        assert!(turn.matches_sender(&message.sender_platform_id));
    }
    let key = credentials::load_for_conversation(
        db,
        &f.state.encryption_keys,
        &f.owner,
        &chat.conversation_id,
    )
    .await
    .unwrap()
    .unwrap();
    let list = mcp_request(router, key.raw_key.as_str(), "tools/list", json!({})).await;
    let name = "api-google-sheets__sheets_append_values";
    assert_eq!(
        list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == name),
        allowed,
        "{list}"
    );
    let found = mcp_request(
        router,
        key.raw_key.as_str(),
        "tools/call",
        json!({"name":"nyx__search_tools","arguments":{"query":"ws6sheets sheets_append_values"}}),
    )
    .await;
    assert_eq!(found["isError"], false, "{found}");
    let found: Value = serde_json::from_str(found["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        found["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == name),
        allowed,
        "{found}"
    );
    if allowed {
        assert_eq!(found["matches"][0]["name"], name);
    }
    let args = json!({"spreadsheetId":"private_workbook_2704","range":"Hidden_Questions!A:D",
        "valueInputOption":"RAW","insertDataOption":"INSERT_ROWS","body":{"values":[["A question for the host"]]}});
    let before = echo.calls.lock().unwrap().len();
    for call in [
        json!({"name":name,"arguments":args}),
        json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}}),
    ] {
        let result = mcp_request(router, key.raw_key.as_str(), "tools/call", call).await;
        assert_eq!(result["isError"], !allowed, "{result}");
    }
    assert_eq!(
        echo.calls.lock().unwrap().len(),
        before + if allowed { 2 } else { 0 }
    );
    let row = crate::services::assistant_nyxagent::get(db, &f.owner, &chat.conversation_id)
        .await
        .unwrap();
    Box::pin(f.finish_turn(&row)).await;
}
