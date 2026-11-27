use super::*;
use crate::services::assistant_skill_authoring::tests::recovery::mixed_authored_row;
use std::sync::Arc;

#[tokio::test]
async fn proposal_storage_authenticates_all_fields_and_fences_the_exact_publication() {
    let db = crate::test_utils::connect_transaction_test_database("proposal_storage").await;
    let entry = REGISTRY
        .iter()
        .find(|e| matches!(e.adapter, Adapter::LearningProposal))
        .unwrap();
    let rows = db.collection::<Document>(entry.collection);
    let mut legacy = crate::test_utils::test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let bundle = Box::pin(mixed_authored_row(Arc::new(legacy))).await;
    let mut source = bundle.get_document("authored").unwrap().clone();
    let id = doc! {"_id": source.get_str("_id").unwrap()};
    let original_bytes = source
        .get_binary_generic("scope_encrypted")
        .unwrap()
        .clone();
    source.insert("scope_encrypted", bson::to_bson(&original_bytes).unwrap());
    source.insert("status", "publishing");
    let p = source.get_document_mut("publication").unwrap();
    p.insert("lease_id", "live-worker");
    p.insert(
        "lease_expires_at",
        bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::minutes(5)),
    );
    rows.insert_one(&source).await.unwrap();
    let keys = crate::test_utils::test_encryption_keys();
    for change in [
        doc! {"revision": 2_i64},
        doc! {"owner_id": "new-owner"},
        doc! {"agent_id": "other-agent"},
        doc! {"scope_digest": "different"},
        doc! {"scope_context_version": Bson::Null},
        doc! {"scope_encrypted": bson::Binary {subtype:bson::spec::BinarySubtype::Generic, bytes:original_bytes.clone()}},
        doc! {"publication.operation_id": "successor"},
        doc! {"publication.attempt": 2_i64},
        doc! {"publication.lease_id": "new-worker"},
        doc! {"publication.lease_expires_at": bson::DateTime::now()},
        doc! {"publication.verified_at": bson::DateTime::now()},
        doc! {"status": "pinned"},
    ] {
        rows.update_one(id.clone(), doc! {"$set": change})
            .await
            .unwrap();
        let winner = rows.find_one(id.clone()).await.unwrap().unwrap();
        assert!(!migrate(entry, &db, &keys, &source).await.unwrap());
        assert!(rows.find_one(id.clone()).await.unwrap().unwrap() == winner);
        rows.replace_one(id.clone(), &source).await.unwrap();
    }
    assert!(verify(entry, &db, &keys, &source).await.is_err());
    assert!(migrate(entry, &db, &keys, &source).await.unwrap());
    let first = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert_registry_fields_preserved(&source, &first);
    super::swaps::assert_registry_swaps(entry, &db, &keys, &first, "owner_id").await;
    let mut config = crate::test_utils::test_app_config();
    config.encryption_key_previous = config.encryption_key.replace("33".repeat(32));
    let rotated = EncryptionKeys::from_config(&config);
    assert!(verify(entry, &db, &rotated, &first).await.is_err());
    assert!(migrate(entry, &db, &rotated, &first).await.unwrap());
    let current = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert_registry_fields_preserved(&first, &current);
    verify(entry, &db, &rotated, &current).await.unwrap();
    assert!(!migrate(entry, &db, &rotated, &current).await.unwrap());
    assert!(verify(entry, &db, &keys, &current).await.is_err());
    for (parent, field, value) in [
        (false, "agent_id", Bson::from("other-agent")),
        (false, "body_context_version", Bson::from(7)),
        (false, "scope_digest", Bson::from("wrong-scope-hash")),
        (false, "body_bytes", Bson::from(0_i64)),
        (true, "operation_id", Bson::from("successor")),
        (true, "sha256", Bson::from("wrong-package-hash")),
        (true, "interface_encrypted", Bson::Null),
    ] {
        let mut broken = current.clone();
        let target = if parent {
            broken.get_document_mut("publication").unwrap()
        } else {
            &mut broken
        };
        target.insert(field, value);
        assert!(
            verify(entry, &db, &rotated, &broken).await.is_err(),
            "{field}"
        );
        assert!(
            migrate(entry, &db, &rotated, &broken).await.is_err(),
            "{field}"
        );
    }
    for sentinel in [
        None,
        Some(bson::bson!([])),
        Some(Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: vec![],
        })),
    ] {
        let mut cleared = source.clone();
        cleared.insert("status", "rejected");
        cleared.insert("body_bytes", 0_i64);
        cleared.remove("body_encrypted");
        if let Some(value) = &sentinel {
            cleared.insert("body_encrypted", value.clone());
        }
        rows.replace_one(id.clone(), &cleared).await.unwrap();
        assert!(migrate(entry, &db, &rotated, &cleared).await.unwrap());
        let mut winner = rows.find_one(id.clone()).await.unwrap().unwrap();
        assert!(winner.get("body_encrypted") == sentinel.as_ref());
        verify(entry, &db, &rotated, &winner).await.unwrap();
        winner.insert("body_context_version", 3_i32);
        assert!(verify(entry, &db, &rotated, &winner).await.is_err());
        winner.remove("body_context_version");
        winner.insert("status", "publishing");
        assert!(verify(entry, &db, &rotated, &winner).await.is_err());
    }
    db.drop().await.unwrap();
}

