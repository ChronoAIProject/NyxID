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
    state.db.collection::<bson::Document>("org_memberships").insert_one(doc!{"_id":&member,"org_user_id":&org,"member_user_id":OWNER,"role":"admin","allowed_service_ids":bson::Bson::Null,"is_active":true,"joined_at":bson::DateTime::now()}).await.unwrap();
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
    state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(doc! {"_id":&member}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    let provider = MockServer::start().await;
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
            "https://open.larksuite.com/open-apis/im/v1/messages/om_receipt/resources/{resource_key}?type={resource_type}"
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
