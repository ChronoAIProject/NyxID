//! Native selectors, persisted source metadata, verified retention and Base wire.
use super::*;
use crate::services::{
    channel_adapters::lark::LarkFamilyAdapter, channel_platform::PlatformAdapter,
};
use wiremock::matchers::path_regex;

#[tokio::test]
async fn lark_channel_original_name_provenance_survives_retention_and_mock_base_exports() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let png = include_bytes!("../../tests/fixtures/generated-artifacts/blue.png").to_vec();
    for (kind, bytes, first, alternate, header, expected, original) in [
        (
            "file",
            pdf(),
            json!("Director's © receipt (final).PDF"),
            None,
            None,
            "Directors  receipt (final).PDF",
            true,
        ),
        (
            "file",
            pdf(),
            json!("attachment"),
            None,
            Some("attachment; filename=different.pdf"),
            "attachment",
            true,
        ),
        (
            "file",
            pdf(),
            Value::Null,
            None,
            Some("attachment; filename=attachment"),
            "attachment",
            true,
        ),
        (
            "file",
            pdf(),
            Value::Null,
            None,
            None,
            "attachment.pdf",
            false,
        ),
        (
            "file",
            pdf(),
            Value::Null,
            Some("Original receipt.pdf"),
            Some("attachment; filename=different.pdf"),
            "Original receipt.pdf",
            true,
        ),
        (
            "image",
            png.clone(),
            json!("Receipt.归档收据原始凭证"),
            None,
            None,
            "Receipt.归档收据原始凭证",
            true,
        ),
        (
            "image",
            raster(image::ImageFormat::Jpeg),
            json!("📷.jpg"),
            None,
            None,
            "attachment.jpg",
            false,
        ),
        (
            "file",
            pdf(),
            json!("🧾.PDF"),
            None,
            None,
            "attachment.pdf",
            false,
        ),
    ] {
        let u = setup("lark_original_name").await;
        Mock::given(method("POST"))
            .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"tenant_access_token":"tenant","expire":7200})),
            )
            .mount(&u.server)
            .await;
        let mut reply = ResponseTemplate::new(200)
            .set_body_bytes(bytes.clone())
            .insert_header("content-type", "application/octet-stream");
        if let Some(header) = header {
            reply = reply.insert_header("content-disposition", header);
        }
        Mock::given(method("GET"))
            .and(path_regex(".*/resources/.*"))
            .respond_with(reply)
            .expect(1)
            .mount(&u.server)
            .await;
        let adapter = LarkFamilyAdapter::media_test_adapter(&u.server.uri(), "lark");
        let fixture = if kind == "image" {
            include_bytes!("../services/channel_adapters/fixtures/lark_callback/lark-image-in.json")
                .as_slice()
        } else {
            include_bytes!("../services/channel_adapters/fixtures/lark_callback/lark-file-in.json")
                .as_slice()
        };
        let mut raw: Value = serde_json::from_slice(fixture).unwrap();
        let mut content: Value =
            serde_json::from_str(raw["event"]["message"]["content"].as_str().unwrap()).unwrap();
        content["file_name"] = first;
        if let Some(name) = alternate {
            content["name"] = json!(name);
        }
        raw["event"]["message"]["content"] = json!(content.to_string());
        let inbound = adapter
            .parse_inbound(&serde_json::to_vec(&raw).unwrap())
            .await
            .unwrap()
            .remove(0);
        let source = crate::services::channel_relay_service::inbound_metadata(
            "channel-bot",
            "route",
            &u.f.owner,
            "lark",
            &inbound,
            "route-key",
            &Uuid::new_v4().to_string(),
        );
        let sources =
            u.f.state
                .db
                .collection::<crate::models::channel_message::ChannelMessage>(
                    crate::models::channel_message::COLLECTION_NAME,
                );
        sources.insert_one(&source).await.unwrap();
        let source = sources
            .find_one(doc! {"_id":&source.id})
            .await
            .unwrap()
            .unwrap();
        let mut selected = inbound.attachments[0].clone();
        selected.filename = crate::services::channel_media_fetch::selector(&source, 0, false)
            .unwrap()
            .filename;
        let fetched = adapter
            .fetch_attachment(
                &u.f.state.http_client,
                &"app:secret".into(),
                &selected,
                crate::services::attachment_extraction::MAX_BYTES as u64,
            )
            .await
            .unwrap();
        assert_eq!(fetched.bytes.as_ref(), bytes);
        assert_eq!(fetched.filename.is_some(), original);
        let item = uploads::upload_lark_channel_authorized(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            &u.f.owner,
            &u.f.row.id,
            fetched.filename.as_deref(),
            fetched.bytes.to_vec(),
            crate::models::assistant_channel_media::UploadSource {
                guest: None,
                turn_id: u.f.chat.turn_id.clone().unwrap(),
                event_key: source.id.clone(),
                index: 0,
                embedded: false,
                declared: 1,
            },
            None,
        )
        .await
        .unwrap();
        bind_upload_id(&u.f, &item.id).await;
        let roots =
            u.f.state
                .db
                .collection::<crate::models::assistant_upload::AssistantUpload>(
                    crate::models::assistant_upload::COLLECTION_NAME,
                );
        let row = roots
            .find_one(doc! {"_id":&item.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.filename_is_original, Some(original));
        assert_eq!(row.label, expected);
        let kind = row.content_type.clone();
        let auth = authenticate(&u.f).await;
        let input = json!({"attachment_id":item.id,"parent_type":if kind.starts_with("image/") {"bitable_image"} else {"bitable_file"},"parent_node":"base"});
        let response = named_call(&u, &auth, false, UPLOAD_OPERATION, input).await;
        assert_eq!(response["result"]["isError"], false, "{response}");
        let response = call(&u, &auth, true, args(&item.id)).await;
        assert_eq!(payload(&response)["record_id"], "rec_receipt");
        let sent = requests(&u).await;
        let sent: Vec<_> = sent
            .iter()
            .filter(|request| request.url.path().ends_with("upload_all"))
            .collect();
        assert_eq!(sent.len(), 2);
        for upload in sent {
            assert_upload(upload, &bytes, expected, &kind);
        }
        if expected == "attachment" {
            let mut legacy = row.clone();
            legacy.filename_is_original = None;
            assert_eq!(
                uploads::EgressAttachment::User(Box::new(legacy)).filename(),
                "attachment.pdf"
            );
        }
        u.server.verify().await;
        cleanup(u).await;
    }
}

