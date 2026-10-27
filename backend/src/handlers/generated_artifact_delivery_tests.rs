//! Real MCP admission, scoped result calls, event settlement and private owner reads.
use super::*;
use crate::models::generated_artifact::{COLLECTION_NAME as ARTIFACTS, DELIVERIES};
use crate::services::{feature_flag_service as flags, generated_artifacts as artifacts};
use base64::Engine;
use tower::ServiceExt;
const MP4: &[u8] = include_bytes!("../../tests/fixtures/generated-artifacts/visual-avc.mp4");

async fn enabled(a: &AsyncFixture, value: bool) {
    flags::set_platform_override(
        &a.f.state.db,
        flags::ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY,
        &flags::FlagTarget::User(a.f.owner.clone()),
        value,
        &a.f.owner,
    )
    .await
    .unwrap();
}
async fn generated() -> AsyncFixture {
    let a = AsyncFixture::new().await;
    enabled(&a, true).await;
    *a.output.lock().unwrap() = json!({"success":true,"output":{"stdout":"created","stderr":"","exit_code":0,
        "files":[{"path":"/tmp/movie.mp4","content":base64::engine::general_purpose::STANDARD.encode(MP4),"error":null,"size":MP4.len()}]}});
    a
}
fn boxed_generated() -> futures::future::BoxFuture<'static, AsyncFixture> {
    Box::pin(generated())
}

#[tokio::test]
async fn generated_artifact_persisted_summary_and_input_context_exclude_process_streams() {
    let a = boxed_generated().await;
    {
        let mut output = a.output.lock().unwrap();
        output["output"]["stdout"] =
            json!("/private/stdout-path-marker stdout-file-content-marker");
        output["output"]["stderr"] =
            json!("/private/stderr-path-marker stderr-secret-content-marker");
    }
    a.submit().await;
    complete(&a).await;
    let w = a.watch().await;
    let plaintext = crate::services::record_field_service::decrypt(
        &a.f.state.encryption_keys,
        WATCHES,
        &w.id,
        &w.user_id,
        "result_encrypted",
        w.secret_context_versions.get("result_encrypted").copied(),
        w.result_encrypted.as_deref().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&plaintext).unwrap(),
        json!({"success":true,"exit_code":0,"verified_file_count":1})
    );
    assert!(!std::str::from_utf8(&plaintext).unwrap().contains("marker"));
    finish(&a, &a.f.row, "submitted").await;
    let event = admit_event(&a).await;
    let context = watches::input_context(&a.f.state.db, &a.f.state.encryption_keys, &event)
        .await
        .unwrap();
    assert!(context.contains("materialized"));
    assert!(context.contains("verified_file_count"));
    assert!(!context.contains("marker"));
    assert!(!context.contains("movie.mp4"));
    assert!(!context.contains(&base64::engine::general_purpose::STANDARD.encode(MP4)));

    // A previously persisted free-form generated summary must also be withheld.
    let old_text =
        b"Sandbox script success: true. stdout: /private/old-path-marker old-content-marker";
    let encrypted = crate::services::record_field_service::encrypt(
        &a.f.state.encryption_keys,
        WATCHES,
        &w.id,
        &w.user_id,
        "result_encrypted",
        old_text,
    )
    .await
    .unwrap();
    a.f.state
        .db
        .collection::<bson::Document>(WATCHES)
        .update_one(
            doc! {"_id":&w.id,"state":"queued"},
            doc! {"$set":{"result_encrypted":bson::to_bson(&Some(encrypted)).unwrap()}},
        )
        .await
        .unwrap();
    let context = watches::input_context(&a.f.state.db, &a.f.state.encryption_keys, &event)
        .await
        .unwrap();
    assert!(context.contains("materialized"));
    assert!(context.contains("verified_file_count"));
    assert!(!context.contains("marker"));
    assert!(!context.contains("stdout"));
    finish(&a, &event, "The retained result is attached.").await;
}

