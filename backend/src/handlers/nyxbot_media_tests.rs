//! Native media authority and admission regressions; real MongoDB, mock providers.
use super::*;
use crate::{
    models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
    services::{
        channel_media_authority::{self as authority, MediaRequest},
        channel_platform::{InboundAttachment, InboundMessage},
    },
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

static INGESTION_ORIGINS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::LazyLock::new(Default::default);

async fn media_source(
    state: &AppState,
    row: &NyxbotChannel,
    id: &str,
    native: &str,
    text: &str,
) -> ChannelMessage {
    let origin = INGESTION_ORIGINS
        .lock()
        .unwrap()
        .get(&row.channel_bot_id)
        .cloned()
        .unwrap_or_else(|| "https://open.larksuite.com".into());
    let inbound = InboundMessage {
        embedded_attachments: Vec::new(),
        embedded_attachments_omitted: 0,
        platform_message_id: native.into(),
        conversation_id: "oc_human".into(),
        conversation_type: "private".into(),
        sender_platform_id: "human".into(),
        sender_display_name: None,
        content_type: "image".into(),
        text: Some(text.into()),
        attachments: vec![InboundAttachment {
            content_type: "image".into(),
            url: format!(
                "{origin}/open-apis/im/v1/messages/{native}/resources/img_receipt?type=image"
            ),
            platform_message_id: Some(native.into()),
            image_key: Some("img_receipt".into()),
            file_key: None,
            filename: None,
            mime_type: None,
            size_bytes: Some(42),
        }],
        reply_to_platform_message_id: None,
        thread_id: None,
        raw_data: json!({"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"human"}}}}),
    };
    let source = crate::services::channel_relay_service::inbound_metadata(
        &row.channel_bot_id,
        row.route_id.as_deref().unwrap(),
        row.bot_owner_id.as_deref().unwrap_or(OWNER),
        &row.platform,
        &inbound,
        &row.route_api_key_id,
        id,
    );
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .insert_one(&source)
        .await
        .unwrap();
    source
}
async fn media_fixture(
    name: &str,
    transport: &str,
) -> (
    AppState,
    NyxbotChannel,
    String,
    Calls,
    tokio::task::JoinHandle<()>,
) {
    let (state, calls, server) = setup(name).await;
    let (mut row, key) = channel(&state, transport).await;
    row.platform = "lark".into();
    row.owner_sender_ids = vec!["human".into()];
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc!{"_id":&row.channel_bot_id},doc!{"$set":{"platform":"lark","bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:state.encryption_keys.encrypt(b"app:secret").await.unwrap()}}}).await.unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.route_id},
            doc! {"$set":{"platform":"lark"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("api_keys")
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"callback_url":"https://example.test/callback"}},
        )
        .await
        .unwrap();
    (state, row, key, calls, server)
}
fn media_request(row: &NyxbotChannel, source: &ChannelMessage) -> MediaRequest {
    MediaRequest {
        channel: row.clone(),
        source_message_id: source.id.clone(),
        chat_id: "oc_human".into(),
        sender_id: "human".into(),
        native_thread_id: None,
        origin: None,
        conversation_id: None,
        agent_id: row.agent_id.clone(),
    }
}

async fn fetch_if_authorized(
    state: &AppState,
    request: &MediaRequest,
    provider: &MockServer,
) -> AppResult<()> {
    let allowed = authority::resolve(&state.db, request).await?;
    let adapter = crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
        &provider.uri(),
        "lark",
    );
    crate::services::channel_media_fetch::fetch(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &allowed.bot,
        &allowed.source.attachments[0],
        &adapter,
        1024,
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn channel_media_authority_refusal_matrix_never_downloads() {
    let (state, row, _, _, server) = media_fixture("media_authority_matrix", "direct").await;
    let source = media_source(&state, &row, "source", "om_receipt", "receipt").await;
    let request = media_request(&row, &source);
    authority::resolve(&state.db, &request).await.unwrap();
    let provider = MockServer::start().await;
    let requests_before_refusal =
        assert_authorized_media_download(&state, &source, &request, &provider).await;
    for case in [
        "cross_sender",
        "cross_chat",
        "cross_thread",
        "cross_owner",
        "cross_bot",
        "wrong_platform",
        "wrong_route_key",
        "wrong_route",
    ] {
        let mut wrong = request.clone();
        match case {
            "cross_sender" => wrong.sender_id = "other".into(),
            "cross_chat" => wrong.chat_id = "other".into(),
            "cross_thread" => wrong.native_thread_id = Some("other".into()),
            "cross_owner" => wrong.channel.user_id = "other".into(),
            "cross_bot" => wrong.channel.channel_bot_id = "other".into(),
            "wrong_platform" => wrong.channel.platform = "telegram".into(),
            "wrong_route_key" => wrong.channel.route_api_key_id = "other".into(),
            "wrong_route" => wrong.channel.route_id = Some("other".into()),
            _ => unreachable!(),
        }
        assert!(
            authority::resolve(&state.db, &wrong).await.is_err(),
            "{case}: authority refusal"
        );
        assert!(
            fetch_if_authorized(&state, &wrong, &provider)
                .await
                .is_err(),
            "{case}"
        );
        assert!(
            provider.received_requests().await.unwrap().len() == requests_before_refusal,
            "{case}"
        );
    }
    for (case, collection, filter, change, restore) in [
        (
            "outbound_row",
            MESSAGES,
            doc! {"_id":&source.id},
            doc! {"direction":"outbound"},
            doc! {"direction":"inbound"},
        ),
        (
            "source_platform",
            MESSAGES,
            doc! {"_id":&source.id},
            doc! {"platform":"feishu"},
            doc! {"platform":"lark"},
        ),
        (
            "owner_revoked",
            USERS,
            doc! {"_id":OWNER},
            doc! {"is_active":false},
            doc! {"is_active":true},
        ),
        (
            "relinked_specialist",
            CHANNELS,
            doc! {"_id":&row.id},
            doc! {"agent_id":Uuid::new_v4().to_string()},
            doc! {"agent_id":bson::Bson::Null},
        ),
        (
            "relinked_route",
            CHANNELS,
            doc! {"_id":&row.id},
            doc! {"route_id":"replacement"},
            doc! {"route_id":&row.route_id},
        ),
        (
            "revoked_route_key",
            "api_keys",
            doc! {"_id":&row.route_api_key_id},
            doc! {"is_active":false},
            doc! {"is_active":true},
        ),
    ] {
        let col = state.db.collection::<bson::Document>(collection);
        col.update_one(filter.clone(), doc! {"$set":change})
            .await
            .unwrap();
        assert!(
            authority::resolve(&state.db, &request).await.is_err(),
            "{case}: authority refusal"
        );
        assert!(
            fetch_if_authorized(&state, &request, &provider)
                .await
                .is_err(),
            "{case}"
        );
        assert!(
            provider.received_requests().await.unwrap().len() == requests_before_refusal,
            "{case}"
        );
        col.update_one(filter, doc! {"$set":restore}).await.unwrap();
    }
    // A higher-priority route supersedes the captured default even if the key remains active.
    let mut route = state
        .db
        .collection::<bson::Document>("channel_conversations")
        .find_one(doc! {"_id":&row.route_id})
        .await
        .unwrap()
        .unwrap();
    route.insert("_id", Uuid::new_v4().to_string());
    route.insert("platform_conversation_id", "oc_human");
    route.insert("default_agent", false);
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .insert_one(route)
        .await
        .unwrap();
    assert!(authority::resolve(&state.db, &request).await.is_err());
    assert!(
        fetch_if_authorized(&state, &request, &provider)
            .await
            .is_err()
    );
    assert!(provider.received_requests().await.unwrap().len() == requests_before_refusal);
    server.abort();
}

#[tokio::test]
async fn channel_media_revoked_org_admin_never_downloads() {
    let (state, mut row, _, _, server) = media_fixture("media_org_authority", "direct").await;
    let org = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let member = Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>("org_memberships").insert_one(doc!{"_id":&member,"org_user_id":&org,"member_user_id":OWNER,"role":"admin","allowed_service_ids":bson::Bson::Null,"created_at":bson::DateTime::now(),"revoked_at":bson::Bson::Null}).await.unwrap();
    row.bot_owner_id = Some(org.clone());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    for (collection, filter) in [
        ("channel_bots", doc! {"_id":&row.channel_bot_id}),
        ("channel_conversations", doc! {"_id":&row.route_id}),
        ("api_keys", doc! {"_id":&row.route_api_key_id}),
    ] {
        state
            .db
            .collection::<bson::Document>(collection)
            .update_one(filter, doc! {"$set":{"user_id":&org}})
            .await
            .unwrap();
    }
    let source = media_source(&state, &row, "source", "om_org_receipt", "receipt").await;
    let request = media_request(&row, &source);
    authority::resolve(&state.db, &request).await.unwrap();
    let provider = MockServer::start().await;
    let requests_before_refusal =
        assert_authorized_media_download(&state, &source, &request, &provider).await;
    state
        .db
        .collection::<bson::Document>(USERS)
        .update_one(doc! {"_id":&org}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(authority::resolve(&state.db, &request).await.is_err());
    assert!(
        fetch_if_authorized(&state, &request, &provider)
            .await
            .is_err()
    );
    assert!(provider.received_requests().await.unwrap().len() == requests_before_refusal);
    state
        .db
        .collection::<bson::Document>(USERS)
        .update_one(doc! {"_id":&org}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(doc! {"_id":&member}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    assert!(authority::resolve(&state.db, &request).await.is_err());
    assert!(
        fetch_if_authorized(&state, &request, &provider)
            .await
            .is_err()
    );
    assert!(provider.received_requests().await.unwrap().len() == requests_before_refusal);
    server.abort();
}

#[tokio::test]
async fn channel_media_shared_fetch_keeps_image_file_audio_video_resources() {
    let (state, row, _, _, server) = media_fixture("media_fetch_kinds", "direct").await;
    let source = media_source(&state, &row, "source", "om_receipt", "receipt").await;
    let allowed = authority::resolve(&state.db, &media_request(&row, &source))
        .await
        .unwrap();
    let provider = MockServer::start().await;
    Mock::given(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"tenant_access_token":"test-token","expire":7200})),
        )
        .mount(&provider)
        .await;
    for kind in ["image", "file", "audio", "video"] {
        let resource_type = if kind == "image" { "image" } else { "file" };
        let resource_key = if kind == "image" {
            "img_receipt"
        } else {
            "file_receipt"
        };
        Mock::given(method("GET"))
            .and(path(format!(
                "/open-apis/im/v1/messages/om_receipt/resources/{resource_key}"
            )))
            .and(query_param("type", resource_type))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"resource bytes")
                    .insert_header("content-type", "application/octet-stream"),
            )
            .mount(&provider)
            .await;
        let mut item = source.attachments[0].clone();
        item.content_type = kind.into();
        item.image_key = (kind == "image").then(|| "img_receipt".into());
        item.file_key = (kind != "image").then(|| "file_receipt".into());
        item.provider_ref = format!(
            "{}/open-apis/im/v1/messages/om_receipt/resources/{resource_key}?type={resource_type}",
            provider.uri()
        );
        let adapter =
            crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                &provider.uri(),
                "lark",
            );
        let fetched = crate::services::channel_media_fetch::fetch(
            &state.db,
            &state.encryption_keys,
            &state.http_client,
            &allowed.bot,
            &item,
            &adapter,
            1024,
        )
        .await
        .unwrap();
        assert_eq!(fetched.bytes.as_ref(), b"resource bytes", "{kind}");
    }
    assert_eq!(
        provider
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "GET")
            .count(),
        4
    );
    server.abort();
}

pub(super) async fn enable_media(state: &AppState) {
    state
        .db
        .collection::<bson::Document>("feature_flag_overrides")
        .insert_one(doc! {
            "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
            "flag_key":crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
            "target_kind":"user","target_key":OWNER,"enabled":true,"updated_by":"test",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn channel_media_gateway_concurrent_duplicates_claim_once_and_keep_same_id_replay() {
    let (state, row, key, calls, server) =
        media_fixture("media_gateway_duplicate", "gateway").await;
    enable_media(&state).await;
    let _provider = ingestion_provider(&state, &row, native_png()).await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(&state, &key, PARTITION).await;
    for id in ["delivery-a", "delivery-b"] {
        media_source(&state, &row, id, "om_same", "receipt").await;
    }
    let make = |id: &str| {
        let mut body = cma_media_event("receipt", "human", id);
        body["event_context"]["activity"]["source"]["platform"] = json!("lark");
        body["event_context"]["activity"]["conversation"]["id"] = json!("oc_human");
        body
    };
    let a = make("delivery-a");
    let b = make("delivery-b");
    let (ra, rb) = tokio::join!(
        respond(&state, &key, &a, "a"),
        respond(&state, &key, &b, "b")
    );
    assert_eq!(ra.status(), StatusCode::OK);
    assert_eq!(rb.status(), StatusCode::OK);
    let (ta, tb) = tokio::join!(body_text(ra), body_text(rb));
    assert_eq!(
        usize::from(ta.contains("Here is your answer"))
            + usize::from(tb.contains("Here is your answer")),
        1
    );
    assert_eq!(calls.lock().await.len(), 1);
    let saved: NyxbotEvent = state
        .db
        .collection(EVENTS)
        .find_one(doc! {"channel_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id":&row.id})
            .await
            .unwrap(),
        1
    );
    assert!(saved.turn_id.is_some());
    assert_eq!((saved.expires_at - saved.created_at).num_days(), 7);
    let media = saved.media.as_ref().unwrap();
    assert_eq!(
        media.key,
        sha256_hex(format!(
            "v1\0{}\0lark\0oc_human\0om_same",
            row.channel_bot_id
        ))
    );
    assert_eq!(media.declared, 1);
    assert_eq!(media.items.len(), 1);
    assert_eq!(media.items[0].index, 0);
    assert!(!media.items[0].embedded);
    assert!(
        !bson::to_document(media)
            .unwrap()
            .to_string()
            .contains("img_receipt")
    );
    let loser = if saved.event_id == "delivery-a" {
        "delivery-b"
    } else {
        "delivery-a"
    };
    let alias = state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {"_id":loser})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        alias.duplicate_of_media_key.as_deref(),
        Some(media.key.as_str())
    );
    check_deliveries(&state).await.unwrap();
    let health = delivery(&state, &row.id).await;
    assert_eq!(health.delivery_status.as_deref(), Some("ok"));
    assert!(health.delivery_error.is_none());
    assert!(health.delivery_notified_at.is_none());
    let winner_body = if saved.event_id == "delivery-a" {
        &a
    } else {
        &b
    };
    let retry = respond(
        &state,
        &key,
        winner_body,
        if saved.event_id == "delivery-a" {
            "a"
        } else {
            "b"
        },
    )
    .await;
    assert_eq!(retry.status(), StatusCode::OK);
    let replayed = body_text(retry).await;
    assert!(replayed.contains("response.completed"));
    assert!(!replayed.contains("response.output_item.done"));
    assert!(!replayed.contains("Here is your answer"));
    assert_eq!(calls.lock().await.len(), 1);
    assert_eq!(
        crate::services::channel_media_events::sweep(&state.db)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
async fn channel_media_flag_off_text_and_guests_do_not_write_claims() {
    let (state, row, _, _, server) = media_fixture("media_no_claim", "direct").await;
    let mut source = media_source(&state, &row, "media-source", "om_media", "receipt").await;
    let request = media_request(&row, &source);
    assert!(
        crate::services::channel_media_events::admission(&state.db, &request)
            .await
            .unwrap()
            .is_none()
    );
    let (_observer, done) = watch_relay_callback(&row.id, &source.id);
    assert_eq!(
        direct_private(&state, &row, &source.id, "human", "").await,
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), done)
        .await
        .unwrap()
        .unwrap();
    let written = state
        .db
        .collection::<bson::Document>(EVENTS)
        .find_one(doc! {"event_id":&source.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!written.contains_key("media"));
    let written: NyxbotEvent = bson::from_document(written).unwrap();
    assert_eq!((written.expires_at - written.created_at).num_hours(), 24);
    let old = admission(&row.id, "old-text");
    let document = bson::to_document(&old).unwrap();
    assert!(!document.contains_key("media"));
    assert!(
        bson::from_document::<NyxbotEvent>(document)
            .unwrap()
            .media
            .is_none()
    );
    enable_media(&state).await;
    assert!(
        crate::services::channel_media_events::admission(&state.db, &request)
            .await
            .unwrap()
            .is_some()
    );
    source.sender_platform_id = Some("guest".into());
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":&source.id}, &source)
        .await
        .unwrap();
    let mut guest = request.clone();
    guest.sender_id = "guest".into();
    assert!(
        authority::source(&state.db, &guest)
            .await
            .unwrap()
            .attachments
            .len()
            == 1
    );
    assert!(
        crate::services::channel_media_events::admission(&state.db, &guest)
            .await
            .unwrap()
            .is_none()
    );
    source.sender_platform_id = Some("human".into());
    source.attachments.clear();
    source.content_type = "text".into();
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":&source.id}, &source)
        .await
        .unwrap();
    assert!(
        crate::services::channel_media_events::admission(&state.db, &request)
            .await
            .unwrap()
            .is_none()
    );
    server.abort();
}

#[tokio::test]
async fn channel_media_interrupted_is_reported_for_duplicate_and_noted_once_atomically() {
    interrupted_media_case(false).await;
    interrupted_media_case(true).await;
}

async fn interrupted_media_case(rebuilt: bool) {
    use crate::models::assistant_conversation::TurnOrigin;
    use crate::services::channel_media_events as media_events;
    let (state, row, _, _, server) = media_fixture(
        if rebuilt {
            "media_interrupted_rebuilt"
        } else {
            "media_interrupted"
        },
        "direct",
    )
    .await;
    enable_media(&state).await;
    let source = media_source(&state, &row, "crashed", "om_crashed", "receipt").await;
    let mut event = admission(&row.id, &source.id);
    event.status = "running".into();
    event.relay_message_id = Some(source.id.clone());
    event.media = media_events::admission(&state.db, &media_request(&row, &source))
        .await
        .unwrap();
    event.created_at = Utc::now() - ChronoDuration::minutes(6);
    event.expires_at = event.created_at + ChronoDuration::days(7);
    if rebuilt {
        let mut retired = row.clone();
        retired.id = Uuid::new_v4().to_string();
        retired.status = "disconnected".into();
        event.channel_id = retired.id.clone();
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .insert_one(retired)
            .await
            .unwrap();
    }
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&event)
        .await
        .unwrap();
    // A busy gateway kept the accepted claim for an identical retry, but the
    // process died before any retry could establish a turn/queue binding.
    let busy_source =
        media_source(&state, &row, "busy-crashed", "om_busy_crashed", "receipt").await;
    let mut busy = event.clone();
    busy.id = Uuid::new_v4().to_string();
    busy.event_id = busy_source.id.clone();
    busy.relay_message_id = Some(busy_source.id.clone());
    busy.status = "retryable".into();
    busy.media = media_events::admission(&state.db, &media_request(&row, &busy_source))
        .await
        .unwrap();
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&busy)
        .await
        .unwrap();
    let source2 = media_source(&state, &row, "redelivery", "om_crashed", "receipt").await;
    let mut duplicate = event.clone();
    duplicate.id = Uuid::new_v4().to_string();
    duplicate.channel_id = row.id.clone();
    duplicate.event_id = source2.id.clone();
    duplicate.relay_message_id = Some(source2.id.clone());
    assert!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .insert_one(&duplicate)
            .await
            .is_err(),
        "the native identity remains claimed across channel rebuilds"
    );
    assert!(
        media_events::record_duplicate(&state.db, &duplicate)
            .await
            .unwrap()
    );
    let chat = chats::record_chat(
        &state,
        &row,
        PARTITION,
        &chats::ChatFacts {
            kind: "private",
            chat_id: "oc_human".into(),
            thread_id: None,
            owner: true,
            title: None,
        },
        Some(&source2.id),
    )
    .await
    .unwrap();
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .update_one(
            doc! {"_id":&source2.id},
            doc! {"$set":{"callback_status":"delivered"}},
        )
        .await
        .unwrap();
    assert_eq!(media_events::sweep(&state.db).await.unwrap(), 2);
    assert_eq!(media_events::sweep(&state.db).await.unwrap(), 0);
    check_deliveries(&state).await.unwrap();
    let health = delivery(&state, &row.id).await;
    assert_eq!(health.delivery_status.as_deref(), Some("interrupted"));
    assert_eq!(health.delivery_error.as_deref(), Some("media_interrupted"));
    assert!(
        health.delivery_notified_at.is_none(),
        "no duplicate resend/remedy advice"
    );
    let origin = ChannelOrigin {
        nyxbot_channel_id: row.id.clone(),
        partition: chat.partition.clone(),
        platform: "lark".into(),
        thread: None,
    };
    // A transaction that is aborted must not consume the notice.
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        media_events::note_in_session(
            &state.db,
            &mut session,
            OWNER,
            std::slice::from_ref(&origin)
        )
        .await
        .unwrap()
    );
    session.abort_transaction().await.unwrap();
    let mut start: engine::TurnStart = (&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        conversation_id: None,
        agent_id: None,
        text: "next owner turn".into(),
        attachment_ids: vec![],
        model: None,
        access_mode: None,
    })
        .into();
    start.origin = TurnOrigin::Channel;
    start.channel = Some(origin.clone());
    start.new_id = Some(Uuid::new_v4().to_string());
    // Guests cannot consume the owner's notice.
    let mut guest = start.clone();
    guest.guest = true;
    let guest_row = engine::begin_turn(&state.db, OWNER, guest, &state.encryption_keys)
        .await
        .unwrap();
    assert!(
        !guest_row
            .active_turn
            .as_ref()
            .unwrap()
            .note
            .as_deref()
            .unwrap_or("")
            .contains(media_events::INTERRUPTED_NOTE)
    );
    settle(&state, &guest_row.id).await;
    start.conversation_id = Some(guest_row.id.clone());
    start.new_id = None;
    let owner = engine::begin_turn(&state.db, OWNER, start.clone(), &state.encryption_keys)
        .await
        .unwrap();
    assert!(
        owner
            .active_turn
            .as_ref()
            .unwrap()
            .note
            .as_deref()
            .unwrap()
            .contains(media_events::INTERRUPTED_NOTE)
    );
    assert!(
        engine::messages(&state.db, OWNER, &owner.id, 100, None)
            .await
            .unwrap()
            .iter()
            .any(|message| message.text.contains(media_events::INTERRUPTED_NOTE))
    );
    settle(&state, &owner.id).await;
    let again = engine::begin_turn(&state.db, OWNER, start, &state.encryption_keys)
        .await
        .unwrap();
    assert!(
        !again
            .active_turn
            .as_ref()
            .unwrap()
            .note
            .as_deref()
            .unwrap_or("")
            .contains(media_events::INTERRUPTED_NOTE)
    );
    // A late worker cannot resurrect the interrupted source.
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        crate::services::channel_turn_delivery::bind_in_session(
            &state.db,
            &mut session,
            &event.id,
            OWNER,
            &owner.id,
            "late"
        )
        .await
        .is_err()
    );
    session.abort_transaction().await.unwrap();
    server.abort();
}

