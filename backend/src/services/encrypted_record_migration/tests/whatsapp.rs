use super::*;
use crate::services::whatsapp_native::token_record::FIELD;

// Unmarked v2 fixtures from the server tests at 846b09d4d.
const LEGACY_RETAINED_TOKEN: &str = concat!(
    "0202003cd244504493555dd17af7cf9146522af16e2a9c140ec9be7ba4345b02aa2b389d809442e6",
    "d1e5877d060a354bad547529d1775e42c1b7a257619722fcc80e5607714ca352d568a15d9ab6a004",
    "c4ce2b83805a1806ab5c98e4ab4532192d8ecdd0b42a343a890391072c6b83c4a21ef9",
);

const LEGACY_PRESENT_TOKEN: &str = concat!(
    "0202003ced5fa5ebe4c3701f8fe567f18fc6d353ba153e107e706b253b5f469b9e6e3aa102d1a5f2",
    "a6ac400ef3b820f8a004c4b829624daee5fe323eba65f3f81263ab8cbfc66fbab9689a43cc8a4000",
    "d7c50e3dc83be9de31c9c2a30354fb20e684f34ba730efe5b1ba16f4bd052cd9bcac6428",
);

fn retained(id: &str) -> Document {
    doc! {"_id":id,"owner_id":"owner","ownership_version":2_i64,
    "meta_app_id":"111","waba_id":"222","phone_number_id":"333","dedicated_app":true,
    "revision":5,"callback":{"phase":"gateway"},"updated_at":bson::DateTime::now(),"future":17}
}

#[tokio::test]
async fn retained_whatsapp_rewrap_fences_raw_source_and_checks_primary_and_scope() {
    let db = crate::test_utils::connect_transaction_test_database("wa_retained_rewrap").await;
    let entry = REGISTRY
        .iter()
        .find(|e| matches!(e.adapter, Adapter::NativeWhatsapp))
        .unwrap();
    let rows = db.collection::<Document>(entry.collection);
    let mut settings = crate::test_utils::test_app_config();
    let old = EncryptionKeys::from_config(&settings);
    settings.encryption_key_previous = settings.encryption_key.replace("22".repeat(32));
    let new = EncryptionKeys::from_config(&settings);
    let mut source = retained("retained");
    let token = b"fixture-wa-verify-token";
    let legacy = hex::decode(LEGACY_RETAINED_TOKEN).unwrap();
    source.insert(FIELD, bson::to_bson(&legacy).unwrap());
    let id = doc! {"_id":"retained"};
    rows.insert_one(&source).await.unwrap();
    let binary = |bytes| bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    };
    for delta in [
        doc! {FIELD:binary(legacy.clone())},
        doc! {"revision":6},
        doc! {"callback.phase":"relay"},
        doc! {"ownership_version":3},
        doc! {"secret_context_versions":{}},
        doc! {"owner_id":"moved"},
    ] {
        rows.update_one(id.clone(), doc! {"$set":delta})
            .await
            .unwrap();
        let winner = rows.find_one(id.clone()).await.unwrap().unwrap();
        assert!(!migrate(entry, &db, &old, &source).await.unwrap());
        assert!(rows.find_one(id.clone()).await.unwrap().unwrap() == winner);
        rows.replace_one(id.clone(), &source).await.unwrap();
    }
    assert!(migrate(entry, &db, &old, &source).await.unwrap());
    let previous = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert!(verify(entry, &db, &new, &previous).await.is_err());
    assert!(migrate(entry, &db, &new, &previous).await.unwrap());
    let current = rows.find_one(id.clone()).await.unwrap().unwrap();
    super::swaps::assert_registry_swaps(entry, &db, &new, &current, "owner_id").await;
    assert!(!migrate(entry, &db, &new, &current).await.unwrap());
    assert!(migrate(entry, &db, &old, &current).await.is_err());
    let mut metadata = current.clone();
    metadata.remove("secret_context_versions");
    metadata.insert(FIELD, source.get(FIELD).unwrap());
    assert!(metadata == source);
    for (collection, field) in [(entry.collection, "other_field"), ("channel_bots", FIELD)] {
        let path = [field, "2"];
        let ctx =
            crate::crypto::record_field::RecordContext::new(collection, "retained", "owner", &path)
                .unwrap();
        let mut swapped = current.clone();
        swapped.insert(
            FIELD,
            binary(new.encrypt_record(token, &ctx).await.unwrap()),
        );
        assert!(verify(entry, &db, &new, &swapped).await.is_err());
        assert!(migrate(entry, &db, &new, &swapped).await.is_err());
    }
    let mut damaged = current.get_binary_generic(FIELD).unwrap().clone();
    *damaged.last_mut().unwrap() ^= 1;
    for delta in [
        doc! {"ownership_version":3},
        doc! {FIELD:binary(damaged)},
        doc! {FIELD:binary(vec![])},
        doc! {FIELD:"malformed"},
        doc! {FIELD:binary(vec![3,78,89,88])},
        doc! {FIELD:Bson::Null},
        doc! {"secret_context_versions":{FIELD:4}},
        doc! {"secret_context_versions":{}},
    ] {
        let mut bad = current.clone();
        bad.extend(delta);
        assert!(verify(entry, &db, &new, &bad).await.is_err());
        assert!(migrate(entry, &db, &new, &bad).await.is_err());
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn whatsapp_generation_reports_retained_absent_null_and_damaged_tokens_after_resume() {
    let db = crate::test_utils::connect_transaction_test_database("wa_generation").await;
    let keys = crate::test_utils::test_encryption_keys();
    let index = REGISTRY
        .iter()
        .position(|e| matches!(e.adapter, Adapter::NativeWhatsapp))
        .unwrap();
    let entry = &REGISTRY[index];
    let mut present = retained("present");
    present.insert(
        FIELD,
        bson::to_bson(&hex::decode(LEGACY_PRESENT_TOKEN).unwrap()).unwrap(),
    );
    let mut null = retained("null");
    null.insert(FIELD, Bson::Null);
    let mut broken = retained("broken");
    broken.insert(FIELD, "bad");
    db.collection::<Document>(entry.collection)
        .insert_many([&present, &null, &retained("absent"), &broken])
        .await
        .unwrap();
    let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
    db.collection::<Document>(RUNS)
        .update_one(doc! {"_id":RUN_ID}, doc! {"$set":{"entry":index as i32}})
        .await
        .unwrap();
    // The source was committed and the worker exited before its checkpoint.
    assert!(migrate(entry, &db, &keys, &present).await.unwrap());
    assert!(advance(&db, &keys, "stale-generation").await.is_err());
    advance(&db, &keys, &generation).await.unwrap();
    let report = status(&db, &keys).await.unwrap().unwrap();
    assert_eq!(
        (report.scanned, report.rewritten, report.unresolved),
        (4, 0, 1)
    );
    let outcome = report
        .field_outcomes
        .iter()
        .find(|r| r.collection == entry.collection && r.field == FIELD)
        .unwrap();
    assert_eq!(
        (
            outcome.authenticated_current,
            outcome.no_material_observed,
            outcome.verification_blocked
        ),
        (1, 2, 1)
    );
    db.drop().await.unwrap();
}
