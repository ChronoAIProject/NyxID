use super::*;
use crate::{
    models::lark_base_write_photo::{COLLECTION_NAME as PHOTOS, LarkBaseWritePhoto},
    test_utils::connect_transaction_test_database,
};
use chrono::Duration;
use mongodb::bson;

fn make_proposal(hashes: &[&str]) -> (LarkBaseWrite, Vec<String>) {
    let now = Utc::now();
    let hashes: Vec<String> = hashes.iter().map(|hash| (*hash).into()).collect();
    (
        LarkBaseWrite {
            id: Uuid::new_v4().to_string(),
            key: stable_key("owner", "app", "table", &hashes),
            user_id: "owner".into(),
            attempt: Uuid::new_v4().to_string(),
            state: WriteState::Uploading,
            principal: WritePrincipal {
                user_service_id: "connection".into(),
                credential_id: "credential".into(),
                epoch: 1,
                credential_override: None,
            },
            contract_digests: BTreeMap::from([("composite".into(), "contract".into())]),
            upload_ids: vec![Uuid::new_v4().to_string()],
            files: Vec::new(),
            logical_request_sha256: "logical".into(),
            create_body_sha256: None,
            client_token: Uuid::new_v4().to_string(),
            record_id: None,
            last_code: None,
            created_at: now,
            updated_at: now,
            deadline_at: now + Duration::seconds(180),
            expires_at: None,
        },
        hashes
            .iter()
            .map(|hash| stable_key("owner", "app", "table", std::slice::from_ref(hash)))
            .collect(),
    )
}
async fn active(db: &Database, row: &LarkBaseWrite, photos: &[String]) -> LarkBaseWrite {
    match ledger::claim(db, row, photos).await.unwrap() {
        ledger::Claim::Active(row) => *row,
        _ => panic!("expected active"),
    }
}
async fn db(name: &str) -> Database {
    let db = connect_transaction_test_database(name).await;
    crate::db::ensure_lark_base_write_indexes(&db)
        .await
        .unwrap();
    db
}
async fn expire(db: &Database, row: &mut LarkBaseWrite) {
    row.deadline_at = Utc::now() - Duration::seconds(1);
    db.collection::<LarkBaseWrite>(COLLECTION)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"deadline_at":bson::DateTime::from_chrono(row.deadline_at)}},
        )
        .await
        .unwrap();
}

#[test]
fn lark_base_write_models_store_bson_dates_and_redact_all_identifiers_and_tokens() {
    let (mut row, _) = make_proposal(&["photo"]);
    row.files.push(UploadedFile {
        upload_id: "secret-upload".into(),
        content_sha256: "secret-digest".into(),
        file_token: "secret-token".into(),
    });
    row.expires_at = Some(Utc::now() + Duration::days(400));
    let encoded = bson::to_document(&row).unwrap();
    for key in ["created_at", "updated_at", "deadline_at", "expires_at"] {
        assert!(matches!(encoded.get(key), Some(bson::Bson::DateTime(_))))
    }
    let restored: LarkBaseWrite = bson::from_document(encoded).unwrap();
    assert_eq!(restored.attempt, row.attempt);
    assert_eq!(restored.files[0].file_token, "secret-token");
    assert_eq!(Uuid::parse_str(&row.id).unwrap().get_version_num(), 4);
    for value in [
        format!("{row:?}"),
        format!("{:?}", row.principal),
        format!("{:?}", row.files),
    ] {
        assert!(!value.contains("secret"));
        assert!(value.contains("REDACTED"))
    }
    let photo = LarkBaseWritePhoto {
        id: Uuid::new_v4().to_string(),
        photo_key: "secret-photo".into(),
        user_id: "owner".into(),
        write_id: row.id,
        attempt: row.attempt,
        created_at: Utc::now(),
        expires_at: None,
    };
    assert!(!format!("{photo:?}").contains("secret"));
    assert!(matches!(
        bson::to_document(&photo).unwrap().get("created_at"),
        Some(bson::Bson::DateTime(_))
    ));
}