async fn mount_media_provider(provider: &MockServer, native: &str) {
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"tenant_access_token":"test-token","expire":7200})),
        )
        .mount(provider)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/open-apis/im/v1/messages/{native}/resources/img_receipt"
        )))
        .and(query_param("type", "image"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"receipt bytes")
                .insert_header("content-type", "image/png"),
        )
        .expect(1)
        .mount(provider)
        .await;
}

#[tokio::test]
async fn channel_media_embedded_selectors_fetch_from_lark_feishu_adapter_origin() {
    use crate::services::channel_platform::PlatformAdapter;
    let (state, row, _, _, server) = media_fixture("media_embedded_origin", "direct").await;
    let mut bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    for platform in ["lark", "feishu"] {
        let provider = MockServer::start().await;
        mount_media_provider(&provider, "om_receipt").await;
        let adapter =
            crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                &provider.uri(),
                platform,
            );
        let inbound = adapter
            .parse_inbound(include_bytes!(
                "../services/channel_adapters/fixtures/lark_callback/lark-post-in.json"
            ))
            .await
            .unwrap();
        let source = crate::services::channel_relay_service::inbound_metadata(
            &row.channel_bot_id,
            row.route_id.as_deref().unwrap(),
            OWNER,
            platform,
            &inbound[0],
            &row.route_api_key_id,
            "embedded-source",
        );
        assert!(source.attachments.is_empty());
        assert_eq!(source.embedded_attachments.len(), 1);
        assert_eq!(
            source.embedded_attachments[0].provider_ref,
            format!(
                "{}/open-apis/im/v1/messages/om_receipt/resources/img_receipt?type=image",
                provider.uri()
            )
        );
        bot.platform = platform.into();
        let fetched = crate::services::channel_media_fetch::fetch(
            &state.db,
            &state.encryption_keys,
            &state.http_client,
            &bot,
            &source.embedded_attachments[0],
            &adapter,
            1024,
        )
        .await
        .unwrap();
        assert_eq!(fetched.bytes.as_ref(), b"receipt bytes");
        assert_eq!(
            provider
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.method == "GET")
                .count(),
            1
        );
    }
    server.abort();
}

