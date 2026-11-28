//! The mixed-binary driver exchanges raw BSON from the serving writers and the
//! one-day cleanup. The writer switch controls the actual stored envelopes.
use super::*;
use crate::crypto::aes::EncryptionKeys;
use crate::services::security_readiness_service as readiness;
use proposals::record::{self, Field};
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn arm<'a>(row: &'a Document, slot: &str) -> &'a Document {
    row.get_document("execution_proposal")
        .unwrap()
        .get_document(slot)
        .unwrap()
}

async fn snapshot(f: &Fixture, id: &str) -> Document {
    f.state
        .db
        .collection::<Document>("approval_requests")
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

async fn capture(
    f: &Fixture,
    id: &str,
    slot: &str,
    field: Field,
    contextual: bool,
) -> (Document, String) {
    let row = snapshot(f, id).await;
    let marker = match field {
        Field::Payload => "payload_context_version",
        Field::Result => "result_context_version",
    };
    if contextual {
        assert_eq!(arm(&row, slot).get_i32(marker), Ok(3));
    } else {
        assert!(!arm(&row, slot).contains_key(marker));
    }
    let request: ApprovalRequest = bson::from_document(row.clone()).unwrap();
    let plain = field
        .open(&f.state.encryption_keys, &request)
        .await
        .unwrap()
        .unwrap();
    (row, hex::encode(Sha256::digest(&plain[..])))
}

async fn lifecycle(version: u32, writes: bool) -> Document {
    use super::super::proposal_v2_tests as external;
    let (mut f, chat) = match version {
        2 => {
            let f = Fixture::new("approval_record_external").await;
            f.enroll(true).await;
            (f, None)
        }
        4 => {
            let (f, turn, _) =
                Box::pin(channel::channel_turn("approval_record_channel", false)).await;
            (f, Some(turn))
        }
        _ => {
            let (f, turn) = Box::pin(assistant("approval_record_assistant")).await;
            if version == 1 {
                let sibling = readiness::overlapping_test_replica(&f.state.db).await;
                readiness::remove_test_replica_capability(
                    &f.state.db,
                    &sibling,
                    proposals::v3::READINESS,
                )
                .await;
            }
            (f, Some(turn))
        }
    };
    let mut keys = test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(writes);
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    f.state.encryption_keys = Arc::new(keys);
    if version == 2 {
        external::call(&f, "/proxy/s/execution-fixture/status", "record-fixture").await;
    } else {
        write_call(&f).await;
    }
    let requests = f
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests");
    let row = requests
        .find_one(doc! {"execution_proposal.version":i64::from(version)})
        .await
        .unwrap()
        .unwrap();
    let id = &row.id;
    let slot = format!("v{version}");
    let (pending, payload_hash) = capture(&f, id, &slot, Field::Payload, writes).await;
    let decision = if version == 2 {
        external::choice(&row, "operation")
    } else {
        decision(&row, "operation")
    };
    let approved = proposals::decide(&f.state, &f.owner, id, &decision)
        .await
        .unwrap();
    if let Some(turn) = chat {
        finish(&f, &turn, "Review in NyxID").await;
        let resumed = event_turn(&f, &turn).await;
        assert!(Box::pin(resume(&f, &resumed)).await.contains("HTTP 200"));
    } else {
        Box::pin(external::run(&f, &approved)).await;
    }
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    let (completed, _) = capture(&f, id, &slot, Field::Result, writes).await;
    assert!(!arm(&completed, &slot).contains_key("payload_context_version"));
    for key in [
        "gap_key",
        "prepared_digest",
        "authority_digest",
        "contract",
        "choices",
    ] {
        assert_eq!(
            arm(&pending, &slot).get(key).unwrap(),
            arm(&completed, &slot).get(key).unwrap(),
            "{key}"
        );
    }
    assert_eq!(
        arm(&completed, &slot)
            .get_str("selected_choice_digest")
            .unwrap(),
        decision.choice_digest.unwrap()
    );

    requests
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{
                "created_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(2)),
                format!("execution_proposal.{slot}.delivered_turn_id"):"finished"
            }},
        )
        .await
        .unwrap();
    Box::pin(proposals::sweep(&f.state)).await.unwrap();
    let cleared = snapshot(&f, id).await;
    for key in [
        "payload",
        "payload_context_version",
        "result",
        "result_context_version",
    ] {
        assert!(!arm(&cleared, &slot).contains_key(key), "{slot}.{key}");
    }
    f.cleanup().await;
    doc! {"pending":pending,"payload_hash":payload_hash,"completed":completed,"cleared":cleared}
}

