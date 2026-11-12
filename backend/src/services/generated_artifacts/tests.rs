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
    bson::from_document(doc!{"_id":Uuid::new_v4().to_string(),"user_id":"00000000-0000-4000-8000-000000002037","conversation_id":"nyxa-2037","agent_id":"agent2037","turn_id":"original2037","api_key_id":"key2037","service_id":"sandbox2037","submit_endpoint_id":"submit2037",
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
    let legacy: Metadata = bson::from_document(doc! {}).unwrap();
    assert!(bson::to_document(&legacy).unwrap().is_empty());
    let legacy = TurnAttachment {
        image_input_reason: None,
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
fn generated_file_rejections_preserve_good_peers_and_accepted_budget() {
    use crate::models::generated_artifact::RejectionReason;
    let encoded = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let entry = |path: &str, content: String| json!({"path":path,"content":content});
    let good = entry("good.png", encoded(PNG));
    let invalid = entry("/private/secret-path/bad.png", encoded(b"invalid-image"));
    let oversized = entry("too-big.bin", encoded(&vec![0; sandbox::FILE_BYTES + 1]));
    let mut nested = json!("private-upstream-detail");
    for _ in 0..16 {
        nested = json!([nested]);
    }
    let mut malformed: Vec<String> = vec![
        json!(null),
        json!(false),
        json!(12),
        json!("private-entry"),
        nested.clone(),
        json!({}),
        json!({"content":encoded(PNG)}),
        json!({"path":null,"content":encoded(PNG)}),
        json!({"path":12,"content":encoded(PNG)}),
        json!({"path":{"private":nested.clone()},"content":encoded(PNG)}),
        json!({"path":"/private/bad.png","content":nested.clone()}),
        json!({"path":"/private/bad.png","content":true}),
        json!({"path":"/private/bad.png","content":encoded(PNG),"error":{"private":nested.clone()}}),
        json!({"path":"/private/bad.png","error":null,"size":null}),
    ]
    .into_iter()
    .map(|value| value.to_string())
    .collect();
    malformed.push(format!(
        r#"{{"path":"/private/first.png","path":"/private/bad.png","content":"{}"}}"#,
        encoded(PNG)
    ));
    let before = entry("before.png", encoded(PNG)).to_string();
    let after = entry("after.png", encoded(PNG)).to_string();
    for peer in malformed {
        let wire = format!(
            r#"{{"success":true,"output":{{"stdout":"","stderr":"","exit_code":0,"files":[{before},{peer},{after}]}}}}"#
        );
        let result = sandbox::parse(&wire, "op").unwrap();
        assert_eq!(result.files.len(), 2, "malformed peer: {peer}");
        assert_eq!(result.files[0].index, 0);
        assert_eq!(result.files[1].index, 2);
        for file in &result.files {
            assert_eq!(&*file.bytes, PNG);
        }
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].index, 1);
        assert!(matches!(
            result.rejected[0].reason,
            RejectionReason::Invalid
        ));
        assert!(
            !serde_json::to_string(&result.rejected)
                .unwrap()
                .contains("private")
        );
    }
    // Declared size is an untrusted hint. Actual decoded bytes determine the
    // retained size, regardless of the hint's value or wire type.
    for size in [
        json!(null),
        json!(-1),
        json!("1"),
        json!(1.5),
        json!(u64::MAX),
    ] {
        let wire = json!({"success":true,"output":{"stdout":"","stderr":"","exit_code":0,
            "files":[{"path":"good.png","content":encoded(PNG),"size":size}]}})
        .to_string();
        let result = sandbox::parse(&wire, "op").unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(&*result.files[0].bytes, PNG);
        assert!(result.rejected.is_empty());
    }
    // Unknown fields and absent/null optional hints keep their prior behavior.
    let wire = json!({"success":true,"output":{"stdout":"","stderr":"","exit_code":0,"files":[
        {"path":"good.png","content":encoded(PNG),"error":null,"size":null,"future":{"private":nested}},
        {"path":"unavailable.bin","error":"private-upstream-detail"}
    ]}}).to_string();
    let result = sandbox::parse(&wire, "op").unwrap();
    assert_eq!(result.files.len(), 1);
    assert_eq!(&*result.files[0].bytes, PNG);
    assert_eq!(result.rejected[0].index, 1);
    assert!(matches!(
        result.rejected[0].reason,
        RejectionReason::Unavailable
    ));
    assert_eq!(sandbox::parse("{", "op").err(), Some("artifact_invalid"));
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
    assert_eq!(first.files[0].filename, second.files[0].filename);
    assert_ne!(
        first.files[0].source_path_sha256,
        second.files[0].source_path_sha256
    );
    assert_ne!(first.identity, second.identity);
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
async fn generated_artifact_admission_abort_keeps_pending_bytes_and_rolls_back_retention_and_association()
 {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let db = crate::test_utils::connect_transaction_test_database("artabort2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    let (w, mut row) = seeded(&db, &keys).await;
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session, Some(&reservation))
        .await
        .unwrap();
    associate_in_session(&db, &mut row, "aborted-message", &mut session)
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

#[test]
fn generated_retention_live_policy_both_orderings_and_immutable_cap() {
    use crate::models::assistant_upload_retention::Policy;
    let now = Utc::now();
    let old = Policy {
        image_days: 30,
        document_days: 60,
        tool_image_days: Some(1),
        ..Default::default()
    };
    let short = Policy {
        image_days: 2,
        document_days: 3,
        ..old
    };
    for (kind, initial_days, short_days) in [
        ("image/png", 30, 2),
        ("video/mp4", 60, 3),
        ("application/octet-stream", 60, 3),
    ] {
        // Materialization before shortening: live reads and cleanup shorten immediately.
        let row = doc! {"origin":"generated_artifact","created_at":bson::DateTime::from_chrono(now),"expires_at":bson::DateTime::from_chrono(now+Duration::days(initial_days)),"content_type":kind};
        let expires =
            super::super::assistant_upload_retention::generated_expiry(&row, short).unwrap();
        assert_eq!(
            expires.timestamp_millis(),
            (now + Duration::days(short_days)).timestamp_millis()
        );
        // Shortening before materialization: immutable cap captures short policy.
        let mut row = row;
        row.insert("expires_at", bson::DateTime::from_chrono(expires));
        assert_eq!(
            super::super::assistant_upload_retention::generated_expiry(&row, old).unwrap(),
            expires
        );
    }
}
#[tokio::test]
async fn generated_delivery_roundtrip_private_binding_recovery_and_ttl() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use mongodb::event::command::CommandEvent;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let artifact_reads = Arc::new(AtomicUsize::new(0));
    let observed = artifact_reads.clone();
    let handler = mongodb::event::EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && ["find", "aggregate"].iter().any(|name| {
                event
                    .command
                    .get_str(name)
                    .is_ok_and(|collection| [ARTIFACTS, DELIVERIES, SERIES].contains(&collection))
            })
        {
            observed.fetch_add(1, Ordering::SeqCst);
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "artround2037",
        handler,
    )
    .await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    for name in [ARTIFACTS, DELIVERIES, SERIES] {
        let indexes: Vec<Document> = db
            .run_command(doc! {"listIndexes":name})
            .await
            .unwrap()
            .get_document("cursor")
            .unwrap()
            .get_array("firstBatch")
            .unwrap()
            .iter()
            .map(|b| b.as_document().unwrap().clone())
            .collect();
        assert!(indexes.iter().any(
            |i| i.get_document("key").unwrap().contains_key("purge_after")
                && i.get("expireAfterSeconds").is_some()
        ));
    }
    let (w, mut row) = seeded(&db, &keys).await;
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        matches!(bind_in_session(&db,&keys,&mut row,&w,&mut session,None).await,Err(AppError::Conflict(reason)) if reason==PREPARE_BUDGET)
    );
    session.abort_transaction().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session, Some(&reservation))
        .await
        .unwrap();
    row.message_count += 1;
    associate_in_session(&db, &mut row, "reply2037", &mut session)
        .await
        .unwrap();
    let outcome = settled_outcome_in_session(&db, &row, &mut session)
        .await
        .unwrap()
        .unwrap();
    let attachments = row.active_turn.as_ref().unwrap().attachments.clone();
    db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME).insert_one(doc!{"_id":"reply2037","user_id":&w.user_id,"conversation_id":&row.id,"turn_id":"completion2037","role":"assistant","origin":"event","seq":2_i64,"text":"done","status":"completed","created_at":bson::DateTime::now(),"attachments":bson::to_bson(&attachments).unwrap(),"turn_metadata":{"artifact_delivery":bson::to_bson(&outcome).unwrap()}}).session(&mut session).await.unwrap();
    row.active_turn = None;
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .session(&mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    drop(reservation);
    let id = &attachments[0].id;
    let read = super::super::assistant_nyxagent::read_attachment_for_caller(
        &db,
        &keys,
        &w.user_id,
        &row.id,
        id,
        Some("another-chat"),
    )
    .await;
    assert!(matches!(read, Err(AppError::NotFound(_))));
    let read = super::read(&db, &keys, &w.user_id, &row.id, id)
        .await
        .unwrap();
    assert_eq!(&*read.bytes, MP4);
    drop(read);
    preflight(&db, &w.user_id, &row.id, "reply2037")
        .await
        .unwrap();
    recover(&db, &keys, &w.user_id, &row.id, "reply2037")
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let pending = db
        .collection::<Document>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!pending.contains_key("source_path"));
    storage::verify(&keys, &pending).await.unwrap();
    use crate::handlers::assistant_nyxagent as http;
    use crate::services::feature_flag_service::{self as flags, FlagTarget};
    use axum::extract::{Path, Query, State};
    let state = crate::test_utils::test_app_state(db.clone());
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    let auth = crate::test_utils::test_auth_user(&w.user_id);
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &w.user_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    flags::set_platform_override(
        &db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &FlagTarget::User(w.user_id.clone()),
        true,
        &w.user_id,
    )
    .await
    .unwrap();
    // Kill switch hides additive history fields, but the retained human GET stays readable.
    let response = http::attachment(
        State(state.clone()),
        auth.clone(),
        Path((row.id.clone(), id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(response.headers()["content-length"], MP4.len().to_string());
    assert!(response.headers().get("accept-ranges").is_none());
    assert!(response.headers().get("content-range").is_none());
    assert_eq!(
        &axum::body::to_bytes(response.into_body(), sandbox::FILE_BYTES)
            .await
            .unwrap()[..],
        MP4
    );
    for enabled in [false, true, false] {
        flags::set_platform_override(
            &db,
            flags::ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY,
            &FlagTarget::User(w.user_id.clone()),
            enabled,
            &w.user_id,
        )
        .await
        .unwrap();
        artifact_reads.store(0, Ordering::SeqCst);
        let history = http::history(
            State(state.clone()),
            auth.clone(),
            Path(row.id.clone()),
            Query(Default::default()),
        )
        .await
        .unwrap();
        assert_eq!(artifact_reads.load(Ordering::SeqCst) > 0, enabled);
        let history = serde_json::to_value(history.0).unwrap();
        let reply = history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "reply2037")
            .unwrap();
        assert_eq!(
            reply["attachments"].as_array().unwrap().len(),
            usize::from(enabled)
        );
        assert_eq!(reply.get("artifact_delivery").is_some(), enabled);
    }
    assert!(matches!(
        http::artifact_delivery(
            State(state.clone()),
            auth.clone(),
            Path((row.id.clone(), "reply2037".into())),
            bytes::Bytes::new()
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    flags::set_platform_override(
        &db,
        flags::ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY,
        &FlagTarget::User(w.user_id.clone()),
        true,
        &w.user_id,
    )
    .await
    .unwrap();
    for (message, body) in [
        ("reply2037", bytes::Bytes::from_static(b"{}")),
        ("missing", bytes::Bytes::new()),
    ] {
        assert!(
            http::artifact_delivery(
                State(state.clone()),
                auth.clone(),
                Path((row.id.clone(), message.into())),
                body
            )
            .await
            .is_err()
        );
    }
    assert_eq!(db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc!{"event_type":"assistant_mcp_tool_call","event_data.tool_name":"nyx__artifact_delivery"}).await.unwrap(),0);
    assert_eq!(
        db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let saved = db
        .collection::<Document>(ATTACHMENTS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    super::super::assistant_record_service::verify_current_attachment(&db, &keys, &saved)
        .await
        .unwrap();
    let mut tampered = saved.clone();
    tampered.insert("source_result_identity", "changed");
    assert!(
        super::super::assistant_record_service::verify_current_attachment(&db, &keys, &tampered)
            .await
            .is_err()
    );
    db.collection::<Document>(ATTACHMENTS)
        .replace_one(doc! {"_id":id}, &tampered)
        .await
        .unwrap();
    assert!(matches!(
        preflight(&db, &w.user_id, &row.id, "reply2037").await,
        Err(AppError::Conflict(_))
    ));
    db.collection::<Document>(ATTACHMENTS)
        .delete_one(doc! {"_id":id})
        .await
        .unwrap();
    assert!(
        preflight(&db, &w.user_id, &row.id, "reply2037")
            .await
            .is_err()
    );
    assert!(
        recover(&db, &keys, &w.user_id, &row.id, "reply2037")
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(
        db.collection::<Document>(crate::models::platform_settings::COLLECTION_NAME)
            .find_one(doc! {"_id":crate::models::assistant_upload_retention::SETTINGS_ID})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn maximum_encrypted_pending_document_fits_bson_and_rejects_oversize() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let keys = crate::test_utils::test_encryption_keys();
    let mut row:PendingArtifact=bson::from_document(doc!{"_id":Uuid::new_v4().to_string(),"watch_id":"watch","user_id":"owner","conversation_id":"conversation","turn_id":"turn","operation_id":"operation","source_result_identity":"a".repeat(64),"source_path_sha256":"b".repeat(64),"artifact_index":0,"filename":"x".repeat(160),"purge_after":bson::DateTime::now(),"content_type":"application/octet-stream","size":sandbox::FILE_BYTES as i64,"sha256":"a".repeat(64),"preview_kind":"download","content_revision":"1","state":"pending","assistant_attachment_id":Uuid::new_v4().to_string(),"created_at":bson::DateTime::now(),"expires_at":bson::DateTime::now()}).unwrap();
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

#[tokio::test]
async fn generated_artifact_missing_saved_bytes_are_no_material_in_actual_registry() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use super::super::encrypted_record_migration as migration;
    let db = crate::test_utils::connect_transaction_test_database("artmissingregistry2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    ensure_indexes(&db).await.unwrap();
    let (w, mut row) = seeded(&db, &keys).await;
    let pending = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &keys, &mut row, &w, &mut session, Some(&reservation))
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let attachment = row.active_turn.as_ref().unwrap().attachments[0].id.clone();
    assert_eq!(
        db.collection::<Document>(ATTACHMENTS)
            .delete_one(doc! {"_id":&attachment})
            .await
            .unwrap()
            .deleted_count,
        1
    );
    session.start_transaction().await.unwrap();
    prepare_settlement_in_session(&db, &mut row, &mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    assert!(row.active_turn.as_ref().unwrap().attachments.is_empty());
    let missing = db
        .collection::<Document>(ARTIFACTS)
        .find_one(doc! {"_id":&pending.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(missing.get_str("state").unwrap(), "missing");
    assert_eq!(missing.get("data_encrypted"), Some(&bson::Bson::Null));
    assert!(
        missing
            .get_document("secret_context_versions")
            .unwrap()
            .is_empty()
    );
    let saved = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.artifact_result.unwrap().rejected.iter().any(|r| {
        r.index == pending.artifact_index
            && matches!(
                r.reason,
                crate::models::generated_artifact::RejectionReason::Missing
            )
    }));

    drop(reservation);
    let mut invalid_rows = Vec::new();
    for invalid in [false, true] {
        if invalid {
            for (state, ciphertext, versions) in [
                ("pending", None, Default::default()),
                ("unknown", None, Default::default()),
                ("missing", None, pending.secret_context_versions.clone()),
                ("missing", Some(Vec::new()), Default::default()),
                ("missing", Some(vec![0; 32]), Default::default()),
                (
                    "missing",
                    pending.data_encrypted.clone(),
                    pending.secret_context_versions.clone(),
                ),
            ] {
                let invalid = PendingArtifact {
                    id: Uuid::new_v4().to_string(),
                    watch_id: Uuid::new_v4().to_string(),
                    state: state.into(),
                    data_encrypted: ciphertext,
                    secret_context_versions: versions,
                    ..pending.clone()
                };
                let raw = bson::to_document(&invalid).unwrap();
                db.collection::<Document>(ARTIFACTS)
                    .insert_one(&raw)
                    .await
                    .unwrap();
                invalid_rows.push(raw);
            }
        }
        for mode in [migration::Mode::Verify, migration::Mode::Migrate] {
            let generation = migration::start(&db, &keys, mode, None).await.unwrap();
            let mut report = migration::status(&db, &keys).await.unwrap().unwrap();
            for _ in 0..128 {
                if report.phase == "observed" {
                    break;
                }
                migration::advance(&db, &keys, &generation).await.unwrap();
                report = migration::status(&db, &keys).await.unwrap().unwrap();
            }
            assert_eq!(
                report.phase, "observed",
                "bounded registry walk must finish"
            );
            assert_eq!(report.unresolved, invalid_rows.len() as i64);
            assert_eq!(report.rewritten, 0);
            let outcome = report
                .field_outcomes
                .iter()
                .find(|outcome| {
                    outcome.collection == ARTIFACTS && outcome.field == "data_encrypted"
                })
                .unwrap();
            assert_eq!(outcome.no_material_observed, 1);
            assert_eq!(outcome.authenticated_current, 0);
            assert_eq!(outcome.verification_blocked, invalid_rows.len() as i64);
            assert_eq!(
                db.collection::<Document>(ARTIFACTS)
                    .find_one(doc! {"_id":&pending.id})
                    .await
                    .unwrap()
                    .unwrap(),
                missing
            );
            for raw in &invalid_rows {
                assert_eq!(
                    db.collection::<Document>(ARTIFACTS)
                        .find_one(doc! {"_id":raw.get_str("_id").unwrap()})
                        .await
                        .unwrap()
                        .unwrap(),
                    *raw
                );
            }
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_artifacts_expiry_and_stale_lease_cannot_publish_or_relabel() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
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
    db.collection::<Document>(WATCHES)
        .update_one(
            doc! {"_id":&w.id},
            doc! {"$set":{"deadline":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}},
        )
        .await
        .unwrap();
    // Even a caller holding the older deadline cannot publish past the live one.
    assert!(matches!(
        persist(
            &db,
            &keys,
            &live,
            &unchanged,
            &sandbox::Status {
                expires_at: None,
                version: None,
            },
            &reservation
        )
        .await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    let past = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    acquisition::acquire(&db, &keys, &past, &json!(null), |_| {
        std::future::ready(Err(AppError::Internal(
            "producer must not be called after deadline".into(),
        )))
    })
    .await
    .unwrap();
    assert_eq!(
        db.collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&w.id})
            .await
            .unwrap()
            .unwrap()
            .reason
            .as_deref(),
        Some("timeout")
    );
    assert_eq!(
        db.collection::<Document>(ARTIFACTS)
            .find_one(doc! {"watch_id":&w.id})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    db.collection::<Document>(ARTIFACTS).update_many(doc!{"watch_id":&w.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    // Expiry is enforced without adding a sweep to the dormant upload path.
    // A generated materialization clears the exact expired pending row; idle
    // records remain bounded by their purge_after TTL.
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let (attachments, reason) = materialize(
        &db,
        &keys,
        &w,
        &mut session,
        "completion2037",
        &[],
        &reservation,
    )
    .await
    .unwrap();
    assert!(attachments.is_empty());
    assert_eq!(reason, "result_expired");
    session.commit_transaction().await.unwrap();
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
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
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
async fn conversation_delete_removes_encrypted_pending_artifacts_and_saved_associations() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
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
async fn generated_registry_migrates_pending_and_materialized_files_with_fallback_and_cas() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use crate::crypto::local_key_provider::LocalKeyProvider;
    use crate::services::encrypted_record_migration as migration;
    use std::sync::Arc;
    crate::test_utils::init_test_audit_chain_hmac_key();
    let db = crate::test_utils::connect_transaction_test_database("artrewrap2037").await;
    let old_provider = Arc::new(LocalKeyProvider::new([0x11; 32], None));
    let old = EncryptionKeys::with_provider(old_provider.clone());
    let (w, mut row) = seeded(&db, &old).await;
    let stale = db
        .collection::<Document>(ARTIFACTS)
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    let budget = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bind_in_session(&db, &old, &mut row, &w, &mut session, Some(&budget))
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    drop(budget);
    // A stale pending snapshot must never recreate bytes after materialization.
    let current = EncryptionKeys::with_provider_and_fallback(
        Arc::new(LocalKeyProvider::new([0x22; 32], None)),
        Some(old_provider),
    );
    assert!(!storage::rewrap(&db, &current, &stale).await.unwrap());
    let (_pending_watch, _) = seeded(&db, &old).await;
    let generation = migration::start(&db, &current, migration::Mode::Migrate, None)
        .await
        .unwrap();
    for _ in 0..256 {
        if migration::status(&db, &current)
            .await
            .unwrap()
            .unwrap()
            .phase
            == "observed"
        {
            break;
        }
        migration::advance(&db, &current, &generation)
            .await
            .unwrap();
    }
    let report = migration::status(&db, &current).await.unwrap().unwrap();
    assert_eq!(report.phase, "observed");
    assert_eq!(report.unresolved, 0);
    assert!(report.rewritten >= 2);
    for collection in [ARTIFACTS, ATTACHMENTS] {
        assert!(
            report
                .field_outcomes
                .iter()
                .any(|f| f.collection == collection && f.authenticated_current > 0)
        );
    }
    let pending: Vec<Document> = db
        .collection::<Document>(ARTIFACTS)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for saved in pending {
        storage::verify(&current, &saved).await.unwrap();
    }
    let saved = db
        .collection::<Document>(ATTACHMENTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    crate::services::assistant_record_service::verify_current_attachment(&db, &current, &saved)
        .await
        .unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_acquisition_preserves_status_version_presence_and_rechecks_without_submission() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let db = crate::test_utils::connect_transaction_test_database("artacquire2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    for (before_version, after_version, success) in [
        (Some(7), Some(7), true),
        (Some(7), None, false),
        (None, Some(7), false),
        (Some(7), Some(8), false),
    ] {
        let (w, _) = seeded(&db, &keys).await;
        db.collection::<Document>(ARTIFACTS)
            .delete_many(doc! {"watch_id":&w.id})
            .await
            .unwrap();
        db.collection::<Document>(WATCHES).update_one(doc!{"_id":&w.id},doc!{"$set":{"state":"waiting","artifact_result":null,"result_encrypted":null,"secret_context_versions":{},"lease_id":"acquire-lease","lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(1))}}).await.unwrap();
        let w = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&w.id})
            .await
            .unwrap()
            .unwrap();
        let status = |version: Option<i32>| {
            let mut v = json!({"operation_id":w.operation_id,"status":"succeeded","result_available":true,"expires_at":(Utc::now()+Duration::minutes(20)).to_rfc3339()});
            if let Some(version) = version {
                v["version"] = json!(version);
            }
            v
        };
        let before = status(before_version);
        let after = status(after_version).to_string();
        let result = body("../../movie.mp4", MP4);
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        acquisition::acquire(&db, &keys, &w, &before, move |operation| {
            observed.fetch_add(1, Ordering::SeqCst);
            let response = match operation.as_str() {
                "get_execution_result_handler" => result.clone(),
                "get_execution_handler" => after.clone(),
                _ => panic!("acquisition must never submit execution"),
            };
            async move {
                Ok(crate::services::mcp_service::tool_response(
                    200,
                    Some("application/json"),
                    response.as_bytes(),
                ))
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let saved = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&w.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.artifact_result.is_some(), success);
        if !success {
            assert_eq!(saved.reason.as_deref(), Some("artifact_source_changed"));
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_acquisition_second_producer_progresses_and_cancellation_releases_budget() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use std::sync::Arc;
    let db = crate::test_utils::connect_transaction_test_database("artslots2037").await;
    let keys = Arc::new(crate::test_utils::test_encryption_keys());
    let mut watches = Vec::new();
    for _ in 0..2 {
        let (w, _) = seeded(&db, &keys).await;
        db.collection::<Document>(ARTIFACTS)
            .delete_many(doc! {"watch_id":&w.id})
            .await
            .unwrap();
        db.collection::<Document>(WATCHES).update_one(doc!{"_id":&w.id},doc!{"$set":{"state":"waiting","artifact_result":null,"result_encrypted":null,"secret_context_versions":{},"lease_id":"slot-lease","lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(1))}}).await.unwrap();
        watches.push(
            db.collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&w.id})
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let available = byte_budget::available_for_tests();
    let entered = Arc::new(tokio::sync::Notify::new());
    let stalled = {
        let entered = entered.clone();
        let db = db.clone();
        let keys = keys.clone();
        let w = watches.remove(0);
        tokio::spawn(async move {
            let status =
                json!({"operation_id":w.operation_id,"status":"succeeded","result_available":true});
            acquisition::acquire(&db, &keys, &w, &status, move |_| {
                entered.notify_one();
                std::future::pending::<AppResult<crate::services::mcp_service::ToolResponse>>()
            })
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let w = &watches[0];
    let status =
        json!({"operation_id":w.operation_id,"status":"succeeded","result_available":true});
    let after = status.to_string();
    let result = body("movie.mp4", MP4);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        acquisition::acquire(&db, &keys, w, &status, move |operation| {
            let response = match operation.as_str() {
                "get_execution_result_handler" => result.clone(),
                "get_execution_handler" => after.clone(),
                _ => panic!("acquisition must never submit execution"),
            };
            async move {
                Ok(crate::services::mcp_service::tool_response(
                    200,
                    Some("application/json"),
                    response.as_bytes(),
                ))
            }
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        db.collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&w.id})
            .await
            .unwrap()
            .unwrap()
            .artifact_result
            .is_some()
    );
    assert!(!stalled.is_finished());
    stalled.abort();
    assert!(stalled.await.unwrap_err().is_cancelled());
    assert_eq!(byte_budget::available_for_tests(), available);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_legacy_http_keeps_main_headers_and_bytes_and_cross_chat_key_behavior() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let db = crate::test_utils::connect_transaction_test_database("artlegacy2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    let (w, row) = seeded(&db, &keys).await;
    let state = crate::test_utils::test_app_state(db.clone());
    let auth = crate::test_utils::test_auth_user(&w.user_id);
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &w.user_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    crate::services::feature_flag_service::set_platform_override(
        &db,
        crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(w.user_id.clone()),
        true,
        &w.user_id,
    )
    .await
    .unwrap();
    let document = b"Owner document\n".as_slice();
    let mut cases = Vec::new();
    for (mime, bytes) in [
        ("text/plain", document),
        ("image/png", PNG),
        ("image/jpeg", JPEG),
    ] {
        let mut attachment = AssistantAttachment {
            chunks: 0,
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
        assert!(matches!(
            crate::services::assistant_nyxagent::read_attachment_for_caller(
                &db,
                &keys,
                &w.user_id,
                &row.id,
                &id,
                Some("another-chat")
            )
            .await
            .unwrap(),
            crate::services::assistant_nyxagent::AttachmentRead::Legacy(_, _)
        ));
        let response = crate::handlers::assistant_nyxagent::attachment(
            axum::extract::State(state.clone()),
            auth.clone(),
            axum::extract::Path((row.id.clone(), id.clone())),
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
    db.drop().await.unwrap();
}

#[tokio::test]
async fn generated_live_retention_enforces_both_materialization_policy_orderings_in_mongo() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use crate::models::assistant_upload_retention::{Policy, SETTINGS_ID, Settings};
    use crate::services::assistant_upload_retention as retention;
    let db = crate::test_utils::connect_transaction_test_database("artpolicy2037").await;
    let keys = crate::test_utils::test_encryption_keys();
    let old = Policy {
        image_days: 30,
        document_days: 60,
        tool_image_days: Some(1),
        ..Default::default()
    };
    let short = Policy {
        image_days: 2,
        document_days: 3,
        ..old
    };
    for (filename, bytes, days) in [("image.png", PNG, 2_i64), ("video.mp4", MP4, 3_i64)] {
        for shorten_first in [false, true] {
            let settings = Settings {
                id: SETTINGS_ID.into(),
                policy: Some(if shorten_first { short } else { old }),
                ..Default::default()
            };
            let policy_collection =
                db.collection::<Settings>(crate::models::platform_settings::COLLECTION_NAME);
            policy_collection
                .replace_one(doc! {"_id":SETTINGS_ID}, &settings)
                .upsert(true)
                .await
                .unwrap();
            let (w, mut row) = seeded_body(
                &db,
                &keys,
                Utc::now() + Duration::minutes(20),
                &body(filename, bytes),
            )
            .await;
            let budget = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
            let mut session = db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            bind_in_session(&db, &keys, &mut row, &w, &mut session, Some(&budget))
                .await
                .unwrap();
            session.commit_transaction().await.unwrap();
            drop(budget);
            let id = &row.active_turn.as_ref().unwrap().attachments[0].id;
            let saved = db
                .collection::<Document>(ATTACHMENTS)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap();
            let cap = saved.get_datetime("expires_at").unwrap().to_chrono();
            // Materialization must not mutate the policy document as a write fence.
            assert_eq!(
                bson::to_document(
                    &policy_collection
                        .find_one(doc! {"_id":SETTINGS_ID})
                        .await
                        .unwrap()
                        .unwrap()
                )
                .unwrap(),
                bson::to_document(&settings).unwrap()
            );
            if shorten_first {
                assert!((cap - Utc::now()).num_seconds() >= days * 86400 - 5);
                assert!(cap <= Utc::now() + Duration::days(days));
                let extended = Settings {
                    policy: Some(old),
                    ..settings
                };
                policy_collection
                    .replace_one(doc! {"_id":SETTINGS_ID}, extended)
                    .await
                    .unwrap();
                assert_eq!(retention::generated_expiry(&saved, old).unwrap(), cap);
                retention::require_available(&db, doc! {"_id":id,"user_id":&w.user_id})
                    .await
                    .unwrap();
            } else {
                db.collection::<Document>(ATTACHMENTS).update_one(doc!{"_id":id},doc!{"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(10))}}).await.unwrap();
                let shortened = Settings {
                    policy: Some(short),
                    ..settings
                };
                policy_collection
                    .replace_one(doc! {"_id":SETTINGS_ID}, shortened)
                    .await
                    .unwrap();
                assert!(matches!(
                    retention::require_available(&db, doc! {"_id":id,"user_id":&w.user_id}).await,
                    Err(AppError::AssistantAttachmentExpired)
                ));
                assert!(
                    retention::expired_ids(&db, &w.user_id, std::slice::from_ref(id))
                        .await
                        .unwrap()
                        .contains(id)
                );
            }
        }
    }
    db.drop().await.unwrap();
}
