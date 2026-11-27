use super::*;
use crate::models::assistant_conversation::{COLLECTION_NAME, ReattachDeployment};
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_encryption_keys};
use bson::Document;
use std::sync::Arc;

fn window(row: &AssistantConversation) -> &ReattachWindow {
    row.active_turn
        .as_ref()
        .unwrap()
        .restart_admission
        .as_ref()
        .unwrap()
        .window
        .as_ref()
        .unwrap()
}

async fn admitted(state: &AppState) -> AssistantConversation {
    let request: engine::TurnRequest =
        serde_json::from_value(json!({"text":"window fixture"})).unwrap();
    let mut start = engine::TurnStart::from(&request);
    start.owner_instruction_source = Some(engine::OwnerInstructionSource::FirstPartyOwner);
    let mut row = engine::begin_turn(&state.db, "window-owner", &request, &state.encryption_keys)
        .await
        .unwrap();
    let deployment: ReattachDeployment = serde_json::from_value(json!({"destination_hash":"test-route","limits":{
        "protocol":"nyxagent-reattach-v1","recent_events":32,"max_events":64,"max_event_bytes":65536,
        "retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32
    }})).unwrap();
    engine::enroll_restart(&mut row, &start, "input", deployment, false);
    state
        .db
        .collection::<AssistantConversation>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    row
}

async fn install(state: &AppState, row: &AssistantConversation, key: &str) -> Document {
    let binding = InstructionBinding {
        session_id: "session".into(),
        fingerprint: "fingerprint".into(),
        marker: Some("test-marker".into()),
        guest: false,
    };
    let encrypted = seal(
        state,
        row,
        key,
        &json!({"input":["fixture",key]}),
        "prefix",
        &binding,
        None,
    )
    .await
    .unwrap();
    crate::services::assistant_turn_observability::attempt(
        &state.db,
        row,
        key,
        Utc::now() + chrono::Duration::minutes(5),
        Some(encrypted),
        None,
    )
    .await
    .unwrap();
    snapshot(state, &row.id).await
}

async fn snapshot(state: &AppState, id: &str) -> Document {
    state
        .db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

async fn read(state: &AppState, source: &Document, reject: bool) {
    let row: AssistantConversation = bson::from_document(source.clone()).unwrap();
    if row.active_turn.is_none() {
        assert!(!reject);
        return;
    }
    let w = window(&row);
    let got = Input::open(state, &row, w).await;
    if reject {
        assert!(got.is_err());
    } else {
        let input = got.unwrap();
        assert_eq!(input.body, json!({"input":["fixture",w.key]}));
        assert_eq!(input.prefix, "prefix");
        assert!(input.binding.marker.as_deref() == Some("test-marker"));
    }
}

#[tokio::test]
async fn conversation_reattach_window() {
    let db = connect_transaction_test_database("reattach_window_binary").await;
    let mut state = test_app_state(db.clone());
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "write".into());
    let path = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let mut keys = test_encryption_keys();
    let writes = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() != Ok("false");
    keys.set_contextual_record_writes(writes);
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    state.encryption_keys = Arc::new(keys);
    let rows = db.collection::<Document>(COLLECTION_NAME);
    let source = if step == "write" {
        let row = Box::pin(admitted(&state)).await;
        Box::pin(install(&state, &row, "window:λ/%2F")).await
    } else {
        let source: Document =
            bson::from_slice(&std::fs::read(path.as_ref().unwrap()).unwrap()).unwrap();
        rows.insert_one(&source).await.unwrap();
        let row: AssistantConversation = bson::from_document(source.clone()).unwrap();
        match step.as_str() {
            "replace" => {
                rows.update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"active_turn.restart_admission.window.terminal":true}},
                )
                .await
                .unwrap();
                Box::pin(install(&state, &row, &format!("{}-next", window(&row).key))).await
            }
            "clear" => {
                settle(&state, &row).await;
                let stored = snapshot(&state, &row.id).await;
                assert!(stored.get_document("active_turn").is_err());
                stored
            }
            "migrate" => {
                migrate_fixture(&state, &source).await;
                snapshot(&state, &row.id).await
            }
            "read" | "reject" => source,
            _ => panic!("unknown fixture step"),
        }
    };
    read(&state, &source, step == "reject").await;
    if matches!(step.as_str(), "write" | "replace") {
        let stored = source
            .get_document("active_turn")
            .unwrap()
            .get_document("restart_admission")
            .unwrap()
            .get_document("window")
            .unwrap();
        assert_eq!(
            stored.get_i32("input_context_version").ok(),
            writes.then_some(3)
        );
        assert!(stored.get_binary_generic("input").is_ok());
        if !writes {
            assert!(!stored.contains_key("input_context_version"));
        }
    }
    if let Some(path) = path
        && !matches!(step.as_str(), "read" | "reject")
    {
        std::fs::write(path, bson::to_vec(&source).unwrap()).unwrap();
    }
    db.drop().await.unwrap();
}