#[tokio::test]
async fn channel_media_relay_post_recomputes_after_ack_and_flag_off_keeps_empty_input() {
    use crate::services::channel_platform::PlatformAdapter;
    let (state, row, _, calls, server) = media_fixture("media_relay_post_ack", "direct").await;
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let identity_key = (bot.id.clone(), bot.app_id.clone().unwrap_or_default());
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .insert(bot.id.clone(), "ou_own_bot".into());
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"tenant_access_token":"test-token","expire":7200})),
        )
        .mount(&provider)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"message_id":"om_reply"}})),
        )
        .mount(&provider)
        .await;
    let adapter = std::sync::Arc::new(
        crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
            &provider.uri(),
            "lark",
        ),
    );
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(bot.id.clone(), adapter.clone());
    Mock::given(method("GET"))
        .and(path_regex("/open-apis/im/v1/messages/.*/resources/.*"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(native_png())
                .insert_header("content-type", "image/png"),
        )
        .mount(&provider)
        .await;
    crate::services::assistant_channel_media::TEST_ADAPTERS
        .lock()
        .unwrap()
        .insert(bot.id.clone(), adapter.clone());
    for enabled in [false, true] {
        if enabled {
            enable_media(&state).await;
        }
        let id = if enabled { "post-on" } else { "post-off" };
        let raw = json!({
            "header":{"event_type":"im.message.receive_v1","event_id":id},
            "event":{
                "sender":{"sender_type":"user","sender_id":{"open_id":"human"}},
                "message":{
                    "message_id":id,"chat_id":"oc_human","chat_type":"p2p","message_type":"post",
                    "mentions":[{"key":"@_user_1","id":{"open_id":"ou_own_bot"},"name":"Receipt helper"}],
                    "content":json!({"title":"Receipt","content":[[
                        {"tag":"at","user_id":"@_user_1"},
                        {"tag":"text","text":"Dinner"},
                        {"tag":"img","image_key":"img_receipt"}
                    ]]}).to_string()
                }
            }
        });
        let inbound = adapter
            .parse_inbound(&serde_json::to_vec(&raw).unwrap())
            .await
            .unwrap();
        assert!(inbound[0].text.is_none(), "callback post text stays empty");
        let source = crate::services::channel_relay_service::inbound_metadata(
            &row.channel_bot_id,
            row.route_id.as_deref().unwrap(),
            OWNER,
            "lark",
            &inbound[0],
            &row.route_api_key_id,
            id,
        );
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(source)
            .await
            .unwrap();
        let body = json!({
            "message_id":id,"correlation_id":format!("jti-{id}"),"platform":"lark",
            "conversation":{"id":row.route_id,"platform_id":"oc_human","type":"private"},
            "sender":{"platform_id":"human","display_name":"Alice"},
            "content":{"type":"unknown","text":null,"attachments":[]},
            "raw_platform_data":raw,
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let token = crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            &format!("jti-{id}"),
            &row.route_api_key_id,
            id,
            "lark",
            &sha256_hex(&bytes),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        let (_watch, done) = watch_relay_callback(&row.id, id);
        let pause = enabled.then(|| {
            crate::services::assistant_channel_media::test_hooks::install(
                &state.db,
                id,
                "relay_before_media_text",
                false,
            )
        });
        assert_eq!(
            relay_callback(
                State(state.clone()),
                Path(row.id.clone()),
                headers,
                Bytes::from(bytes)
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        // On this current-thread runtime, newly spawned work cannot run until we yield.
        // An identity cache entry here would mean lookup still preceded the acknowledgement.
        assert!(!OWN_USER_IDS.lock().unwrap().contains_key(&identity_key));
        if let Some(pause) = pause {
            tokio::time::timeout(Duration::from_secs(20), pause.entered.notified())
                .await
                .unwrap();
            state
                .db
                .collection::<bson::Document>("feature_flag_overrides")
                .update_many(
                    doc! {"flag_key":feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY},
                    doc! {"$set":{"enabled":false}},
                )
                .await
                .unwrap();
            pause.release.notify_one();
        }
        tokio::time::timeout(Duration::from_secs(20), done)
            .await
            .unwrap()
            .unwrap();
        if enabled {
            let calls = calls.lock().await;
            assert_eq!(calls.len(), 1);
            let input = captured_text(&calls[0]);
            assert!(input.contains("Receipt\nDinner"), "{input}");
            assert!(!input.contains("@Receipt helper"), "{input}");
            assert!(calls[0].to_string().contains("input_image"));
            assert_eq!(
                OWN_USER_IDS.lock().unwrap()[&identity_key].0.as_deref(),
                Some("ou_own_bot")
            );
        } else {
            assert!(calls.lock().await.is_empty());
            let event = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"event_id":id})
                .await
                .unwrap()
                .unwrap();
            assert!(event.turn_id.is_none());
            assert!(!OWN_USER_IDS.lock().unwrap().contains_key(&identity_key));
        }
    }
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&bot.id);
    TEST_BOT_USER_IDS.lock().unwrap().remove(&bot.id);
    OWN_USER_IDS.lock().unwrap().remove(&identity_key);
    server.abort();
}

/// Both refusal harnesses must first demonstrate a real token exchange and resource GET.
async fn assert_authorized_media_download(
    state: &AppState,
    source: &ChannelMessage,
    request: &MediaRequest,
    provider: &MockServer,
) -> usize {
    let native = source.platform_message_id.as_deref().unwrap();
    let url = format!(
        "{}/open-apis/im/v1/messages/{native}/resources/img_receipt?type=image",
        provider.uri()
    );
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .update_one(
            doc! {"_id":&source.id},
            doc! {"$set":{"attachments.0.provider_ref":url}},
        )
        .await
        .unwrap();
    mount_media_provider(provider, native).await;
    authority::resolve(&state.db, request).await.unwrap();
    fetch_if_authorized(state, request, provider).await.unwrap();
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        2,
        "one token exchange and one resource download"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "GET" && request.url.path().contains("/resources/"))
            .count(),
        1
    );
    requests.len()
}

