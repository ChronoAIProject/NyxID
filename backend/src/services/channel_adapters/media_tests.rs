use super::*;
use crate::services::channel_platform::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, header, method, path},
};

fn attachment(url: String) -> InboundAttachment {
    InboundAttachment {
        content_type: "file".into(),
        url,
        platform_message_id: Some("123".into()),
        file_key: Some("456".into()),
        image_key: None,
        filename: Some("report.pdf".into()),
        mime_type: Some("application/pdf".into()),
        size_bytes: None,
    }
}
fn outbound(kind: MediaKind) -> OutboundReply {
    OutboundReply {
        attachments: vec![MaterializedAttachment {
            kind,
            bytes: bytes::Bytes::from_static(b"media-bytes"),
            filename: Some("test.bin".into()),
            mime_type: Some("application/octet-stream".into()),
            caption: None,
        }],
        text: None,
        metadata: None,
        reply_to_platform_message_id: None,
    }
}
fn adapters(base: &str) -> Vec<Box<dyn PlatformAdapter>> {
    vec![
        Box::new(telegram::TelegramAdapter::media_test_adapter(base)),
        Box::new(telegram_new::TelegramNewAdapter::media_test_adapter(base)),
        Box::new(discord::DiscordAdapter::media_test_adapter(base)),
        Box::new(slack::SlackAdapter::media_test_adapter(base)),
        Box::new(lark::LarkFamilyAdapter::media_test_adapter(base, "lark")),
        Box::new(lark::LarkFamilyAdapter::media_test_adapter(base, "feishu")),
        Box::new(x::XAdapter {
            api_base: Some(base.into()),
        }),
    ]
}

#[tokio::test]
async fn channel_image_adapters_classify_refusals_and_unknown_receipts() {
    use crate::services::channel_media_service::is_delivery_uncertain;
    let server = MockServer::builder().start().await;
    // A short client timeout exercises an absent response without waiting for
    // the production media deadline. Successful control calls remain local.
    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(100))
        .build()
        .unwrap();
    let image_adapters = adapters(&server.uri());
    for adapter in &image_adapters {
        let platform = adapter.platform_id();
        let (endpoint, refusal) = match platform {
            "telegram" | "telegram-new" => (
                "/bottoken/sendPhoto",
                serde_json::json!({"ok":false,"error_code":400,"description":"invalid request"}),
            ),
            "lark" | "feishu" => (
                "/open-apis/im/v1/images",
                serde_json::json!({"code":999,"msg":"invalid request"}),
            ),
            "discord" => (
                "/channels/123/messages",
                serde_json::json!({"code":50035,"message":"invalid request"}),
            ),
            "slack" => (
                "/files.getUploadURLExternal",
                serde_json::json!({"ok":false,"error":"invalid_arguments"}),
            ),
            "x" => (
                "/2/media/upload",
                serde_json::json!({"errors":[{"message":"invalid request"}]}),
            ),
            _ => unreachable!(),
        };
        for (status, body, uncertain) in [
            (400, refusal.to_string(), false),
            (408, refusal.to_string(), false),
            (503, refusal.to_string(), false),
            (200, refusal.to_string(), false),
            (200, "not JSON".into(), true),
            (200, "{}".into(), true),
        ] {
            server.reset().await;
            if matches!(platform, "lark" | "feishu") {
                Mock::given(path("/open-apis/auth/v3/tenant_access_token/internal"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(
                        serde_json::json!({"code":0,"tenant_access_token":"tenant","expire":7200}),
                    ))
                    .mount(&server)
                    .await;
            }
            Mock::given(method("POST"))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;
            let token = if matches!(platform, "lark" | "feishu") {
                "app:secret"
            } else {
                "token"
            };
            let error = adapter
                .send_reply(&http, &token.into(), "123", &outbound(MediaKind::Image))
                .await
                .unwrap_err();
            assert_eq!(
                is_delivery_uncertain(&error),
                uncertain,
                "{platform} HTTP {status}: {error}"
            );
            server.verify().await;
        }
    }
    // Reuse these adapters after closing their server. Lark retains the tenant
    // token obtained above, so this exercises the media POST itself.
    drop(server);
    for adapter in image_adapters {
        let token = if matches!(adapter.platform_id(), "lark" | "feishu") {
            "app:secret"
        } else {
            "token"
        };
        let error = adapter
            .send_reply(&http, &token.into(), "123", &outbound(MediaKind::Image))
            .await
            .unwrap_err();
        assert!(
            is_delivery_uncertain(&error),
            "{}: {error}",
            adapter.platform_id()
        );
    }
}

