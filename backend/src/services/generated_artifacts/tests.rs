use super::*;
use base64::Engine;
use serde_json::json;
const MP4: &[u8] = include_bytes!("../../../tests/fixtures/generated-artifacts/visual-avc.mp4");
const PNG: &[u8] = include_bytes!("../../../tests/fixtures/generated-artifacts/blue.png");
const JPEG: &[u8] = include_bytes!("../../../tests/fixtures/generated-artifacts/blue.jpg");
fn body(name: &str, bytes: &[u8]) -> String {
    json!({"success":true,"output":{"stdout":"done","stderr":"","exit_code":0,"files":[{"path":name,"content":base64::engine::general_purpose::STANDARD.encode(bytes),"size":1}]}}).to_string()
}
fn watch() -> AsyncServiceOperation {
    bson::from_document(doc!{"_id":Uuid::new_v4().to_string(),"user_id":"owner2037","conversation_id":"nyxa-2037","agent_id":"agent2037","turn_id":"original2037","api_key_id":"key2037","service_id":"sandbox2037","submit_endpoint_id":"submit2037",
        "contract":{"status_operation":"get_execution_handler","result_operation":"get_execution_result_handler","id_field":"/operation_id","id_parameter":"operation_id","status_field":"/status","success_states":["succeeded"],"failure_states":["failed"]},
        "delivery":{"origin":"user","platform_reply":{"kind":"web_only"}},"operation_id":"operation2037","artifact_mode":sandbox::MODE,"artifact_catalog_service_id":"00000000-0000-4000-8000-000000002037","state":"waiting","attempts":0,"lease_id":"lease2037","lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(60)),"created_at":bson::DateTime::now(),"deadline":bson::DateTime::from_chrono(Utc::now()+Duration::hours(2)),"poll_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(4))}).unwrap()
}
#[test]
fn generated_artifact_metadata_omits_absent_fields_but_persists_populated_bson() {
    assert!(bson::to_document(&Metadata::default()).unwrap().is_empty());
    let expiry = Utc::now();
    let metadata = Metadata {
        filename: Some("result.mp4".into()),
        source: Some("sandbox".into()),
        source_watch_id: Some("watch".into()),
        source_turn_id: Some("turn".into()),
        source_operation_id: Some("operation".into()),
        source_result_identity: Some("identity".into()),
        source_status_version: Some("17".into()),
        content_revision: Some("2".into()),
        preview_kind: Some("video".into()),
        expires_at: Some(expiry),
    };
    let stored = bson::to_document(&metadata).unwrap();
    assert_eq!(stored.len(), 10);
    assert_eq!(stored.get_str("source_operation_id").unwrap(), "operation");
    assert_eq!(stored.get_str("content_revision").unwrap(), "2");
    assert_eq!(
        stored.get_datetime("expires_at").unwrap(),
        &bson::DateTime::from_chrono(expiry)
    );
    let restored: Metadata = bson::from_document(stored.clone()).unwrap();
    assert_eq!(bson::to_document(&restored).unwrap(), stored);
    let legacy_manifest: ResultManifest = bson::from_document(doc! {
        "source_result_identity":"legacy","source_status_version":null,
        "content_revision":"1","count":0_i64,"delivery_reason":"materialized",
        "expires_at":bson::DateTime::now(),
    })
    .unwrap();
    assert_eq!(legacy_manifest.content_revision.as_deref(), Some("1"));
    assert!(legacy_manifest.files.is_empty());
    let legacy: Metadata = bson::from_document(doc! {}).unwrap();
    assert!(bson::to_document(&legacy).unwrap().is_empty());
    let legacy = TurnAttachment {
        artifact: Metadata::default(),
        chat_delivery: None,
        image_input: None,
        image_inspection: None,
        origin: "tool".into(),
        pages: None,
        id: "legacy-image".into(),
        content_type: "image/png".into(),
        size: 42,
        label: "legacy tool".into(),
    };
    let stored_legacy = bson::to_document(&legacy).unwrap();
    for key in stored.keys() {
        assert!(!stored_legacy.contains_key(key), "absent metadata: {key}");
    }
    let response = serde_json::to_value(
        crate::handlers::assistant_nyxagent::AttachmentResponse::from(legacy.clone()),
    )
    .unwrap();
    for key in [
        "filename",
        "source",
        "result_version",
        "preview_kind",
        "expires_at",
    ] {
        assert!(
            response.get(key).is_none(),
            "absent response metadata: {key}"
        );
    }
    let populated = TurnAttachment {
        artifact: metadata,
        ..legacy
    };
    let flattened = bson::to_document(&populated).unwrap();
    for (key, value) in stored {
        assert_eq!(flattened.get(&key), Some(&value));
    }
    let response = serde_json::to_value(
        crate::handlers::assistant_nyxagent::AttachmentResponse::from(populated),
    )
    .unwrap();
    assert_eq!(response["filename"], "result.mp4");
    assert_eq!(response["result_version"], "2");
}
#[test]
fn generated_summary_selects_metadata_and_discards_process_streams() {
    let mut value: serde_json::Value = serde_json::from_str(&body(
        "/private/path-marker/file-marker.bin",
        b"file-content-marker",
    ))
    .unwrap();
    value["output"]["stdout"] = json!("/private/stdout-path-marker stdout-content-marker");
    value["output"]["stderr"] = json!("/private/stderr-path-marker stderr-secret-marker");
    value["output"]["exit_code"] = json!(7);
    value["success"] = json!(false);
    let result = sandbox::parse(&value.to_string(), "op").unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result.summary).unwrap(),
        json!({"success":false,"exit_code":7,"verified_file_count":1})
    );
    assert!(!result.summary.contains("marker"));
    assert_eq!(sandbox::context_summary(&result.summary), result.summary);
    // Old free-form summaries and extra string fields never cross the boundary.
    for unsafe_summary in [
        "Sandbox script success: true. stdout: /private/stdout-path-marker",
        r#"{"success":true,"exit_code":0,"verified_file_count":1,"stdout":"stderr-secret-marker"}"#,
        r#"{"success":true,"exit_code":"stdout-content-marker","verified_file_count":1}"#,
    ] {
        assert!(sandbox::context_summary(unsafe_summary).is_empty());
    }
    value["output"]["stdout"] = json!(42);
    assert!(sandbox::parse(&value.to_string(), "op").is_err());
    value["output"]["stdout"] = json!("x".repeat(sandbox::RESPONSE_BYTES));
    assert!(matches!(
        sandbox::parse(&value.to_string(), "op"),
        Err("artifact_too_large")
    ));
}
#[test]
fn sandbox_files_contract_and_structured_visual_track_validation() {
    for (name, bytes, preview) in [
        ("movie.mp4", MP4, "video"),
        ("photo.png", PNG, "image"),
        ("photo.jpg", JPEG, "image"),
        (
            "report.html",
            b"<script>alert(1)</script>".as_slice(),
            "download",
        ),
    ] {
        let result = sandbox::parse(&body(name, bytes), "operation2037").unwrap();
        assert_eq!(result.files[0].preview, preview);
        assert_eq!(&*result.files[0].bytes, bytes);
        assert_ne!(
            result.identity,
            sandbox::parse(&body(name, bytes), "another-operation")
                .unwrap()
                .identity
        );
    }
    assert!(mp4::validate(b"ftypisommoovtrakmdiahdlrvidestblstsdavc1").is_err());
    for end in [4, 8, 32, MP4.len() - 1] {
        assert!(mp4::validate(&MP4[..end]).is_err());
    }
    let mut audio = MP4.to_vec();
    let pos = audio.windows(4).position(|b| b == b"vide").unwrap();
    audio[pos..pos + 4].copy_from_slice(b"soun");
    assert!(mp4::validate(&audio).is_err());
    let mut oversized = MP4.to_vec();
    oversized[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(mp4::validate(&oversized).is_err());
    let high = include_bytes!("../../../tests/fixtures/generated-artifacts/visual-high.mp4");
    assert_eq!(mp4::validate(high), Ok(true));
    assert_eq!(
        sandbox::parse(&body("high.mp4", high), "op").unwrap().files[0].preview,
        "video"
    );
    let fragmented =
        include_bytes!("../../../tests/fixtures/generated-artifacts/visual-fragmented.mp4");
    assert_eq!(mp4::validate(fragmented), Ok(false));
    assert_eq!(
        sandbox::parse(&body("fragmented.mp4", fragmented), "op")
            .unwrap()
            .files[0]
            .preview,
        "download"
    );
    for kind in [b"tfhd", b"trun", b"trex"] {
        let mut damaged = fragmented.to_vec();
        let offset = damaged.windows(4).position(|b| b == kind).unwrap() + 4;
        match kind {
            b"tfhd" | b"trex" => {
                damaged[offset + 4..offset + 8].copy_from_slice(&0u32.to_be_bytes())
            }
            _ => damaged[offset + 8..offset + 12].copy_from_slice(&i32::MAX.to_be_bytes()),
        }
        assert!(mp4::validate(&damaged).is_err());
    }
    for end in [32, fragmented.len() - 1] {
        assert!(mp4::validate(&fragmented[..end]).is_err());
    }
    let mixed = include_bytes!("../../../tests/fixtures/generated-artifacts/visual-mixed.mp4");
    assert_eq!(mp4::validate(mixed), Ok(false));
    assert_eq!(
        sandbox::parse(&body("mixed.mp4", mixed), "op")
            .unwrap()
            .files[0]
            .preview,
        "download"
    );
    // Genuine encoder output contains both classic and fragmented samples;
    // malformed extents in either half must still reject the whole container.
    for kind in [b"stco", b"trun"] {
        let mut damaged = mixed.to_vec();
        let offset = damaged.windows(4).position(|b| b == kind).unwrap() + 4;
        damaged[offset + 8..offset + 12].copy_from_slice(&i32::MAX.to_be_bytes());
        assert!(mp4::validate(&damaged).is_err());
    }
    let mut high_tampered = high.to_vec();
    let offset = high_tampered.windows(4).position(|b| b == b"avcC").unwrap() + 4;
    high_tampered[offset + 1] = 66; // A profile byte alone is not codec proof.
    assert!(mp4::validate(&high_tampered).is_err());
    // Full sample tables and codec configuration matter, not marker strings.
    for kind in [b"stts", b"stsc", b"stco", b"avcC"] {
        let mut damaged = MP4.to_vec();
        let offset = damaged.windows(4).position(|b| b == kind).unwrap() + 4;
        match kind {
            b"stts" => damaged[offset + 8..offset + 12].copy_from_slice(&0u32.to_be_bytes()),
            b"stsc" => damaged[offset + 8..offset + 12].copy_from_slice(&2u32.to_be_bytes()),
            b"stco" => damaged[offset + 8..offset + 12]
                .copy_from_slice(&((MP4.len() - 1) as u32).to_be_bytes()),
            _ => damaged[offset + 5] = 0xe0,
        }
        assert!(mp4::validate(&damaged).is_err());
    }
    assert_eq!(
        sandbox::filename("../../a\\secret\r\n.mp4", 0),
        "secret.mp4"
    );
    assert!(sandbox::filename(&"界".repeat(100), 0).len() <= 160);
    for value in [
        json!({"success":true,"output":null}),
        json!({"success":false,"output":{"stdout":"","stderr":"","exit_code":2,"files":null}}),
    ] {
        assert!(
            sandbox::parse(&value.to_string(), "op")
                .unwrap()
                .files
                .is_empty()
        );
    }
    for content in ["YQ", "YQ==\n", "!@==", "YR=="] {
        let value = json!({"success":true,"output":{"stdout":"","stderr":"","exit_code":0,"files":[{"path":"file","content":content}]}});
        let result = sandbox::parse(&value.to_string(), "op").unwrap();
        assert!(result.files.is_empty());
        assert_eq!(result.rejected.len(), 1);
        assert!(matches!(
            result.rejected[0].reason,
            crate::models::generated_artifact::RejectionReason::Invalid
        ));
    }
    let mut value: serde_json::Value = serde_json::from_str(&body("x.png", PNG)).unwrap();
    value["output"]["files"][0]["error"] = json!("cannot read");
    let result = sandbox::parse(&value.to_string(), "op").unwrap();
    assert!(result.files.is_empty());
    assert!(matches!(
        result.rejected[0].reason,
        crate::models::generated_artifact::RejectionReason::Unavailable
    ));
    let result = sandbox::parse(&body("x.bin", &vec![0; sandbox::FILE_BYTES + 1]), "op").unwrap();
    assert!(result.files.is_empty());
    assert!(matches!(
        result.rejected[0].reason,
        crate::models::generated_artifact::RejectionReason::TooLarge
    ));
    let mut too_many: serde_json::Value = serde_json::from_str(&body("x.png", PNG)).unwrap();
    too_many["output"]["files"] = json!(vec![too_many["output"]["files"][0].clone(); 9]);
    assert!(matches!(
        sandbox::parse(&too_many.to_string(), "op"),
        Err("artifact_too_large")
    ));
}
#[test]
fn status_identity_expiry_and_http_ranges_are_independent() {
    let value = json!({"operation_id":"op","status":"succeeded","result_available":true,"version":1,"expires_at":(Utc::now()+Duration::minutes(1)).to_rfc3339()});
    assert!(sandbox::status(&value, "op", Utc::now()).is_ok());
    assert!(sandbox::status(&value, "wrong", Utc::now()).is_err());
    for version in [json!("1"), json!(-1), json!({}), json!(true)] {
        let mut malformed = value.clone();
        malformed["version"] = version;
        assert!(matches!(
            sandbox::status(&malformed, "op", Utc::now()),
            Err("artifact_invalid")
        ));
    }
    assert!(matches!(
        sandbox::status(&value, "op", Utc::now() + Duration::minutes(2)),
        Err("result_expired")
    ));
    for (range, expected) in [
        ("bytes=0-3", (0, 3)),
        ("bytes=5-", (5, 9)),
        ("bytes=-3", (7, 9)),
        ("bytes=2-100", (2, 9)),
        ("bytes=-100", (0, 9)),
        ("bytes=2-184467440737095516160", (2, 9)),
        ("bytes=-184467440737095516160", (0, 9)),
        ("bytes=00000000000000000000000002-0003", (2, 3)),
    ] {
        assert_eq!(
            http::select_range(range, 10),
            http::RangeSelection::Partial(expected.0, expected.1)
        );
    }
    for range in [
        "bytes=10-",
        "bytes=20-30",
        "bytes=-0",
        "bytes=184467440737095516160-",
        "bytes=184467440737095516160-184467440737095516161",
    ] {
        assert_eq!(
            http::select_range(range, 10),
            http::RangeSelection::Unsatisfiable
        );
    }
    for range in [
        "bytes=4-2",
        "bytes=0-1,2-3",
        "bytes=0-x",
        "bytes=184467440737095516161-184467440737095516160",
        "items=1-2",
    ] {
        assert_eq!(http::select_range(range, 10), http::RangeSelection::Ignore);
    }
    assert_eq!(
        http::select_range("bytes=0-", 0),
        http::RangeSelection::Unsatisfiable
    );
}
#[tokio::test]
async fn maximum_encrypted_pending_document_fits_bson_and_rejects_oversize() {
    let keys = crate::test_utils::test_encryption_keys();
    let mut row:PendingArtifact=bson::from_document(doc!{"_id":Uuid::new_v4().to_string(),"watch_id":"watch","user_id":"owner","conversation_id":"conversation","turn_id":"turn","operation_id":"operation","source_result_identity":"a".repeat(64),"artifact_index":0,"filename":"x".repeat(160),"source_path":"x".repeat(512),"content_type":"application/octet-stream","size":sandbox::FILE_BYTES as i64,"sha256":"a".repeat(64),"preview_kind":"download","content_revision":"1","state":"pending","assistant_attachment_id":Uuid::new_v4().to_string(),"created_at":bson::DateTime::now(),"expires_at":bson::DateTime::now()}).unwrap();
    let bytes = zeroize::Zeroizing::new(vec![42; sandbox::FILE_BYTES]);
    let path = context_path(&row);
    row.data_encrypted = Some(
        keys.encrypt_record(
            &bytes,
            &RecordContext::new(ARTIFACTS, &row.id, &row.user_id, &path).unwrap(),
        )
        .await
        .unwrap(),
    );
    assert!(bson::to_vec(&row).unwrap().len() < 16 * 1024 * 1024);
    check_bson(&row).unwrap();
    row.data_encrypted = Some(vec![0; 16 * 1024 * 1024]);
    assert!(check_bson(&row).is_err());
}

async fn seeded(
    db: &Database,
    keys: &EncryptionKeys,
) -> (AsyncServiceOperation, AssistantConversation) {
    seeded_until(db, keys, Utc::now() + Duration::minutes(30)).await
}
async fn seeded_until(
    db: &Database,
    keys: &EncryptionKeys,
    source_expiry: DateTime<Utc>,
) -> (AsyncServiceOperation, AssistantConversation) {
    seeded_body(db, keys, source_expiry, &body("../../movie.mp4", MP4)).await
}
async fn seeded_body(
    db: &Database,
    keys: &EncryptionKeys,
    source_expiry: DateTime<Utc>,
    result_body: &str,
) -> (AsyncServiceOperation, AssistantConversation) {
    let mut w = watch();
    w.expires_at = w.expires_at.max(source_expiry + Duration::hours(1));
    w.conversation_id = format!("nyxa-{}", Uuid::new_v4().simple());
    let raw = doc! {"_id":&w.conversation_id,"user_id":&w.user_id,"title":"artifacts","model":"nyxagent/chat","credential_api_key_id":&w.api_key_id,"message_count":1,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    "active_turn":{"turn_id":"completion2037","origin":"event","started_at":bson::DateTime::now(),"events":[{"id":&w.id,"kind":super::super::async_service_operation::EVENT_KIND,"text":"done","created_at":bson::DateTime::now()}]}};
    let row: AssistantConversation = bson::from_document(raw).unwrap();
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .insert_one(&row)
        .await
        .unwrap();
    db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME).insert_one(doc!{"_id":format!("original-message-{}",w.id),"user_id":&w.user_id,"conversation_id":&w.conversation_id,"turn_id":&w.turn_id,"role":"user","origin":"user","seq":1,"text":"create a video","status":"completed","created_at":bson::DateTime::now()}).await.unwrap();
    db.collection::<AsyncServiceOperation>(WATCHES)
        .insert_one(&w)
        .await
        .unwrap();
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let result = sandbox::parse(result_body, w.operation_id.as_deref().unwrap()).unwrap();
    persist(
        db,
        keys,
        &w,
        &result,
        &sandbox::Status {
            expires_at: Some(source_expiry),
            version: Some("7".into()),
        },
        &reservation,
    )
    .await
    .unwrap();
    db.collection::<Document>(WATCHES)
        .update_one(doc! {"_id":&w.id}, doc! {"$set":{"state":"queued"}})
        .await
        .unwrap();
    w = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    (w, row)
}
#[tokio::test]
async fn generated_artifacts_materialize_history_owner_http_ranges_recovery_and_retention() {
    let db = crate::test_utils::connect_transaction_test_database("art2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    let (w, mut row) = seeded(&db, &keys).await;
    assert!(
        !super::super::feature_flag_service::personal_flag_enabled(
            &db,
            &w.user_id,
            super::super::feature_flag_service::ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY
        )
        .await
        .unwrap()
    );
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session)
        .await
        .unwrap();
    let id = row.active_turn.as_ref().unwrap().attachments[0].id.clone();
    assert!(
        row.active_turn.as_ref().unwrap().attachments[0]
            .artifact
            .expires_at
            .unwrap()
            > Utc::now() + Duration::days(29)
    );
    associate_in_session(&db, &row, "assistant-message2037", &mut session)
        .await
        .unwrap();
    associate_in_session(&db, &row, "assistant-message2037", &mut session)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>(DELIVERIES)
            .count_documents(doc! {})
            .session(&mut session)
            .await
            .unwrap(),
        1
    );
    let mut other_event = row.clone();
    other_event.active_turn.as_mut().unwrap().turn_id = "different-completion".into();
    assert!(
        associate_in_session(&db, &other_event, "different-message", &mut session)
            .await
            .is_err()
    );
    let mut other_source = row.clone();
    other_source.active_turn.as_mut().unwrap().attachments[0]
        .artifact
        .source_result_identity = Some("different-source".into());
    assert!(
        associate_in_session(&db, &other_source, "different-message", &mut session)
            .await
            .is_err()
    );
    db.collection::<Document>(DELIVERIES).insert_one(doc! {"_id":"conflicting-message","user_id":"another-owner","conversation_id":&row.id,"originating_turn_id":&w.turn_id,"event_turn_id":"completion2037","watch_id":"conflicting-watch","operation_id":&w.operation_id,"source_result_identity":"changed","attachment_ids":[],"expires_at":bson::DateTime::from_chrono(w.expires_at)}).session(&mut session).await.unwrap();
    assert!(
        associate_in_session(&db, &row, "conflicting-message", &mut session)
            .await
            .is_err()
    );
    db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME).insert_one(doc!{"_id":"assistant-message2037","user_id":&w.user_id,"conversation_id":&w.conversation_id,"turn_id":"completion2037","role":"assistant","origin":"event","seq":2,"text":"file result","status":"completed","created_at":bson::DateTime::now(),"attachments":bson::to_bson(&row.active_turn.as_ref().unwrap().attachments).unwrap()}).session(&mut session).await.unwrap();
    super::super::async_service_operation::delivered_in_session(&db, &row, &mut session)
        .await
        .unwrap();
    db.collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"active_turn":null}})
        .session(&mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let pending = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert!(pending.data_encrypted.is_none());
    assert!(pending.secret_context_versions.is_empty());
    assert_eq!(pending.state, "materialized");
    let (mime, bytes) =
        super::super::assistant_nyxagent::read_attachment(&db, &keys, &w.user_id, &row.id, &id)
            .await
            .unwrap();
    assert_eq!(mime, "video/mp4");
    assert_eq!(bytes, MP4);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("range", "bytes=4-15".parse().unwrap());
    let budget_before_egress = byte_budget::available_for_tests();
    let response = http::owner_response(&db, &keys, &w.user_id, &row.id, &id, &headers)
        .await
        .unwrap();
    assert_eq!(
        budget_before_egress - byte_budget::available_for_tests(),
        MP4.len() * 2,
        "owner egress reservation must remain held while the response body owns its bytes"
    );
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.headers()["content-range"],
        format!("bytes 4-15/{}", MP4.len())
    );
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("movie.mp4")
    );
    assert_eq!(
        &axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()[..],
        &MP4[4..16]
    );
    assert_eq!(
        byte_budget::available_for_tests(),
        budget_before_egress,
        "owner egress reservation must release after the body is consumed"
    );
    assert!(
        http::owner_response(&db, &keys, "foreign2037", &row.id, &id, &headers)
            .await
            .is_err()
    );
    headers.insert("range", "bytes=184467440737095516160-".parse().unwrap());
    let unsatisfiable = http::owner_response(&db, &keys, &w.user_id, &row.id, &id, &headers)
        .await
        .unwrap();
    assert_eq!(unsatisfiable.status(), 416);
    assert_eq!(
        unsatisfiable.headers()["content-range"],
        format!("bytes */{}", MP4.len())
    );
    assert_eq!(unsatisfiable.headers()["content-length"], "0");
    assert!(
        axum::body::to_bytes(unsatisfiable.into_body(), 0)
            .await
            .unwrap()
            .is_empty()
    );
    for range in [
        "bytes=0-184467440737095516160",
        "bytes=-184467440737095516160",
    ] {
        headers.insert("range", range.parse().unwrap());
        let response = http::owner_response(&db, &keys, &w.user_id, &row.id, &id, &headers)
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(
            response.headers()["content-range"],
            format!("bytes 0-{}/{}", MP4.len() - 1, MP4.len())
        );
        assert_eq!(
            &axum::body::to_bytes(response.into_body(), sandbox::FILE_BYTES)
                .await
                .unwrap()[..],
            MP4
        );
    }
    for range in ["bytes=0-1,4-5", "bytes=invalid", "bytes=4-2", "items=1-2"] {
        headers.insert("range", range.parse().unwrap());
        let response = http::owner_response(&db, &keys, &w.user_id, &row.id, &id, &headers)
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.headers().get("content-range").is_none());
        assert_eq!(
            &axum::body::to_bytes(response.into_body(), sandbox::FILE_BYTES)
                .await
                .unwrap()[..],
            MP4
        );
    }
    headers.insert("range", "bytes=0-1".parse().unwrap());
    headers.append("range", "bytes=4-5".parse().unwrap());
    let response = http::owner_response(&db, &keys, &w.user_id, &row.id, &id, &headers)
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        &axum::body::to_bytes(response.into_body(), sandbox::FILE_BYTES)
            .await
            .unwrap()[..],
        MP4
    );
    headers.remove("range");
    headers.insert("range", "bytes=0-1".parse().unwrap());
    headers.insert("if-range", "an-unknown-validator".parse().unwrap());
    assert_eq!(
        http::owner_response(&db, &keys, &w.user_id, &row.id, &id, &headers)
            .await
            .unwrap()
            .status(),
        200
    );
    recover(&db, &keys, &w.user_id, &row.id, "assistant-message2037")
        .await
        .unwrap();
    recover(&db, &keys, &w.user_id, &row.id, "assistant-message2037")
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        recover(&db, &keys, "foreign2037", &row.id, "assistant-message2037")
            .await
            .is_err()
    );
    assert!(
        recover(&db, &keys, &w.user_id, &row.id, "original-message2037")
            .await
            .is_err()
    );
    db.collection::<Document>(ATTACHMENTS).update_one(doc!{"_id":&id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(matches!(
        super::super::assistant_nyxagent::read_attachment(&db, &keys, &w.user_id, &row.id, &id)
            .await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    db.drop().await.unwrap();
}
#[tokio::test]
async fn generated_artifacts_exact_event_binding_cancellation_capacity_and_recovery() {
    let db = crate::test_utils::connect_transaction_test_database("artrec2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    let (w, mut row) = seeded(&db, &keys).await;
    let mut wrong = row.clone();
    wrong.active_turn.as_mut().unwrap().events[0].id = "foreign-watch".into();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        bind_in_session(&db, &keys, &mut wrong, &w, &mut session)
            .await
            .is_err()
    );
    session.abort_transaction().await.unwrap();
    let mut meta: TurnAttachment = bson::from_document(
        doc! {"id":"old","content_type":"image/png","size":1_i64,"label":"old"},
    )
    .unwrap();
    for index in 0..8 {
        meta.id = format!("old{index}");
        row.active_turn
            .as_mut()
            .unwrap()
            .attachments
            .push(meta.clone());
    }
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session)
        .await
        .unwrap();
    associate_in_session(&db, &row, "capacity-message", &mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let pending = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert!(pending.data_encrypted.is_some());
    assert_eq!(pending.state, "pending");
    db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME).insert_one(doc!{"_id":"capacity-message","user_id":&w.user_id,"conversation_id":&w.conversation_id,"turn_id":"completion2037","role":"assistant","origin":"event","seq":2,"text":"pending delivery","status":"failed","created_at":bson::DateTime::now(),"attachments":[]}).await.unwrap();
    db.collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"active_turn":null}})
        .await
        .unwrap();
    recover(&db, &keys, &w.user_id, &row.id, "capacity-message")
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    // A new operation creates a distinct file identity; cancelled pending bytes cannot reappear.
    let (second, _) = seeded(&db, &keys).await;
    super::super::async_service_operation::cancel(&db, &second.user_id, &second.conversation_id)
        .await
        .unwrap();
    let pending = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find_one(doc! {"watch_id":&second.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.state, "discarded");
    assert!(pending.data_encrypted.is_none());
    db.drop().await.unwrap();
}