#[tokio::test]
async fn generated_artifact_recovery_audits_http_direct_and_wrapped_requests_without_lineage() {
    let a = AsyncFixture::new().await;
    let auth = authenticate(&a.f).await;
    let secret_id = "must-not-enter-the-audit";
    let args = json!({"message_id":secret_id,"operation_id":secret_id});
    // Even a refusal/acknowledgement is an execution request and is audited.
    let _ = direct_call(&a.f, &auth, "nyx__artifact_delivery", args.clone()).await;
    let _ = call(&a.f, &auth, "nyx__artifact_delivery", args).await;
    assert!(
        crate::handlers::assistant_nyxagent::artifact_delivery(
            axum::extract::State(a.f.state.clone()),
            crate::test_utils::test_auth_user(&a.f.owner),
            axum::extract::Path((a.f.row.id.clone(), secret_id.into())),
            axum::body::Bytes::from_static(b"{}"),
        )
        .await
        .is_err()
    );
    let entries: Vec<crate::models::audit_log::AuditLog> = a.f.state.db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":"assistant_mcp_tool_call","event_data.tool_name":"nyx__artifact_delivery"})
        .await.unwrap().try_collect().await.unwrap();
    assert_eq!(entries.len(), 3);
    let mut transports = Vec::new();
    for entry in entries {
        assert!(entry.seq.is_some() && entry.entry_hash.is_some());
        let data = entry.event_data.unwrap();
        assert!(!data.to_string().contains(secret_id));
        let http = data["transport"] == "http";
        assert_eq!(data, artifacts::recovery_audit_data(http));
        transports.push(http);
    }
    assert_eq!(transports.iter().filter(|http| **http).count(), 1);
    assert_eq!(a.calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn generated_artifact_flag_off_uses_legacy_and_admitted_watch_drains_after_disable() {
    check_artifact_flag_off_legacy().await;
    let a = boxed_generated().await;
    let (w, event, calls) = check_admitted_artifact_drain(&a).await;
    check_artifact_chat_recovery(&a, &w, &event).await;
    assert_eq!(
        a.calls.lock().unwrap().len(),
        calls,
        "recovery must make no Sandbox request"
    );
    assert_eq!(
        a.f.state
            .db
            .collection::<bson::Document>(ARTIFACTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        a.f.state
            .db
            .collection::<bson::Document>(ARTIFACTS)
            .find_one(doc! {"data_encrypted":{"$ne":null}})
            .await
            .unwrap()
            .is_none()
    );
}

// Construct each phase outside the test driver. The combined debug poll frame
// otherwise stays on the stack throughout the nested MCP/proxy/BSON calls.
fn check_artifact_flag_off_legacy() -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async {
        let legacy = AsyncFixture::new().await;
        let auth = authenticate(&legacy.f).await;
        let tools = chat_native_definitions(&legacy.f.state, auth.chat.as_ref().unwrap()).await;
        assert!(
            !tools
                .iter()
                .any(|tool| tool.name == "nyx__artifact_delivery")
        );
        enabled(&legacy, true).await;
        let enabled_tools =
            chat_native_definitions(&legacy.f.state, auth.chat.as_ref().unwrap()).await;
        let without_recovery: Vec<_> = enabled_tools
            .iter()
            .filter(|tool| tool.name != "nyx__artifact_delivery")
            .map(|tool| (&tool.name, &tool.description, &tool.input_schema))
            .collect();
        assert_eq!(enabled_tools.len(), tools.len() + 1);
        assert_eq!(
            without_recovery,
            tools
                .iter()
                .map(|tool| (&tool.name, &tool.description, &tool.input_schema))
                .collect::<Vec<_>>()
        );
        enabled(&legacy, false).await;
        assert!(legacy.submit().await.artifact_mode.is_none());
        complete(&legacy).await;
        assert_eq!(
            legacy
                .f
                .state
                .db
                .collection::<bson::Document>(ARTIFACTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    })
}

fn check_admitted_artifact_drain(
    a: &AsyncFixture,
) -> futures::future::BoxFuture<
    '_,
    (
        AsyncServiceOperation,
        crate::models::assistant_conversation::AssistantConversation,
        usize,
    ),
> {
    Box::pin(async move {
        let w = a.submit().await;
        assert_eq!(w.artifact_mode.as_deref(), Some(artifacts::sandbox::MODE));
        enabled(a, false).await;
        let auth = authenticate(&a.f).await;
        let tools = chat_native_definitions(&a.f.state, auth.chat.as_ref().unwrap()).await;
        assert!(
            !tools
                .iter()
                .any(|tool| tool.name == "nyx__artifact_delivery"),
            "admitted history must not change flag-off discovery"
        );
        complete(a).await;
        assert_eq!(a.watch().await.artifact_result.as_ref().unwrap().count, 1);
        finish(a, &a.f.row, "submitted").await;
        let event = admit_event(a).await;
        let attachment = event.active_turn.as_ref().unwrap().attachments[0].clone();
        assert_eq!(attachment.origin, "generated_artifact");
        let context = watches::input_context(&a.f.state.db, &a.f.state.encryption_keys, &event)
            .await
            .unwrap();
        assert!(context.contains("materialized"));
        assert!(!context.contains("/tmp/movie.mp4"));
        assert!(!context.contains(&base64::engine::general_purpose::STANDARD.encode(MP4)));
        finish(a, &event, "The retained result is attached.").await;
        let messages: Vec<crate::models::assistant_message::AssistantMessage> =
            a.f.state
                .db
                .collection(crate::models::assistant_message::COLLECTION_NAME)
                .find(doc! {
                    "conversation_id": &event.id,
                    "role": "assistant",
                    "turn_id": &event.active_turn.as_ref().unwrap().turn_id,
                })
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].attachments[0].id, attachment.id);
        let delivery =
            a.f.state
                .db
                .collection::<crate::models::generated_artifact::Delivery>(DELIVERIES)
                .find_one(doc! {"_id":&messages[0].id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(delivery.attachment_ids, vec![attachment.id.clone()]);
        let calls = a.calls.lock().unwrap().len();
        for _ in 0..2 {
            let axum::Json(acknowledgement) =
                crate::handlers::assistant_nyxagent::artifact_delivery(
                    axum::extract::State(a.f.state.clone()),
                    crate::test_utils::test_auth_user(&a.f.owner),
                    axum::extract::Path((event.id.clone(), messages[0].id.clone())),
                    axum::body::Bytes::new(),
                )
                .await
                .unwrap();
            assert_eq!(acknowledgement, json!({"status":"materialized"}));
        }
        assert!(
            crate::handlers::assistant_nyxagent::artifact_delivery(
                axum::extract::State(a.f.state.clone()),
                crate::test_utils::test_auth_user(&a.f.owner),
                axum::extract::Path((event.id.clone(), messages[0].id.clone())),
                axum::body::Bytes::from_static(b"{}"),
            )
            .await
            .is_err()
        );
        let (mime, bytes) = engine::read_attachment(
            &a.f.state.db,
            &a.f.state.encryption_keys,
            &a.f.owner,
            &event.id,
            &attachment.id,
        )
        .await
        .unwrap();
        assert_eq!(mime, "video/mp4");
        assert_eq!(bytes, MP4);
        (w, event, calls)
    })
}

fn check_artifact_chat_recovery<'a>(
    a: &'a AsyncFixture,
    w: &'a AsyncServiceOperation,
    event: &'a crate::models::assistant_conversation::AssistantConversation,
) -> futures::future::BoxFuture<'a, ()> {
    Box::pin(async move {
        // Unrelated messages do not erase the latest server-owned saved result.
        let unrelated = engine::begin_turn(
            &a.f.state.db,
            &a.f.owner,
            &user_start(&event.id, "What time is it?"),
            &a.f.state.encryption_keys,
        )
        .await
        .unwrap();
        finish(
            a,
            &unrelated,
            "An unrelated answer with no generated files.",
        )
        .await;
        // A natural-language turn selects the latest preceding server association.
        let next = engine::begin_turn(
            &a.f.state.db,
            &a.f.owner,
            &user_start(&event.id, "Recover the preceding result"),
            &a.f.state.encryption_keys,
        )
        .await
        .unwrap();
        assert!(next.active_turn.as_ref().unwrap().platform_reply.is_none());
        let auth = authenticate(&a.f).await;
        let chat = auth.chat.as_ref().unwrap();
        assert!(
            artifacts::recover_for_chat(
                &a.f.state.db,
                &a.f.state.encryption_keys,
                chat,
                &json!({"watch_id":w.id})
            )
            .await
            .is_err()
        );
        artifacts::recover_for_chat(&a.f.state.db, &a.f.state.encryption_keys, chat, &json!({}))
            .await
            .unwrap();
        let mut guest = chat.clone();
        guest.guest = true;
        assert!(
            artifacts::recover_for_chat(
                &a.f.state.db,
                &a.f.state.encryption_keys,
                &guest,
                &json!({})
            )
            .await
            .is_err()
        );
        set_access_mode(
            &a.f,
            crate::models::assistant_settings::AccessMode::AskEveryTime,
        )
        .await;
        let refusal = result(
            direct_call(&a.f, &auth, "nyx__artifact_delivery", json!({})).await,
            true,
        )
        .await;
        assert_eq!(refusal["error"], "acknowledgement_required");
        let card = acks::history(&a.f.state.db, &a.f.owner, &event.id)
            .await
            .unwrap()
            .into_iter()
            .find(|card| {
                card.tool_name.as_deref() == Some("nyx__artifact_delivery")
                    && card.status == "pending"
            })
            .unwrap();
        finish(a, &next, "Please review the recovery action.").await;
        acks::decide(&a.f.state.db, &a.f.owner, &event.id, &card.id, true)
            .await
            .unwrap();
        let mut approved = user_start(&event.id, "Continue the approved recovery");
        approved.acknowledgement_id = Some(card.id.clone());
        let continued = engine::begin_turn(
            &a.f.state.db,
            &a.f.owner,
            &approved,
            &a.f.state.encryption_keys,
        )
        .await
        .unwrap();
        let auth = authenticate(&a.f).await;
        // The action remains {} even on approval continuation. Both the card and
        // original recovery source come from server-written message associations.
        let response = call(&a.f, &auth, "nyx__artifact_delivery", json!({})).await;
        let envelope: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 100_000)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(envelope["result"]["isError"], false, "{envelope}");
        assert_eq!(
            acks::history(&a.f.state.db, &a.f.owner, &event.id)
                .await
                .unwrap()
                .into_iter()
                .find(|row| row.id == card.id)
                .unwrap()
                .status,
            "used"
        );
        finish(a, &continued, "Recovered the saved attachments.").await;
    })
}

