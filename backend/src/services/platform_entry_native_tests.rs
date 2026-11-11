use super::*;
use crate::models::platform_entry_identity::{EntryTransport, NativeEntryIdentity};
use serde_json::json;

pub(crate) fn configuration(
    bot: &ChannelBot,
    surface: NativeControlSurface,
) -> NativeEntryConfiguration {
    NativeEntryConfiguration {
        identity: NativeEntryIdentity {
            version: 2,
            platform: match bot.platform.as_str() {
                "slack" => EntryPlatform::Slack,
                "discord" => EntryPlatform::Discord,
                "x" => EntryPlatform::X,
                "whatsapp" => EntryPlatform::Whatsapp,
                "lark" => EntryPlatform::Lark,
                "feishu" => EntryPlatform::Feishu,
                _ => panic!("fixture platform"),
            },
            channel_bot_id: bot.id.clone(),
            platform_bot_id: bot.platform_bot_id.clone(),
        },
        transport: EntryTransport::Direct,
        transport_generation: 1,
        credential_fingerprint: "11".repeat(32),
        control_surface: surface,
    }
}

fn slack_headers(body: &[u8]) -> HeaderMap {
    use hmac::{Hmac, Mac};
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"synthetic-signing-key").unwrap();
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
    let mut headers = HeaderMap::new();
    headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
    headers.insert("x-slack-signature", signature.parse().unwrap());
    headers
}

fn slack_message(text: &str) -> Value {
    json!({
        "type":"event_callback", "api_app_id":"A-app", "team_id":"T-team", "event_id":"Ev-message",
        "authorizations":[{"team_id":"T-team","user_id":"U-bot","is_bot":true}],
        "event":{"type":"message","user":"U-person","channel":"D-private","channel_type":"im","ts":"1730000000.000001","text":text}
    })
}

