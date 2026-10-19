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

async fn media_source(
    state: &AppState,
    row: &NyxbotChannel,
    id: &str,
    native: &str,
    text: &str,
) -> ChannelMessage {
    let inbound = InboundMessage {
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
                "https://open.larksuite.com/open-apis/im/v1/messages/{native}/resources/img_receipt?type=image"
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
            provider.received_requests().await.unwrap().is_empty(),
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
            provider.received_requests().await.unwrap().is_empty(),
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
    assert!(provider.received_requests().await.unwrap().is_empty());
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
    assert!(provider.received_requests().await.unwrap().is_empty());
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
    assert!(provider.received_requests().await.unwrap().is_empty());
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

async fn enable_media(state: &AppState) {
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
        let mut body = event("receipt", "human", id);
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
    use crate::models::assistant_conversation::TurnOrigin;
    use crate::services::channel_media_events as media_events;
    let (state, row, _, _, server) = media_fixture("media_interrupted", "direct").await;
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
    duplicate.event_id = source2.id.clone();
    duplicate.relay_message_id = Some(source2.id.clone());
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