async fn check_rows(state: &crate::AppState, bundle: &Document, reject: bool) {
    for slot in ["v1", "v2", "v3", "v4"] {
        let rows = bundle.get_document(slot).unwrap();
        let request: ApprovalRequest =
            bson::from_document(rows.get_document("pending").unwrap().clone()).unwrap();
        let payload = Field::Payload.open(&state.encryption_keys, &request).await;
        let completed =
            bson::from_document(rows.get_document("completed").unwrap().clone()).unwrap();
        let result = proposals::result_for_human(state, &completed).await;
        if reject {
            assert!(payload.is_err() && result.is_err(), "{slot}");
        } else {
            assert_eq!(
                hex::encode(Sha256::digest(&payload.unwrap().unwrap()[..])),
                rows.get_str("payload_hash").unwrap()
            );
            assert!(result.unwrap().unwrap().contains("HTTP 200"), "{slot}");
        }
        let cleared = bson::from_document(rows.get_document("cleared").unwrap().clone()).unwrap();
        assert!(
            proposals::result_for_human(state, &cleared)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn approval_proposal_records() {
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let writes =
        step == "roundtrip" || std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    let bundle = match step.as_str() {
        "write" | "roundtrip" => {
            let mut bundle = Document::new();
            for version in 1..=4 {
                bundle.insert(
                    format!("v{version}"),
                    Box::pin(lifecycle(version, writes)).await,
                );
            }
            if let Some(path) = file {
                std::fs::write(path, bson::to_vec(&bundle).unwrap()).unwrap();
            }
            bundle
        }
        "migrate" => {
            let path = file.unwrap();
            let mut bundle = bson::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            Box::pin(migrate_retained_rows(&mut bundle)).await;
            std::fs::write(path, bson::to_vec(&bundle).unwrap()).unwrap();
            bundle
        }
        "read" | "reject" => bson::from_slice(&std::fs::read(file.unwrap()).unwrap()).unwrap(),
        _ => panic!("unknown approval fixture step"),
    };
    let db = test_utils::connect_transaction_test_database("approval_record_read").await;
    let mut state = test_utils::test_app_state(db.clone());
    let mut keys: EncryptionKeys = test_utils::test_encryption_keys();
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    state.encryption_keys = Arc::new(keys);
    Box::pin(check_rows(&state, &bundle, step == "reject")).await;
    db.drop().await.unwrap();
}

// A process can lose its lease after committing a source update and before
// checkpointing it. Resume must authenticate the installed winner.
async fn migrate_retained_rows(bundle: &mut Document) {
    use crate::services::encrypted_record_migration as migration;
    let keys = test_utils::test_encryption_keys();
    for phase in ["pending", "completed", "cleared"] {
        let db = test_utils::connect_transaction_test_database("approval_record_migrate").await;
        let rows = db.collection::<Document>("approval_requests");
        let generation = migration::start(&db, &keys, migration::Mode::Migrate, None)
            .await
            .unwrap();
        for slot in ["v1", "v2", "v3", "v4"] {
            let source = bundle
                .get_document(slot)
                .unwrap()
                .get_document(phase)
                .unwrap();
            rows.insert_one(source).await.unwrap();
            if slot == "v1" && phase != "cleared" {
                assert!(record::migrate(&db, &keys, source).await.unwrap());
            }
        }
        let report = loop {
            let r = migration::status(&db, &keys).await.unwrap().unwrap();
            if r.phase == "observed" {
                break r;
            }
            Box::pin(migration::advance(&db, &keys, &generation))
                .await
                .unwrap();
        };
        assert_eq!(report.scanned, 4);
        assert_eq!(report.unresolved, 0);
        assert_eq!(report.rewritten, if phase == "cleared" { 0 } else { 3 });
        let outcomes: Vec<_> = report
            .field_outcomes
            .iter()
            .filter(|f| f.collection == "approval_requests")
            .collect();
        assert_eq!(outcomes.len(), 8);
        for f in outcomes {
            let present = match phase {
                "pending" => f.field.ends_with(".payload"),
                "completed" => f.field.ends_with(".result"),
                _ => false,
            };
            assert_eq!(
                (
                    f.authenticated_current,
                    f.no_material_observed,
                    f.verification_blocked
                ),
                (i64::from(present), 4 - i64::from(present), 0)
            );
        }
        for slot in ["v1", "v2", "v3", "v4"] {
            let snapshots = bundle.get_document_mut(slot).unwrap();
            let source = snapshots.get_document(phase).unwrap();
            let winner = rows
                .find_one(doc! {"_id": source.get_str("_id").unwrap()})
                .await
                .unwrap()
                .unwrap();
            // Compare the entire BSON document outside the two encrypted pairs.
            let mut expected = source.clone();
            let dest = expected
                .get_document_mut("execution_proposal")
                .unwrap()
                .get_document_mut(slot)
                .unwrap();
            for field in ["payload", "result"] {
                if dest.get_binary_generic(field).is_ok() {
                    dest.insert(field, arm(&winner, slot).get(field).unwrap().clone());
                    dest.insert(format!("{field}_context_version"), 3_i32);
                }
            }
            assert!(expected == winner);
            snapshots.insert(phase, winner);
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn approval_proposal_field_outcomes_require_one_well_formed_arm() {
    use crate::services::encrypted_record_migration as migration;
    let db = test_utils::connect_transaction_test_database("approval_field_outcomes").await;
    let keys = test_utils::test_encryption_keys();
    let snapshots = Box::pin(lifecycle(2, false)).await;
    let empty = snapshots.get_document("cleared").unwrap();
    let mut documents = vec![
        doc! {"_id":"ordinary"},
        doc! {"_id":"ordinary-null","execution_proposal":bson::Bson::Null},
        empty.clone(),
    ];
    let mut wrong = |envelope: bson::Bson| {
        documents.push(doc! {"_id":Uuid::new_v4().to_string(),"execution_proposal":envelope});
    };
    wrong(bson::bson!([]));
    wrong(bson::bson!({"version":9}));
    wrong(bson::bson!({"version":2,"v2":null}));
    wrong(bson::bson!({"version":2,"v2":{}}));
    let original = empty.get_document("execution_proposal").unwrap();
    for (key, value) in [
        ("v1", bson::bson!({})),
        ("v3", bson::bson!({"result":"malformed"})),
        ("v4", bson::bson!([])),
        ("v5", bson::bson!({})),
        ("version", bson::bson!(3)),
    ] {
        let mut e = original.clone();
        e.insert(key, value);
        wrong(e.into());
    }
    for (key, value) in [
        ("result_context_version", bson::bson!(3)),
        ("payload_context_version", bson::bson!(259)),
        ("result", bson::bson!("")),
        ("payload", bson::bson!([])),
    ] {
        let mut e = original.clone();
        e.get_document_mut("v2").unwrap().insert(key, value);
        wrong(e.into());
    }
    // Two fully decodable arms also conflict, including an inactive arm with retained bytes.
    let assistant = Box::pin(lifecycle(3, false)).await;
    let pending = assistant.get_document("pending").unwrap();
    let mut competing = original.clone();
    competing.insert("v3", arm(pending, "v3").clone());
    let mut ambiguous = empty.clone();
    ambiguous.insert("execution_proposal", competing);
    ambiguous.insert("_id", "conflicting");
    let typed: ApprovalRequest = bson::from_document(ambiguous.clone()).unwrap();
    assert!(proposals::v2::proposal(&typed).is_err());
    documents.push(ambiguous);
    documents.push(snapshots.get_document("pending").unwrap().clone());
    documents.last_mut().unwrap().insert("_id", "pending");
    let blocked = (documents.len() - 3) as i64;
    db.collection::<Document>("approval_requests")
        .insert_many(documents)
        .await
        .unwrap();
    let generation = migration::start(&db, &keys, migration::Mode::Verify, None)
        .await
        .unwrap();
    loop {
        let report = migration::status(&db, &keys).await.unwrap().unwrap();
        if report.phase == "observed" {
            let fields: Vec<_> = report
                .field_outcomes
                .iter()
                .filter(|f| f.collection == "approval_requests")
                .collect();
            assert_eq!(fields.len(), 8);
            assert_eq!(report.unresolved, blocked);
            for field in fields {
                assert_eq!(
                    (
                        field.no_material_observed,
                        field.verification_blocked,
                        field.authenticated_current
                    ),
                    (3, blocked, 0),
                    "{}",
                    field.field
                );
            }
            break;
        }
        Box::pin(migration::advance(&db, &keys, &generation))
            .await
            .unwrap();
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn approval_proposal_context_binds_user_request_arm_and_field() {
    let bundle = Box::pin(lifecycle(3, true)).await;
    let keys = test_utils::test_encryption_keys();
    for (phase, field) in [("pending", Field::Payload), ("completed", Field::Result)] {
        let raw = bundle.get_document(phase).unwrap();
        let row: ApprovalRequest = bson::from_document(raw.clone()).unwrap();
        assert!(field.open(&keys, &row).await.unwrap().is_some());
        let mut other = row.clone();
        other.user_id = "another-user".into();
        assert!(field.open(&keys, &other).await.is_err());
        other = row.clone();
        other.id = "another-request".into();
        assert!(field.open(&keys, &other).await.is_err());
        for version in [1, 4] {
            let mut other = row.clone();
            let e = other.execution_proposal.as_mut().unwrap();
            e.version = version;
            if version == 1 {
                e.v1 = e.v3.take();
            } else {
                e.v4 = e.v3.take();
            }
            assert!(field.open(&keys, &other).await.is_err());
        }
        let mut other = row.clone();
        let arm = other
            .execution_proposal
            .as_mut()
            .unwrap()
            .v3
            .as_mut()
            .unwrap();
        std::mem::swap(&mut arm.result, &mut arm.payload);
        std::mem::swap(
            &mut arm.result_context_version,
            &mut arm.payload_context_version,
        );
        let wrong = match field {
            Field::Payload => Field::Result,
            Field::Result => Field::Payload,
        };
        assert!(wrong.open(&keys, &other).await.is_err());
        let mut missing_marker = row;
        let arm = missing_marker
            .execution_proposal
            .as_mut()
            .unwrap()
            .v3
            .as_mut()
            .unwrap();
        arm.payload_context_version = None;
        arm.result_context_version = None;
        assert!(field.open(&keys, &missing_marker).await.is_err());
    }
}

async fn parked_rewrap(
    db: &mongodb::Database,
    source: Document,
) -> (
    tokio::task::JoinHandle<bool>,
    Arc<crate::services::record_field_service::tests::HoldFirstWrap>,
) {
    let (keys, pause) = crate::services::record_field_service::tests::pause_first_wrap();
    let db = db.clone();
    let worker = tokio::spawn(async move { record::migrate(&db, &keys, &source).await.unwrap() });
    pause.entered.notified().await;
    (worker, pause)
}

#[tokio::test]
async fn approval_proposal_rewrap_races_dispatch_and_retention_without_another_effect() {
    use super::super::proposal_v2_tests as external;
    for dispatch_first in [false, true] {
        let mut f = Fixture::new("approval_record_dispatch").await;
        f.enroll(true).await;
        let mut legacy = test_utils::test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        f.state.encryption_keys = Arc::new(legacy);
        external::call(&f, "/proxy/s/execution-fixture/status", "race").await;
        let requests = f
            .state
            .db
            .collection::<ApprovalRequest>("approval_requests");
        let requested = requests.find_one(doc! {}).await.unwrap().unwrap();
        let id = &requested.id;
        let choice = external::choice(&requested, "operation");
        let approved = proposals::decide(&f.state, &f.owner, id, &choice)
            .await
            .unwrap();

        let claim = if dispatch_first {
            None
        } else {
            Some(
                proposals::v2::claim_for_test(&f.state, &approved)
                    .await
                    .unwrap()
                    .unwrap(),
            )
        };
        let source = snapshot(&f, id).await;
        let (worker, pause) = parked_rewrap(&f.state.db, source).await;
        if dispatch_first {
            Box::pin(external::run(&f, &approved)).await;
        }
        pause.release.notify_one();
        assert_eq!(worker.await.unwrap(), !dispatch_first);
        if let Some((resume, adapter)) = claim {
            // Cross the real send barrier with the claim admitted before rewrap.
            let mut auth = f.auth().await;
            proposals::v2::attach_resume(&mut auth, resume.clone()).unwrap();
            auth.execution_permission
                .as_ref()
                .unwrap()
                .ingress
                .set(adapter.clone())
                .ok()
                .unwrap();
            let response = Box::pin(crate::handlers::execution_permission_resume::send(
                &f.state,
                &auth,
                resume.effect.clone(),
                adapter,
                resume.independent_card_id.clone(),
            ))
            .await
            .unwrap();
            let response =
                proposals::v2::complete_response_for_test(&f.state, &approved, &resume, response)
                    .await
                    .unwrap()
                    .unwrap();
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
        }
        // Two deliveries retain the same pre-rewrap approval and race its live send fence.
        tokio::join!(
            Box::pin(external::run(&f, &approved)),
            Box::pin(external::run(&f, &approved))
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        let completed = requests.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_eq!(
            proposals::v2::proposal(&completed).unwrap().state,
            "completed"
        );

        requests
            .update_one(
                doc! {"_id":id},
                doc! {"$set":{
                    "created_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(2)),
                    "execution_proposal.v2.delivered_turn_id":"finished"
                }},
            )
            .await
            .unwrap();
        let (worker, pause) = parked_rewrap(&f.state.db, snapshot(&f, id).await).await;
        Box::pin(proposals::sweep(&f.state)).await.unwrap();
        pause.release.notify_one();
        assert!(!worker.await.unwrap());
        let cleared = snapshot(&f, id).await;
        for key in [
            "payload",
            "payload_context_version",
            "result",
            "result_context_version",
        ] {
            assert!(!arm(&cleared, "v2").contains_key(key));
        }
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        f.cleanup().await;
    }
}

#[tokio::test]
async fn approval_proposal_rotation_verifies_both_fields_before_either_is_written() {
    let db = test_utils::connect_transaction_test_database("approval_record_rotation").await;
    let keys = test_utils::test_encryption_keys();
    let mut config = test_utils::test_app_config();
    config.encryption_key_previous = config.encryption_key.replace("33".repeat(32));
    let rotated = EncryptionKeys::from_config(&config);
    let rows = db.collection::<Document>("approval_requests");
    let bundle = Box::pin(lifecycle(4, true)).await;
    let mut both = bundle.get_document("pending").unwrap().clone();
    let result = arm(bundle.get_document("completed").unwrap(), "v4");
    let selected = both
        .get_document_mut("execution_proposal")
        .unwrap()
        .get_document_mut("v4")
        .unwrap();
    for key in ["result", "result_context_version"] {
        selected.insert(key, result.get(key).unwrap().clone());
    }
    let mut broken = both.clone();
    broken
        .get_document_mut("execution_proposal")
        .unwrap()
        .get_document_mut("v4")
        .unwrap()
        .insert("result_context_version", 7);
    rows.insert_one(&broken).await.unwrap();
    let id = doc! {"_id":both.get_str("_id").unwrap()};
    assert!(record::migrate(&db, &rotated, &broken).await.is_err());
    assert!(rows.find_one(id.clone()).await.unwrap().unwrap() == broken);
    rows.replace_one(id.clone(), &both).await.unwrap();
    record::verify(&keys, &both).await.unwrap();
    assert!(record::verify(&rotated, &both).await.is_err());
    assert!(record::migrate(&db, &rotated, &both).await.unwrap());
    let winner = rows.find_one(id).await.unwrap().unwrap();
    record::verify(&rotated, &winner).await.unwrap();
    assert!(!record::migrate(&db, &rotated, &winner).await.unwrap());
    assert!(record::verify(&keys, &winner).await.is_err());
    db.drop().await.unwrap();
}