#[test]
fn lark_base_write_logical_digest_ignores_upload_ids_and_object_order_but_preserves_assignments_and_array_order()
 {
    let a = InputUpload {
        id: "a".into(),
        sha256: "hash-a".into(),
        size: 1,
    };
    let b = InputUpload {
        id: "b".into(),
        sha256: "hash-b".into(),
        size: 1,
    };
    let inputs = vec![a, b];
    let assignments = BTreeMap::from([("Receipt".into(), vec!["a".into(), "b".into()])]);
    let hash = validation::logical_digest(
        &json!({"Amount":12.34,"Type":"Meal"}),
        &assignments,
        &inputs,
    )
    .unwrap();
    let renamed = vec![
        InputUpload {
            id: "new-a".into(),
            sha256: "hash-a".into(),
            size: 1,
        },
        InputUpload {
            id: "new-b".into(),
            sha256: "hash-b".into(),
            size: 1,
        },
    ];
    assert_eq!(
        hash,
        validation::logical_digest(
            &json!({"Type":"Meal","Amount":12.34}),
            &BTreeMap::from([("Receipt".into(), vec!["new-a".into(), "new-b".into()])]),
            &renamed
        )
        .unwrap()
    );
    for (fields, assignment) in [
        (json!({"Amount":13.34,"Type":"Meal"}), assignments.clone()),
        (
            json!({"Amount":12.34,"Type":"Meal"}),
            BTreeMap::from([("Other".into(), vec!["a".into(), "b".into()])]),
        ),
        (
            json!({"Amount":12.34,"Type":"Meal"}),
            BTreeMap::from([("Receipt".into(), vec!["b".into(), "a".into()])]),
        ),
    ] {
        assert_ne!(
            hash,
            validation::logical_digest(&fields, &assignment, &inputs).unwrap()
        );
    }
}

#[test]
fn lark_base_write_closed_body_rejects_empty_duplicate_too_many_and_override_inputs() {
    let id = Uuid::new_v4().to_string();
    for body in [
        json!({"fields":{},"attachments":{}}),
        json!({"fields":{},"attachments":{"Receipt":[]}}),
        json!({"fields":{}}),
        json!({"fields":{},"attachments":{"Receipt":[id,id]}}),
        json!({"fields":{},"attachments":{"Receipt":[id]},"allow_duplicate_photo":true}),
        json!({"fields":{},"attachments":{"Receipt":(0..11).map(|_|Uuid::new_v4().to_string()).collect::<Vec<_>>()}}),
    ] {
        assert!(validation::body(&body).is_err(), "{body}");
    }
    assert!(
        validation::body(&json!({"fields":{"Amount":12.34},"attachments":{"Receipt":[id]}}))
            .is_ok()
    );
}

#[test]
fn lark_base_write_schema_refuses_unknown_fields_attachment_bypass_select_creation_and_invalid_numbers()
 {
    let schema = BTreeMap::from([
        ("Receipt".into(), json!({"type":17})),
        (
            "Type".into(),
            json!({"type":3,"property":{"options":[{"name":"Meal"}]}}),
        ),
        (
            "Tags".into(),
            json!({"type":4,"property":{"options":[{"name":"Meal"}]}}),
        ),
        ("Amount".into(), json!({"type":2})),
    ]);
    let attachments = BTreeMap::from([("Receipt".into(), vec!["id".into()])]);
    for fields in [
        json!({"Unknown":1}),
        json!({"Receipt":"token"}),
        json!({"Type":"New"}),
        json!({"Tags":["New"]}),
        json!({"Amount":"12.34"}),
        json!({"Amount":null}),
    ] {
        assert!(
            validation::validate_fields(&fields, &attachments, &schema).is_err(),
            "{fields}"
        );
    }
    assert!(
        validation::validate_fields(
            &json!({"Amount":12.34,"Type":"Meal","Tags":["Meal"]}),
            &attachments,
            &schema
        )
        .is_ok()
    );
    for token in ["", "../x", "a b", "a\r\n", "a%2fb"] {
        assert!(validation::token(token).is_err())
    }
}