fn cma_media_event(text: &str, sender: &str, id: &str) -> Value {
    let mut body = event(text, sender, id);
    body["event_context"]["activity"]["source"]["platform"] = json!("lark");
    body["event_context"]["activity"]["conversation"]["id"] = json!("oc_human");
    body["event_context"]["activity"]["kind"]["attachments"] =
        json!([{"kind":"image","source":{"type":"nyxid_relay","index":0}}]);
    body
}

pub(super) fn native_png() -> Vec<u8> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}
pub(super) async fn ingestion_provider(
    _state: &AppState,
    row: &NyxbotChannel,
    bytes: Vec<u8>,
) -> MockServer {
    let provider = MockServer::start().await;
    INGESTION_ORIGINS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), provider.uri());
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"tenant_access_token":"mock-secret","expire":7200})),
        )
        .mount(&provider)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("/open-apis/im/v1/messages/.*/resources/.*"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(bytes)
                .insert_header("content-type", "image/png"),
        )
        .mount(&provider)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"message_id":"reply"}})),
        )
        .mount(&provider)
        .await;
    let adapter = Arc::new(
        crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
            &provider.uri(),
            "lark",
        ),
    );
    crate::services::assistant_channel_media::TEST_ADAPTERS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), adapter.clone());
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), adapter);
    provider
}
async fn gateway_ready(state: &AppState, key: &str) {
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(key),
            Json(binding_body(key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(state, key, PARTITION).await;
}

#[tokio::test]
async fn channel_media_relay_gateway_first_photo_home_guest_and_caption_never_decides() {
    for transport in ["direct", "gateway"] {
        let (state, mut row, key, calls, server) =
            media_fixture(&format!("media_first_{transport}"), transport).await;
        enable_media(&state).await;
        row.private_chats = Some("everyone".into());
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        if transport == "gateway" {
            gateway_ready(&state, &key).await;
        }
        let provider = ingestion_provider(&state, &row, native_png()).await;
        for (n, sender, text) in [
            (0, "human", ""),
            (1, "human", "yes 4821"),
            (2, "guest", ""),
            (3, "guest", "yes 4821"),
        ] {
            let id = format!("media-{n}");
            let mut source = media_source(&state, &row, &id, &id, text).await;
            source.sender_platform_id = Some(sender.into());
            if transport == "direct" {
                source.platform_conversation_id = Some(format!("oc_{sender}"));
            }
            state
                .db
                .collection::<ChannelMessage>(MESSAGES)
                .replace_one(doc! {"_id":&id}, &source)
                .await
                .unwrap();
            let downloads_before = provider
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method == "GET")
                .count();
            if transport == "direct" {
                let (_guard, done) = watch_relay_callback(&row.id, &id);
                assert_eq!(
                    direct_private(&state, &row, &id, sender, text).await,
                    StatusCode::ACCEPTED
                );
                tokio::time::timeout(Duration::from_secs(30), done)
                    .await
                    .unwrap()
                    .unwrap();
            } else {
                let body = cma_media_event(text, sender, &id);
                let response = respond(&state, &key, &body, &id).await;
                assert_eq!(response.status(), StatusCode::OK);
                assert!(body_text(response).await.contains("Here is your answer"));
            }
            let written = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"event_id":&id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(written.source_media_declared, Some(1));
            let current = state
                .db
                .collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {"_id":&written.conversation_id})
                .await
                .unwrap()
                .unwrap();
            let messages = state
                .db
                .collection::<crate::models::assistant_message::AssistantMessage>(
                    crate::models::assistant_message::COLLECTION_NAME,
                );
            let message=messages.find_one(doc! {"conversation_id":&current.id,"turn_id":&written.turn_id,"role":{"$ne":"assistant"}}).await.unwrap().unwrap();
            let captured = calls.lock().await.last().unwrap().clone();
            assert!(current.active_turn.is_none());
            if sender == "guest" {
                assert!(written.media.is_none());
                assert!(message.attachments.is_empty());
                assert!(
                    captured
                        .to_string()
                        .contains(crate::services::assistant_channel_media::GUEST_NOTE)
                );
                assert_eq!(
                    provider
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .filter(|r| r.method == "GET")
                        .count(),
                    downloads_before
                );
            } else {
                assert!(written.media.is_some());
                assert_eq!(
                    message.attachments.len(),
                    1,
                    "{transport}/{n}: {}",
                    captured
                );
                assert!(message.media_bound.unwrap().failed.is_empty());
                assert!(captured.to_string().contains("input_image"), "{captured}");
                assert!(message.media_pending.is_none());
            }
        }
        server.abort();
    }
}

