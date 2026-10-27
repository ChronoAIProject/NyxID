//! Real MCP admission, scoped result calls, event settlement and private owner reads.
use super::*;
use crate::models::generated_artifact::{COLLECTION_NAME as ARTIFACTS, DELIVERIES};
use crate::services::{feature_flag_service as flags, generated_artifacts as artifacts};
use base64::Engine;
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
    let legacy = AsyncFixture::new().await;
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
    let a = boxed_generated().await;
    let w = a.submit().await;
    assert_eq!(w.artifact_mode.as_deref(), Some(artifacts::sandbox::MODE));
    enabled(&a, false).await;
    complete(&a).await;
    assert_eq!(a.watch().await.artifact_result.as_ref().unwrap().count, 1);
    finish(&a, &a.f.row, "submitted").await;
    let event = admit_event(&a).await;
    let attachment = event.active_turn.as_ref().unwrap().attachments[0].clone();
    assert_eq!(attachment.origin, "generated_artifact");
    let context = watches::input_context(&a.f.state.db, &a.f.state.encryption_keys, &event)
        .await
        .unwrap();
    assert!(context.contains("materialized"));
    assert!(!context.contains("/tmp/movie.mp4"));
    assert!(!context.contains(&base64::engine::general_purpose::STANDARD.encode(MP4)));
    finish(&a, &event, "The retained result is attached.").await;
    let messages: Vec<crate::models::assistant_message::AssistantMessage> = a.f.state.db
        .collection(crate::models::assistant_message::COLLECTION_NAME)
        .find(doc! {"conversation_id":&event.id,"role":"assistant","turn_id":&event.active_turn.as_ref().unwrap().turn_id})
        .await.unwrap().try_collect().await.unwrap();
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
        crate::handlers::assistant_nyxagent::artifact_delivery(
            axum::extract::State(a.f.state.clone()),
            crate::test_utils::test_auth_user(&a.f.owner),
            axum::extract::Path((event.id.clone(), messages[0].id.clone())),
            axum::body::Bytes::new(),
        )
        .await
        .unwrap();
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
    // A natural-language turn can recover only the preceding server association.
    let next = engine::begin_turn(
        &a.f.state.db,
        &a.f.owner,
        &user_start(&event.id, "Recover the preceding result"),
        &a.f.state.encryption_keys,
    )
    .await
    .unwrap();
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
            card.tool_name.as_deref() == Some("nyx__artifact_delivery") && card.status == "pending"
        })
        .unwrap();
    finish(&a, &next, "Please review the recovery action.").await;
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
    finish(&a, &continued, "Recovered the saved attachments.").await;
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