fn assert_registry_fields_preserved(before: &Document, after: &Document) {
    fn metadata(mut row: Document) -> Document {
        for key in [
            "body_encrypted",
            "body_context_version",
            "scope_encrypted",
            "scope_context_version",
        ] {
            row.remove(key);
        }
        let p = row.get_document_mut("publication").unwrap();
        p.remove("interface_encrypted");
        p.remove("interface_context_version");
        row
    }
    assert!(metadata(before.clone()) == metadata(after.clone()));
}

#[tokio::test]
async fn proposal_generation_resumes_after_source_cas_and_accounts_for_erased_fields() {
    let db = crate::test_utils::connect_transaction_test_database("proposal_generation").await;
    let keys = crate::test_utils::test_encryption_keys();
    let mut legacy = crate::test_utils::test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let bundle = Box::pin(mixed_authored_row(Arc::new(legacy))).await;
    let source = bundle.get_document("authored").unwrap();
    let index = REGISTRY
        .iter()
        .position(|e| matches!(e.adapter, Adapter::LearningProposal))
        .unwrap();
    let entry = &REGISTRY[index];
    let rows = db.collection::<Document>(entry.collection);
    let mut erased = source.clone();
    erased.insert("_id", "erased");
    erased.insert("status", "invalidated");
    erased.insert("body_bytes", 0_i64);
    for key in [
        "body_encrypted",
        "scope_encrypted",
        "scope_digest",
        "publication",
    ] {
        erased.remove(key);
    }
    let mut damaged = source.clone();
    damaged.insert("_id", "damaged");
    damaged.insert("scope_context_version", 259_i32);
    rows.insert_many([source, &erased, &damaged]).await.unwrap();
    let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
    db.collection::<Document>(RUNS)
        .update_one(doc! {"_id": RUN_ID}, doc! {"$set": {"entry": index as i32}})
        .await
        .unwrap();
    assert!(migrate(entry, &db, &keys, source).await.unwrap());
    assert!(advance(&db, &keys, "stale-generation").await.is_err());
    Box::pin(advance(&db, &keys, &generation)).await.unwrap();
    let report = status(&db, &keys).await.unwrap().unwrap();
    assert_eq!(
        (report.scanned, report.rewritten, report.unresolved),
        (3, 0, 1)
    );
    for name in entry.fields {
        let outcome = report
            .field_outcomes
            .iter()
            .find(|f| f.field == *name && f.collection == entry.collection)
            .unwrap();
        assert_eq!(
            (
                outcome.authenticated_current,
                outcome.no_material_observed,
                outcome.verification_blocked
            ),
            (1, 1, 1)
        );
    }
    db.drop().await.unwrap();
}