#[tokio::test]
async fn channel_telegram_photo_fallback_preserves_context_and_reports_document() {
    let server = MockServer::start().await;
    for adapter in adapters(&server.uri()).into_iter().take(2) {
        for marker in [
            "PHOTO_INVALID_DIMENSIONS",
            "PHOTO_SAVE_FILE_INVALID",
            "PHOTO_EXT_INVALID",
            "PHOTO_CROP_SIZE_SMALL",
            "IMAGE_PROCESS_FAILED",
        ] {
            server.reset().await;
            Mock::given(path("/bottoken/sendPhoto"))
                .respond_with(ResponseTemplate::new(400).set_body_json(
                    serde_json::json!({"ok":false,"error_code":400,"description":marker}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(path("/bottoken/sendDocument"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok":true,"result":{"message_id":42}})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let mut reply = outbound(MediaKind::Image);
            reply.reply_to_platform_message_id = Some("17".into());
            reply.metadata = Some(serde_json::json!({"message_thread_id":"19"}));
            reply.attachments[0].caption = Some("caption".into());
            let outcome = adapter
                .send_reply_outcome(&reqwest::Client::new(), &"token".into(), "123", &reply)
                .await
                .unwrap();
            assert_eq!(outcome.final_id.as_deref(), Some("42"));
            assert_eq!(outcome.delivered_kinds, Some(vec![MediaKind::File]));
            assert!(outcome.error.is_none());
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 2);
            for request in requests {
                let body = String::from_utf8_lossy(&request.body);
                for part in [
                    "name=\"chat_id\"\r\n\r\n123",
                    "name=\"reply_to_message_id\"\r\n\r\n17",
                    "name=\"message_thread_id\"\r\n\r\n19",
                    "name=\"caption\"\r\n\r\ncaption",
                    "media-bytes",
                ] {
                    assert!(body.contains(part), "missing {part}");
                }
            }
            server.verify().await;
        }
    }
}

#[tokio::test]
async fn channel_telegram_fallback_never_retries_other_failures_and_document_is_final() {
    use crate::services::channel_media_service::is_delivery_uncertain;
    let server = MockServer::start().await;
    let adapter = telegram::TelegramAdapter::media_test_adapter(&server.uri());
    for (status, description) in [
        (400, "wrong file identifier"),
        (400, "wrong type of the web page content"),
        (400, "failed to get HTTP URL content"),
        (400, "file must be non-empty"),
        (400, "wrong padding in the data"),
        (400, "chat not found"),
        (400, "message thread not found"),
        (400, "replied message not found"),
        (400, "bot was blocked"),
        (400, "TOPIC_CLOSED"),
        (400, "not enough rights"),
        (400, "have no rights to send photos"),
        (400, "PHOTO_INVALID_DIMENSIONS; chat not found"),
        (403, "PHOTO_INVALID_DIMENSIONS"),
        (429, "PHOTO_INVALID_DIMENSIONS"),
        (500, "PHOTO_INVALID_DIMENSIONS"),
    ] {
        server.reset().await;
        Mock::given(path("/bottoken/sendPhoto"))
            .respond_with(ResponseTemplate::new(status).set_body_json(
                serde_json::json!({"ok":false,"error_code":400,"description":description}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let error = adapter
            .send_reply(
                &reqwest::Client::new(),
                &"token".into(),
                "123",
                &outbound(MediaKind::Image),
            )
            .await
            .unwrap_err();
        assert!(!is_delivery_uncertain(&error));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        server.verify().await;
    }
    for (status, body, uncertain) in [
        (
            400,
            serde_json::json!({"ok":false,"description":"PHOTO_INVALID_DIMENSIONS"}).to_string(),
            false,
        ),
        (200, "not JSON".into(), true),
    ] {
        server.reset().await;
        Mock::given(path("/bottoken/sendPhoto"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({"ok":false,"error_code":400,"description":"PHOTO_INVALID_DIMENSIONS"})))
            .expect(1).mount(&server).await;
        Mock::given(path("/bottoken/sendDocument"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let error = adapter
            .send_reply(
                &reqwest::Client::new(),
                &"token".into(),
                "123",
                &outbound(MediaKind::Image),
            )
            .await
            .unwrap_err();
        assert_eq!(is_delivery_uncertain(&error), uncertain);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        server.verify().await;
    }
}

#[tokio::test]
async fn channel_media_fetch_paths_success_size_and_host_contracts() {
    let server = MockServer::start().await;
    let http = reqwest::Client::new();
    for adapter in adapters(&server.uri()) {
        server.reset().await;
        let telegram = matches!(adapter.platform_id(), "telegram" | "telegram-new");
        let lark = matches!(adapter.platform_id(), "lark" | "feishu");
        let token = if lark { "app:secret" } else { "token" };
        if lark {
            Mock::given(method("POST"))
                .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"code":0,"tenant_access_token":"tenant","expire":7200}),
                ))
                .expect(1)
                .mount(&server)
                .await;
        }
        if telegram {
            Mock::given(method("GET"))
                .and(path("/bottoken/getFile"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"ok":true,"result":{"file_path":"document/file.pdf"}}),
                ))
                .expect(2)
                .mount(&server)
                .await;
        }
        let download_path = if telegram {
            "/file/bottoken/document/file.pdf"
        } else {
            "/resource"
        };
        let mut download = Mock::given(method("GET")).and(path(download_path));
        if !telegram && adapter.platform_id() != "discord" {
            download = download.and(header(
                "Authorization",
                if lark {
                    "Bearer tenant"
                } else {
                    "Bearer token"
                },
            ));
        }
        download
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/pdf")
                    .set_body_bytes(b"document".to_vec()),
            )
            .expect(2)
            .mount(&server)
            .await;
        let a = attachment(if telegram {
            "file-id".into()
        } else {
            format!("{}/resource", server.uri())
        });
        let fetched = adapter
            .fetch_attachment(&http, &token.into(), &a, 20)
            .await
            .unwrap();
        assert_eq!(fetched.bytes, b"document"[..]);
        assert_eq!(fetched.mime_type.as_deref(), Some("application/pdf"));
        assert!(
            matches!(
                adapter.fetch_attachment(&http, &token.into(), &a, 2).await,
                Err(AppError::ChannelMediaTooLarge)
            ),
            "{}",
            adapter.platform_id()
        );
        let bad = attachment("https://evil.example/secret".into());
        assert!(matches!(
            adapter
                .fetch_attachment(&http, &token.into(), &bad, 20)
                .await,
            Err(AppError::ChannelMediaFetchFailed(_))
        ));
        server.verify().await;
    }
}

#[tokio::test]
async fn channel_media_native_upload_paths_all_declared_kinds() {
    let server = MockServer::start().await;
    let http = reqwest::Client::new();
    for adapter in adapters(&server.uri()) {
        for &kind in adapter.media_capabilities().outbound {
            server.reset().await;
            let mut expected = "last";
            let token = if matches!(adapter.platform_id(), "lark" | "feishu") {
                "app:secret"
            } else {
                "token"
            };
            match adapter.platform_id() {
                "telegram" | "telegram-new" => {
                    let endpoint = match kind {
                        MediaKind::Image => "sendPhoto",
                        MediaKind::File => "sendDocument",
                        MediaKind::Audio => "sendAudio",
                        MediaKind::Video => "sendVideo",
                    };
                    Mock::given(method("POST"))
                        .and(path(format!("/bottoken/{endpoint}")))
                        .and(body_string_contains("media-bytes"))
                        .respond_with(ResponseTemplate::new(200).set_body_json(
                            serde_json::json!({"ok":true,"result":{"message_id":42}}),
                        ))
                        .expect(1)
                        .mount(&server)
                        .await;
                    expected = "42";
                }
                "discord" => {
                    Mock::given(method("POST"))
                        .and(path("/channels/123/messages"))
                        .and(header("Authorization", "Bot token"))
                        .and(body_string_contains("files[0]"))
                        .and(body_string_contains("payload_json"))
                        .and(body_string_contains("media-bytes"))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .set_body_json(serde_json::json!({"id":"last"})),
                        )
                        .expect(1)
                        .mount(&server)
                        .await;
                }
                "slack" => {
                    for (endpoint, value) in [
                        (
                            "files.getUploadURLExternal",
                            serde_json::json!({"ok":true,"file_id":"F1","upload_url":format!("{}/upload",server.uri())}),
                        ),
                        (
                            "files.completeUploadExternal",
                            serde_json::json!({"ok":true,"files":[{"id":"F1"}]}),
                        ),
                    ] {
                        Mock::given(method("POST"))
                            .and(path(format!("/{endpoint}")))
                            .and(header("Authorization", "Bearer token"))
                            .respond_with(ResponseTemplate::new(200).set_body_json(value))
                            .expect(1)
                            .mount(&server)
                            .await;
                    }
                    Mock::given(method("PUT"))
                        .and(path("/upload"))
                        .and(body_string_contains("media-bytes"))
                        .respond_with(ResponseTemplate::new(200))
                        .expect(1)
                        .mount(&server)
                        .await;
                    Mock::given(method("GET")).and(path("/files.info")).respond_with(ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok":true,"file":{"shares":{"private":{"123":[{"ts":"last"}]}}}})))
                        .expect(1).mount(&server).await;
                }
                "lark" | "feishu" => {
                    Mock::given(method("POST")).and(path("/open-apis/auth/v3/tenant_access_token/internal"))
                        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"code":0,"tenant_access_token":"tenant","expire":7200})))
                        .mount(&server).await;
                    let (endpoint, key) = if kind == MediaKind::Image {
                        ("images", "image_key")
                    } else {
                        ("files", "file_key")
                    };
                    Mock::given(method("POST"))
                        .and(path(format!("/open-apis/im/v1/{endpoint}")))
                        .and(header("Authorization", "Bearer tenant"))
                        .and(body_string_contains("media-bytes"))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .set_body_json(serde_json::json!({"code":0,"data":{key:"handle"}})),
                        )
                        .expect(1)
                        .mount(&server)
                        .await;
                    Mock::given(method("POST"))
                        .and(path("/open-apis/im/v1/messages"))
                        .and(body_string_contains("handle"))
                        .respond_with(ResponseTemplate::new(200).set_body_json(
                            serde_json::json!({"code":0,"data":{"message_id":"last"}}),
                        ))
                        .expect(1)
                        .mount(&server)
                        .await;
                }
                "x" => {
                    Mock::given(method("POST"))
                        .and(path("/2/media/upload"))
                        .and(header("Authorization", "Bearer token"))
                        .and(body_string_contains(if kind == MediaKind::Image {
                            "dm_image"
                        } else {
                            "dm_video"
                        }))
                        .and(body_string_contains("media-bytes"))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .set_body_json(serde_json::json!({"data":{"id":"12345"}})),
                        )
                        .expect(1)
                        .mount(&server)
                        .await;
                    Mock::given(method("POST"))
                        .and(path("/2/dm_conversations/123/messages"))
                        .and(body_string_contains("12345"))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .set_body_json(serde_json::json!({"data":{"dm_event_id":"last"}})),
                        )
                        .expect(1)
                        .mount(&server)
                        .await;
                }
                _ => unreachable!(),
            }
            assert_eq!(
                adapter
                    .send_reply(&http, &token.into(), "123", &outbound(kind))
                    .await
                    .unwrap()
                    .as_deref(),
                Some(expected),
                "{}",
                adapter.platform_id()
            );
            server.verify().await;
        }
    }
}

