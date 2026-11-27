use super::*;
use crate::{
    crypto::{record_field::RecordContext, token::hash_token},
    models::{api_key, nyxbot_channel::NyxbotChannel},
    services::nyxbot_channel_key::{FIELD, open},
};
const SECRET: &str = "managed-channel-fixture-key";

// Unmarked v2 fixtures from the server tests at 846b09d4d.
const LEGACY_CHANNEL_KEY: &str = concat!(
    "0202003c970328ef44ef1c4967bba9b3c53e0df81371da3bc135ee1b72132673d3ea00640565942f",
    "0ed1cc507ba1ee15ab4ad3aaf608d982e0053c0bca841adb7ddb7da49d3d9ed46a8d6d4f926f4567",
    "bf6d70a14c34a7b8d3ce8b457d70b939cbd191b493735cddaf3f9d3fec6c8481578b0ee0f213ee",
);

fn channel(id: &str) -> Document {
    doc! {"_id":id,"user_id":"person","bot_owner_id":"org","channel_bot_id":"bot",
    "platform":"lark","bot_label":"fixture","route_api_key_id":"org-route",
    "transport":"gateway","status":"active","agent_api_key_id":"installed",
    "updated_at":bson::DateTime::now(),"created_at":bson::DateTime::now()}
}

async fn companion(db: &Database) {
    // Retired keys keep their owner and digest for storage verification.
    db.collection::<Document>(api_key::COLLECTION_NAME)
        .insert_one(doc! {"_id":"installed",
        "user_id":"person","key_hash":hash_token(SECRET),"is_active":false})
        .await
        .unwrap();
}

fn binary(bytes: Vec<u8>) -> Bson {
    bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    }
    .into()
}

#[tokio::test]
async fn channel_key_rewrap_authenticates_the_installed_personal_key_and_exact_release_snapshot() {
    let db = crate::test_utils::connect_transaction_test_database("channel_key_cas").await;
    companion(&db).await;
    let entry = REGISTRY
        .iter()
        .find(|e| matches!(e.adapter, Adapter::NyxbotChannelKey))
        .unwrap();
    let rows = db.collection::<Document>(entry.collection);
    let mut settings = crate::test_utils::test_app_config();
    let old = EncryptionKeys::from_config(&settings);
    settings.encryption_key_previous = settings.encryption_key.replace("22".repeat(32));
    let current_keys = EncryptionKeys::from_config(&settings);
    let mut source = channel("channel");
    let legacy = hex::decode(LEGACY_CHANNEL_KEY).unwrap();
    source.insert(FIELD, bson::to_bson(&legacy).unwrap());
    source.insert("unknown_sibling", 7);
    rows.insert_one(&source).await.unwrap();
    let id = doc! {"_id":"channel"};
    for delta in [
        doc! {FIELD:binary(legacy)},
        doc! {"gateway_setup_release_id":"claim"},
        doc! {"pending_agent_api_key_id":"next"},
        doc! {"pending_route_api_key_id":"next-route"},
        doc! {"binding_id":Bson::Null},
        doc! {"agent_api_key_id":"next"},
        doc! {"user_id":"org"},
        doc! {"secret_context_versions":{}},
        doc! {"status":"disconnected",FIELD:Bson::Null},
    ] {
        rows.update_one(id.clone(), doc! {"$set":delta})
            .await
            .unwrap();
        let winner = rows.find_one(id.clone()).await.unwrap().unwrap();
        assert!(!migrate(entry, &db, &old, &source).await.unwrap());
        assert!(rows.find_one(id.clone()).await.unwrap().unwrap() == winner);
        rows.replace_one(id.clone(), &source).await.unwrap();
    }
    let mut staged = crate::test_utils::test_encryption_keys();
    staged.set_contextual_record_writes(false);
    let wrong_key = crate::services::nyxbot_channel_key::seal(
        &staged,
        "channel",
        "person",
        "installed",
        "different-fixture-key",
    )
    .await
    .unwrap();
    for delta in [
        doc! {"agent_api_key_id":"absent","pending_agent_api_key_id":"installed"},
        doc! {"user_id":"org"},
        doc! {FIELD:binary(wrong_key)},
    ] {
        let mut wrong = source.clone();
        wrong.extend(delta);
        assert!(migrate(entry, &db, &old, &wrong).await.is_err());
    }
    assert!(migrate(entry, &db, &old, &source).await.unwrap());
    let previous = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert!(verify(entry, &db, &current_keys, &previous).await.is_err());
    assert!(migrate(entry, &db, &current_keys, &previous).await.unwrap());
    let current = rows.find_one(id.clone()).await.unwrap().unwrap();
    super::swaps::assert_registry_swaps(entry, &db, &current_keys, &current, "user_id").await;
    assert!(!migrate(entry, &db, &current_keys, &current).await.unwrap());
    assert!(migrate(entry, &db, &old, &current).await.is_err());
    let mut metadata = current.clone();
    metadata.remove("secret_context_versions");
    metadata.insert(FIELD, source.get(FIELD).unwrap());
    assert!(metadata == source);
    let mut corrupted = current.get_binary_generic(FIELD).unwrap().clone();
    *corrupted.last_mut().unwrap() ^= 1;
    for delta in [
        doc! {"agent_api_key_id":"org-route","pending_agent_api_key_id":"installed"},
        doc! {FIELD:binary(corrupted)},
        doc! {FIELD:binary(vec![3,78,89,88])},
        doc! {FIELD:binary(vec![])},
        doc! {FIELD:Bson::Null},
        doc! {"secret_context_versions":{FIELD:4}},
        doc! {FIELD:"invalid-type"},
        doc! {"secret_context_versions":{}},
    ] {
        let mut damaged = current.clone();
        damaged.extend(delta);
        assert!(verify(entry, &db, &current_keys, &damaged).await.is_err());
        assert!(migrate(entry, &db, &current_keys, &damaged).await.is_err());
    }
    for (collection, field) in [(entry.collection, "route_key"), ("channel_bots", FIELD)] {
        let path = [field, "installed"];
        let ctx = RecordContext::new(collection, "channel", "person", &path).unwrap();
        let mut swapped = current.clone();
        swapped.insert(
            FIELD,
            binary(
                current_keys
                    .encrypt_record(SECRET.as_bytes(), &ctx)
                    .await
                    .unwrap(),
            ),
        );
        let row: NyxbotChannel = bson::from_document(swapped.clone()).unwrap();
        assert!(open(&current_keys, &row).await.is_err());
        assert!(migrate(entry, &db, &current_keys, &swapped).await.is_err());
    }
}