#[tokio::test]
async fn browser_named_attachment_and_long_suffix_reach_base_without_legacy_repair() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    for name in [
        None,
        Some("attachment".to_owned()),
        Some(format!("{}.pdf", "a".repeat(121))),
        Some(format!("{}a.pdf", " ".repeat(120))),
    ] {
        let u = setup("browser_original_name").await;
        let bytes = pdf();
        // Exercise the real browser upload handler, including its optional name.
        let mut request = axum::http::Request::builder();
        if let Some(name) = &name {
            request = request.header("x-attachment-name", urlencoding::encode(name).into_owned());
        }
        let (_, axum::Json(item)) = crate::handlers::assistant_uploads::upload(
            axum::extract::State(u.f.state.clone()),
            crate::test_utils::test_auth_user(&u.f.owner),
            axum::extract::Path(u.f.row.id.clone()),
            request.body(axum::body::Body::from(bytes.clone())).unwrap(),
        )
        .await
        .unwrap();
        let item = serde_json::to_value(item).unwrap();
        let id = item["id"].as_str().unwrap().to_owned();
        bind_upload_id(&u.f, &id).await;
        let expected = name
            .as_deref()
            .map(uploads::safe_name)
            .unwrap_or_else(|| "attachment.pdf".into());
        let stored =
            u.f.state
                .db
                .collection::<crate::models::assistant_upload::AssistantUpload>(
                    crate::models::assistant_upload::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(stored.filename_is_original, Some(name.is_some()));
        assert_eq!(stored.label, expected);
        assert!(expected.chars().count() <= uploads::MAX_FILENAME_CHARS);
        if name.as_deref() != Some("attachment") {
            assert!(expected.ends_with(".pdf"));
        }
        let auth = authenticate(&u.f).await;
        let response = call(&u, &auth, false, args(&id)).await;
        assert_eq!(payload(&response)["record_id"], "rec_receipt");
        let sent = requests(&u).await;
        assert_upload(
            sent.iter()
                .find(|request| request.url.path().ends_with("upload_all"))
                .unwrap(),
            &bytes,
            &expected,
            "application/pdf",
        );
        cleanup(u).await;
    }
}