#[test]
fn generated_artifacts_saved_admission_is_web_user_only_and_legacy_stays_legacy() {
    let admitted_watch = watch();
    assert!(admitted(&admitted_watch));
    let mut legacy = admitted_watch.clone();
    legacy.artifact_mode = None;
    assert!(!admitted(&legacy));
    for origin in [
        TurnOrigin::Event,
        TurnOrigin::Channel,
        TurnOrigin::Trigger,
        TurnOrigin::Group,
    ] {
        let mut denied = admitted_watch.clone();
        denied.delivery.origin = origin;
        assert!(!admitted(&denied));
    }
    for field in [
        "voice_request_id",
        "trigger_run_id",
        "group_id",
        "group_request_id",
        "invocation_id",
    ] {
        let mut denied = bson::to_document(&admitted_watch).unwrap();
        denied
            .get_document_mut("delivery")
            .unwrap()
            .insert(field, "unrelated");
        assert!(!admitted(&bson::from_document(denied).unwrap()));
    }
    let mut denied = admitted_watch.clone();
    denied.delivery.platform_reply = None;
    assert!(!admitted(&denied));
}
#[tokio::test]
async fn generated_artifacts_expiry_and_stale_lease_cannot_publish_or_relabel() {
    let db = crate::test_utils::connect_transaction_test_database("artrace2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    let (w, row) = seeded(&db, &keys).await;
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let changed = sandbox::parse(
        &body("changed.png", PNG),
        w.operation_id.as_deref().unwrap(),
    )
    .unwrap();
    let before = db
        .collection::<Document>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    // First reach the identity fence with a genuinely live waiting lease. A
    // queued/no-lease watch would fail before comparing the saved identity.
    db.collection::<Document>(WATCHES)
        .update_one(
            doc! {"_id":&w.id},
            doc! {"$set":{"state":"waiting","lease_id":"identity-lease",
            "lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(1))}},
        )
        .await
        .unwrap();
    let live = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        persist(
            &db,
            &keys,
            &live,
            &changed,
            &sandbox::Status {
                expires_at: None,
                version: Some("999".into())
            },
            &reservation
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<Document>(ARTIFACTS)
            .find_one(doc! {"watch_id":&w.id})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    let unchanged = sandbox::parse(
        &body("../../movie.mp4", MP4),
        live.operation_id.as_deref().unwrap(),
    )
    .unwrap();
    let mut stale = live.clone();
    stale.lease_id = Some("stale-lease".into());
    assert!(
        persist(
            &db,
            &keys,
            &stale,
            &unchanged,
            &sandbox::Status {
                expires_at: None,
                version: None
            },
            &reservation
        )
        .await
        .is_err()
    );
    db.collection::<Document>(ARTIFACTS).update_many(doc!{"watch_id":&w.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    sweep(&db).await.unwrap();
    let expired = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expired.state, "expired");
    assert!(expired.data_encrypted.is_none());
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {"conversation_id":&row.id})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_artifact_worker_reservation_spans_persist_and_releases_on_error() {
    let db = crate::test_utils::connect_transaction_test_database("artbudget2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    let (w, _row) = seeded(&db, &keys).await;
    let budget_before_success = byte_budget::available_for_tests();
    db.collection::<Document>(WATCHES)
        .update_one(
            doc! {"_id": &w.id},
            doc! {"$set": {"state": "waiting", "lease_id": "budget-lease", "lease_until": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(1))}},
        )
        .await
        .unwrap();
    let live = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id": &w.id})
        .await
        .unwrap()
        .unwrap();
    let result = sandbox::parse(
        &body("../../movie.mp4", MP4),
        live.operation_id.as_deref().unwrap(),
    )
    .unwrap();
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let budget_held = byte_budget::available_for_tests();
    persist(
        &db,
        &keys,
        &live,
        &result,
        &sandbox::Status {
            expires_at: None,
            version: Some("7".into()),
        },
        &reservation,
    )
    .await
    .unwrap();
    assert_eq!(
        budget_held,
        byte_budget::available_for_tests(),
        "the worker reservation must remain held through decode, encryption and persistence"
    );
    drop(reservation);
    assert_eq!(byte_budget::available_for_tests(), budget_before_success);

    // A dropped reservation on a failed persist has the same release contract.
    let budget_before_error = byte_budget::available_for_tests();
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let budget_held = byte_budget::available_for_tests();
    let mut stale = live.clone();
    stale.lease_id = Some("stale-lease".into());
    assert!(
        persist(
            &db,
            &keys,
            &stale,
            &result,
            &sandbox::Status {
                expires_at: None,
                version: Some("7".into()),
            },
            &reservation,
        )
        .await
        .is_err()
    );
    assert_eq!(budget_held, byte_budget::available_for_tests());
    drop(reservation);
    assert_eq!(byte_budget::available_for_tests(), budget_before_error);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn owner_http_preserves_legacy_document_upload_and_image_contracts() {
    let db = crate::test_utils::connect_transaction_test_database("artlegacy2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    let (w, row) = seeded(&db, &keys).await;
    let document = b"Owner document\n".as_slice();
    let mut cases = Vec::new();
    for (mime, bytes) in [
        ("text/plain", document),
        ("image/png", PNG),
        ("image/jpeg", JPEG),
    ] {
        let mut attachment = AssistantAttachment {
            artifact: Default::default(),
            secret_context_versions: Default::default(),
            id: Uuid::new_v4().to_string(),
            origin: "tool".into(),
            user_id: w.user_id.clone(),
            conversation_id: row.id.clone(),
            turn_id: "legacy-turn".into(),
            content_type: mime.into(),
            size: bytes.len() as i64,
            data_encrypted: Vec::new(),
            created_at: Utc::now(),
        };
        attachment.data_encrypted =
            super::super::assistant_record_service::encrypt_image(&keys, &attachment, bytes)
                .await
                .unwrap();
        super::super::credential_record_service::mark_version(
            &mut attachment.secret_context_versions,
            &keys,
            "data_encrypted",
        );
        db.collection::<AssistantAttachment>(ATTACHMENTS)
            .insert_one(&attachment)
            .await
            .unwrap();
        cases.push((attachment.id, mime.to_owned(), bytes.to_vec()));
    }
    // Actual user-upload documents share this route with DocumentAttachment.
    for (name, bytes) in [("owner.txt", document), ("owner.png", PNG)] {
        let meta = super::super::assistant_upload_service::upload(
            &db,
            &keys,
            &w.user_id,
            &row.id,
            name,
            bytes.to_vec(),
        )
        .await
        .unwrap();
        cases.push((meta.id, meta.content_type, bytes.to_vec()));
    }
    for (id, mime, bytes) in cases {
        let response = http::owner_response(
            &db,
            &keys,
            &w.user_id,
            &row.id,
            &id,
            &axum::http::HeaderMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], mime);
        assert_eq!(response.headers()["content-disposition"], "inline");
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            response.headers()["content-security-policy"],
            "default-src 'none'; sandbox"
        );
        assert_eq!(
            &axum::body::to_bytes(response.into_body(), sandbox::FILE_BYTES)
                .await
                .unwrap()[..],
            bytes
        );
    }
    let pending = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    let mut active = AssistantAttachment {
        artifact: metadata(&pending, pending.expires_at),
        secret_context_versions: Default::default(),
        id: Uuid::new_v4().to_string(),
        origin: "generated_artifact".into(),
        user_id: w.user_id.clone(),
        conversation_id: row.id.clone(),
        turn_id: "completion2037".into(),
        content_type: "text/html".into(),
        size: document.len() as i64,
        data_encrypted: Vec::new(),
        created_at: Utc::now(),
    };
    active.artifact.filename = Some("../../unsafe\r\n.html".into());
    active.artifact.preview_kind = Some("download".into());
    active.data_encrypted =
        super::super::assistant_record_service::encrypt_image(&keys, &active, document)
            .await
            .unwrap();
    active
        .secret_context_versions
        .insert("data_encrypted".into(), 3);
    db.collection::<AssistantAttachment>(ATTACHMENTS)
        .insert_one(&active)
        .await
        .unwrap();
    let response = http::owner_response(
        &db,
        &keys,
        &w.user_id,
        &row.id,
        &active.id,
        &axum::http::HeaderMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    assert!(disposition.starts_with("attachment;"));
    assert!(
        disposition.contains("unsafe.html")
            && !disposition.contains("../")
            && !disposition.contains('\r')
            && !disposition.contains('\n')
    );
    assert_eq!(
        &axum::body::to_bytes(response.into_body(), sandbox::FILE_BYTES)
            .await
            .unwrap()[..],
        document
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn delivery_expiry_uses_effective_attachment_retention_and_recovery_checks_live_policy() {
    use crate::models::{
        assistant_message::COLLECTION_NAME as MESSAGES,
        assistant_upload_retention::{Policy, SETTINGS_ID, Settings},
        platform_settings::COLLECTION_NAME as SETTINGS,
    };
    let db = crate::test_utils::connect_transaction_test_database("artretention2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    db.collection::<Settings>(SETTINGS)
        .insert_one(Settings {
            policy: Some(Policy {
                document_days: 1,
                ..Policy::default()
            }),
            ..Settings::default()
        })
        .await
        .unwrap();
    let (w, mut row) = seeded_until(&db, &keys, Utc::now() + Duration::days(10)).await;
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session)
        .await
        .unwrap();
    associate_in_session(&db, &row, "retention-message", &mut session)
        .await
        .unwrap();
    db.collection::<Document>(MESSAGES).insert_one(doc! {"_id":"retention-message",
        "user_id":&w.user_id,"conversation_id":&row.id,"turn_id":"completion2037",
        "role":"assistant","origin":"event","seq":2,"text":"result","status":"completed",
        "created_at":bson::DateTime::now(),"attachments":bson::to_bson(&row.active_turn.as_ref().unwrap().attachments).unwrap()})
        .session(&mut session).await.unwrap();
    db.collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"active_turn":null}})
        .session(&mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let id = row.active_turn.as_ref().unwrap().attachments[0].id.clone();
    let attachment = db
        .collection::<AssistantAttachment>(ATTACHMENTS)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    let delivery = db
        .collection::<Delivery>(DELIVERIES)
        .find_one(doc! {"_id":"retention-message"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(Some(delivery.expires_at), attachment.artifact.expires_at);
    assert!(delivery.expires_at < Utc::now() + Duration::days(2));
    assert!(delivery.expires_at < w.artifact_result.as_ref().unwrap().expires_at);
    recover(&db, &keys, &w.user_id, &row.id, "retention-message")
        .await
        .unwrap();
    db.collection::<Document>(MESSAGES)
        .update_one(
            doc! {"_id":"retention-message"},
            doc! {"$set":{"status":"streaming"}},
        )
        .await
        .unwrap();
    assert!(
        recover(&db, &keys, &w.user_id, &row.id, "retention-message")
            .await
            .is_err()
    );
    db.collection::<Document>(MESSAGES)
        .update_one(
            doc! {"_id":"retention-message"},
            doc! {"$set":{"status":"completed"}},
        )
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Delivery>(DELIVERIES)
            .find_one(doc! {"_id":"retention-message"})
            .await
            .unwrap()
            .unwrap()
            .expires_at,
        delivery.expires_at
    );
    // Live age policy can shorten availability even before the stored deadline.
    db.collection::<Document>(ATTACHMENTS)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(2))}},
        )
        .await
        .unwrap();
    let states = delivery_states(&db, &w.user_id, &row.id, &["retention-message".into()])
        .await
        .unwrap();
    assert_eq!(states["retention-message"].state, "retention_expired");
    assert!(!states["retention-message"].recovery_available);
    assert!(matches!(
        recover(&db, &keys, &w.user_id, &row.id, "retention-message").await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    // The rejected transaction must not relabel or extend the association.
    assert!(
        db.collection::<Delivery>(DELIVERIES)
            .find_one(doc! {"_id":"retention-message"})
            .await
            .unwrap()
            .unwrap()
            == delivery
    );
    assert!(
        db.collection::<Document>(SETTINGS)
            .find_one(doc! {"_id":SETTINGS_ID})
            .await
            .unwrap()
            .is_some()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_artifact_admission_abort_keeps_pending_bytes_and_rolls_back_retention_and_association()
 {
    let db = crate::test_utils::connect_transaction_test_database("artabort2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    let (w, mut row) = seeded(&db, &keys).await;
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session)
        .await
        .unwrap();
    associate_in_session(&db, &row, "aborted-message", &mut session)
        .await
        .unwrap();
    session.abort_transaction().await.unwrap();
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(DELIVERIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let pending = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.state, "pending");
    assert!(pending.data_encrypted.is_some());
    assert!(
        db.collection::<Document>(crate::models::platform_settings::COLLECTION_NAME)
            .find_one(doc! {"_id":crate::models::assistant_upload_retention::SETTINGS_ID})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&w.id})
            .await
            .unwrap()
            .unwrap()
            .artifact_event_turn_id
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_artifact_new_operation_has_new_identity_and_failed_result_preserves_old_bytes() {
    let db = crate::test_utils::connect_transaction_test_database("artrevision2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    let (w, mut row) = seeded(&db, &keys).await;
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session)
        .await
        .unwrap();
    associate_in_session(&db, &row, "first-revision", &mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let old_id = row.active_turn.as_ref().unwrap().attachments[0].id.clone();
    assert_eq!(
        row.active_turn.as_ref().unwrap().attachments[0]
            .artifact
            .content_revision
            .as_deref(),
        Some("1")
    );
    let old = db
        .collection::<Document>(ATTACHMENTS)
        .find_one(doc! {"_id":&old_id})
        .await
        .unwrap()
        .unwrap();
    let mut next = watch();
    next.conversation_id = row.id.clone();
    next.turn_id = "revision-two-user-turn".into();
    next.operation_id = Some("operation-revision-two".into());
    db.collection::<AsyncServiceOperation>(WATCHES)
        .insert_one(&next)
        .await
        .unwrap();
    db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id":"revision-two-request","user_id":&w.user_id,"conversation_id":&row.id,
            "turn_id":&next.turn_id,"role":"user","origin":"user","author":"owner",
            "seq":3,"text":"new revision","status":"completed","created_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    // A failed result of the new operation cannot modify the old attachment.
    let mut failed: serde_json::Value = serde_json::from_str(&body("new.mp4", MP4)).unwrap();
    failed["output"]["files"][0]["error"] = json!("read failed");
    let failed_result =
        sandbox::parse(&failed.to_string(), next.operation_id.as_deref().unwrap()).unwrap();
    assert!(failed_result.files.is_empty());
    assert_eq!(failed_result.rejected.len(), 1);
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .find_one(doc! {"_id":&old_id})
            .await
            .unwrap()
            .unwrap(),
        old
    );
    let changed = include_bytes!("../../../tests/fixtures/generated-artifacts/visual-high.mp4");
    let verified = sandbox::parse(
        &body("../../movie.mp4", changed),
        next.operation_id.as_deref().unwrap(),
    )
    .unwrap();
    assert_ne!(
        verified.identity,
        w.artifact_result.as_ref().unwrap().source_result_identity
    );
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    persist(
        &db,
        &keys,
        &next,
        &verified,
        &sandbox::Status {
            expires_at: None,
            version: Some("1".into()),
        },
        &reservation,
    )
    .await
    .unwrap();
    drop(reservation);
    db.collection::<Document>(WATCHES)
        .update_one(doc! {"_id":&next.id}, doc! {"$set":{"state":"queued"}})
        .await
        .unwrap();
    next = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id":&next.id})
        .await
        .unwrap()
        .unwrap();
    let turn = row.active_turn.as_mut().unwrap();
    turn.turn_id = "revision-two-completion".into();
    turn.events[0].id = next.id.clone();
    turn.attachments.clear();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &next, &mut session)
        .await
        .unwrap();
    associate_in_session(&db, &row, "second-revision", &mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let new_id = &row.active_turn.as_ref().unwrap().attachments[0].id;
    assert_ne!(*new_id, old_id);
    assert_eq!(
        row.active_turn.as_ref().unwrap().attachments[0]
            .artifact
            .content_revision
            .as_deref(),
        Some("2")
    );
    let old_after = db
        .collection::<Document>(ATTACHMENTS)
        .find_one(doc! {"_id":&old_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old_after, old);
    assert_eq!(
        super::super::assistant_nyxagent::read_attachment(&db, &keys, &w.user_id, &row.id, &old_id)
            .await
            .unwrap()
            .1,
        MP4
    );
    assert_eq!(
        super::super::assistant_nyxagent::read_attachment(&db, &keys, &w.user_id, &row.id, new_id)
            .await
            .unwrap()
            .1,
        changed
    );
    check_revision_replay_failure_and_concurrency(&db, &keys, &w, &next).await;
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .find_one(doc! {"_id":&old_id})
            .await
            .unwrap()
            .unwrap(),
        old
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn conversation_delete_removes_encrypted_pending_artifacts_and_saved_associations() {
    let db = crate::test_utils::connect_transaction_test_database("artdelete2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    let (w, row) = seeded(&db, &keys).await;
    db.collection::<Document>(DELIVERIES)
        .insert_one(doc! {"_id":"deleted-delivery",
        "user_id":&w.user_id,"conversation_id":&row.id})
        .await
        .unwrap();
    for collection in [ARTIFACTS, DELIVERIES, SERIES] {
        db.collection::<Document>(collection)
            .insert_one(doc! {"_id":Uuid::new_v4().to_string(),
            "user_id":"foreign-owner","conversation_id":&row.id})
            .await
            .unwrap();
    }
    super::super::assistant_nyxagent::delete(&db, &w.user_id, &row.id)
        .await
        .unwrap();
    for collection in [WATCHES, ARTIFACTS, DELIVERIES, SERIES, ATTACHMENTS] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {"user_id":&w.user_id,"conversation_id":&row.id})
                .await
                .unwrap(),
            0
        );
    }
    for collection in [ARTIFACTS, DELIVERIES, SERIES] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {"user_id":"foreign-owner"})
                .await
                .unwrap(),
            1
        );
    }
    db.drop().await.unwrap();
}

#[test]
fn generated_file_rejections_preserve_good_peers_and_accepted_budget() {
    use crate::models::generated_artifact::RejectionReason;
    let encoded = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let entry = |path: &str, content: String| json!({"path":path,"content":content});
    let good = entry("good.png", encoded(PNG));
    let invalid = entry("/private/secret-path/bad.png", encoded(b"invalid-image"));
    let oversized = entry("too-big.bin", encoded(&vec![0; sandbox::FILE_BYTES + 1]));
    for entries in [
        vec![invalid.clone(), good.clone(), oversized.clone()],
        vec![oversized, good.clone(), invalid],
    ] {
        let wire = json!({"success":true,"output":{"stdout":"private-stream","stderr":"","exit_code":0,"files":entries}}).to_string();
        assert!(wire.len() < sandbox::RESPONSE_BYTES);
        let result = sandbox::parse(&wire, "op").unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].index, 1);
        assert_eq!(&*result.files[0].bytes, PNG);
        assert_eq!(result.rejected.len(), 2);
        assert_eq!(result.rejected[0].index, 0);
        assert_eq!(result.rejected[1].index, 2);
        assert!(
            result
                .rejected
                .iter()
                .any(|r| matches!(r.reason, RejectionReason::TooLarge))
        );
        assert!(
            result
                .rejected
                .iter()
                .any(|r| matches!(r.reason, RejectionReason::Invalid))
        );
        assert!(
            !serde_json::to_string(&result.rejected)
                .unwrap()
                .contains("private")
        );
        assert!(!result.summary.contains("good.png"));
        assert!(!result.summary.contains("private"));
    }
    // Four rejected MiB may be decoded for validation, but do not consume the
    // accepted-total allowance or hide the final small valid peer.
    for invalid_first in [true, false] {
        let rejected = entry("bad.png", encoded(&vec![0; 4 * 1024 * 1024]));
        let accepted = entry("good.bin", encoded(&vec![42; 5 * 1024 * 1024]));
        let entries = if invalid_first {
            vec![rejected, accepted, good.clone()]
        } else {
            vec![accepted, good.clone(), rejected]
        };
        let wire = json!({"success":true,"output":{"stdout":"","stderr":"","exit_code":0,"files":entries}}).to_string();
        assert!(wire.len() <= sandbox::RESPONSE_BYTES);
        let result = sandbox::parse(&wire, "op").unwrap();
        assert_eq!(result.files.len(), 2);
        assert_eq!(result.rejected.len(), 1);
        assert!(matches!(
            result.rejected[0].reason,
            RejectionReason::Invalid | RejectionReason::TooLarge
        ));
        assert_eq!(
            result.files.iter().map(|f| f.bytes.len()).sum::<usize>(),
            5 * 1024 * 1024 + PNG.len()
        );
    }
}

#[test]
fn generated_artifacts_full_canonical_path_binds_identity_before_display_truncation() {
    let parsed = |path: &str| sandbox::parse(&body(path, MP4), "same-operation").unwrap();
    let canonical = parsed("/tmp/movie.mp4");
    let equivalent = parsed("/tmp/ignored/.././movie.mp4");
    assert_eq!(canonical.identity, equivalent.identity);
    assert_eq!(
        canonical.files[0].source_path_sha256,
        equivalent.files[0].source_path_sha256
    );
    let elsewhere = parsed("/elsewhere/movie.mp4");
    assert_eq!(canonical.files[0].filename, elsewhere.files[0].filename);
    assert_ne!(canonical.identity, elsewhere.identity);
    let prefix = format!("/{}", "long/".repeat(140));
    let first = parsed(&format!("{prefix}first/movie.mp4"));
    let second = parsed(&format!("{prefix}second/movie.mp4"));
    assert_eq!(first.files[0].source_path, second.files[0].source_path);
    assert_eq!(first.files[0].source_path.len(), 512);
    assert_eq!(first.files[0].filename, second.files[0].filename);
    assert_ne!(
        first.files[0].source_path_sha256,
        second.files[0].source_path_sha256
    );
    assert_ne!(first.identity, second.identity);
}

fn revision_watch<'a>(
    db: &'a Database,
    original: &'a AsyncServiceOperation,
) -> futures::future::BoxFuture<'a, AsyncServiceOperation> {
    Box::pin(async move {
        let mut next = watch();
        next.conversation_id = original.conversation_id.clone();
        next.operation_id = Some(Uuid::new_v4().to_string());
        db.collection::<AsyncServiceOperation>(WATCHES)
            .insert_one(&next)
            .await
            .unwrap();
        next
    })
}
fn persist_revision<'a>(
    db: &'a Database,
    keys: &'a EncryptionKeys,
    w: &'a AsyncServiceOperation,
    result: &'a sandbox::ResultFiles,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let reservation = byte_budget::reserve(sandbox::RESERVATION).await?;
        persist(
            db,
            keys,
            w,
            result,
            &sandbox::Status {
                expires_at: None,
                version: Some("999".into()),
            },
            &reservation,
        )
        .await
    })
}
fn check_revision_replay_failure_and_concurrency<'a>(
    db: &'a Database,
    keys: &'a EncryptionKeys,
    original: &'a AsyncServiceOperation,
    changed: &'a AsyncServiceOperation,
) -> futures::future::BoxFuture<'a, ()> {
    Box::pin(async move {
        let high = include_bytes!("../../../tests/fixtures/generated-artifacts/visual-high.mp4");
        let head = db
            .collection::<ContentSeries>(SERIES)
            .find_one(doc! {"conversation_id":&original.conversation_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(head.revision, 2);
        assert_eq!(
            head.catalog_service_id,
            original.artifact_catalog_service_id.as_deref().unwrap()
        );
        let unchanged = revision_watch(db, original).await;
        let result = sandbox::parse(
            &body("../../movie.mp4", high),
            unchanged.operation_id.as_deref().unwrap(),
        )
        .unwrap();
        persist_revision(db, keys, &unchanged, &result)
            .await
            .unwrap();
        let committed = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&unchanged.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            committed.artifact_result.as_ref().unwrap().files[0]
                .content_revision
                .as_deref(),
            Some("2")
        );
        assert_eq!(
            committed
                .artifact_result
                .as_ref()
                .unwrap()
                .source_status_version
                .as_deref(),
            Some("999")
        );
        let pending = db
            .collection::<Document>(ARTIFACTS)
            .find_one(doc! {"watch_id":&unchanged.id})
            .await
            .unwrap()
            .unwrap();
        let series_before = db
            .collection::<Document>(SERIES)
            .find_one(doc! {"_id":&head.id})
            .await
            .unwrap()
            .unwrap();
        // Restore a live lease to exercise a real persist retry with an existing
        // immutable result, rather than merely calling the same parser twice.
        db.collection::<Document>(WATCHES)
            .update_one(
                doc! {"_id":&unchanged.id},
                doc! {"$set":{"state":&unchanged.state,"lease_id":&unchanged.lease_id,
                "lease_until":bson::to_bson(&unchanged.lease_until).unwrap()}},
            )
            .await
            .unwrap();
        persist_revision(db, keys, &unchanged, &result)
            .await
            .unwrap();
        assert_eq!(
            db.collection::<Document>(ARTIFACTS)
                .find_one(doc! {"watch_id":&unchanged.id})
                .await
                .unwrap()
                .unwrap(),
            pending
        );
        assert_eq!(
            db.collection::<Document>(SERIES)
                .find_one(doc! {"_id":&head.id})
                .await
                .unwrap()
                .unwrap(),
            series_before
        );
        // Same basename/bytes under a different full path is a changed result
        // for this operation; it cannot mint another series or replace descriptors.
        db.collection::<Document>(WATCHES)
            .update_one(
                doc! {"_id":&unchanged.id},
                doc! {"$set":{"state":&unchanged.state,"lease_id":&unchanged.lease_id,
                "lease_until":bson::to_bson(&unchanged.lease_until).unwrap()}},
            )
            .await
            .unwrap();
        let moved = sandbox::parse(
            &body("/different/movie.mp4", high),
            unchanged.operation_id.as_deref().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            persist_revision(db, keys, &unchanged, &moved).await,
            Err(AppError::Conflict(_))
        ));
        let after_refusal = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&unchanged.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_bson(&after_refusal.artifact_result).unwrap(),
            bson::to_bson(&committed.artifact_result).unwrap()
        );
        assert_eq!(
            db.collection::<Document>(ARTIFACTS)
                .find_one(doc! {"watch_id":&unchanged.id})
                .await
                .unwrap()
                .unwrap(),
            pending
        );
        assert_eq!(
            db.collection::<Document>(SERIES)
                .count_documents(doc! {"conversation_id":&original.conversation_id})
                .await
                .unwrap(),
            1
        );
        for rejected in [false, true] {
            let failed = revision_watch(db, original).await;
            let mut output: serde_json::Value =
                serde_json::from_str(&body("../../movie.mp4", MP4)).unwrap();
            output["success"] = json!(false);
            if rejected {
                output["output"]["files"][0]["error"] = json!("private failure");
            }
            let failed_result =
                sandbox::parse(&output.to_string(), failed.operation_id.as_deref().unwrap())
                    .unwrap();
            persist_revision(db, keys, &failed, &failed_result)
                .await
                .unwrap();
            let saved = db
                .collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&failed.id})
                .await
                .unwrap()
                .unwrap();
            let manifest = saved.artifact_result.unwrap();
            assert_eq!(manifest.count, usize::from(!rejected));
            assert!(manifest.files.iter().all(|f| f.content_revision.is_none()));
            assert_eq!(
                db.collection::<Document>(SERIES)
                    .find_one(doc! {"_id":&head.id})
                    .await
                    .unwrap()
                    .unwrap(),
                series_before
            );
        }
        check_concurrent_revisions(db, keys, original).await;
        // Catalog identity and conversation are part of the scope. New scopes
        // start at1 even with the same path and content used by this conversation.
        let mut other_catalog = revision_watch(db, original).await;
        other_catalog.artifact_catalog_service_id =
            Some("00000000-0000-4000-8000-000000002038".into());
        db.collection::<Document>(WATCHES).update_one(doc! {"_id":&other_catalog.id},
            doc! {"$set":{"artifact_catalog_service_id":&other_catalog.artifact_catalog_service_id}}).await.unwrap();
        let result = sandbox::parse(
            &body("../../movie.mp4", MP4),
            other_catalog.operation_id.as_deref().unwrap(),
        )
        .unwrap();
        persist_revision(db, keys, &other_catalog, &result)
            .await
            .unwrap();
        let saved = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&other_catalog.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.artifact_result.unwrap().files[0]
                .content_revision
                .as_deref(),
            Some("1")
        );
        let (separate, _) = seeded(db, keys).await;
        assert_eq!(
            separate.artifact_result.unwrap().files[0]
                .content_revision
                .as_deref(),
            Some("1")
        );
        assert_eq!(
            db.collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&changed.id})
                .await
                .unwrap()
                .unwrap()
                .artifact_result
                .unwrap()
                .files[0]
                .content_revision
                .as_deref(),
            Some("2")
        );
    })
}
fn check_concurrent_revisions<'a>(
    db: &'a Database,
    keys: &'a EncryptionKeys,
    original: &'a AsyncServiceOperation,
) -> futures::future::BoxFuture<'a, ()> {
    Box::pin(async move {
        let first = revision_watch(db, original).await;
        let second = revision_watch(db, original).await;
        let mixed = include_bytes!("../../../tests/fixtures/generated-artifacts/visual-mixed.mp4");
        let first_result = sandbox::parse(
            &body("../../movie.mp4", MP4),
            first.operation_id.as_deref().unwrap(),
        )
        .unwrap();
        let second_result = sandbox::parse(
            &body("../../movie.mp4", mixed),
            second.operation_id.as_deref().unwrap(),
        )
        .unwrap();
        let (one, two) = tokio::join!(
            persist_revision(db, keys, &first, &first_result),
            persist_revision(db, keys, &second, &second_result)
        );
        one.unwrap();
        two.unwrap();
        let mut revisions = Vec::new();
        for w in [&first, &second] {
            let saved = db
                .collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&w.id})
                .await
                .unwrap()
                .unwrap();
            revisions.push(
                saved.artifact_result.unwrap().files[0]
                    .content_revision
                    .clone()
                    .unwrap(),
            );
        }
        revisions.sort();
        assert_eq!(revisions, ["3", "4"]);
        let head = db
            .collection::<ContentSeries>(SERIES)
            .find_one(doc! {"conversation_id":&original.conversation_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(head.revision, 4);
    })
}