async fn settle(state: &AppState, row: &AssistantConversation) {
    Box::pin(engine::finish_turn(
        &state.db,
        row,
        &row.credential_api_key_id,
        "reply",
        &TurnResult {
            text: "settled".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    ))
    .await
    .unwrap();
}

async fn migrate_fixture(state: &AppState, source: &Document) {
    assert!(
        record::migrate(&state.db, &state.encryption_keys, source)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn reattach_window_storage_loses_to_worker_restart_rotation_stop_and_settlement() {
    use crate::services::{
        assistant_turn_restart as restart, record_field_service::tests::pause_first_wrap,
    };
    for change in ["worker", "window", "rotation", "stop", "settlement"] {
        let db = connect_transaction_test_database("window_storage_race").await;
        let mut state = test_app_state(db.clone());
        let mut legacy = test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        state.encryption_keys = Arc::new(legacy);
        let row = Box::pin(admitted(&state)).await;
        let source = Box::pin(install(&state, &row, "original")).await;
        let row: AssistantConversation = bson::from_document(source.clone()).unwrap();
        let (keys, hold) = pause_first_wrap();
        let worker = tokio::spawn({
            let db = db.clone();
            async move { record::migrate(&db, &keys, &source).await.unwrap() }
        });
        hold.entered.notified().await;
        match change {
            "worker" => {
                let from = &row
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .worker_instance
                    .as_ref()
                    .unwrap()
                    .id;
                assert!(
                    restart::offer(&db, &row, restart::Loss::Shutdown(from))
                        .await
                        .unwrap()
                );
                assert!(restart::yield_on_shutdown(&db, &row, from).await.unwrap());
                let parked = bson::from_document(snapshot(&state, &row.id).await).unwrap();
                let survivor = crate::models::assistant_conversation::EngineWorkerInstance {
                    id: Uuid::new_v4().to_string(),
                    hostname: None,
                };
                let claimed = restart::claim(&db, &parked, &survivor)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    Input::open(&state, &claimed, window(&claimed))
                        .await
                        .is_ok()
                );
                assert!(
                    !engine::heartbeat(&db, &engine::WorkerFence::capture(&row).unwrap())
                        .await
                        .unwrap()
                );
            }
            "window" => {
                db.collection::<Document>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$set":{"active_turn.restart_admission.window.terminal":true}},
                    )
                    .await
                    .unwrap();
                let next = Box::pin(install(&state, &row, "continued")).await;
                read(&state, &next, false).await;
            }
            "rotation" => {
                crate::services::key_service::rotate_api_key(
                    &db,
                    &state.encryption_keys,
                    &row.user_id,
                    &row.credential_api_key_id,
                )
                .await
                .unwrap();
                let rotated: AssistantConversation =
                    bson::from_document(snapshot(&state, &row.id).await).unwrap();
                assert!(
                    rotated
                        .active_turn
                        .as_ref()
                        .unwrap()
                        .restart_admission
                        .as_ref()
                        .unwrap()
                        .credential_ended
                );
            }
            "stop" => {
                engine::request_stop(&db, &row.user_id, &row.id)
                    .await
                    .unwrap();
            }
            _ => {
                settle(&state, &row).await;
            }
        }
        let winner = snapshot(&state, &row.id).await;
        hold.release.notify_one();
        assert!(!worker.await.unwrap(), "{change}");
        assert!(snapshot(&state, &row.id).await == winner, "{change}");
        let keys = test_encryption_keys();
        if matches!(change, "stop" | "settlement") {
            let row: AssistantConversation = bson::from_document(winner.clone()).unwrap();
            assert!(row.active_turn.is_none(), "{change}");
            record::verify(&keys, &winner).await.unwrap();
            assert!(
                !record::migrate(&db, &keys, &winner).await.unwrap(),
                "{change}"
            );
            assert!(snapshot(&state, &row.id).await == winner, "{change}");
        } else {
            assert!(
                record::migrate(&db, &keys, &winner).await.unwrap(),
                "{change}"
            );
            let current = snapshot(&state, &row.id).await;
            record::verify(&keys, &current).await.unwrap();
            let mut expected = winner;
            let stored: AssistantConversation = bson::from_document(current.clone()).unwrap();
            let pair = window_doc_mut(&mut expected);
            pair.insert("input", window(&stored).input.clone().unwrap());
            pair.insert("input_context_version", 3_i32);
            assert!(current == expected, "{change}");
        }
        db.drop().await.unwrap();
    }
}

fn window_doc_mut(source: &mut Document) -> &mut Document {
    source
        .get_document_mut("active_turn")
        .unwrap()
        .get_document_mut("restart_admission")
        .unwrap()
        .get_document_mut("window")
        .unwrap()
}

#[tokio::test]
async fn reattach_window_authenticates_each_identity_and_the_decoded_input() {
    use crate::crypto::{aes::EncryptionKeys, record_field::RecordContext};
    let db = connect_transaction_test_database("reattach_binding").await;
    let state = test_app_state(db.clone());
    let row = Box::pin(admitted(&state)).await;
    let material = Box::pin(install(&state, &row, "fresh")).await;
    let keys = &state.encryption_keys;
    let row: AssistantConversation = bson::from_document(material.clone()).unwrap();
    let w = window(&row);
    let input = record::Identity::of(&row, &w.key)
        .open(keys, w)
        .await
        .unwrap();
    let mut config = crate::test_utils::test_app_config();
    config.encryption_key_previous = config.encryption_key.replace("33".repeat(32));
    let rotated = EncryptionKeys::from_config(&config);
    assert!(record::verify(&rotated, &material).await.is_err());
    assert!(record::migrate(&db, &rotated, &material).await.unwrap());
    let current = snapshot(&state, &row.id).await;
    record::verify(&rotated, &current).await.unwrap();
    assert!(!record::migrate(&db, &rotated, &current).await.unwrap());
    assert!(record::verify(keys, &current).await.is_err());

    for dimension in [
        "owner",
        "id",
        "credential",
        "key",
        "field",
        "collection",
        "shape",
        "damaged",
        "marker",
        "no marker",
    ] {
        let mut changed = material.clone();
        match dimension {
            "owner" => {
                changed.insert("user_id", "other-owner");
            }
            "id" => {
                changed.insert("_id", "same-owner-other-row");
            }
            "credential" => {
                changed.insert("credential_api_key_id", "other-key");
                changed
                    .get_document_mut("active_turn")
                    .unwrap()
                    .get_document_mut("restart_admission")
                    .unwrap()
                    .insert("credential_id", "other-key");
            }
            "key" => {
                window_doc_mut(&mut changed).insert("key", "different-window");
            }
            "field" | "collection" | "shape" => {
                let field = if dimension == "field" {
                    "different"
                } else {
                    "input"
                };
                let collection = if dimension == "collection" {
                    "other_collection"
                } else {
                    COLLECTION_NAME
                };
                let path = [
                    "active_turn",
                    "restart_admission",
                    "window",
                    field,
                    &row.credential_api_key_id,
                    &w.key,
                ];
                let ctx = RecordContext::new(collection, &row.id, &row.user_id, &path).unwrap();
                let bytes = if dimension == "shape" {
                    b"{}".as_slice()
                } else {
                    input.as_slice()
                };
                let blob = keys.encrypt_record(bytes, &ctx).await.unwrap();
                window_doc_mut(&mut changed).insert(
                    "input",
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: blob,
                    },
                );
            }
            "damaged" => {
                let mut b = w.input.clone().unwrap();
                *b.bytes.last_mut().unwrap() ^= 1;
                window_doc_mut(&mut changed).insert("input", b);
            }
            "marker" => {
                window_doc_mut(&mut changed).insert("input_context_version", 7);
            }
            _ => {
                window_doc_mut(&mut changed).remove("input_context_version");
            }
        }
        read(&state, &changed, true).await;
        assert!(record::verify(keys, &changed).await.is_err(), "{dimension}");
        assert!(
            record::migrate(&db, keys, &changed).await.is_err(),
            "{dimension}"
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn reattach_window_generation_resumes_from_the_installed_input() {
    use crate::services::encrypted_record_migration as migration;
    let db = connect_transaction_test_database("reattach_generation").await;
    let mut state = test_app_state(db.clone());
    let mut legacy = test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    state.encryption_keys = Arc::new(legacy);
    let first = Box::pin(admitted(&state)).await;
    let source = Box::pin(install(&state, &first, "first")).await;
    let second = Box::pin(admitted(&state)).await;
    Box::pin(install(&state, &second, "second")).await;
    let rows = db.collection::<Document>(COLLECTION_NAME);
    rows.insert_one(doc! {"_id":"settled","active_turn":null})
        .await
        .unwrap();
    let mut orphan = source.clone();
    orphan.insert("_id", "orphan-marker");
    let w = window_doc_mut(&mut orphan);
    w.remove("input");
    w.insert("input_context_version", 3);
    rows.insert_one(orphan).await.unwrap();
    rows.insert_one(doc! {"_id":"malformed","active_turn":{"restart_admission":{"window":[]}}})
        .await
        .unwrap();
    let keys = test_encryption_keys();
    let generation = migration::start(&db, &keys, migration::Mode::Migrate, None)
        .await
        .unwrap();
    assert!(record::migrate(&db, &keys, &source).await.unwrap());
    assert!(
        Box::pin(migration::advance(&db, &keys, "stale-generation"))
            .await
            .is_err()
    );
    loop {
        let report = migration::status(&db, &keys).await.unwrap().unwrap();
        if report.phase == "observed" {
            let outcome = report
                .field_outcomes
                .iter()
                .find(|f| f.collection == COLLECTION_NAME)
                .unwrap();
            assert_eq!(
                (
                    outcome.authenticated_current,
                    outcome.no_material_observed,
                    outcome.verification_blocked
                ),
                (2, 1, 2)
            );
            record::verify(&keys, &snapshot(&state, &second.id).await)
                .await
                .unwrap();
            break;
        }
        Box::pin(migration::advance(&db, &keys, &generation))
            .await
            .unwrap();
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn restarted_window_replaces_input_and_marker_together_in_both_directions() {
    let db = connect_transaction_test_database("restart_window_marker").await;
    let mut state = test_app_state(db.clone());
    let row = Box::pin(admitted(&state)).await;
    let initial = Box::pin(install(&state, &row, "first")).await;
    let mut previous: String = window(&bson::from_document(initial).unwrap()).key.clone();
    for (next, writes) in [("rolled-back", false), ("activated", true)] {
        let mut keys = test_encryption_keys();
        keys.set_contextual_record_writes(writes);
        state.encryption_keys = Arc::new(keys);
        let old: AssistantConversation =
            bson::from_document(snapshot(&state, &row.id).await).unwrap();
        let saved = Input::open(&state, &old, window(&old)).await.unwrap();
        let input = seal(
            &state,
            &old,
            next,
            &saved.body,
            &saved.prefix,
            &saved.binding,
            saved.follow_up,
        )
        .await
        .unwrap();
        let changed = db
            .collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id,"active_turn.restart_admission.window.key":&previous},
                restarted_window(next, input),
            )
            .await
            .unwrap();
        assert_eq!(changed.matched_count, 1);
        let mut winner = snapshot(&state, &row.id).await;
        let raw = window_doc_mut(&mut winner);
        assert_eq!(raw.contains_key("input_context_version"), writes);
        let stored: AssistantConversation = bson::from_document(winner).unwrap();
        let reopened = Input::open(&state, &stored, window(&stored)).await.unwrap();
        assert_eq!(reopened.body, saved.body);
        previous = next.into();
    }
    db.drop().await.unwrap();
}