#[tokio::test]
async fn slack_authenticated_dm_controls_and_questions_have_positive_provenance() {
    let adapter = crate::services::channel_adapters::slack::SlackAdapter::new();
    let bot = bot();
    let config = configuration(
        &bot,
        NativeControlSurface::SlackDirectMessage {
            app_id: "A-app".into(),
            team_id: "T-team".into(),
            bot_user_id: "U-bot".into(),
        },
    );
    let secrets = PlatformVerifySecrets::from([("app_secret", "synthetic-signing-key")]);
    for (native, expected) in [
        ("login", "/login"),
        ("status", "/status"),
        ("logout", "/logout"),
        ("LOGIN", "/login"),
        ("Logout", "/logout"),
        ("Confirm p-abcd-efgh", "/confirm p-abcd-efgh"),
        ("confirm P-ABCD-EFGH", "/confirm P-ABCD-EFGH"),
        ("What is NyxID?", "What is NyxID?"),
        (
            "Remember my usual settings 中文",
            "Remember my usual settings 中文",
        ),
    ] {
        let payload = serde_json::to_vec(&slack_message(native)).unwrap();
        let batch = prepare_with_secrets(
            &adapter,
            &bot,
            Some(&secrets),
            &slack_headers(&payload),
            &payload,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(batch.messages().len(), 1);
        let fact = batch.fact(0).unwrap();
        assert_eq!(fact.text(), expected);
        assert_eq!(batch.messages()[0].text.as_deref(), Some(expected));
        assert_eq!(fact.sender.sender_id, "U-person");
        assert_eq!(fact.sender.private_conversation_id, "D-private");
        assert_ne!(
            config.identity.platform_bot_id, "U-bot",
            "Slack B bot identity is not its U authorization identity"
        );
    }
    // Real adapter-parsed file_share events retain human provenance only for
    // fixed help. Even proof-shaped captions cannot become controls or turns.
    for caption in [Some("confirm P-ABCD-EFGH"), None] {
        let mut media = slack_message("");
        media["event"]["subtype"] = json!("file_share");
        media["event"]["files"] = json!([{"id":"F-file","mimetype":"image/png","url_private":"https://files.slack.com/private","name":"image.png"}]);
        if let Some(caption) = caption {
            media["event"]["text"] = json!(caption);
        } else {
            media["event"].as_object_mut().unwrap().remove("text");
        }
        let payload = serde_json::to_vec(&media).unwrap();
        let batch = prepare_with_secrets(
            &adapter,
            &bot,
            Some(&secrets),
            &slack_headers(&payload),
            &payload,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(batch.messages().len(), 1);
        assert_eq!(batch.fact(0).unwrap().text(), "/unsupported");
        assert_eq!(batch.messages()[0].text.as_deref(), Some("/unsupported"));
    }
    let payload = serde_json::to_vec(&slack_message("login")).unwrap();
    let mut headers = slack_headers(&payload);
    headers.insert("x-slack-signature", "v0=invalid".parse().unwrap());
    assert!(
        prepare_with_secrets(&adapter, &bot, Some(&secrets), &headers, &payload, &config)
            .await
            .is_err()
    );
    for (pointer, replacement) in [
        ("/event/channel_type", json!("group")),
        ("/event/user", json!("")),
        ("/api_app_id", json!("A-other")),
        ("/authorizations/0/user_id", json!("U-other")),
    ] {
        let mut value = slack_message("logout");
        *value.pointer_mut(pointer).unwrap() = replacement;
        let payload = serde_json::to_vec(&value).unwrap();
        let batch = prepare_with_secrets(
            &adapter,
            &bot,
            Some(&secrets),
            &slack_headers(&payload),
            &payload,
            &config,
        )
        .await
        .unwrap();
        assert!(batch.fact(0).is_none(), "{pointer}");
    }
    let mut forwarded = slack_message("logout");
    forwarded["event"]["attachments"] = json!([{"is_msg_unfurl":true}]);
    let payload = serde_json::to_vec(&forwarded).unwrap();
    let batch = prepare_with_secrets(
        &adapter,
        &bot,
        Some(&secrets),
        &slack_headers(&payload),
        &payload,
        &config,
    )
    .await
    .unwrap();
    assert!(batch.fact(0).is_none());
}

pub(crate) fn discord_message(command: &str, argument: Option<(&str, &str)>) -> Value {
    let mut selected = json!({"type":1,"name":command});
    if let Some((key, value)) = argument {
        selected["options"] = json!([{"type":3,"name":key,"value":value}]);
    }
    json!({
        "type":2,"id":(((chrono::Utc::now().timestamp_millis()-1_420_070_400_000) as u64)<<22).to_string(),"application_id":"100","context":1,
        "channel_id":"dm-opaque","channel":{"id":"dm-opaque","type":1},
        "user":{"id":"person-opaque","username":"Person"},
        "data":{"type":1,"id":"300","name":"nyx","options":[selected]},
        "token":"synthetic-interaction-token"
    })
}

pub(crate) fn discord_headers(
    key: &ed25519_dalek::SigningKey,
    body: &[u8],
    timestamp: i64,
) -> HeaderMap {
    use ed25519_dalek::Signer;
    let timestamp = timestamp.to_string();
    let mut material = timestamp.as_bytes().to_vec();
    material.extend_from_slice(body);
    let signature = key.sign(&material);
    let mut headers = HeaderMap::new();
    headers.insert("x-signature-timestamp", timestamp.parse().unwrap());
    headers.insert(
        "x-signature-ed25519",
        hex::encode(signature.to_bytes()).parse().unwrap(),
    );
    headers
}

#[tokio::test]
async fn actual_discord_http_ack_authenticates_ping_and_defers_only_signed_fresh_commands() {
    use axum::{
        body::{Bytes, to_bytes},
        extract::{Path, State},
        http::StatusCode,
    };
    use bson::doc;
    let db = crate::test_utils::connect_transaction_test_database("native_discord_http_ack").await;
    let state = crate::test_utils::test_app_state(db);
    let signing = ed25519_dalek::SigningKey::from_bytes(&[17; 32]);
    let mut bot = bot();
    bot.platform = "discord".into();
    bot.public_key = Some(hex::encode(signing.verifying_key().as_bytes()));
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    let ping =
        serde_json::to_vec(&json!({"type":1,"application_id":"100","id":"ping-opaque"})).unwrap();
    let command = serde_json::to_vec(&discord_message("status", None)).unwrap();
    for (body, headers, status, ack) in [
        (
            ping.clone(),
            HeaderMap::new(),
            StatusCode::UNAUTHORIZED,
            None,
        ),
        (
            command.clone(),
            HeaderMap::new(),
            StatusCode::UNAUTHORIZED,
            None,
        ),
        (
            ping.clone(),
            discord_headers(&signing, &ping, chrono::Utc::now().timestamp() - 301),
            StatusCode::UNAUTHORIZED,
            None,
        ),
        (
            ping.clone(),
            discord_headers(&signing, &ping, chrono::Utc::now().timestamp()),
            StatusCode::OK,
            Some(1),
        ),
        (
            command.clone(),
            discord_headers(&signing, &command, chrono::Utc::now().timestamp()),
            StatusCode::OK,
            Some(5),
        ),
    ] {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            crate::handlers::channel_webhooks::channel_webhook(
                State(state.clone()),
                Path(("discord".into(), bot.id.clone())),
                "/api/v1/webhooks/channel/discord/test".parse().unwrap(),
                headers,
                Bytes::from(body),
            ),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), status);
        if let Some(ack) = ack {
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["type"], ack);
        }
    }
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(doc! {"_id":"malformed-discord","platform":"discord"})
        .await
        .unwrap();
    for unavailable in ["malformed-discord", "missing-discord"] {
        let response = crate::handlers::channel_webhooks::channel_webhook(
            State(state.clone()),
            Path(("discord".into(), unavailable.into())),
            "/api/v1/webhooks/channel/discord/test".parse().unwrap(),
            discord_headers(&signing, &ping, chrono::Utc::now().timestamp()),
            Bytes::from(ping.clone()),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication storage/state failures are not invalid signatures"
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::platform_link_intent::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::platform_personal_binding::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn discord_real_chat_input_dm_interactions_validate_controls_options_context_and_age() {
    let adapter = crate::services::channel_adapters::discord::DiscordAdapter::default();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let mut bot = bot();
    bot.platform = "discord".into();
    bot.platform_bot_id = "bot-opaque".into();
    bot.public_key = Some(hex::encode(signing.verifying_key().as_bytes()));
    let config = configuration(
        &bot,
        NativeControlSurface::DiscordCommand {
            application_id: "100".into(),
            command_id: "300".into(),
            command_digest: super::super::platform_entry_discord_controls::schema_digest(),
        },
    );
    for (command, argument, expected) in [
        ("login", None, "/login"),
        ("status", None, "/status"),
        ("logout", None, "/logout"),
        (
            "confirm",
            Some(("code", "P-ABCD-EFGH")),
            "/confirm P-ABCD-EFGH",
        ),
        (
            "ask",
            Some(("question", "What is NyxID?")),
            "What is NyxID?",
        ),
        (
            "ask",
            Some(("question", "Use my usual settings")),
            "Use my usual settings",
        ),
    ] {
        let payload = serde_json::to_vec(&discord_message(command, argument)).unwrap();
        let batch = prepare_with_secrets(
            &adapter,
            &bot,
            None,
            &discord_headers(&signing, &payload, chrono::Utc::now().timestamp()),
            &payload,
            &config,
        )
        .await
        .unwrap();
        let fact = batch.fact(0).unwrap();
        assert_eq!(fact.text(), expected);
        assert_eq!(batch.messages()[0].text.as_deref(), Some(expected));
        assert_eq!(fact.sender.sender_id, "person-opaque");
        assert_eq!(fact.sender.private_conversation_id, "dm-opaque");
        if command == "confirm" {
            assert_ne!(
                adapter.parse_inbound(&payload).await.unwrap()[0]
                    .text
                    .as_deref(),
                Some(expected),
                "the ordinary adapter representation is preserved"
            );
        }
    }
    for (pointer, replacement) in [
        ("/context", json!(0)),
        ("/channel/type", json!(3)),
        ("/user/bot", json!(true)),
        ("/application_id", json!("other-app")),
        ("/data/id", json!("other-command")),
        ("/data/options/0/options/0/name", json!("unknown")),
        ("/data/options/0/options/0/type", json!(4)),
    ] {
        let mut value = discord_message("confirm", Some(("code", "P-ABCD-EFGH")));
        if pointer == "/user/bot" {
            value["user"]["bot"] = replacement;
        } else {
            *value.pointer_mut(pointer).unwrap() = replacement;
        }
        let payload = serde_json::to_vec(&value).unwrap();
        let batch = prepare_with_secrets(
            &adapter,
            &bot,
            None,
            &discord_headers(&signing, &payload, chrono::Utc::now().timestamp()),
            &payload,
            &config,
        )
        .await
        .unwrap();
        assert!(batch.fact(0).is_none(), "{pointer}");
    }
    let payload = serde_json::to_vec(&discord_message("login", None)).unwrap();
    let batch = prepare_with_secrets(
        &adapter,
        &bot,
        None,
        &discord_headers(&signing, &payload, chrono::Utc::now().timestamp() - 301),
        &payload,
        &config,
    )
    .await
    .unwrap();
    assert!(
        batch.fact(0).is_none(),
        "a valid old signature is not fresh command proof"
    );
    let gateway_payload=serde_json::to_vec(&json!({"t":"MESSAGE_CREATE","d":{"id":"event","channel_id":"dm","author":{"id":"person"},"content":"login"}})).unwrap();
    let batch = prepare_with_secrets(
        &adapter,
        &bot,
        None,
        &discord_headers(&signing, &gateway_payload, chrono::Utc::now().timestamp()),
        &gateway_payload,
        &config,
    )
    .await
    .unwrap();
    assert!(
        batch.fact(0).is_none(),
        "a synthetic signed Gateway message cannot become interaction provenance"
    );
}
pub(crate) fn x_headers(body: &[u8]) -> HeaderMap {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"synthetic-x-client-secret").unwrap();
    mac.update(body);
    HeaderMap::from_iter([(
        "x-twitter-webhooks-signature-oauth2".parse().unwrap(),
        format!(
            "sha256={}",
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        )
        .parse()
        .unwrap(),
    )])
}

#[tokio::test]
async fn x_signed_targeted_activity_dm_binds_exact_private_participants_and_original_text() {
    let adapter = crate::services::channel_adapters::x::XAdapter::default();
    let mut bot = bot();
    bot.platform = "x".into();
    bot.platform_bot_id = "10".into();
    let config = configuration(&bot, NativeControlSurface::PrivateMessage);
    let secrets = PlatformVerifySecrets::from([("client_secret", "synthetic-x-client-secret")]);
    let payload = |text: &str, conversation: Option<&str>| {
        let mut event = json!({"type":"message_create","id":"500","message_create":{
            "sender_id":"2","target":{"recipient_id":"10"},"message_data":{"text":text}}});
        if let Some(conversation) = conversation {
            event["message_create"]["dm_conversation_id"] = json!(conversation);
        }
        json!({"data":{"event_type":"dm.received","tag":format!("nyxid:{}",bot.id),"filter":{"user_id":"10"},
            "payload":{"users":{"2":{"data":{"name":"Native sender"}}},"direct_message_events":[event]}}})
    };
    for (text, expected) in [
        ("login", "/login"),
        ("confirm P-ABCD-EFGH", "/confirm P-ABCD-EFGH"),
        ("What is NyxID?", "What is NyxID?"),
        ("Remember my own preferences", "Remember my own preferences"),
    ] {
        let body = serde_json::to_vec(&payload(text, None)).unwrap();
        let batch = prepare_with_secrets(
            &adapter,
            &bot,
            Some(&secrets),
            &x_headers(&body),
            &body,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(batch.messages().len(), 1);
        let fact = batch.fact(0).unwrap();
        assert_eq!(fact.text(), expected);
        assert_eq!(fact.sender.private_conversation_id, "2-10");
    }
    // Signed provider data that names other private participants remains
    // ordinary-compatible but is ineligible for shared sender authority.
    let body = serde_json::to_vec(&payload("logout", Some("2-99"))).unwrap();
    let batch = prepare_with_secrets(
        &adapter,
        &bot,
        Some(&secrets),
        &x_headers(&body),
        &body,
        &config,
    )
    .await
    .unwrap();
    assert_eq!(batch.messages().len(), 1);
    assert!(batch.fact(0).is_none());
    for field in ["tag", "filter"] {
        let mut value = payload("logout", None);
        if field == "tag" {
            value["data"][field] = json!("nyxid:other");
        } else {
            value["data"][field]["user_id"] = json!("99");
        }
        let body = serde_json::to_vec(&value).unwrap();
        assert!(
            prepare_with_secrets(
                &adapter,
                &bot,
                Some(&secrets),
                &x_headers(&body),
                &body,
                &config
            )
            .await
            .is_err()
        );
    }
    let mut value = payload("logout", None);
    value["data"]["payload"]["direct_message_events"][0]["message_create"]["forwarded"] =
        json!(true);
    let body = serde_json::to_vec(&value).unwrap();
    assert!(
        prepare_with_secrets(
            &adapter,
            &bot,
            Some(&secrets),
            &x_headers(&body),
            &body,
            &config
        )
        .await
        .unwrap()
        .fact(0)
        .is_none()
    );
    let body = serde_json::to_vec(&payload("logout", None)).unwrap();
    assert!(
        prepare_with_secrets(
            &adapter,
            &bot,
            Some(&secrets),
            &HeaderMap::new(),
            &body,
            &config
        )
        .await
        .is_err()
    );
}

pub(crate) async fn stored_x_fixture() -> (
    crate::AppState,
    ChannelBot,
    NativeEntryConfiguration,
    String,
) {
    crate::test_utils::init_test_audit_chain_hmac_key();
    use super::super::platform_entry_credentials::EntryCredentialSnapshot;
    use bson::doc;
    let db =
        crate::test_utils::connect_transaction_test_database("native_x_stored_credentials").await;
    let state = crate::test_utils::test_app_state(db.clone());
    super::super::platform_entry_migration::ensure(&db)
        .await
        .unwrap();
    let mut bot = bot();
    bot.platform = "x".into();
    bot.platform_bot_id = "10".into();
    bot.credential_source = "connection".into();
    bot.connection_id = Some(uuid::Uuid::new_v4().to_string());
    let profile_id = uuid::Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &bot.user_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let provider_id = uuid::Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME).insert_one(doc! {
        "_id":&provider_id,"slug":"twitter","name":"X","provider_type":"oauth2","is_active":true,
        "credential_mode":"user","supports_pkce":true,"created_by":&bot.user_id,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
    let token = state
        .encryption_keys
        .encrypt(b"synthetic-user-token")
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME).insert_one(doc! {
        "_id":bot.connection_id.as_ref().unwrap(),"user_id":&bot.user_id,"label":"X","credential_type":"oauth2",
        "credential_source":"platform","provider_config_id":&provider_id,"status":"active","credential_epoch":1_i64,
        "token_scopes":crate::services::channel_adapters::x::REQUIRED_SCOPES.join(" "),
        "access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:token},
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
    let adapter = crate::services::channel_adapters::x::XAdapter::default();
    crate::services::platform_credential_service::update(
        &db,
        &state.encryption_keys,
        &adapter.platform_credentials().unwrap(),
        &bot.user_id,
        &[
            (
                "client_id".into(),
                Some(Zeroizing::new("synthetic-client".into())),
            ),
            (
                "client_secret".into(),
                Some(Zeroizing::new("synthetic-x-client-secret".into())),
            ),
            (
                "app_bearer_token".into(),
                Some(Zeroizing::new("synthetic-app-token".into())),
            ),
            (
                "consumer_secret".into(),
                Some(Zeroizing::new("synthetic-legacy-secret".into())),
            ),
        ]
        .into(),
        false,
    )
    .await
    .unwrap();
    let mut row = bson::to_document(&bot).unwrap();
    row.insert("platform_profile_id", &profile_id);
    row.insert("platform_entry_version", 2);
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
    let mut config = configuration(&bot, NativeControlSurface::PrivateMessage);
    config.credential_fingerprint = EntryCredentialSnapshot::load(&db, &adapter, &bot)
        .await
        .unwrap()
        .fingerprint()
        .unwrap();
    db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME).insert_one(doc! {
        "_id":&profile_id,"channel_bot_id":&bot.id,"manager_bot_id":&bot.platform_bot_id,"identity_version":2,
        "native_entry":bson::to_bson(&config).unwrap(),"enabled":true,"deleted_at":bson::Bson::Null
    }).await.unwrap();
    db.collection::<bson::Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME).insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"platform_config_id":&profile_id,"contract_version":2,
        "platform_entry":bson::to_bson(&config.identity).unwrap(),"manager_authority_epoch":1_i64,"retired":false
    }).await.unwrap();
    (state, bot, config, profile_id)
}