#[tokio::test]
async fn generated_artifact_production_routes_enforce_owner_bytes_and_recovery_lineage() {
    // Compare complete saved lineage rows, not just counts: refusals must not
    // create, delete, relabel, or rewrite delivery/message/attachment associations.
    fn saved_associations(
        db: &mongodb::Database,
    ) -> futures::future::BoxFuture<'_, Vec<Vec<bson::Document>>> {
        Box::pin(async move {
            let mut rows = Vec::new();
            for collection in [
                DELIVERIES,
                crate::models::assistant_message::COLLECTION_NAME,
                crate::models::assistant_attachment::COLLECTION_NAME,
            ] {
                let saved: Vec<bson::Document> = db
                    .collection(collection)
                    .find(doc! {})
                    .sort(doc! {"_id": 1})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                rows.push(saved);
            }
            rows
        })
    }

    fn assert_private_refusal<'a>(
        a: &'a AsyncFixture,
        response: axum::response::Response,
        expected_rows: &'a [Vec<bson::Document>],
        expected_calls: &'a [String],
        private_markers: &'a [String],
        content_revision: &'a str,
    ) -> futures::future::BoxFuture<'a, ()> {
        fn assert_private_json(value: &Value, private_markers: &[String], content_revision: &str) {
            let assert_private_text = |text: &str| {
                assert!(
                    !text.contains(content_revision)
                        && private_markers.iter().all(|marker| !text.contains(marker)),
                    "refusal disclosed saved artifact metadata in a JSON key or string"
                );
            };
            match value {
                Value::Object(fields) => {
                    for (key, value) in fields {
                        assert_private_text(key);
                        assert_private_json(value, private_markers, content_revision);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        assert_private_json(value, private_markers, content_revision);
                    }
                }
                Value::String(text) => assert_private_text(text),
                Value::Number(number) => {
                    if let Ok(revision) = content_revision.parse::<serde_json::Number>() {
                        assert_ne!(number, &revision, "refusal disclosed the numeric revision");
                    }
                }
                Value::Null | Value::Bool(_) => {}
            }
        }

        Box::pin(async move {
            use crate::errors::{AppError, access_denial::CredentialType};

            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert!(!content_revision.is_empty());
            assert!(!body.windows(MP4.len()).any(|bytes| bytes == MP4));
            assert!(!body.windows(12).any(|bytes| bytes == &MP4[4..16]));
            for marker in private_markers {
                assert!(!marker.is_empty());
                assert!(
                    !body
                        .windows(marker.len())
                        .any(|bytes| bytes == marker.as_bytes()),
                    "refusal disclosed saved artifact metadata or encoded bytes"
                );
            }
            if status == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                // The mounted zero-byte body limit rejects before AppError handling.
                assert_eq!(
                    &body[..],
                    b"Failed to buffer the request body: length limit exceeded"
                );
            } else {
                let error: Value = serde_json::from_slice(&body).expect("JSON AppError refusal");
                let envelope = error.as_object().expect("AppError object envelope");
                assert!(envelope.keys().all(|key| matches!(
                    key.as_str(),
                    "error" | "error_code" | "message" | "details"
                )));
                let public_error = match error["error"].as_str().expect("public error key") {
                    "unauthorized" => AppError::Unauthorized(String::new()),
                    "authentication_failed" => AppError::AuthenticationFailed(String::new()),
                    "token_expired" => AppError::TokenExpired,
                    "forbidden" if envelope.contains_key("details") => {
                        let credential_type = match error["details"]["credential_type"].as_str() {
                            Some("api_key") => CredentialType::ApiKey,
                            Some("delegated") => CredentialType::Delegated,
                            Some("service_account") => CredentialType::ServiceAccount,
                            Some("relay") => CredentialType::Relay,
                            _ => panic!("unexpected credential refusal details"),
                        };
                        AppError::unsupported_credential("", credential_type)
                    }
                    "forbidden" => AppError::Forbidden(String::new()),
                    "not_found" => AppError::NotFound(String::new()),
                    "attachment_expired" => AppError::AssistantAttachmentExpired,
                    _ => panic!("unexpected refusal error key"),
                };
                assert_eq!(status, public_error.status_code());
                let expected = public_error.response_body();
                assert_eq!(
                    error["error_code"].as_u64(),
                    Some(u64::from(expected.error_code))
                );
                assert!(error["message"].is_string());
                // Details must exactly match the fixed authentication reason or
                // credential guidance; extra fields and nested metadata fail.
                assert_eq!(envelope.get("details"), expected.details.as_ref());
                if status == axum::http::StatusCode::GONE {
                    assert_eq!(error["message"], expected.message);
                }
                // Numeric public codes such as 12101 are compared as whole numbers,
                // while revision strings remain forbidden in every key/string leaf.
                assert_private_json(&error, private_markers, content_revision);
            }
            let rows = saved_associations(&a.f.state.db).await;
            assert!(
                rows.as_slice() == expected_rows,
                "refusal changed saved delivery/message/attachment associations"
            );
            assert_eq!(a.calls.lock().unwrap().as_slice(), expected_calls);
        })
    }

    let a = boxed_generated().await;
    a.submit().await;
    complete(&a).await;
    finish(&a, &a.f.row, "submitted").await;
    let event = admit_event(&a).await;
    let event_turn_id = event.active_turn.as_ref().unwrap().turn_id.clone();
    let attachment = event.active_turn.as_ref().unwrap().attachments[0].clone();
    finish(&a, &event, "The retained result is attached.").await;
    let message =
        a.f.state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .find_one(doc! {
                "conversation_id": &a.f.row.id,
                "turn_id": &event_turn_id,
                "role": "assistant",
            })
            .await
            .unwrap()
            .expect("production route fixture must create the event message");
    assert_eq!(message.attachments[0].id, attachment.id);

    // The owner attachment route is the actual mounted session route. Use a
    // signed first-party owner token against the completed/idle conversation,
    // matching the Web UI contract; the generated bytes remain bound to the
    // completed event message.
    let owner_token = crate::crypto::jwt::generate_access_token(
        &a.f.state.jwt_keys,
        &a.f.state.config,
        &uuid::Uuid::parse_str(&a.f.owner).unwrap(),
        "",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = crate::routes::build_router_with_state(a.f.state.clone());
    let app = private.with_state(a.f.state.clone());
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/{}/attachments/{}",
        a.f.row.id, attachment.id
    );
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(&path)
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=4-15")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(
        response.headers()["content-range"],
        format!("bytes 4-15/{}", MP4.len())
    );
    assert_eq!(response.headers()["content-length"], "12");
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        response.headers()["content-security-policy"],
        "default-src 'none'; sandbox"
    );
    assert_eq!(
        response.headers()["content-disposition"],
        "inline; filename=\"movie.mp4\"; filename*=UTF-8''movie.mp4"
    );
    assert_eq!(
        &axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()[..],
        &MP4[4..16]
    );
    let saved_expiry =
        a.f.state
            .db
            .collection::<crate::models::assistant_attachment::AssistantAttachment>(
                crate::models::assistant_attachment::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &attachment.id})
            .await
            .unwrap()
            .unwrap()
            .artifact
            .expires_at
            .expect("generated attachment expiry");
    let recovery_path = format!(
        "/api/v1/assistant/nyxagent/conversations/{}/messages/{}/artifact-delivery",
        a.f.row.id, message.id
    );
    let source = a.watch().await;
    let manifest = source.artifact_result.as_ref().unwrap();
    let private_markers = vec![
        a.f.row.id.clone(),
        message.id.clone(),
        event_turn_id.clone(),
        attachment.id.clone(),
        source.id.clone(),
        source.operation_id.clone().expect("the saved operation ID"),
        manifest.source_result_identity.clone(),
        attachment
            .artifact
            .filename
            .clone()
            .expect("the saved filename"),
        "/tmp/movie.mp4".into(),
        base64::engine::general_purpose::STANDARD.encode(MP4),
    ];
    let associations = saved_associations(&a.f.state.db).await;
    assert_eq!(associations[0].len(), 1, "the saved delivery association");
    let calls_before_refusals = a.calls.lock().unwrap().clone();
    let calls_before_recovery = calls_before_refusals.len();

    // Only the fixture changes expiry. Keep a separate expected snapshot so
    // neither an expired GET nor recovery can hide a persistent lineage write.
    let expired_at = bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::seconds(1));
    let mut expired_associations = associations.clone();
    let expired_attachment = expired_associations[2]
        .iter_mut()
        .find(|row| row.get_str("_id").unwrap() == attachment.id)
        .unwrap();
    assert_eq!(
        expired_attachment.insert("expires_at", expired_at),
        Some(bson::Bson::DateTime(bson::DateTime::from_chrono(
            saved_expiry
        )))
    );
    a.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &attachment.id},
            doc! {"$set": {"expires_at": expired_at}},
        )
        .await
        .unwrap();
    assert!(
        saved_associations(&a.f.state.db).await == expired_associations,
        "the expiry fixture must change only the saved attachment expiry"
    );
    assert_eq!(*a.calls.lock().unwrap(), calls_before_refusals);
    let expired = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(&path)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), axum::http::StatusCode::GONE);
    assert_private_refusal(
        &a,
        expired,
        &expired_associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;
    let expired_recovery = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&recovery_path)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired_recovery.status(), axum::http::StatusCode::GONE);
    assert_private_refusal(
        &a,
        expired_recovery,
        &expired_associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;
    a.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &attachment.id},
            doc! {"$set": {"expires_at": bson::DateTime::from_chrono(saved_expiry)}},
        )
        .await
        .unwrap();
    assert!(
        saved_associations(&a.f.state.db).await == associations,
        "the expiry fixture must restore the original saved associations"
    );
    assert_eq!(*a.calls.lock().unwrap(), calls_before_refusals);

    // Missing credentials and a foreign first-party token are rejected before
    // bytes or recovery effects can be disclosed.
    let unauthorized = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(&path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_private_refusal(
        &a,
        unauthorized,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;
    let foreign = uuid::Uuid::new_v4().to_string();
    a.f.state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &foreign,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let foreign_token = crate::crypto::jwt::generate_access_token(
        &a.f.state.jwt_keys,
        &a.f.state.config,
        &uuid::Uuid::parse_str(&foreign).unwrap(),
        "",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let foreign_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(&path)
                .header("authorization", format!("Bearer {foreign_token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        foreign_response.status(),
        axum::http::StatusCode::FORBIDDEN | axum::http::StatusCode::NOT_FOUND
    ));
    assert_private_refusal(
        &a,
        foreign_response,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;

    // Delegated account reads are denied by the existing assistant route
    // policy before the handler can disclose attachment bytes.
    let delegated_token = crate::crypto::jwt::generate_delegated_access_token(
        &a.f.state.jwt_keys,
        &a.f.state.config,
        &uuid::Uuid::parse_str(&a.f.owner).unwrap(),
        "account:read",
        "artifact-route-test",
        600,
        None,
    )
    .unwrap();
    let delegated_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(&path)
                .header("authorization", format!("Bearer {delegated_token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        delegated_response.status(),
        axum::http::StatusCode::FORBIDDEN
    );
    assert_private_refusal(
        &a,
        delegated_response,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;

    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &a.f.state.db,
        &a.f.state.encryption_keys,
        &a.f.owner,
        &a.f.row.id,
    )
    .await
    .unwrap()
    .expect("the fixture conversation Agent Key");
    let api_key_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(&path)
                .header("x-api-key", credential.raw_key.as_str())
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(api_key_response.status(), axum::http::StatusCode::FORBIDDEN);
    assert_private_refusal(
        &a,
        api_key_response,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;

    let (service_account, service_secret) =
        crate::services::service_account_service::create_service_account(
            &a.f.state.db,
            "artifact-route-test",
            None,
            "proxy",
            &[],
            None,
            &a.f.owner,
        )
        .await
        .unwrap();
    let service_token = crate::services::service_account_service::authenticate_client_credentials(
        &a.f.state.db,
        &a.f.state.config,
        &a.f.state.jwt_keys,
        &service_account.client_id,
        &service_secret,
        Some("proxy"),
    )
    .await
    .unwrap()
    .access_token;
    let relay_token = crate::crypto::jwt::generate_relay_access_token(
        &a.f.state.jwt_keys,
        &a.f.state.config,
        &uuid::Uuid::parse_str(&a.f.owner).unwrap(),
        "proxy",
        None,
        &crate::crypto::jwt::RelayAgentScope {
            api_key_id: a.f.row.credential_api_key_id.clone(),
            api_key_name: "artifact-route-test".into(),
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: false,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    for token in [&service_token, &relay_token] {
        let rejected = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(&path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), axum::http::StatusCode::FORBIDDEN);
        assert_private_refusal(
            &a,
            rejected,
            &associations,
            &calls_before_refusals,
            &private_markers,
            &manifest.content_revision,
        )
        .await;
    }
    assert_eq!(a.calls.lock().unwrap().len(), calls_before_recovery);

    // Recovery is also exercised through its production mounted route, with
    // the same first-party access token and an empty body. The server-written
    // message association selects the result; no client IDs are accepted.
    let recovery_unauthorized = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&recovery_path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        recovery_unauthorized.status(),
        axum::http::StatusCode::UNAUTHORIZED
    );
    assert_private_refusal(
        &a,
        recovery_unauthorized,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;

    let recovery_foreign = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&recovery_path)
                .header("authorization", format!("Bearer {foreign_token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        recovery_foreign.status(),
        axum::http::StatusCode::FORBIDDEN | axum::http::StatusCode::NOT_FOUND
    ));
    assert_private_refusal(
        &a,
        recovery_foreign,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;

    let recovery_delegated = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&recovery_path)
                .header("authorization", format!("Bearer {delegated_token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        recovery_delegated.status(),
        axum::http::StatusCode::FORBIDDEN
    );
    assert_private_refusal(
        &a,
        recovery_delegated,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;

    let recovery_api_key = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&recovery_path)
                .header("x-api-key", credential.raw_key.as_str())
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recovery_api_key.status(), axum::http::StatusCode::FORBIDDEN);
    assert_private_refusal(
        &a,
        recovery_api_key,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;

    // Recovery is first-party human only; service-account and relay bearer
    // classes are rejected before the saved association can be touched.
    for token in [&service_token, &relay_token] {
        let rejected = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(&recovery_path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), axum::http::StatusCode::FORBIDDEN);
        assert_private_refusal(
            &a,
            rejected,
            &associations,
            &calls_before_refusals,
            &private_markers,
            &manifest.content_revision,
        )
        .await;
    }
    assert_eq!(a.calls.lock().unwrap().len(), calls_before_recovery);

    let recovery_nonempty = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&recovery_path)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(axum::body::Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        recovery_nonempty.status(),
        axum::http::StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_private_refusal(
        &a,
        recovery_nonempty,
        &associations,
        &calls_before_refusals,
        &private_markers,
        &manifest.content_revision,
    )
    .await;
    assert_eq!(a.calls.lock().unwrap().len(), calls_before_recovery);

    let recovery = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&recovery_path)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recovery.status(), axum::http::StatusCode::OK);
    let recovery_body = axum::body::to_bytes(recovery.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&recovery_body).unwrap()["status"],
        "materialized"
    );
    assert_eq!(a.calls.lock().unwrap().len(), calls_before_recovery);
    assert_eq!(*a.calls.lock().unwrap(), calls_before_refusals);
}

#[tokio::test]
async fn generated_artifact_worker_fences_lifecycle_version_across_result_fetch() {
    for (before_version, after_version) in [
        (Some(1u64), Some(2u64)),
        (Some(1), None),
        (None, Some(1)),
        (Some(1), Some(1)),
        (Some(2), Some(2)),
        (None, None),
    ] {
        let a = boxed_generated().await;
        let submitted = a.submit().await;
        let operation = submitted.operation_id.as_deref().unwrap();
        let snapshot = |version: Option<u64>| {
            let mut value =
                json!({"operation_id":operation,"status":"succeeded","result_available":true});
            if let Some(version) = version {
                value["version"] = json!(version);
            }
            value
        };
        *a.status_override.lock().unwrap() = Some(snapshot(before_version));
        *a.after_result_status.lock().unwrap() = Some(snapshot(after_version));
        complete(&a).await;
        let w = a.watch().await;
        assert_eq!(w.operation_id, submitted.operation_id);
        let published =
            a.f.state
                .db
                .collection::<bson::Document>(ARTIFACTS)
                .count_documents(doc! {})
                .await
                .unwrap();
        if before_version != after_version {
            assert_eq!(w.reason.as_deref(), Some("artifact_source_changed"));
            assert!(w.artifact_result.is_none());
            assert!(w.result_encrypted.is_none());
            assert_eq!(published, 0);
            assert_eq!(
                a.f.state
                    .db
                    .collection::<bson::Document>(
                        crate::models::assistant_attachment::COLLECTION_NAME
                    )
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        } else {
            assert_eq!(w.reason.as_deref(), Some("completed"));
            let result =
                artifacts::sandbox::parse(&a.output.lock().unwrap().to_string(), operation)
                    .unwrap();
            let manifest = w.artifact_result.as_ref().unwrap();
            assert_eq!(manifest.source_result_identity, result.identity);
            assert_eq!(
                manifest.source_status_version,
                after_version.map(|v| v.to_string())
            );
            assert_eq!(published, 1);
        }
        let calls = a.calls.lock().unwrap();
        let result_path = format!("/executions/{operation}/result");
        let status_path = format!("/executions/{operation}");
        assert_eq!(calls.iter().filter(|path| *path == &result_path).count(), 1);
        assert_eq!(calls.iter().filter(|path| *path == &status_path).count(), 2);
        assert!(
            calls
                .iter()
                .filter(|path| path.starts_with("/executions/"))
                .all(|path| path == &status_path || path == &result_path)
        );
    }
}

#[tokio::test]
async fn generated_artifact_malformed_status_is_terminal_before_or_after_result_fetch() {
    const MALFORMED: &str = "__malformed_json_after_result_status__";
    check_artifact_malformed_initial_status(MALFORMED).await;
    check_artifact_malformed_post_result_status(MALFORMED).await;
    check_ordinary_async_malformed_status(MALFORMED).await;
}

// Construct each phase separately so its debug poll frame is released before
// the next phase enters the nested MCP/proxy/Mongo sweep calls.
fn check_artifact_malformed_initial_status(
    malformed: &'static str,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        // A malformed initial successful status is terminal for an admitted
        // generated watch and never reaches the result operation.
        let initial = boxed_generated().await;
        initial.submit().await;
        *initial.status_override.lock().unwrap() = Some(json!(malformed));
        complete(&initial).await;
        let initial_watch = initial.watch().await;
        assert_eq!(initial_watch.reason.as_deref(), Some("artifact_invalid"));
        assert_eq!(
            initial
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|path| path.ends_with("/result"))
                .count(),
            0
        );
        assert_eq!(
            initial
                .f
                .state
                .db
                .collection::<bson::Document>(ARTIFACTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    })
}

fn check_artifact_malformed_post_result_status(
    malformed: &'static str,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        // A malformed successful post-result status also settles terminally. A
        // second sweep cannot reclaim it, so the result is fetched exactly once.
        let after = boxed_generated().await;
        after.submit().await;
        *after.after_result_status.lock().unwrap() = Some(json!(malformed));
        complete(&after).await;
        let first_calls = after.calls.lock().unwrap().clone();
        assert_eq!(
            first_calls
                .iter()
                .filter(|path| path.ends_with("/result"))
                .count(),
            1
        );
        let after_watch = after.watch().await;
        assert_eq!(after_watch.reason.as_deref(), Some("artifact_invalid"));
        assert!(after_watch.artifact_result.is_none());
        assert!(after_watch.result_encrypted.is_none());
        assert_eq!(
            after
                .f
                .state
                .db
                .collection::<bson::Document>(ARTIFACTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        after.due().await;
        Box::pin(async_operations::sweep(&after.f.state))
            .await
            .unwrap();
        let second_calls = after.calls.lock().unwrap().clone();
        assert_eq!(
            second_calls
                .iter()
                .filter(|path| path.ends_with("/result"))
                .count(),
            1
        );
    })
}

fn check_ordinary_async_malformed_status(
    malformed: &'static str,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        // Ordinary async watches keep the existing retryable malformed-status
        // contract; only admitted generated watches settle terminally.
        let ordinary = AsyncFixture::new().await;
        ordinary.submit().await;
        *ordinary.status_override.lock().unwrap() = Some(json!(malformed));
        ordinary.due().await;
        Box::pin(async_operations::sweep(&ordinary.f.state))
            .await
            .unwrap();
        let ordinary_watch = ordinary.watch().await;
        assert_eq!(
            (
                ordinary_watch.state.as_str(),
                ordinary_watch.reason.as_deref()
            ),
            ("waiting", None)
        );
    })
}

