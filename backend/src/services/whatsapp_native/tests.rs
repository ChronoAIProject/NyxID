use super::*;
use mongodb::bson::{self, doc};

pub(super) fn bot() -> ChannelBot {
    bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": uuid::Uuid::new_v4().to_string(),
        "platform": "whatsapp", "label": "WhatsApp", "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1] },
        "app_secret_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![2] },
        "platform_bot_id": "333", "platform_bot_username": "+15551234567", "app_id": "222", "webhook_registered": true,
        "webhook_secret_hash": "relay-hash-only", "status": "active", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).unwrap()
}
pub(super) fn config() -> NativeWhatsapp {
    NativeWhatsapp {
        bot_id: "bot".into(),
        owner_id: "owner".into(),
        ownership_version: 0,
        meta_app_id: "111".into(),
        waba_id: "222".into(),
        phone_number_id: "333".into(),
        dedicated_app: true,
        native_verify_token_encrypted: None,
        revision: 1,
        updated_at: Utc::now(),
        callback: CallbackState {
            phase: CallbackPhase::Gateway,
            gateway_channel_id: Some("channel".into()),
            gateway_verified_at: Some(Utc::now()),
            ..Default::default()
        },
    }
}
pub(super) fn context() -> Value {
    json!({"channel_id":"channel", "event_ref":"opaque-secret", "activity":{
        "event_id":"wa-native-message", "source":{"type":"whatsapp", "app_id":"111", "waba_id":"222", "phone_number_id":"333"},
        "actor":{"kind":"unknown", "id":"0015551234567"},
        "conversation":{"kind":"private", "id":"0015551234567"},
        "kind":{"type":"message", "text":"hello", "attachments":[]}}})
}

#[test]
fn identity_is_explicit_and_managed_bots_are_ineligible() {
    let mut bot = bot();
    let mut config = config();
    config.bot_id = bot.id.clone();
    config.owner_id = bot.user_id.clone();
    let source = source(&config, &bot).unwrap();
    assert_eq!(source["app_id"], "111");
    assert_eq!(source["waba_id"], "222");
    assert!(source.get("group_ids").is_none());
    for kind in ["platform", "connection", "unknown"] {
        bot.credential_source = kind.into();
        assert_eq!(eligibility(&bot), Err("dedicated_byo_app_required"));
        assert!(super::source(&config, &bot).is_err());
    }
}

#[test]
fn verified_account_preserves_wa_id_but_never_accepts_controls_groups_or_wrong_custody() {
    let bot = bot();
    let mut config = config();
    config.bot_id = bot.id.clone();
    config.owner_id = bot.user_id.clone();
    let original = context();
    assert_eq!(
        verified_account(&config, &bot, "channel", &original),
        Some("0015551234567")
    );
    for (path, value) in [
        ("/channel_id", json!("other")),
        ("/activity/source/type", json!("nyxid_relay")),
        ("/activity/source/app_id", json!("222")),
        ("/activity/source/waba_id", json!("111")),
        ("/activity/source/phone_number_id", json!("444")),
        ("/activity/actor/kind", json!("human")),
        ("/activity/actor/id", json!("+1555")),
        ("/activity/conversation/kind", json!("group")),
        ("/activity/conversation/id", json!("group")),
        ("/activity/kind/type", json!("event")),
        ("/activity/kind/type", json!("interaction")),
        ("/event_ref", json!("")),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(path).unwrap() = value;
        assert_eq!(
            verified_account(&config, &bot, "channel", &changed),
            None,
            "{path}"
        );
    }
    config.ownership_version += 1;
    assert_eq!(verified_account(&config, &bot, "channel", &original), None);
}

#[test]
fn media_feedback_needs_no_attachment_fetch_or_locator_parsing() {
    let mut activity = context()["activity"].clone();
    assert!(!has_unavailable_media(&activity));
    activity["kind"]["attachments"] = json!([{"url":"http://169.254.169.254/secrets"}]);
    assert!(has_unavailable_media(&activity));
    assert!(MEDIA_UNAVAILABLE.contains("cannot be received or sent"));
}

#[test]
fn atomic_bundle_and_explicit_capability_negotiation() {
    let creds = Credentials::new("a".repeat(16), "b".repeat(20), "c".repeat(32)).unwrap();
    assert!(!format!("{creds:?}").contains(&"a".repeat(16)));
    let wire = serde_json::to_value(creds).unwrap();
    assert_eq!(wire.as_object().unwrap().len(), 3);
    assert!(Credentials::new("a".repeat(16), "b".repeat(20), "relay-hash".into()).is_err());
    assert!(!supported(&json!({})));
    assert!(!supported(
        &json!({"relay_authoring":{"native_whatsapp_writes_enabled":true}})
    ));
    assert!(supported(
        &json!({"relay_authoring":{"native_whatsapp_writes_enabled":true,"native_whatsapp_auto_activation_enabled":true,"multi_consumer_writes_enabled":true}})
    ));
}

