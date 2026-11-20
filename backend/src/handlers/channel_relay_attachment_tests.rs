use crate::services::{
    channel_adapters::lark::LarkFamilyAdapter, channel_platform::PlatformAdapter,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path_regex},
};

#[tokio::test]
async fn lark_feishu_attachment_downloads_derive_image_headers_from_bytes_and_keep_source_metadata()
{
    for platform in ["lark", "feishu"] {
        let (mut fixture, provider, adapter) = lark_download_fixture(platform).await;
        let inbound = adapter
            .parse_inbound(include_bytes!(
                "../services/channel_adapters/fixtures/lark_callback/lark-image-in.json"
            ))
            .await
            .unwrap()
            .remove(0);
        // Real parser -> relay metadata -> persisted source -> authorized GET.
        fixture.message = channel_relay_service::inbound_metadata(
            &fixture.bot.id,
            &fixture.conversation.id,
            &fixture.api_key.user_id,
            platform,
            &inbound,
            &fixture.api_key.id,
            &fixture.message.id,
        );
        fixture
            .state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .replace_one(doc! {"_id":&fixture.message.id}, &fixture.message)
            .await
            .unwrap();
        let token = valid_reply_token(&fixture);
        for (format, name, provider_filename) in [
            (
                image::ImageFormat::Png,
                "Receipt (final).pdf",
                Some("attachment; filename=\"Receipt (final).pdf\""),
            ),
            (image::ImageFormat::Png, "attachment.png", None),
            (image::ImageFormat::Jpeg, "attachment.jpg", None),
            (
                image::ImageFormat::Png,
                "收据.png",
                Some("attachment; filename=\"收据.png\""),
            ),
            (
                image::ImageFormat::Png,
                "照片.jpg",
                Some("attachment; filename=ascii.jpg; filename*=UTF-8''%E7%85%A7%E7%89%87.jpg"),
            ),
            (
                image::ImageFormat::Png,
                "Receipt",
                Some("attachment; filename=Receipt"),
            ),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2)
                .write_to(&mut bytes, format)
                .unwrap();
            let bytes = bytes.into_inner();
            let mut reply = ResponseTemplate::new(200)
                .set_body_bytes(bytes.clone())
                .insert_header("content-type", "application/octet-stream");
            if let Some(name) = provider_filename {
                reply = reply.insert_header("content-disposition", name);
            }
            Mock::given(method("GET"))
                .and(path_regex(".*/resources/.*"))
                .respond_with(reply)
                .up_to_n_times(1)
                .expect(1)
                .mount(&provider)
                .await;
            let context = resolve_reply_token_message_context(
                &fixture.state,
                &token,
                &fixture.message.id,
                false,
            )
            .await
            .unwrap();
            let response = fetch_resolved_attachment(&fixture.state, context, 0, &adapter)
                .await
                .unwrap();
            assert_eq!(response.headers()["content-type"], format.to_mime_type());
            let mut disposition = format!(
                "attachment; filename=\"{}\"",
                channel_media_service::safe_filename(Some(name))
            );
            if channel_media_service::safe_filename(Some(name)) != name {
                disposition.push_str(&format!("; filename*=UTF-8''{}", urlencoding::encode(name)));
            }
            assert_eq!(response.headers()["content-disposition"], disposition);
            assert_eq!(
                response.headers()["content-length"],
                bytes.len().to_string()
            );
            assert_eq!(response.headers()["cache-control"], "private, no-store");
            assert_eq!(
                axum::body::to_bytes(response.into_body(), bytes.len() + 1)
                    .await
                    .unwrap()
                    .as_ref(),
                bytes
            );
        }
        let stored = fixture
            .state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .find_one(doc! {"_id":&fixture.message.id})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.attachments[0].filename.is_none());
        assert!(stored.attachments[0].mime_type.is_none());
        fixture.state.db.drop().await.unwrap();
    }
}