#[tokio::test]
async fn generated_artifact_requires_saved_trusted_catalog_for_acquisition() {
    check_custom_sandbox_lookalike().await;
    for case in ["missing_snapshot", "changed_snapshot", "changed_catalog"] {
        check_artifact_catalog_revalidation(case).await;
    }
}
fn check_custom_sandbox_lookalike() -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async {
        let a = boxed_generated().await;
        a.f.state
            .db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&a.service},
                doc! {"$unset":{"catalog_service_id":""}},
            )
            .await
            .unwrap();
        let instance =
            a.f.state
                .db
                .collection::<crate::models::user_service::UserService>(
                    crate::models::user_service::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&a.service})
                .await
                .unwrap()
                .unwrap();
        a.f.state.db.collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
            .update_one(doc! {"_id":&instance.endpoint_id}, doc! {"$set":{"openapi_spec_url":"https://custom.invalid/api/v1/catalog-specs/chrono-sandbox/openapi.json"}}).await.unwrap();
        // This custom instance retains the same hosted spec and endpoint shape.
        // Neither the name nor URL confers producer catalog identity.
        assert!(a.submit().await.artifact_mode.is_none());
        assert_eq!(
            a.f.state
                .db
                .collection::<bson::Document>(ARTIFACTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    })
}
fn check_artifact_catalog_revalidation(
    case: &'static str,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        let a = boxed_generated().await;
        let watch = a.submit().await;
        let catalog = watch
            .artifact_catalog_service_id
            .as_ref()
            .expect("server verified catalog snapshot");
        if case == "changed_catalog" {
            a.f.state
                .db
                .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":catalog},
                    doc! {"$set":{"slug":"custom-sandbox-lookalike"}},
                )
                .await
                .unwrap();
        } else {
            let update = if case == "missing_snapshot" {
                doc! {"$unset":{"artifact_catalog_service_id":""}}
            } else {
                doc! {"$set":{"artifact_catalog_service_id":uuid::Uuid::new_v4().to_string()}}
            };
            a.f.state
                .db
                .collection::<bson::Document>(WATCHES)
                .update_one(doc! {"_id":&watch.id}, update)
                .await
                .unwrap();
        }
        let calls = a.calls.lock().unwrap().len();
        complete(&a).await;
        assert_eq!(
            a.watch().await.reason.as_deref(),
            Some("artifact_source_changed")
        );
        assert_eq!(
            a.calls.lock().unwrap().len(),
            calls,
            "no source call after catalog trust changes"
        );
        Box::pin(async_operations::sweep(&a.f.state)).await.unwrap();
        assert_eq!(
            a.watch().await.reason.as_deref(),
            Some("artifact_source_changed")
        );
        assert_eq!(
            a.calls.lock().unwrap().len(),
            calls,
            "terminal failure must not repeat source calls"
        );
        finish(&a, &a.f.row, "submitted").await;
        let event = admit_event(&a).await;
        assert!(event.active_turn.as_ref().unwrap().attachments.is_empty());
        let context = watches::input_context(&a.f.state.db, &a.f.state.encryption_keys, &event)
            .await
            .unwrap();
        assert!(context.contains("artifact_source_changed"));
        finish(
            &a,
            &event,
            "The saved source could not be verified for delivery.",
        )
        .await;

        let message =
            a.f.state
                .db
                .collection::<crate::models::assistant_message::AssistantMessage>(
                    crate::models::assistant_message::COLLECTION_NAME,
                )
                .find_one(
                    doc! {"user_id":&a.f.row.user_id,"conversation_id":&a.f.row.id,
                    "turn_id":&event.active_turn.as_ref().unwrap().turn_id,"role":"assistant"},
                )
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            message
                .turn_metadata
                .as_ref()
                .unwrap()
                .artifact_delivery
                .as_ref()
                .unwrap()
                .state,
            "source_verification_failed"
        );
        // Simulate Mongo TTL deleting exactly the now-eligible terminal watch.
        a.f.state.db.collection::<bson::Document>(WATCHES).update_one(doc! {"_id":&watch.id},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
        assert_eq!(
            a.f.state
                .db
                .collection::<bson::Document>(WATCHES)
                .delete_many(doc! {"_id":&watch.id,
                "state":"delivered","expires_at":{"$lte":bson::DateTime::now()}})
                .await
                .unwrap()
                .deleted_count,
            1
        );
        let states = artifacts::delivery_states(
            &a.f.state.db,
            &a.f.row.user_id,
            &a.f.row.id,
            &[message.id.clone()],
        )
        .await
        .unwrap();
        assert_eq!(states[&message.id].state, "source_verification_failed");
        assert!(!states[&message.id].recovery_available);
        assert_eq!(
            a.f.state
                .db
                .collection::<bson::Document>(DELIVERIES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(a.calls.lock().unwrap().len(), calls);

        assert_eq!(
            a.f.state
                .db
                .collection::<bson::Document>(ARTIFACTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    })
}

#[tokio::test]
async fn generated_artifact_worker_rejects_expired_changed_and_unavailable_sources() {
    for case in [
        "result_410",
        "changed_operation",
        "expiry_race",
        "unavailable",
    ] {
        let a = boxed_generated().await;
        a.submit().await;
        match case {
            "result_410" => a.result_status.store(410, Ordering::SeqCst),
            "changed_operation" => {
                *a.after_result_status.lock().unwrap() = Some(
                    json!({"operation_id":"different","status":"succeeded","result_available":true}),
                )
            }
            "expiry_race" => {
                *a.after_result_status.lock().unwrap() = Some(
                    json!({"operation_id":"op-1","status":"succeeded","result_available":true,"expires_at":(chrono::Utc::now()-chrono::Duration::seconds(1)).to_rfc3339()}),
                )
            }
            _ => {
                *a.status_override.lock().unwrap() = Some(
                    json!({"operation_id":"op-1","status":"succeeded","result_available":false}),
                )
            }
        }
        complete(&a).await;
        assert_eq!(
            a.watch().await.reason.as_deref(),
            Some(match case {
                "changed_operation" => "artifact_source_changed",
                "unavailable" => "result_unavailable",
                _ => "result_expired",
            })
        );
        assert_eq!(
            a.f.state
                .db
                .collection::<bson::Document>(ARTIFACTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        // A bounded failure notification must still be consumable as an event.
        finish(&a, &a.f.row, "submitted").await;
        let event = admit_event(&a).await;
        assert!(event.active_turn.as_ref().unwrap().attachments.is_empty());
        finish(&a, &event, "The source could not be delivered.").await;
    }
}

#[tokio::test]
async fn generated_artifact_worker_reserves_shared_budget_before_result_body_and_releases_on_cancel()
 {
    use crate::services::assistant_upload_service::byte_budget;
    let a = boxed_generated().await;
    a.submit().await;
    a.due().await;
    let held = a.result_gate.acquire().await.unwrap();
    let baseline = byte_budget::available_for_tests();
    let mut sweep = Box::pin(async_operations::sweep(&a.f.state));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            tokio::select! {
                result = &mut sweep => panic!("worker completed before result was released: {result:?}"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
            if a.calls.lock().unwrap().iter().any(|p| p.ends_with("/result")) { break; }
        }
    }).await.unwrap();
    assert_eq!(
        baseline - byte_budget::available_for_tests(),
        2 * artifacts::sandbox::RESERVATION
    );
    drop(sweep);
    assert_eq!(byte_budget::available_for_tests(), baseline);
    drop(held);
    assert_eq!(
        a.f.state
            .db
            .collection::<bson::Document>(ARTIFACTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn generated_artifact_worker_delivers_valid_peers_and_settles_envelope_refusals() {
    check_artifact_partial_worker("mixed").await;
    check_artifact_partial_worker("overcount").await;
    check_artifact_partial_worker("transport_limit").await;
}
fn check_artifact_partial_worker(case: &'static str) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        let a = boxed_generated().await;
        let valid = a.output.lock().unwrap()["output"]["files"][0].clone();
        let files = match case {
            "overcount" => json!(vec![valid; 9]),
            "transport_limit" => {
                json!([{"path":"huge.bin","content":"x".repeat(artifacts::sandbox::RESPONSE_BYTES)}])
            }
            _ => json!([
                {"path":"/private/bad.mp4","content":"bm90LW1wNA=="},
                valid,
                {"path":"/private/bad.mp4","error":"private-error-marker"}
            ]),
        };
        a.output.lock().unwrap()["output"]["files"] = files;
        a.submit().await;
        complete(&a).await;
        let calls = a.calls.lock().unwrap().len();
        Box::pin(async_operations::sweep(&a.f.state)).await.unwrap();
        assert_eq!(a.calls.lock().unwrap().len(), calls);
        finish(&a, &a.f.row, "submitted").await;
        let event = admit_event(&a).await;
        let attachments = &event.active_turn.as_ref().unwrap().attachments;
        assert_eq!(attachments.len(), usize::from(case == "mixed"));
        if case == "mixed" {
            let (mime, bytes) = engine::read_attachment(
                &a.f.state.db,
                &a.f.state.encryption_keys,
                &a.f.owner,
                &event.id,
                &attachments[0].id,
            )
            .await
            .unwrap();
            assert_eq!(mime, "video/mp4");
            assert_eq!(bytes, MP4);
        } else {
            assert_eq!(
                a.watch().await.reason.as_deref(),
                Some("artifact_too_large")
            );
            assert_eq!(
                a.f.state
                    .db
                    .collection::<bson::Document>(ARTIFACTS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        finish(&a, &event, "File delivery has settled.").await;
        let axum::Json(history) = crate::handlers::assistant_nyxagent::history(
            axum::extract::State(a.f.state.clone()),
            crate::test_utils::test_auth_user(&a.f.owner),
            axum::extract::Path(event.id.clone()),
            axum::extract::Query(Default::default()),
        )
        .await
        .unwrap();
        let history = serde_json::to_value(history).unwrap();
        let message = history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| {
                m["role"] == "assistant"
                    && m["turn_id"] == event.active_turn.as_ref().unwrap().turn_id
            })
            .unwrap();
        let outcome = &message["artifact_delivery"];
        assert_eq!(
            outcome["state"],
            if case == "mixed" {
                "materialized"
            } else {
                "artifact_too_large"
            }
        );
        assert_eq!(outcome["recovery_available"], false);
        assert_eq!(outcome["count"], usize::from(case == "mixed"));
        if case == "mixed" {
            assert_eq!(
                outcome["rejected"],
                json!([
                    {"index":0,"filename":"bad.mp4","reason":"invalid"},
                    {"index":2,"filename":"bad.mp4","reason":"unavailable"}
                ])
            );
        } else {
            assert_eq!(
                a.f.state
                    .db
                    .collection::<bson::Document>(DELIVERIES)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        assert!(!outcome.to_string().contains("private"));
        assert_eq!(a.calls.lock().unwrap().len(), calls);
    })
}
