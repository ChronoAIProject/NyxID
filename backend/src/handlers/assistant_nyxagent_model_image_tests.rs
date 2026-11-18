use super::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

#[test]
fn modalities_are_optional_and_malformed_metadata_is_unknown() {
    for (metadata, expected) in [
        (json!({}), None),
        (json!({"input_modalities":null}), None),
        (json!({"input_modalities":"text"}), None),
        (json!({"input_modalities":["text",17]}), None),
        (json!({"input_modalities":["text"]}), Some(vec!["text"])),
        (
            json!({"input_modalities":["text","image"]}),
            Some(vec!["text", "image"]),
        ),
        (json!({"input_modalities":[]}), Some(vec![])),
    ] {
        let mut row = metadata;
        row["id"] = json!(engine::DEFAULT_MODEL);
        let model = model_response(&row).unwrap();
        assert_eq!(
            model.input_modalities,
            expected.map(|values| values.into_iter().map(str::to_owned).collect())
        );
        assert_eq!(
            serde_json::to_value(&model)
                .unwrap()
                .get("input_modalities")
                .is_some(),
            model.input_modalities.is_some()
        );
    }
}

#[tokio::test]
async fn image_turn_respects_model_modalities_and_records_only_known_upstream_notices() {
    for (modalities, failed_lookup, upstream_notice, expected_reason) in [
        (Some(json!(["text"])), false, None, Some("model_text_only")),
        (None, false, None, None),
        (None, true, None, None),
        (
            Some(json!(["text", "image"])),
            false,
            Some("image_input_unsupported"),
            Some("image_input_unsupported"),
        ),
    ] {
        let (state, _, server) = setup(None, Duration::ZERO).await;
        let upstream = MockServer::start().await;
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .update_one(
                doc! {"slug":engine::SERVICE_SLUG},
                doc! {"$set":{"base_url":upstream.uri()}},
            )
            .await
            .unwrap();
        let mut model = json!({"id":engine::DEFAULT_MODEL});
        if let Some(modalities) = modalities {
            model["input_modalities"] = modalities;
        }
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(query_param("include_capabilities", "true"))
            .respond_with(if failed_lookup {
                ResponseTemplate::new(502)
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"data":[model]}))
            })
            .expect(1)
            .mount(&upstream)
            .await;
        Mock::given(method("GET")).and(path("/v1/capabilities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"protocol":"nyxagent-input-image-v1","input_image":{
                "version":1,"sources":["nyxid_attachment_url"],"max_images":8,"max_image_bytes":5_000_000,"max_total_bytes":20_000_000,"content_types":["image/png"]}})))
            .mount(&upstream).await;
        let mut completed = json!({"type":"response.completed","sequence_number":0,"response":{
            "id":RESPONSE,"conversation":{"id":SESSION},"status":"completed",
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Answer completed"}]}]}});
        if let Some(code) = upstream_notice {
            completed["response"]["nyxagent_notices"] = json!([{"code":code,"message":"SECRET upstream notice text"},{"code":"unknown_notice"},{"code":"private prose"}]);
        }
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(format!("data: {completed}\n\n"), "text/event-stream"),
            )
            .expect(2)
            .mount(&upstream)
            .await;
        let id = crate::handlers::assistant_uploads::draft(
            State(state.clone()),
            test_auth_user(OWNER),
            Json(crate::handlers::assistant_uploads::Draft { agent_id: None }),
        )
        .await
        .unwrap()
        .0["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut text_request = Request::builder()
            .method("POST")
            .body(Body::from(
                json!({"conversation_id":id,"text":"Hello"}).to_string(),
            ))
            .unwrap();
        text_request.extensions_mut().insert(SERVER_TURN_POLICY);
        drop(
            turns(State(state.clone()), test_auth_user(OWNER), text_request)
                .await
                .unwrap(),
        );
        settled(&state).await;
        assert!(
            !upstream
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/v1/models" || r.url.path() == "/v1/capabilities")
        );
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let attachment = crate::services::assistant_upload_service::upload(
            &state.db,
            &state.encryption_keys,
            OWNER,
            &id,
            "photo.png",
            png.into_inner(),
        )
        .await
        .unwrap();
        let mut request=Request::builder().method("POST").body(Body::from(json!({"conversation_id":id,"text":"Describe this image","attachment_ids":[attachment.id]}).to_string())).unwrap();
        request.extensions_mut().insert(SERVER_TURN_POLICY);
        let response = turns(State(state.clone()), test_auth_user(OWNER), request)
            .await
            .unwrap();
        let output = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let row = settled(&state).await;
        let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
            .await
            .unwrap();
        let user = messages
            .iter()
            .find(|m| m.role == "user" && !m.attachments.is_empty())
            .unwrap();
        assert_eq!(
            user.attachments[0].image_input.as_deref(),
            Some(if expected_reason.is_some() {
                "unavailable"
            } else {
                "sent"
            })
        );
        assert_eq!(
            user.attachments[0].image_input_reason.as_deref(),
            expected_reason
        );
        let reply = messages
            .iter()
            .find(|m| m.role == "assistant" && m.turn_id == user.turn_id)
            .unwrap();
        assert_eq!(
            reply.status, "completed",
            "error_code={:?}",
            reply.error_code
        );
        assert!(!String::from_utf8_lossy(&output).contains("SECRET upstream notice text"));
        let dto =
            serde_json::to_value(AttachmentResponse::from(user.attachments[0].clone())).unwrap();
        assert_eq!(dto["image_input_reason"].as_str(), expected_reason);
        let requests = upstream.received_requests().await.unwrap();
        let sent = requests
            .iter()
            .rfind(|r| r.method == "POST" && r.url.path() == "/v1/responses")
            .unwrap();
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(
            body.to_string().contains("/assistant-attachments/"),
            expected_reason != Some("model_text_only")
        );
        if expected_reason == Some("model_text_only") {
            assert!(body.to_string().contains("this model can’t view images"));
            assert!(
                !body["input"]
                    .as_str()
                    .unwrap()
                    .contains("nyx__machine_save_attachment")
            );
        }
        // This also proves the per-owner success/failure cache is shared by
        // turn planning and account-level discovery, with no second GET.
        let _ = live_models(&state, &test_auth_user(OWNER), Some(SERVER_TURN_POLICY)).await;
        upstream.verify().await;
        MODELS
            .lock()
            .await
            .remove(&format!("{}:{OWNER}", state.db.name()));
        server.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn text_only_tool_images_continue_with_a_notice_and_no_image_url() {
    let capability_calls = Arc::new(AtomicUsize::new(0));
    let (state, calls, server) = setup_options_with_tools(
        None,
        Duration::ZERO,
        Vec::new(),
        true,
        1,
        capability_calls.clone(),
    )
    .await;
    let cache_key = format!("{}:{OWNER}", state.db.name());
    MODELS.lock().await.insert(
        cache_key.clone(),
        (
            Instant::now(),
            Some(vec![ModelInfo {
                id: engine::DEFAULT_MODEL.into(),
                label: "Text model".into(),
                available: true,
                description: None,
                input_modalities: Some(vec!["text".into()]),
            }]),
        ),
    );
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    let _ = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let row = settled(&state).await;
    let history = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    let reply = history.iter().find(|m| m.role == "assistant").unwrap();
    assert_eq!(reply.status, "completed");
    assert_eq!(
        reply.attachments[0].image_input.as_deref(),
        Some("unavailable")
    );
    assert_eq!(
        reply.attachments[0].image_input_reason.as_deref(),
        Some("model_text_only")
    );
    let requests = calls.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].body["input"]
            .as_str()
            .unwrap()
            .contains("this model can’t view images")
    );
    assert!(
        !requests[1]
            .body
            .to_string()
            .contains("/assistant-attachments/")
    );
    assert_eq!(capability_calls.load(Ordering::SeqCst), 0);
    MODELS.lock().await.remove(&cache_key);
    server.abort();
    state.db.drop().await.unwrap();
}