async fn lark_download_fixture(
    platform: &str,
) -> (ReplyTokenFixture, MockServer, LarkFamilyAdapter) {
    let mut fixture = setup_reply_token_fixture("lark_image_download")
        .await
        .expect("MongoDB required");
    fixture.bot.platform = platform.into();
    fixture.bot.bot_token_encrypted = fixture
        .state
        .encryption_keys
        .encrypt(b"app:secret")
        .await
        .unwrap();
    fixture.conversation.platform = platform.into();
    fixture
        .state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .replace_one(doc! {"_id":&fixture.bot.id}, &fixture.bot)
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<ChannelConversation>(crate::models::channel_conversation::COLLECTION_NAME)
        .replace_one(doc! {"_id":&fixture.conversation.id}, &fixture.conversation)
        .await
        .unwrap();
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"code":0,"tenant_access_token":"tenant","expire":7200}),
        ))
        .mount(&provider)
        .await;
    let adapter = LarkFamilyAdapter::media_test_adapter(&provider.uri(), platform);
    (fixture, provider, adapter)
}

#[tokio::test]
async fn lark_raw_callback_history_and_private_download_names_remain_distinct() {
    for platform in ["lark", "feishu"] {
        let (mut fixture, provider, adapter) = lark_download_fixture(platform).await;
        let bytes = crate::test_utils::test_pdf();
        Mock::given(method("GET"))
            .and(path_regex(".*/resources/.*"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(bytes.clone())
                    .insert_header("content-type", "application/pdf"),
            )
            .expect(2)
            .mount(&provider)
            .await;
        for (first, effective) in [
            (
                serde_json::json!("Director's © receipt (final).PDF"),
                "Directors  receipt (final).PDF",
            ),
            (serde_json::Value::Null, "Original receipt.pdf"),
        ] {
            let mut raw: serde_json::Value = serde_json::from_slice(include_bytes!(
                "../services/channel_adapters/fixtures/lark_callback/lark-file-in.json"
            ))
            .unwrap();
            let mut content: serde_json::Value =
                serde_json::from_str(raw["event"]["message"]["content"].as_str().unwrap()).unwrap();
            content["file_name"] = first.clone();
            content["name"] = serde_json::json!("Original receipt.pdf");
            raw["event"]["message"]["content"] = serde_json::json!(content.to_string());
            let inbound = adapter
                .parse_inbound(&serde_json::to_vec(&raw).unwrap())
                .await
                .unwrap()
                .remove(0);
            fixture.message = channel_relay_service::inbound_metadata(
                &fixture.bot.id,
                &fixture.conversation.id,
                &fixture.api_key.user_id,
                platform,
                &inbound,
                &fixture.api_key.id,
                &fixture.message.id,
            );
            let rows = fixture
                .state
                .db
                .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME);
            rows.replace_one(doc! {"_id":&fixture.message.id}, &fixture.message)
                .await
                .unwrap();
            fixture.message = rows
                .find_one(doc! {"_id":&fixture.message.id})
                .await
                .unwrap()
                .unwrap();
            let callback = channel_relay_service::build_callback_payload(
                &fixture.message,
                &fixture.conversation,
                &fixture.api_key.id,
                "agent",
                &inbound,
                None,
                "https://nyxid.example",
            );
            let history =
                serde_json::to_value(message_to_item(&fixture.message, "https://nyxid.example"))
                    .unwrap();
            assert_eq!(
                callback.content.attachments[0].filename.as_deref(),
                first.as_str()
            );
            assert_eq!(
                history["attachments"][0]["filename"].as_str(),
                first.as_str()
            );
            assert!(history.get("effective_attachment_filenames").is_none());
            let token = valid_reply_token(&fixture);
            let context = resolve_reply_token_message_context(
                &fixture.state,
                &token,
                &fixture.message.id,
                false,
            )
            .await
            .unwrap();
            let response = fetch_resolved_attachment(&fixture.state, context, 0, &adapter)
                .await
                .unwrap();
            let ascii = channel_media_service::safe_filename(Some(effective));
            let mut expected = format!("attachment; filename=\"{ascii}\"");
            if ascii != effective {
                expected.push_str(&format!(
                    "; filename*=UTF-8''{}",
                    urlencoding::encode(effective)
                ));
            }
            assert_eq!(response.headers()["content-disposition"], expected);
            assert_eq!(
                axum::body::to_bytes(response.into_body(), bytes.len() + 1)
                    .await
                    .unwrap()
                    .as_ref(),
                bytes
            );
        }
        provider.verify().await;
        fixture.state.db.drop().await.unwrap();
    }
}