#[test]
fn native_outbound_templates_and_url_metadata_cannot_escape_text_only_policy() {
    use crate::services::channel_platform::OutboundReply;
    let mut reply = OutboundReply {
        attachments: Vec::new(),
        text: Some("hello".into()),
        reply_to_platform_message_id: None,
        metadata: None,
    };
    assert_eq!(text_reply(&reply).unwrap(), "hello");
    for metadata in [
        json!({"template":{"name":"test"}}),
        json!({"image":{"link":"https://arbitrary.example"}}),
    ] {
        reply.metadata = Some(metadata);
        assert!(text_reply(&reply).is_err());
    }
}

#[test]
fn activation_failures_use_a_durable_fifteen_minute_backoff() {
    let mut config = config();
    assert!(!activation_backoff_active(&config));
    config.callback.activation_retry_at = Some(Utc::now() + chrono::Duration::minutes(15));
    assert!(activation_backoff_active(&config));
    config.callback.activation_retry_at = Some(Utc::now() - chrono::Duration::seconds(1));
    assert!(!activation_backoff_active(&config));
}

#[test]
fn native_whatsapp_long_replies_are_split_without_truncation() {
    let text = "x".repeat(4_097);
    let reply = crate::services::channel_platform::OutboundReply {
        attachments: Vec::new(),
        text: Some(text.clone()),
        reply_to_platform_message_id: None,
        metadata: None,
    };
    let parts = crate::services::channel_reply_plan::text_parts(&reply, "whatsapp").unwrap();
    assert_eq!(parts.len(), 2);
    assert!(parts.iter().all(|part| {
        crate::services::channel_text::TextLimit::for_platform("whatsapp")
            .measure(part.text.as_deref().unwrap())
            <= 4_096
    }));
    assert_eq!(
        parts
            .iter()
            .map(|part| part.text.as_deref().unwrap())
            .collect::<String>(),
        text
    );
}

#[test]
fn additive_state_defaults_and_unknown_phases_fail_closed() {
    let mut document = bson::to_document(&config()).unwrap();
    document.remove("callback");
    let legacy: NativeWhatsapp = bson::from_document(document).unwrap();
    assert_eq!(legacy.callback.phase, CallbackPhase::Relay);
    let unknown: CallbackState = bson::from_document(doc! {"phase":"future_phase"}).unwrap();
    assert_eq!(unknown.phase, CallbackPhase::Unknown);
    assert!(!format!("{:?}", config()).contains("channel"));
}

#[tokio::test]
async fn begin_restore_exposes_owner_rollback_phase_with_a_revision_fence() {
    let db = crate::test_utils::connect_transaction_test_database("wa_begin_restore").await;
    let mut bot = bot();
    let mut config = config();
    config.bot_id = bot.id.clone();
    config.owner_id = bot.user_id.clone();
    db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    db.collection::<NativeWhatsapp>(COLLECTION_NAME)
        .insert_one(&config)
        .await
        .unwrap();
    begin_restore(&db, &bot).await.unwrap();
    let restored = load(&db, &bot).await.unwrap().unwrap();
    assert_eq!(restored.callback.phase, CallbackPhase::AwaitingRelay);
    assert_eq!(restored.revision, config.revision + 1);
    begin_restore(&db, &bot).await.unwrap();
    let unchanged = load(&db, &bot).await.unwrap().unwrap();
    assert_eq!(unchanged.revision, restored.revision);
    bot.ownership_version += 1;
    assert!(begin_restore(&db, &bot).await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn settings_use_revision_fence_preserve_relay_and_block_changes_during_cutover() {
    let db = crate::test_utils::connect_transaction_test_database("wa_native").await;
    let bot = bot();
    db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    let before = bson::to_document(&bot).unwrap();
    let config = configure(&db, &bot, "111", "222", true, 0).await.unwrap();
    assert!(configure(&db, &bot, "111", "222", true, 0).await.is_err());
    assert!(configure(&db, &bot, "111", "999", true, 1).await.is_err());
    db.collection::<NativeWhatsapp>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set":{"callback.phase":"awaiting_gateway"}},
        )
        .await
        .unwrap();
    assert!(
        configure(&db, &bot, "444", "222", true, config.revision)
            .await
            .is_err()
    );
    let after = db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": &bot.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bson::to_document(&after).unwrap(), before);
    let mut moved = bot;
    moved.ownership_version += 1;
    assert!(load(&db, &moved).await.unwrap().is_none());
    db.drop().await.unwrap();
}