#[test]
fn lark_base_write_every_allowlisted_code_is_definite_and_unknown_server_transport_outcomes_are_uncertain()
 {
    for code in [
        1254001, 1254004, 1254045, 1254069, 1254027, 1254303, 1254302,
    ] {
        assert_eq!(
            classify(Reply::Json(200, json!({"code":code}))).0,
            WriteState::FailedDefinite
        );
        assert_eq!(
            classify(Reply::Json(503, json!({"code":code}))).0,
            WriteState::Uncertain
        );
    }
    for code in [1254607, 1254291, 1255006, 1254608, 91403, 777, 0] {
        assert_eq!(
            classify(Reply::Json(200, json!({"code":code}))).0,
            WriteState::Uncertain
        )
    }
    assert_eq!(classify(Reply::Transport).0, WriteState::Uncertain);
    assert_eq!(
        classify(Reply::Json(
            200,
            json!({"code":0,"data":{"record":{"record_id":"record"}}})
        ))
        .0,
        WriteState::Succeeded
    );
}

#[tokio::test]
async fn lark_base_write_succeeded_replay_uses_logical_digest_before_any_provider_or_reservation_mutation()
 {
    let db = db("base_replay").await;
    let (proposal, photos) = make_proposal(&["a"]);
    let mut row = active(&db, &proposal, &photos).await;
    ledger::barrier(&db, &row, &photos, "body").await.unwrap();
    row.state = WriteState::Dispatching;
    ledger::terminal(&db, &row, WriteState::Succeeded, "0", Some("record"))
        .await
        .unwrap();
    let mut again = proposal.clone();
    again.id = Uuid::new_v4().to_string();
    again.attempt = Uuid::new_v4().to_string();
    again.upload_ids = vec![Uuid::new_v4().to_string()];
    assert!(
        matches!(ledger::claim(&db,&again,&photos).await.unwrap(),ledger::Claim::Replay(record) if record=="record")
    );
    again.principal.epoch += 1;
    again.principal.credential_id = "rotated credential".into();
    assert!(
        matches!(ledger::claim(&db,&again,&photos).await.unwrap(),ledger::Claim::Replay(record) if record=="record")
    );
    again
        .contract_digests
        .insert("composite".into(), "new contract".into());
    assert!(
        matches!(ledger::claim(&db,&again,&photos).await.unwrap(),ledger::Claim::Replay(record) if record=="record")
    );
    again.logical_request_sha256 = "changed amount/assignment".into();
    assert!(ledger::claim(&db, &again, &photos).await.is_err());
    let stored = db
        .collection::<LarkBaseWrite>(COLLECTION)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.attempt, row.attempt);
    assert_eq!(stored.state, WriteState::Succeeded);
    assert_eq!(stored.principal, row.principal);
    assert_eq!(stored.contract_digests, row.contract_digests);
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {"write_id":&row.id})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_overlapping_batches_and_uncertain_holders_keep_one_atomic_photo_reservation()
 {
    let db = db("base_overlap").await;
    let (a, pa) = make_proposal(&["a", "b"]);
    let (b, pb) = make_proposal(&["a", "c"]);
    let (left, right) = tokio::join!(ledger::claim(&db, &a, &pa), ledger::claim(&db, &b, &pb));
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let (mut winner, photos) = match left {
        Ok(ledger::Claim::Active(row)) => (row, pa),
        _ => match right.unwrap() {
            ledger::Claim::Active(row) => (row, pb),
            _ => panic!("active"),
        },
    };
    assert_eq!(
        db.collection::<LarkBaseWrite>(COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    ledger::barrier(&db, &winner, &photos, "body")
        .await
        .unwrap();
    winner.state = WriteState::Dispatching;
    ledger::terminal(&db, &winner, WriteState::Uncertain, "transport", None)
        .await
        .unwrap();
    let (overlap, keys) = make_proposal(&["a"]);
    assert!(ledger::claim(&db, &overlap, &keys).await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_photo_holder_deadlines_commit_uncertainty_before_refusal() {
    let db = db("base_holder_deadline").await;
    let (proposal, photos) = make_proposal(&["a"]);
    let mut holder = active(&db, &proposal, &photos).await;
    let (overlap, overlap_photos) = make_proposal(&["a", "b"]);
    for phase in [WriteState::Uploading, WriteState::Dispatching] {
        if phase == WriteState::Dispatching {
            ledger::barrier(&db, &holder, &photos, "body")
                .await
                .unwrap();
            holder.state = phase;
        }
        for (incoming, keys) in [(&proposal, &photos), (&overlap, &overlap_photos)] {
            let Err(AppError::Conflict(message)) = ledger::claim(&db, incoming, keys).await else {
                panic!("live {phase:?} must refuse as in progress");
            };
            assert_eq!(
                message,
                "A Base write for this photo is in progress; try again shortly"
            );
            assert!(!message.contains("outcome unknown"));
        }
    }
    expire(&db, &mut holder).await;
    let ledger::Claim::Active(refused) =
        ledger::claim(&db, &overlap, &overlap_photos).await.unwrap()
    else {
        panic!("expired dispatch must commit its uncertainty before refusal");
    };
    assert_eq!(refused.id, holder.id);
    assert_eq!(refused.attempt, holder.attempt);
    assert_eq!(refused.state, WriteState::Uncertain);
    let stored = db
        .collection::<LarkBaseWrite>(COLLECTION)
        .find_one(doc! {"_id":&holder.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, WriteState::Uncertain);
    assert_eq!(stored.last_code.as_deref(), Some("deadline"));
    assert_eq!(stored.attempt, holder.attempt);
    assert!(stored.expires_at.is_some());
    assert_eq!(
        db.collection::<LarkBaseWrite>(COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        db.collection::<LarkBaseWrite>(COLLECTION)
            .find_one(doc! {"key":&overlap.key})
            .await
            .unwrap()
            .is_none()
    );
    let photo = db
        .collection::<LarkBaseWritePhoto>(PHOTOS)
        .find_one(doc! {"photo_key":&photos[0]})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(photo.write_id, holder.id);
    assert_eq!(photo.attempt, holder.attempt);
    assert!(photo.expires_at.is_some());
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    for (incoming, keys) in [(&proposal, &photos), (&overlap, &overlap_photos)] {
        let Err(AppError::Conflict(message)) = ledger::claim(&db, incoming, keys).await else {
            panic!("uncertainty must refuse subsequent calls");
        };
        assert_eq!(
            message,
            "Base write outcome unknown; check the Base; NyxID will not create it again"
        );
        assert!(!message.contains("in progress"));
    }
    assert!(
        effect_gate(&db, &holder, WriteState::Dispatching)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_failure_release_corrected_retry_and_stale_cleanup_are_attempt_fenced() {
    let db = db("base_retry_race").await;
    let (proposal, photos) = make_proposal(&["a"]);
    let old = active(&db, &proposal, &photos).await;
    assert!(
        ledger::terminal(&db, &old, WriteState::FailedPreDispatch, "upload", None)
            .await
            .unwrap()
    );
    let mut correction = proposal.clone();
    correction.attempt = Uuid::new_v4().to_string();
    correction.logical_request_sha256 = "corrected".into();
    correction.client_token = Uuid::new_v4().to_string();
    let new = active(&db, &correction, &photos).await;
    assert!(
        !ledger::terminal(
            &db,
            &old,
            WriteState::FailedPreDispatch,
            "late cleanup",
            None
        )
        .await
        .unwrap()
    );
    let photo = db
        .collection::<LarkBaseWritePhoto>(PHOTOS)
        .find_one(doc! {"photo_key":&photos[0]})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(photo.attempt, new.attempt);
    assert!(effect_gate(&db, &old, WriteState::Uploading).await.is_err());
    assert!(effect_gate(&db, &new, WriteState::Uploading).await.is_ok());
    let (overlap, keys) = make_proposal(&["a", "b"]);
    assert!(ledger::claim(&db, &overlap, &keys).await.is_err());
    let mut dispatched = new.clone();
    ledger::barrier(&db, &new, &photos, "body").await.unwrap();
    dispatched.state = WriteState::Dispatching;
    assert!(
        ledger::terminal(
            &db,
            &dispatched,
            WriteState::FailedDefinite,
            "1254001",
            None
        )
        .await
        .unwrap()
    );
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    correction.attempt = Uuid::new_v4().to_string();
    correction.client_token = Uuid::new_v4().to_string();
    let next = active(&db, &correction, &photos).await;
    assert_ne!(next.client_token, new.client_token);
    assert!(effect_gate(&db, &next, WriteState::Uploading).await.is_ok());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_stale_upload_takeover_reuses_only_same_principal_tokens_and_fences_old_worker()
 {
    let db = db("base_takeover").await;
    let (proposal, photos) = make_proposal(&["a"]);
    let mut old = active(&db, &proposal, &photos).await;
    ledger::save_file(
        &db,
        &old,
        &UploadedFile {
            upload_id: "old".into(),
            content_sha256: "a".into(),
            file_token: "token".into(),
        },
    )
    .await
    .unwrap();
    expire(&db, &mut old).await;
    let mut next = proposal.clone();
    next.attempt = Uuid::new_v4().to_string();
    let new = active(&db, &next, &photos).await;
    assert_ne!(old.attempt, new.attempt);
    assert_eq!(new.files.len(), 1);
    assert!(ledger::barrier(&db, &old, &photos, "body").await.is_err());
    assert!(
        !ledger::terminal(&db, &old, WriteState::FailedPreDispatch, "stale", None)
            .await
            .unwrap()
    );
    ledger::terminal(&db, &new, WriteState::FailedPreDispatch, "failed", None)
        .await
        .unwrap();
    next.attempt = Uuid::new_v4().to_string();
    next.principal.epoch += 1;
    let changed = active(&db, &next, &photos).await;
    assert!(changed.files.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_dispatch_barrier_crash_replay_and_changed_authority_never_reactivate() {
    let db = db("base_barrier").await;
    let (proposal, photos) = make_proposal(&["a"]);
    let mut row = active(&db, &proposal, &photos).await;
    for change in [0, 1] {
        let mut changed = proposal.clone();
        if change == 0 {
            changed.principal.epoch += 1
        } else {
            changed
                .contract_digests
                .insert("composite".into(), "new".into());
        }
        assert!(ledger::claim(&db, &changed, &photos).await.is_err());
    }
    ledger::barrier(&db, &row, &photos, "body").await.unwrap();
    row.state = WriteState::Dispatching;
    expire(&db, &mut row).await;
    assert!(
        matches!(ledger::claim(&db,&proposal,&photos).await.unwrap(),ledger::Claim::Active(row) if row.state==WriteState::Uncertain)
    );
    assert!(ledger::claim(&db, &proposal, &photos).await.is_err());
    assert!(
        effect_gate(&db, &row, WriteState::Dispatching)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_rollback_rehearsal_flags_off_sweeps_without_tool_calls_and_query_reaches_zero()
 {
    let db = db("base_rollback").await;
    for phase in [WriteState::Uploading, WriteState::Dispatching] {
        for i in 0..101 {
            let (mut row, photos) = make_proposal(&[&format!("{phase:?}-{i}")]);
            row.deadline_at = Utc::now() + Duration::seconds(180);
            let mut row = active(&db, &row, &photos).await;
            if phase == WriteState::Dispatching {
                ledger::barrier(&db, &row, &photos, "body").await.unwrap();
                row.state = phase;
            }
            expire(&db, &mut row).await;
        }
    }
    assert_eq!(
        db.collection::<bson::Document>("feature_flag_overrides")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(sweep(&db).await.unwrap(), 100);
    assert_eq!(sweep(&db).await.unwrap(), 100);
    assert_eq!(sweep(&db).await.unwrap(), 2);
    assert_eq!(
        db.collection::<LarkBaseWrite>(COLLECTION)
            .count_documents(doc! {"state":{"$in":["uploading","dispatching"]}})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<LarkBaseWrite>(COLLECTION)
            .count_documents(doc! {"state":"uncertain"})
            .await
            .unwrap(),
        101
    );
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        101
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_retention_keeps_active_and_400_day_terminal_metadata_then_removes_parent_and_photos_together()
 {
    let db = db("base_retention").await;
    let (proposal, photos) = make_proposal(&["a"]);
    let mut row = active(&db, &proposal, &photos).await;
    db.collection::<LarkBaseWrite>(COLLECTION)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(1))}},
        )
        .await
        .unwrap();
    sweep(&db).await.unwrap();
    assert!(effect_gate(&db, &row, WriteState::Uploading).await.is_ok());
    ledger::barrier(&db, &row, &photos, "body").await.unwrap();
    row.state = WriteState::Dispatching;
    ledger::terminal(&db, &row, WriteState::Succeeded, "0", Some("record"))
        .await
        .unwrap();
    let stored = db
        .collection::<LarkBaseWrite>(COLLECTION)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.expires_at.unwrap() > Utc::now() + Duration::days(399));
    sweep(&db).await.unwrap();
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.collection::<LarkBaseWrite>(COLLECTION).update_one(doc! {"_id":&row.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    sweep(&db).await.unwrap();
    assert_eq!(
        db.collection::<LarkBaseWrite>(COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_failure_release_races_corrected_retry_and_overlapping_batch_without_losing_new_reservations()
 {
    let db = db("base_release_overlap_race").await;
    for i in 0..8 {
        let hash = format!("a-{i}");
        let extra = format!("b-{i}");
        let (proposal, photos) = make_proposal(&[&hash]);
        let old = active(&db, &proposal, &photos).await;
        let mut correction = proposal.clone();
        correction.attempt = Uuid::new_v4().to_string();
        let (overlap, keys) = make_proposal(&[&hash, &extra]);
        let (released, retry, overlapping) = tokio::join!(
            ledger::terminal(&db, &old, WriteState::FailedPreDispatch, "upload", None),
            ledger::claim(&db, &correction, &photos),
            ledger::claim(&db, &overlap, &keys)
        );
        assert!(released.unwrap());
        let mut active_rows = Vec::new();
        if let Ok(ledger::Claim::Active(row)) = retry {
            active_rows.push((row, photos.clone()));
        }
        if let Ok(ledger::Claim::Active(row)) = overlapping {
            active_rows.push((row, keys.clone()));
        }
        assert!(active_rows.len() <= 1);
        if active_rows.is_empty() {
            // Both contenders can observe the old uploading attempt. A fresh
            // authorized call must reacquire every reservation after release.
            active_rows.push((Box::new(active(&db, &correction, &photos).await), photos));
        }
        let (winner, keys) = active_rows.pop().unwrap();
        assert!(
            !ledger::terminal(&db, &old, WriteState::FailedPreDispatch, "stale", None)
                .await
                .unwrap()
        );
        assert_eq!(
            db.collection::<LarkBaseWritePhoto>(PHOTOS)
                .count_documents(doc! {"write_id":&winner.id,"attempt":&winner.attempt})
                .await
                .unwrap(),
            keys.len() as u64
        );
        ledger::barrier(&db, &winner, &keys, "body").await.unwrap();
        assert!(effect_gate(&db, &old, WriteState::Uploading).await.is_err());
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_erasure_fence_rejects_held_claim_after_source_removal() {
    let db = db("base_erasure_fence").await;
    let (row, photos) = make_proposal(&["photo"]);
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    crate::services::erasure_admission::close_in_session(&db, &row.user_id, &mut tx)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    assert!(matches!(
        ledger::claim(&db, &row, &photos).await,
        Err(AppError::Conflict(_))
    ));
    for collection in [COLLECTION, PHOTOS] {
        assert_eq!(
            db.collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    db.drop().await.unwrap();
}