pub(crate) fn stored_x_message(bot: &ChannelBot, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"data":{"event_type":"dm.received","tag":format!("nyxid:{}",bot.id),
        "filter":{"user_id":"10"},"payload":{"direct_message_events":[{"type":"message_create","id":"500",
            "message_create":{"sender_id":"2","target":{"recipient_id":"10"},"message_data":{"text":text}}}]}}})).unwrap()
}

#[tokio::test]
async fn stored_connection_signing_rotation_and_epoch_replacement_refuse_native_receipt_admission()
{
    use bson::doc;
    let adapter = crate::services::channel_adapters::x::XAdapter::default();
    for mutation in [
        "modern-signing",
        "legacy-signing",
        "revoked",
        "owner",
        "provider",
        "credential-epoch",
    ] {
        let (state, bot, config, profile_id) = stored_x_fixture().await;
        let body = stored_x_message(&bot, "logout");
        let batch = super::prepare(
            &state.db,
            &state.encryption_keys,
            &adapter,
            &bot,
            &x_headers(&body),
            &body,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(batch.fact(0).unwrap().text(), "/logout");
        match mutation {
            "modern-signing" | "legacy-signing" => {
                let field = if mutation == "modern-signing" {
                    "client_secret"
                } else {
                    "consumer_secret"
                };
                crate::services::platform_credential_service::update(
                    &state.db,
                    &state.encryption_keys,
                    &adapter.platform_credentials().unwrap(),
                    &bot.user_id,
                    &[(
                        field.into(),
                        Some(Zeroizing::new("rotated-synthetic-key".into())),
                    )]
                    .into(),
                    false,
                )
                .await
                .unwrap();
            }
            "provider" => {
                state
                    .db
                    .collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
                    .update_one(doc! {"slug":"twitter"}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            _ => {
                let (field, value) = match mutation {
                    "revoked" => ("status", bson::Bson::String("revoked".into())),
                    "owner" => (
                        "user_id",
                        bson::Bson::String(uuid::Uuid::new_v4().to_string()),
                    ),
                    _ => ("credential_epoch", bson::Bson::Int64(2)),
                };
                state
                    .db
                    .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":bot.connection_id.as_ref().unwrap()},
                        doc! {"$set":{field:value}},
                    )
                    .await
                    .unwrap();
            }
        }
        let mut session = state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let result = super::super::platform_entry_receipts::record_in_session(
            &state.db,
            &mut session,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            super::super::platform_entry_receipts::NativeReceiptInput {
                profile_id: &profile_id,
                epoch: 1,
                configuration: &config,
                batch: &batch,
                index: 0,
                relay_message_id: &uuid::Uuid::new_v4().to_string(),
            },
        )
        .await;
        assert!(
            result.is_err(),
            "{mutation} must fail before a native admission exists"
        );
        session.abort_transaction().await.unwrap();
        for collection in [
            crate::models::platform_entry_identity::NATIVE_RECEIPTS,
            crate::models::platform_link_intent::COLLECTION_NAME,
            crate::models::platform_personal_binding::COLLECTION_NAME,
            crate::models::platform_manager_ingress::COLLECTION_NAME,
        ] {
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{mutation}: {collection}"
            );
        }
    }
}

#[tokio::test]
async fn real_oauth_refresh_before_admission_preserves_scope_set_identity_and_live_epoch() {
    use bson::doc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };
    let adapter = crate::services::channel_adapters::x::XAdapter::default();
    let (state, bot, config, profile_id) = stored_x_fixture().await;
    let server = MockServer::start().await;
    let scopes = crate::services::channel_adapters::x::REQUIRED_SCOPES;
    let reordered = scopes
        .iter()
        .rev()
        .copied()
        .chain(scopes.first().copied())
        .collect::<Vec<_>>()
        .join("  ");
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains(
            "refresh_token=synthetic-refresh-token",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token":"synthetic-refreshed-access-token",
            "refresh_token":"synthetic-refreshed-refresh-token",
            "expires_in":3600,"scope":reordered
        })))
        .expect(1)
        .mount(&server)
        .await;
    state
        .db
        .collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
        .update_one(
            doc! {"slug":"twitter"},
            doc! {"$set":{"token_url":format!("{}/token",server.uri())}},
        )
        .await
        .unwrap();
    let mut key = state
        .db
        .collection::<crate::models::user_api_key::UserApiKey>(
            crate::models::user_api_key::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":bot.connection_id.as_ref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let refresh_token = crate::services::credential_record_service::encrypt(
        &state.encryption_keys,
        crate::models::user_api_key::COLLECTION_NAME,
        &key.id,
        &key.user_id,
        "refresh_token_encrypted",
        b"synthetic-refresh-token",
    )
    .await
    .unwrap();
    crate::services::credential_record_service::mark_version(
        &mut key.secret_context_versions,
        &state.encryption_keys,
        "refresh_token_encrypted",
    );
    state.db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_one(doc! {"_id":&key.id},doc! {"$set":{
            "refresh_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:refresh_token},
            "secret_context_versions":bson::to_bson(&key.secret_context_versions).unwrap(),
            "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::minutes(1))}})
        .await.unwrap();
    let body = stored_x_message(&bot, "What is NyxID?");
    assert!(
        super::prepare(
            &state.db,
            &state.encryption_keys,
            &adapter,
            &bot,
            &HeaderMap::new(),
            &body,
            &config
        )
        .await
        .is_err(),
        "an unsigned request must not trigger refresh"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    let batch = super::prepare(
        &state.db,
        &state.encryption_keys,
        &adapter,
        &bot,
        &x_headers(&body),
        &body,
        &config,
    )
    .await
    .unwrap();
    let current = state
        .db
        .collection::<crate::models::user_api_key::UserApiKey>(
            crate::models::user_api_key::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":bot.connection_id.as_ref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.credential_epoch, 1);
    assert_eq!(current.token_scopes.as_deref(), Some(reordered.as_str()));
    assert!(current.expires_at.unwrap() > chrono::Utc::now() + chrono::Duration::minutes(30));
    assert_eq!(
        crate::services::credential_record_service::CredentialRecord::key(&current)
            .decrypt(
                &state.encryption_keys,
                "access_token_encrypted",
                current.access_token_encrypted.as_ref().unwrap()
            )
            .await
            .unwrap()
            .as_slice(),
        b"synthetic-refreshed-access-token"
    );
    assert_eq!(
        current.oauth_app_observation.as_ref().unwrap().source,
        "platform"
    );
    assert_eq!(
        super::super::platform_entry_credentials::EntryCredentialSnapshot::load(
            &state.db, &adapter, &bot
        )
        .await
        .unwrap()
        .fingerprint()
        .unwrap(),
        config.credential_fingerprint
    );
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let receipt = super::super::platform_entry_receipts::record_in_session(
        &state.db,
        &mut session,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        super::super::platform_entry_receipts::NativeReceiptInput {
            profile_id: &profile_id,
            epoch: 1,
            configuration: &config,
            batch: &batch,
            index: 0,
            relay_message_id: &uuid::Uuid::new_v4().to_string(),
        },
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    assert_eq!(receipt.receipt().sender.sender_id, "2");
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::platform_entry_identity::NATIVE_RECEIPTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn equivalent_scope_reordering_retains_identity_but_fences_an_already_captured_snapshot() {
    use bson::doc;
    let adapter = crate::services::channel_adapters::x::XAdapter::default();
    let (state, bot, config, profile_id) = stored_x_fixture().await;
    let body = stored_x_message(&bot, "logout");
    let batch = super::prepare(
        &state.db,
        &state.encryption_keys,
        &adapter,
        &bot,
        &x_headers(&body),
        &body,
        &config,
    )
    .await
    .unwrap();
    let reordered = crate::services::channel_adapters::x::REQUIRED_SCOPES
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("  ");
    state
        .db
        .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":bot.connection_id.as_ref().unwrap()},
            doc! {"$set":{"token_scopes":reordered}},
        )
        .await
        .unwrap();
    assert_eq!(
        super::super::platform_entry_credentials::EntryCredentialSnapshot::load(
            &state.db, &adapter, &bot
        )
        .await
        .unwrap()
        .fingerprint()
        .unwrap(),
        config.credential_fingerprint
    );
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        super::super::platform_entry_receipts::record_in_session(
            &state.db,
            &mut session,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            super::super::platform_entry_receipts::NativeReceiptInput {
                profile_id: &profile_id,
                epoch: 1,
                configuration: &config,
                batch: &batch,
                index: 0,
                relay_message_id: &uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .is_err()
    );
    session.abort_transaction().await.unwrap();
    let retry = super::prepare(
        &state.db,
        &state.encryption_keys,
        &adapter,
        &bot,
        &x_headers(&body),
        &body,
        &config,
    )
    .await
    .unwrap();
    session.start_transaction().await.unwrap();
    super::super::platform_entry_receipts::record_in_session(
        &state.db,
        &mut session,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        super::super::platform_entry_receipts::NativeReceiptInput {
            profile_id: &profile_id,
            epoch: 1,
            configuration: &config,
            batch: &retry,
            index: 0,
            relay_message_id: &uuid::Uuid::new_v4().to_string(),
        },
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::platform_entry_identity::NATIVE_RECEIPTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn authenticated_batch_cannot_be_relabelled_to_a_live_entry_or_transport() {
    use crate::models::platform_entry_identity::{EntryPlatform, EntryTransport};
    use bson::doc;
    for substitution in ["other-bot", "other-platform", "same-bot-transport"] {
        let (state, bot, config, profile_id) = stored_x_fixture().await;
        let adapter = crate::services::channel_adapters::x::XAdapter::default();
        let body = stored_x_message(&bot, "logout");
        let batch = super::prepare(
            &state.db,
            &state.encryption_keys,
            &adapter,
            &bot,
            &x_headers(&body),
            &body,
            &config,
        )
        .await
        .unwrap();
        let mut replacement = config.clone();
        let target_profile;
        if substitution == "same-bot-transport" {
            target_profile = profile_id.clone();
            replacement.transport_generation += 1;
            replacement.transport = EntryTransport::NyxagentGateway {
                channel_id: uuid::Uuid::new_v4().to_string(),
                gateway_channel_id: serde_json::from_str::<serde_json::Value>(include_str!(
                    "../../../docs/fixtures/platform-entry-cma/discord-ready-record.json"
                ))
                .unwrap()["channel_id"]
                    .as_str()
                    .unwrap()
                    .into(),
                gateway_version: 1,
            };
            state
                .db
                .collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&profile_id},
                    doc! {"$set":{"native_entry":bson::to_bson(&replacement).unwrap()}},
                )
                .await
                .unwrap();
        } else {
            target_profile = uuid::Uuid::new_v4().to_string();
            let mut target_bot = bot.clone();
            target_bot.id = uuid::Uuid::new_v4().to_string();
            target_bot.platform_bot_id = "11".into();
            replacement.identity.channel_bot_id = target_bot.id.clone();
            replacement.identity.platform_bot_id = target_bot.platform_bot_id.clone();
            if substitution == "other-platform" {
                target_bot.platform = "slack".into();
                target_bot.platform_bot_id = "B-native-second".into();
                target_bot.credential_source = "user".into();
                target_bot.connection_id = None;
                replacement.identity.platform = EntryPlatform::Slack;
                replacement.identity.platform_bot_id = target_bot.platform_bot_id.clone();
                replacement.control_surface = NativeControlSurface::SlackDirectMessage {
                    app_id: "A-second".into(),
                    team_id: "T-second".into(),
                    bot_user_id: "U-second".into(),
                };
            }
            let target_adapter = crate::services::channel_adapters::resolve_adapter(
                &target_bot.platform,
                &state.token_exchange_cache,
            )
            .unwrap();
            replacement.credential_fingerprint =
                super::super::platform_entry_credentials::EntryCredentialSnapshot::load(
                    &state.db,
                    target_adapter.as_ref(),
                    &target_bot,
                )
                .await
                .unwrap()
                .fingerprint()
                .unwrap();
            let mut row = bson::to_document(&target_bot).unwrap();
            row.insert("platform_profile_id", &target_profile);
            row.insert("platform_entry_version", 2);
            state
                .db
                .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
                .insert_one(row)
                .await
                .unwrap();
            state.db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
                .insert_one(doc! {"_id":&target_profile,"channel_bot_id":&target_bot.id,
                    "manager_bot_id":&target_bot.platform_bot_id,"identity_version":2,
                    "native_entry":bson::to_bson(&replacement).unwrap(),"enabled":true,"deleted_at":bson::Bson::Null})
                .await.unwrap();
            state.db.collection::<bson::Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
                .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"platform_config_id":&target_profile,
                    "contract_version":2,"platform_entry":bson::to_bson(&replacement.identity).unwrap(),
                    "manager_authority_epoch":1_i64,"retired":false}).await.unwrap();
        }
        super::super::platform_entry_identity::validate_configuration(&replacement).unwrap();
        let mut session = state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            super::super::platform_entry_receipts::record_in_session(
                &state.db,
                &mut session,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                super::super::platform_entry_receipts::NativeReceiptInput {
                    profile_id: &target_profile,
                    epoch: 1,
                    configuration: &replacement,
                    batch: &batch,
                    index: 0,
                    relay_message_id: &uuid::Uuid::new_v4().to_string(),
                },
            )
            .await
            .is_err(),
            "{substitution}"
        );
        // Commit the refused helper call to expose even speculative fence writes.
        session.commit_transaction().await.unwrap();
        for (collection, field) in [
            (
                crate::models::platform_bot_config::COLLECTION_NAME,
                "native_effect_revision",
            ),
            (
                crate::models::channel_bot::COLLECTION_NAME,
                "platform_entry_effect_revision",
            ),
            (
                crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
                "effect_revision",
            ),
            (
                crate::models::user_api_key::COLLECTION_NAME,
                "platform_entry_effect_revision",
            ),
        ] {
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(collection)
                    .count_documents(doc! {field:{"$exists":true}})
                    .await
                    .unwrap(),
                0,
                "{substitution}: refusal must precede {collection} fence writes"
            );
        }
        for collection in [
            crate::models::platform_entry_identity::NATIVE_RECEIPTS,
            crate::models::platform_link_intent::COLLECTION_NAME,
            crate::models::platform_personal_binding::COLLECTION_NAME,
            crate::models::platform_manager_ingress::COLLECTION_NAME,
        ] {
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{substitution}: {collection}"
            );
        }
    }
}

pub(crate) fn bot() -> ChannelBot {
    ChannelBot {
        manager_instance: None,
        diagnostics: Default::default(),
        x_events: None,
        last_verification: None,
        ownership_version: 0,
        id: uuid::Uuid::new_v4().to_string(),
        user_id: uuid::Uuid::new_v4().to_string(),
        platform: "slack".to_string(),
        label: "Test Slack Bot".to_string(),
        credential_source: "user".to_string(),
        connection_id: None,
        poll_cursor: None,
        poll_lease_until: None,
        last_polled_at: None,
        poll_backoff_until: None,
        poll_error_count: 0,
        last_poll_notice: None,
        error: None,
        registration_pin_encrypted: None,
        webhook_secret_encrypted: None,
        managed_setup: None,
        bot_token_encrypted: vec![0; 16],
        platform_bot_id: "B12345".to_string(),
        platform_bot_username: "testbot".to_string(),
        webhook_registered: true,
        webhook_secret_hash: "unused_for_slack".to_string(),
        app_id: None,
        app_secret_encrypted: None,
        lark_verification_token_encrypted: None,
        lark_encrypt_key_encrypted: None,
        public_key: None,
        status: "active".to_string(),
        is_active: true,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn faq_alias_uses_the_same_trimmed_unicode_boundary() {
    for text in [
        "faq unlink",
        "   faq unlink",
        "\u{2003}faq unlink",
        "\u{2003}  faq unlink",
        "FAQ unlink",
        "\u{2003}FaQ unlink",
    ] {
        assert_eq!(super::text_controls(text), "/faq unlink");
    }
    for (text, expected) in [
        ("LOGIN", "/login"),
        ("Logout", "/logout"),
        ("Confirm p-aaaa-bbbb", "/confirm p-aaaa-bbbb"),
        ("CONFIRM", "/confirm"),
        ("Confirm p-aaaa-bbbb extra", "/confirm p-aaaa-bbbb extra"),
        ("Login to my bank", "Login to my bank"),
        ("Help me write", "Help me write"),
    ] {
        assert_eq!(super::text_controls(text), expected);
    }
    for (question, expected) in [
        ("faq unlink", "/faq unlink"),
        ("FAQ unlink", "/faq unlink"),
        ("\u{2003}FAQ memory", "/faq memory"),
        ("Logout", "Logout"),
        ("Help me write", "Help me write"),
    ] {
        let payload = discord_message("ask", Some(("question", question)));
        assert_eq!(
            super::discord_command_text(&payload["data"]).unwrap(),
            expected
        );
    }
    for question in [
        "/logout",
        "/confirm P-AAAA-BBBB",
        "/faq unlink",
        "confirm p-aaaa-bbbb",
    ] {
        let payload = discord_message("ask", Some(("question", question)));
        assert_eq!(
            super::discord_command_text(&payload["data"]).unwrap(),
            "/unsupported"
        );
    }
}

#[tokio::test]
async fn strict_native_snapshot_and_live_channel_use_context_and_preserve_captured_generation() {
    use crate::services::platform_entry_credentials::EntryCredentialSnapshot;
    use bson::doc;
    let (state, bot, config, _profile) = stored_x_fixture().await;
    let collection = crate::models::user_api_key::COLLECTION_NAME;
    let source = state
        .db
        .collection::<bson::Document>(collection)
        .find_one(doc! {"_id":bot.connection_id.as_deref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert!(
        crate::services::credential_record_service::migrate_snapshot(
            &state.db,
            &state.encryption_keys,
            collection,
            &source,
        )
        .await
        .unwrap()
    );
    let mut strict = crate::test_utils::test_encryption_keys();
    strict.set_allow_legacy_record_reads(false);
    let adapter = crate::services::channel_adapters::x::XAdapter::default();
    let captured = EntryCredentialSnapshot::load(&state.db, &adapter, &bot)
        .await
        .unwrap();
    assert_eq!(
        captured.fingerprint().unwrap(),
        config.credential_fingerprint
    );
    assert_eq!(
        captured.send_token(&strict).await.unwrap().as_str(),
        "synthetic-user-token"
    );
    assert_eq!(
        crate::services::channel_credentials::connection_token(
            &state.db,
            &strict,
            &bot.user_id,
            bot.connection_id.as_deref().unwrap(),
            "twitter",
            crate::services::channel_adapters::x::REQUIRED_SCOPES,
        )
        .await
        .unwrap()
        .as_str(),
        "synthetic-user-token"
    );
    crate::services::user_api_key_service::update_api_key(
        &state.db,
        &strict,
        &bot.user_id,
        bot.connection_id.as_deref().unwrap(),
        None,
        Some("successor-user-token"),
    )
    .await
    .unwrap();
    // Captured authority keeps the captured credential; a later same-ID row
    // cannot substitute a different epoch into an admitted native effect.
    assert_eq!(
        captured.send_token(&strict).await.unwrap().as_str(),
        "synthetic-user-token"
    );
    let mut tx = state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert!(!captured.fence_in_session(&state.db, &mut tx).await.unwrap());
    tx.abort_transaction().await.unwrap();
    state.db.drop().await.unwrap();
}