#[tokio::test]
async fn channel_media_gateway_current_request_contains_all_failed_partial_and_capped_post_context()
{
    for kind in ["all_failed", "partial", "capped"] {
        let (state, row, key, calls, server) =
            media_fixture(&format!("media_context_{kind}"), "gateway").await;
        enable_media(&state).await;
        gateway_ready(&state, &key).await;
        let provider = ingestion_provider(
            &state,
            &row,
            if kind == "all_failed" {
                vec![0xff, 0, 0xff]
            } else {
                native_png()
            },
        )
        .await;
        let mut source = media_source(&state, &row, "post", "om_post", "Dinner").await;
        source.embedded_attachments = source.attachments.clone();
        source.attachments.clear();
        if kind != "all_failed" {
            let mut failed = source.embedded_attachments[0].clone();
            failed.image_key = Some("bad".into());
            failed.size_bytes = Some(21 * 1024 * 1024);
            source.embedded_attachments.push(failed);
        }
        if kind == "partial" {
            source
                .embedded_attachments
                .push(source.embedded_attachments[0].clone());
        }
        if kind == "capped" {
            while source.embedded_attachments.len() < 10 {
                source
                    .embedded_attachments
                    .push(source.embedded_attachments[1].clone());
            }
            source.embedded_attachments_omitted = 1;
        }
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .replace_one(doc! {"_id":&source.id}, &source)
            .await
            .unwrap();
        let mut body = cma_media_event("Dinner", "human", "post");
        body["input"][0]["content"][0]["text"] = json!("OLD RENDERING must disappear");
        let text = if kind == "capped" {
            format!("{}\n[post truncated]", "Dinner ".repeat(5000))
        } else {
            "Dinner".into()
        };
        body["event_context"]["activity"]["kind"]["text"] = json!(text);
        body["event_context"]["activity"]["kind"]["attachments"] =
            json!([{"kind":"image","variant":"embedded"}]);
        assert!(
            body_text(respond(&state, &key, &body, kind).await)
                .await
                .contains("Here is your answer")
        );
        let input = calls.lock().await.last().unwrap().to_string();
        assert!(!input.contains("OLD RENDERING"));
        if kind == "all_failed" {
            assert!(input.contains(crate::services::assistant_channel_media::FAILED_NOTE));
        } else {
            assert!(input.contains("too large"));
            assert!(!input.contains("too_large"));
            assert!(input.contains("input_image"));
            if kind == "partial" {
                assert!(input.contains("Attachment 2 of 3 could not be read (too large)."));
                assert!(!input.contains("Attachment 1 of 3 could not be read"));
            }
        }
        if kind == "capped" {
            assert!(input.contains("post truncated"));
            assert!(input.contains("1 more embedded media omitted"));
        }
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"event_id":"post"})
            .await
            .unwrap()
            .unwrap();
        let message = state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .find_one(doc! {"turn_id":&event.turn_id,"role":{"$ne":"assistant"}})
            .await
            .unwrap()
            .unwrap();
        let receipt = message.media_bound.unwrap();
        if kind == "capped" {
            assert_eq!(event.media.as_ref().unwrap().declared, 11);
            assert_eq!(receipt.ok.len(), 1);
            assert_eq!(receipt.failed.len(), 10);
        }
        assert!(message.text.contains(&receipt.context));
        assert!(!provider.received_requests().await.unwrap().is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn channel_media_gateway_post_with_embedded_image_and_video_uses_text_and_ingests_both() {
    use crate::services::channel_platform::PlatformAdapter;
    let (state, row, key, calls, server) = media_fixture("media_post_video", "gateway").await;
    enable_media(&state).await;
    gateway_ready(&state, &key).await;
    let provider = ingestion_provider(&state, &row, native_png()).await;
    Mock::given(method("GET"))
        .and(path(
            "/open-apis/im/v1/messages/om_post_video/resources/file_video",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xff, 0, 0xff]))
        .with_priority(1)
        .mount(&provider)
        .await;
    let adapter = crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
        &provider.uri(),
        "lark",
    );
    let raw = json!({
        "header":{"event_type":"im.message.receive_v1","event_id":"post-video"},
        "event":{
            "sender":{"sender_type":"user","sender_id":{"open_id":"human"}},
            "message":{
                "message_id":"om_post_video","chat_id":"oc_human","chat_type":"p2p","message_type":"post",
                "content":json!({"title":"Receipt","content":[[
                    {"tag":"text","text":"Dinner"},
                    {"tag":"img","image_key":"img_receipt"},
                    {"tag":"media","file_key":"file_video"}
                ]]}).to_string()
            }
        }
    });
    let inbound = adapter
        .parse_inbound(&serde_json::to_vec(&raw).unwrap())
        .await
        .unwrap();
    let source = crate::services::channel_relay_service::inbound_metadata(
        &row.channel_bot_id,
        row.route_id.as_deref().unwrap(),
        OWNER,
        "lark",
        &inbound[0],
        &row.route_api_key_id,
        "post-video",
    );
    assert!(source.attachments.is_empty());
    assert_eq!(source.embedded_attachments.len(), 2);
    assert_eq!(source.embedded_attachments[0].content_type, "image");
    assert_eq!(source.embedded_attachments[1].content_type, "file");
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .insert_one(source)
        .await
        .unwrap();
    let mut body = cma_media_event("Receipt\nDinner", "human", "post-video");
    body["input"][0]["content"][0]["text"] = json!("this agent cannot view images");
    body["event_context"]["activity"]["kind"]["text"] = json!("Receipt\nDinner");
    body["event_context"]["activity"]["kind"]["attachments"] = json!([
        {"kind":"image","variant":"embedded"},
        {"kind":"video","variant":"embedded"}
    ]);
    assert!(
        body_text(respond(&state, &key, &body, "post-video").await)
            .await
            .contains("Here is your answer")
    );
    let captured = calls.lock().await.last().unwrap().to_string();
    assert!(captured_text(calls.lock().await.last().unwrap()).contains("Receipt\nDinner"));
    assert!(!captured.contains("this agent cannot view images"));
    assert!(captured.contains("input_image"));
    assert!(
        captured.contains("Attachment 2 of 2 could not be read (unsupported or invalid file).")
    );
    let saved = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"event_id":"post-video"})
        .await
        .unwrap()
        .unwrap();
    let media = saved.media.unwrap();
    assert_eq!(media.declared, 2);
    assert_eq!(media.items.len(), 2);
    assert_eq!(media.items[0].outcome.as_ref().unwrap().status, "bound");
    assert_eq!(
        media.items[1].outcome.as_ref().unwrap().status,
        "unsupported_or_invalid"
    );
    let requests = provider.received_requests().await.unwrap();
    for handle in ["img_receipt", "file_video"] {
        assert!(
            requests
                .iter()
                .any(|r| r.method == "GET" && r.url.path().ends_with(handle))
        );
    }
    server.abort();
}