#[tokio::test]
async fn channel_x_media_waits_for_processing_and_never_sends_failed_uploads() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    let adapter = x::XAdapter {
        api_base: Some(server.uri()),
    };
    for failed in [false, true] {
        server.reset().await;
        Mock::given(method("POST")).and(path("/2/media/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":{"id":"12345","processing_info":{"state":"pending","check_after_secs":1}}})))
            .expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path("/2/media/upload")).and(query_param("command","STATUS"))
            .and(query_param("media_id","12345")).and(header("Authorization","Bearer token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":{"processing_info":{"state": if failed {"failed"} else {"succeeded"}}}})))
            .expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/2/dm_conversations/123/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":{"dm_event_id":"last"}})),
            )
            .expect(if failed { 0 } else { 1 })
            .mount(&server)
            .await;
        let result = adapter
            .send_reply(
                &reqwest::Client::new(),
                &"token".into(),
                "123",
                &outbound(MediaKind::Video),
            )
            .await;
        if failed {
            assert!(matches!(result, Err(AppError::ChannelPlatformError(_))));
        } else {
            assert_eq!(result.unwrap().as_deref(), Some("last"));
        }
        server.verify().await;
    }
}

#[tokio::test]
async fn channel_whatsapp_image_classification_includes_window_refusal() {
    use crate::services::channel_media_service::is_delivery_uncertain;
    let server = MockServer::builder().start().await;
    let base = server.uri();
    let credentials = BotCredentials {
        billing: None,
        token: "token",
        platform_bot_id: Some("123"),
        platform_secrets: None,
    };
    for (status, body, uncertain) in [
        (
            400,
            serde_json::json!({"error":{"code":131047,"message":"outside window"}}).to_string(),
            false,
        ),
        (503, "unavailable".into(), false),
        (
            200,
            serde_json::json!({"error":{"code":100,"message":"refused"}}).to_string(),
            false,
        ),
        (200, "not JSON".into(), true),
        (200, "{}".into(), true),
    ] {
        server.reset().await;
        Mock::given(path("/123/media"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let error = whatsapp::send_outcome_at(
            &reqwest::Client::new(),
            &credentials,
            "234",
            &outbound(MediaKind::Image),
            &base,
        )
        .await
        .and_then(SendOutcome::into_result)
        .unwrap_err();
        assert_eq!(
            (is_delivery_uncertain(&error) || whatsapp::image_delivery_uncertain(&error)),
            uncertain,
            "HTTP {status}: {error}"
        );
        if status == 400 {
            assert!(matches!(
                error,
                AppError::ChannelConversationNotReachable(_)
            ));
        }
        server.verify().await;
    }
    drop(server);
    let error = whatsapp::send_outcome_at(
        &reqwest::Client::new(),
        &credentials,
        "234",
        &outbound(MediaKind::Image),
        &base,
    )
    .await
    .and_then(SendOutcome::into_result)
    .unwrap_err();
    assert!(is_delivery_uncertain(&error) || whatsapp::image_delivery_uncertain(&error));
}

#[tokio::test]
async fn channel_whatsapp_image_unknown_message_preserves_operator_diagnostic() {
    let server = MockServer::start().await;
    let credentials = BotCredentials {
        billing: None,
        token: "token",
        platform_bot_id: Some("123"),
        platform_secrets: None,
    };
    for (body, diagnostic) in [
        ("not JSON", "invalid JSON"),
        ("{}", "no usable message ID; acceptance is unknown"),
    ] {
        server.reset().await;
        Mock::given(path("/123/media"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"upload"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let outcome = whatsapp::send_outcome_at(
            &reqwest::Client::new(),
            &credentials,
            "234",
            &outbound(MediaKind::Image),
            &server.uri(),
        )
        .await
        .unwrap();
        assert!(outcome.observation.unwrap().uncertain);
        let error = outcome.error.unwrap();
        assert!(error.to_string().contains(diagnostic), "{error}");
        assert!(whatsapp::image_delivery_uncertain(&error));
        assert!(!crate::services::channel_media_service::is_delivery_uncertain(&error));
        server.verify().await;
    }
}