#[tokio::test]
async fn channel_key_generation_counts_adopted_staged_retired_and_missing_keys_after_restart() {
    let db = crate::test_utils::connect_transaction_test_database("channel_key_generation").await;
    let keys = crate::test_utils::test_encryption_keys();
    companion(&db).await;
    let index = REGISTRY
        .iter()
        .position(|e| matches!(e.adapter, Adapter::NyxbotChannelKey))
        .unwrap();
    let entry = &REGISTRY[index];
    let rows = db.collection::<Document>(entry.collection);
    let mut adopted = channel("adopted");
    adopted.insert(FIELD, binary(hex::decode(LEGACY_CHANNEL_KEY).unwrap()));
    let mut staged = channel("staged");
    staged.remove("agent_api_key_id");
    staged.extend(doc! {"transport":"direct","pending_agent_api_key_id":"next"});
    let mut retired = channel("retired");
    retired.extend(doc! {"status":"disconnected",FIELD:Bson::Null});
    let missing = channel("missing");
    rows.insert_many([&adopted, &staged, &retired, &missing])
        .await
        .unwrap();
    let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
    db.collection::<Document>(RUNS)
        .update_one(doc! {"_id":RUN_ID}, doc! {"$set":{"entry":index as i32}})
        .await
        .unwrap();
    assert!(migrate(entry, &db, &keys, &adopted).await.unwrap());
    assert!(advance(&db, &keys, "stale").await.is_err());
    advance(&db, &keys, &generation).await.unwrap();
    let report = status(&db, &keys).await.unwrap().unwrap();
    assert_eq!(
        (report.scanned, report.rewritten, report.unresolved),
        (4, 0, 1)
    );
    let f = report
        .field_outcomes
        .iter()
        .find(|f| f.collection == entry.collection && f.field == FIELD)
        .unwrap();
    assert_eq!(
        (
            f.authenticated_current,
            f.no_material_observed,
            f.verification_blocked
        ),
        (1, 2, 1)
    );
}