#[tokio::test]
async fn channel_media_gateway_closed_shape_compatibility_forged_sources_and_missing_note() {
    let (state, row, key, calls, server) = media_fixture("media_gateway_shapes", "gateway").await;
    enable_media(&state).await;
    gateway_ready(&state, &key).await;
    for shape in ["image", "post", "audio", "video", "text", "telegram"] {
        let mut body = cma_media_event("native text", "human", shape);
        body["input"][0]["content"][0]["text"] = json!("rendered bytes\noriginal");
        match shape {
            "post" => {
                body["event_context"]["activity"]["kind"]["attachments"] =
                    json!([{"kind":"image","variant":"embedded"}])
            }
            "audio" | "video" => {
                body["event_context"]["activity"]["kind"]["attachments"][0]["kind"] = json!(shape)
            }
            "text" => body["event_context"]["activity"]["kind"]["attachments"] = json!([]),
            "telegram" => {
                body["event_context"]["activity"]["source"] =
                    json!({"type":"telegram","platform":"telegram","bot_id":"77"});
                body["event_context"]["activity"]["kind"]["attachments"] = json!([
                    {"kind":"image","source":{"type":"telegram","file_id":"fixture_file","file_unique_id":"fixture_unique"}}
                ]);
            }
            _ => {}
        }
        let response = respond(&state, &key, &body, shape).await;
        assert_eq!(response.status(), StatusCode::OK);
        body_text(response).await;
        let input = calls.lock().await.last().unwrap()["input"]
            .as_str()
            .unwrap()
            .to_owned();
        if matches!(shape, "image" | "post") {
            assert!(input.contains("native text"));
            assert!(!input.contains("rendered bytes"));
            assert!(input.contains(crate::services::assistant_channel_media::FAILED_NOTE));
        } else {
            assert!(
                input.ends_with("rendered bytes\noriginal"),
                "{shape}: {input}"
            );
        }
        let saved = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"event_id":shape})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.media.is_none());
    }
    let provider = ingestion_provider(&state, &row, native_png()).await;
    let mut source = media_source(&state, &row, "forged", "native", "receipt").await;
    source.sender_platform_id = Some("victim".into());
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":&source.id}, &source)
        .await
        .unwrap();
    let before = calls.lock().await.len();
    let response = respond(
        &state,
        &key,
        &cma_media_event("", "human", "forged"),
        "forged",
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let refusal: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(refusal["code"], "media_source_refused");
    assert_eq!(refusal["error"]["code"], "media_source_refused");
    assert!(provider.received_requests().await.unwrap().is_empty());
    assert_eq!(calls.lock().await.len(), before);
    server.abort();
}

fn captured_text(call: &Value) -> &str {
    call["input"]
        .as_str()
        .unwrap_or_else(|| call["input"][0]["content"][0]["text"].as_str().unwrap())
}

async fn streaming_ingestion_provider(
    state: &AppState,
    row: &NyxbotChannel,
    payload: Vec<u8>,
    revoke: bool,
    gate: Option<Arc<tokio::sync::Notify>>,
) -> (
    Arc<std::sync::atomic::AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    use axum::{
        Router,
        body::Body,
        routing::{get, post},
    };
    let downloads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = downloads.clone();
    let db = state.db.clone();
    let channel = row.id.clone();
    let payload = Arc::new(payload);
    let app = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post(|| async {
                Json(json!({"code":0,"tenant_access_token":"local-only","expire":7200}))
            }),
        )
        .route(
            "/open-apis/im/v1/messages/{native}/resources/{key}",
            get(
                move |axum::extract::Path((_native, resource)): axum::extract::Path<(
                    String,
                    String,
                )>| {
                    let counted = counted.clone();
                    let db = db.clone();
                    let channel = channel.clone();
                    let payload = payload.clone();
                    let gate = gate.clone();
                    async move {
                        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        if revoke {
                            db.collection::<bson::Document>(CHANNELS)
                                .update_one(
                                    doc! {"_id":channel},
                                    doc! {"$set":{"owner_sender_ids":[]}},
                                )
                                .await
                                .unwrap();
                        }
                        if let Some(gate) = gate
                            && resource != "fast"
                        {
                            gate.notified().await;
                        }
                        let chunks: Vec<_> = payload
                            .chunks(32768)
                            .map(|c| Ok::<_, std::io::Error>(Bytes::copy_from_slice(c)))
                            .collect();
                        axum::http::Response::builder()
                            .header("content-type", "image/png")
                            .body(Body::from_stream(futures::stream::iter(chunks)))
                            .unwrap()
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    INGESTION_ORIGINS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), origin.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let adapter = Arc::new(
        crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
            &origin, "lark",
        ),
    );
    crate::services::assistant_channel_media::TEST_ADAPTERS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), adapter);
    (downloads, task)
}

#[tokio::test]
async fn channel_media_chunked_unknown_understated_bodies_retry_once_and_recheck_authority() {
    use std::sync::atomic::Ordering;
    for (declared, revoke) in [
        (None, false),
        (Some(0), false),
        (Some(1024), false),
        (Some(1024), true),
    ] {
        let (state, row, key, calls, server) =
            media_fixture("media_stream_limits", "gateway").await;
        enable_media(&state).await;
        gateway_ready(&state, &key).await;
        let mut payload = native_png();
        payload.resize(5 * 1024 * 1024, 0);
        let (downloads, provider) =
            streaming_ingestion_provider(&state, &row, payload, revoke, None).await;
        let mut source = media_source(&state, &row, "streamed", "streamed", "").await;
        source.attachments[0].size_bytes = declared;
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .replace_one(doc! {"_id":&source.id}, source)
            .await
            .unwrap();
        body_text(
            respond(
                &state,
                &key,
                &cma_media_event("", "human", "streamed"),
                "streamed",
            )
            .await,
        )
        .await;
        assert_eq!(downloads.load(Ordering::SeqCst), if revoke { 1 } else { 2 });
        let input = calls.lock().await.last().unwrap().to_string();
        if revoke {
            assert!(input.contains("Attachment 1 of 1 could not be read (source unavailable)."));
            assert!(!input.contains("source_unavailable"));
            assert!(!input.contains("input_image"));
        } else {
            assert!(input.contains("input_image"), "{input}");
        }
        let attempts = state
            .db
            .collection::<bson::Document>("assistant_upload_limits")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            attempts.get_i32("attempts").unwrap(),
            2,
            "failed attempt and retry both admitted"
        );
        provider.abort();
        server.abort();
    }
}

#[tokio::test]
async fn channel_media_gateway_headers_return_while_download_is_blocked_in_run_task() {
    let (state, row, key, calls, server) =
        media_fixture("media_header_no_download_wait", "gateway").await;
    enable_media(&state).await;
    gateway_ready(&state, &key).await;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_downloads, provider) =
        streaming_ingestion_provider(&state, &row, native_png(), false, Some(gate.clone())).await;
    let mut source = media_source(&state, &row, "blocked", "blocked", "").await;
    source.attachments[0].size_bytes = None;
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":&source.id}, source)
        .await
        .unwrap();
    let body = cma_media_event("", "human", "blocked");
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        respond(&state, &key, &body, "blocked"),
    )
    .await
    .expect("header path waited for blocked download");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(calls.lock().await.is_empty());
    gate.notify_one();
    assert!(body_text(response).await.contains("Here is your answer"));
    assert!(
        calls
            .lock()
            .await
            .last()
            .unwrap()
            .to_string()
            .contains("input_image")
    );
    provider.abort();
    server.abort();
}

#[tokio::test]
async fn channel_media_stop_and_whole_step_deadline_cancel_blocked_fetch_and_finalize() {
    for deadline in [false, true] {
        let (state, row, key, calls, server) =
            media_fixture("media_fetch_cancellation", "gateway").await;
        enable_media(&state).await;
        gateway_ready(&state, &key).await;
        let gate = Arc::new(tokio::sync::Notify::new());
        let (downloads, provider) =
            streaming_ingestion_provider(&state, &row, native_png(), false, Some(gate)).await;
        let mut source = media_source(&state, &row, "cancel-fetch", "cancel-fetch", "").await;
        if deadline {
            let mut fast = source.attachments[0].clone();
            fast.provider_ref = fast.provider_ref.replace("img_receipt", "fast");
            fast.image_key = Some("fast".into());
            fast.size_bytes = None;
            source.attachments[0].size_bytes = None;
            source.attachments.insert(0, fast);
            let mut not_started = source.attachments[1].clone();
            not_started.provider_ref = not_started
                .provider_ref
                .replace("img_receipt", "not-started");
            not_started.image_key = Some("not-started".into());
            source.attachments.push(not_started);
            state
                .db
                .collection::<ChannelMessage>(MESSAGES)
                .replace_one(doc! {"_id": &source.id}, &source)
                .await
                .unwrap();
        }
        let response = respond(
            &state,
            &key,
            &cma_media_event("", "human", "cancel-fetch"),
            "cancel-fetch",
        )
        .await;
        for _ in 0..1000 {
            if downloads.load(std::sync::atomic::Ordering::SeqCst) == if deadline { 2 } else { 1 } {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            downloads.load(std::sync::atomic::Ordering::SeqCst),
            if deadline { 2 } else { 1 }
        );
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"event_id":"cancel-fetch"})
            .await
            .unwrap()
            .unwrap();
        if deadline {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(91)).await;
            tokio::time::resume();
        } else {
            engine::request_stop(&state.db, OWNER, event.conversation_id.as_deref().unwrap())
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(30), body_text(response))
            .await
            .unwrap();
        let message = state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .find_one(doc! {"turn_id":&event.turn_id,"role":"user"})
            .await
            .unwrap()
            .unwrap();
        assert!(message.media_pending.is_none());
        let receipt = message.media_bound.unwrap();
        if deadline {
            assert_eq!(message.attachments.len(), 1);
            assert_eq!(receipt.ok.len(), 1);
            assert_eq!(receipt.failed.len(), 2);
            assert!(receipt.failed.iter().all(|item| item.status == "timeout"));
            assert_eq!(receipt.failed[0].index, Some(1));
            assert_eq!(receipt.failed[1].index, Some(2));
            let input = calls.lock().await.last().unwrap().to_string();
            assert!(input.contains("input_image"));
            assert!(input.contains("Attachment 2 of 3 could not be read (timed out)."));
            assert!(input.contains("Attachment 3 of 3 could not be read (timed out)."));
            assert_eq!(downloads.load(std::sync::atomic::Ordering::SeqCst), 2);
        } else {
            assert!(message.attachments.is_empty());
            assert_eq!(receipt.failed[0].status, "not_processed: stopped");
            assert!(calls.lock().await.is_empty());
        }
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .count_documents(
                    doc! {"message_id": bson::Bson::Null, "parent_attachment_id": bson::Bson::Null}
                )
                .await
                .unwrap(),
            0
        );
        provider.abort();
        server.abort();
    }
}

#[tokio::test]
async fn channel_media_caption_with_matching_confirmation_code_never_decides_pending_card() {
    use crate::services::assistant_acknowledgement_service as acks;
    let (state, row, key, calls, server) = media_fixture("media_caption_card", "gateway").await;
    enable_media(&state).await;
    gateway_ready(&state, &key).await;
    let _provider = ingestion_provider(&state, &row, native_png()).await;
    media_source(&state, &row, "first", "first", "photo").await;
    body_text(
        respond(
            &state,
            &key,
            &cma_media_event("photo", "human", "first"),
            "first",
        )
        .await,
    )
    .await;
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"event_id":"first"})
        .await
        .unwrap()
        .unwrap();
    let row = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":&event.conversation_id})
        .await
        .unwrap()
        .unwrap();
    let chat = acks::for_key(&state.db, OWNER, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let (card, _) = acks::request_tracked(
        &state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_agent_key"),
            arguments: Some(&json!({"key_id":"test"})),
            summary: "Delete test key",
            platform: false,
        },
    )
    .await
    .unwrap();
    let caption = format!("yes {}", acks::confirm_code(&card.id));
    let channel = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&event.channel_id})
        .await
        .unwrap()
        .unwrap();
    media_source(&state, &channel, "caption", "caption", &caption).await;
    body_text(
        respond(
            &state,
            &key,
            &cma_media_event(&caption, "human", "caption"),
            "caption",
        )
        .await,
    )
    .await;
    let history = acks::history(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        history.iter().find(|r| r.id == card.id).unwrap().status,
        "pending"
    );
    assert!(
        !captured_text(calls.lock().await.last().unwrap())
            .contains("the owner confirmed the pending action")
    );
    server.abort();
}

fn owner_turn_request(id: &str, text: &str) -> engine::TurnRequest {
    engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        conversation_id: Some(id.into()),
        attachment_ids: vec![],
        text: text.into(),
        agent_id: None,
        model: None,
        access_mode: None,
    }
}
async fn finish_media_turn(state: &AppState, row: &AssistantConversation) {
    engine::finish_turn(
        &state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "done".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn channel_media_busy_mixed_queue_preserves_order_and_drains_after_flag_disable() {
    let (state, row, key, calls, server) = media_fixture("media_busy_queue", "gateway").await;
    enable_media(&state).await;
    gateway_ready(&state, &key).await;
    let _provider = ingestion_provider(&state, &row, native_png()).await;
    media_source(&state, &row, "first", "first", "first").await;
    body_text(
        respond(
            &state,
            &key,
            &cma_media_event("first", "human", "first"),
            "first",
        )
        .await,
    )
    .await;
    let original = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"event_id":"first"})
        .await
        .unwrap()
        .unwrap();
    let home = original.conversation_id.unwrap();
    let blocked = engine::begin_turn(
        &state.db,
        OWNER,
        &owner_turn_request(&home, "ongoing work"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let mut text = cma_media_event("queued text", "human", "text");
    text["event_context"]["activity"]["kind"]["attachments"] = json!([]);
    body_text(respond(&state, &key, &text, "text").await).await;
    media_source(&state, &row, "photo", "photo", "").await;
    body_text(
        respond(
            &state,
            &key,
            &cma_media_event("", "human", "photo"),
            "photo",
        )
        .await,
    )
    .await;
    let queued = engine::get(&state.db, OWNER, &home).await.unwrap();
    assert_eq!(queued.pending_events.len(), 2);
    assert!(queued.pending_events[0].media_event_ids.is_empty());
    assert_eq!(queued.pending_events[1].media_event_ids.len(), 1);
    assert_eq!(queued.pending_events[1].media_declared, Some(1));
    assert!(queued.pending_events[1].media_sent_at.is_some());
    state
        .db
        .collection::<bson::Document>("feature_flag_overrides")
        .update_many(
            doc! {"flag_key":feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY},
            doc! {"$set":{"enabled":false}},
        )
        .await
        .unwrap();
    finish_media_turn(&state, &blocked).await;
    for expected_media in [false, true] {
        let mut start: TurnStart = (&owner_turn_request(&home, "")).into();
        start.origin = TurnOrigin::Event;
        let started = start_server_turn(&state, OWNER, start, Pool::Channel { owner: OWNER })
            .await
            .unwrap();
        let Started::Turn { receiver, .. } = started else {
            panic!("queued turn was not started")
        };
        final_reply(receiver).await.unwrap();
        let captured = calls.lock().await.last().unwrap().clone();
        assert_eq!(
            captured.to_string().contains("input_image"),
            expected_media,
            "{captured}"
        );
    }
    assert!(
        engine::get(&state.db, OWNER, &home)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    server.abort();
}

#[tokio::test]
async fn channel_media_first_photo_org_bot_and_specialist_thread_bind_to_correct_agent() {
    for mode in ["org", "specialist"] {
        let (state, mut row, key, calls, server) =
            media_fixture(&format!("media_first_{mode}"), "gateway").await;
        enable_media(&state).await;
        if mode == "org" {
            let org = Uuid::new_v4().to_string();
            state
                .db
                .collection(USERS)
                .insert_one(test_user(&org, UserType::Org))
                .await
                .unwrap();
            state.db.collection::<bson::Document>("org_memberships").insert_one(doc! {"_id":Uuid::new_v4().to_string(),"org_user_id":&org,"member_user_id":OWNER,"role":"admin","allowed_service_ids":bson::Bson::Null,"created_at":bson::DateTime::now(),"revoked_at":bson::Bson::Null}).await.unwrap();
            row.bot_owner_id = Some(org.clone());
            for (collection, filter) in [
                ("channel_bots", doc! {"_id":&row.channel_bot_id}),
                ("channel_conversations", doc! {"_id":&row.route_id}),
                ("api_keys", doc! {"_id":&row.route_api_key_id}),
            ] {
                state
                    .db
                    .collection::<bson::Document>(collection)
                    .update_one(filter, doc! {"$set":{"user_id":&org}})
                    .await
                    .unwrap();
            }
        } else {
            let (agent, _) = crate::services::assistant_team_service::create_specialist(
                &state.db,
                &state.encryption_keys,
                OWNER,
                crate::services::assistant_team_service::CreateRequest {
                    name: "receipts".into(),
                    description: "Receipts".into(),
                    display_name: None,
                    persona: None,
                    targets: Default::default(),
                    account_read: false,
                    specialty: None,
                    machines: None,
                    logins: None,
                    created_by: "user",
                },
            )
            .await
            .unwrap()
            .unwrap();
            row.agent_id = Some(agent.id);
        }
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        gateway_ready(&state, &key).await;
        let _provider = ingestion_provider(&state, &row, native_png()).await;
        media_source(&state, &row, "first", "first", "").await;
        assert!(
            body_text(
                respond(
                    &state,
                    &key,
                    &cma_media_event("", "human", "first"),
                    "first"
                )
                .await
            )
            .await
            .contains("Here is your answer")
        );
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"event_id":"first"})
            .await
            .unwrap()
            .unwrap();
        let conversation = engine::get(&state.db, OWNER, event.conversation_id.as_deref().unwrap())
            .await
            .unwrap();
        if mode == "org" {
            assert!(conversation.channel.is_some());
        } else {
            assert_eq!(conversation.agent_id, row.agent_id);
        }
        assert!(
            calls
                .lock()
                .await
                .last()
                .unwrap()
                .to_string()
                .contains("input_image")
        );
        server.abort();
    }
}

#[tokio::test]
async fn channel_media_conversation_deletion_during_fetch_cannot_commit_upload() {
    let (state, row, key, calls, server) =
        media_fixture("media_delete_during_fetch", "gateway").await;
    enable_media(&state).await;
    gateway_ready(&state, &key).await;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (downloads, provider) =
        streaming_ingestion_provider(&state, &row, native_png(), false, Some(gate.clone())).await;
    let mut source = media_source(&state, &row, "delete", "delete", "").await;
    source.attachments[0].size_bytes = None;
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":&source.id}, source)
        .await
        .unwrap();
    let response = respond(
        &state,
        &key,
        &cma_media_event("", "human", "delete"),
        "delete",
    )
    .await;
    for _ in 0..200 {
        if downloads.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(downloads.load(std::sync::atomic::Ordering::SeqCst), 1);
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"event_id":"delete"})
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .delete_one(doc! {"_id":event.conversation_id})
        .await
        .unwrap();
    gate.notify_one();
    tokio::time::timeout(Duration::from_secs(20), body_text(response))
        .await
        .unwrap();
    assert!(calls.lock().await.is_empty());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {"user_id":OWNER})
            .await
            .unwrap(),
        0
    );
    provider.abort();
    server.abort();
}

#[tokio::test]
async fn channel_media_follow_post_owner_guest_and_unaddressed_photo_preserve_flag_semantics() {
    use crate::models::channel_thread::{
        ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind,
    };
    use crate::services::channel_platform::PlatformAdapter;
    for media_on in [false, true] {
        let (state, row, _, calls, server) = media_fixture("media_follow_posts", "direct").await;
        if media_on {
            enable_media(&state).await;
        }
        state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {"_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,"flag_key":feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,"target_kind":"user","target_key":OWNER,"enabled":true,"updated_by":"test","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        let provider = ingestion_provider(&state, &row, native_png()).await;
        let adapter = Arc::new(
            crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                &provider.uri(),
                "lark",
            ),
        );
        thread_follow::TEST_ADAPTERS
            .lock()
            .unwrap()
            .insert(row.channel_bot_id.clone(), adapter.clone());
        TEST_BOT_USER_IDS
            .lock()
            .unwrap()
            .insert(row.channel_bot_id.clone(), "own-bot".into());
        Mock::given(method("GET")).and(path_regex("^/open-apis/im/v1/messages/[^/]+$"))
            .respond_with(|request:&wiremock::Request| {
                let id=request.url.path_segments().unwrap().next_back().unwrap();
                ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[{"message_id":id,"chat_id":"oc_human","root_id":"root","thread_id":"omt_native","msg_type":"post","create_time":"1790000000000","sender":{"sender_type":"user","id":"human"},"body":{"content":"{}"}}]}}))
            }).mount(&provider).await;
        Mock::given(method("POST"))
            .and(path_regex("/open-apis/im/v1/messages/.*/reply"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"thread-reply"}})),
            )
            .mount(&provider)
            .await;
        for (n, sender, addressed, post) in [
            (0, "human", false, false),
            (1, "human", true, true),
            (2, "guest", false, false),
        ] {
            let id = format!("follow-{n}");
            let root = if n == 0 { "silentroot" } else { "root" };
            let content = if post {
                json!({"title":"Receipt","content":[[{"tag":"at","user_id":"@_user_1"},{"tag":"text","text":"Dinner"},{"tag":"img","image_key":"img_receipt"}]]})
            } else {
                json!({"image_key":"img_receipt"})
            };
            let raw = json!({"header":{"event_type":"im.message.receive_v1","event_id":id},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":sender}},"message":{
                "message_id":id,"chat_id":"oc_human","chat_type":"group","thread_id":"omt_native","root_id":root,
                "message_type":if post {"post"} else {"image"},"content":content.to_string(),"mentions":if addressed {json!([{"key":"@_user_1","id":{"open_id":"own-bot"},"name":"Receipt helper"}])} else {json!([])}
            }}});
            let inbound = adapter
                .parse_inbound(&serde_json::to_vec(&raw).unwrap())
                .await
                .unwrap()
                .remove(0);
            let mut source = crate::services::channel_relay_service::inbound_metadata(
                &row.channel_bot_id,
                row.route_id.as_deref().unwrap(),
                OWNER,
                "lark",
                &inbound,
                &row.route_api_key_id,
                &id,
            );
            let facts = ChannelThreadFacts {
                version: 1,
                kind: ThreadKind::Native,
                chat_id: "oc_human".into(),
                message_id: id.clone(),
                root_id: Some(root.into()),
                native_thread_id: Some("omt_native".into()),
                sender_kind: ThreadSenderKind::Human,
                address: if addressed {
                    ThreadAddress::Mention
                } else {
                    ThreadAddress::NotAddressed
                },
                ..Default::default()
            };
            source.thread_context = Some(facts.clone());
            state
                .db
                .collection::<ChannelMessage>(MESSAGES)
                .insert_one(source)
                .await
                .unwrap();
            let body = json!({"message_id":id,"correlation_id":format!("jti-{id}"),"platform":"lark","thread_context":facts,"thread_id":"omt_native","conversation":{"id":row.route_id,"platform_id":"oc_human","type":"group"},"sender":{"platform_id":sender,"display_name":"Alice"},"content":{"type":if post {"unknown"} else {"image"},"text":null,"attachments":[]},"raw_platform_data":raw});
            let bytes = serde_json::to_vec(&body).unwrap();
            let token = crate::crypto::jwt::generate_relay_callback_token(
                &state.jwt_keys,
                &state.config,
                &format!("jti-{id}"),
                &row.route_api_key_id,
                &id,
                "lark",
                &sha256_hex(&bytes),
            )
            .unwrap();
            let mut headers = HeaderMap::new();
            headers.insert("x-nyxid-callback-token", token.parse().unwrap());
            let (_guard, done) = watch_relay_callback(&row.id, &id);
            assert_eq!(
                relay_callback(
                    State(state.clone()),
                    Path(row.id.clone()),
                    headers,
                    Bytes::from(bytes)
                )
                .await
                .status(),
                StatusCode::ACCEPTED
            );
            tokio::time::timeout(Duration::from_secs(30), done)
                .await
                .unwrap()
                .unwrap();
            let captured = calls.lock().await;
            assert_eq!(captured.len(), if media_on { n as usize } else { 0 });
            if media_on && n == 1 {
                assert!(captured_text(captured.last().unwrap()).contains("Receipt\nDinner"));
                assert!(!captured_text(captured.last().unwrap()).contains("@Receipt helper"));
                assert!(captured.last().unwrap().to_string().contains("input_image"));
            }
            if media_on && n == 2 {
                assert!(
                    captured_text(captured.last().unwrap())
                        .contains(crate::services::assistant_channel_media::GUEST_NOTE)
                );
                assert!(!captured.last().unwrap().to_string().contains("input_image"));
            }
        }
        thread_follow::TEST_ADAPTERS
            .lock()
            .unwrap()
            .remove(&row.channel_bot_id);
        TEST_BOT_USER_IDS
            .lock()
            .unwrap()
            .remove(&row.channel_bot_id);
        server.abort();
    }
}
