use super::*;
use crate::services::platform_entry_gateway_service::tests::setup::SetupFixture;

const FIELD: &str = "request_ciphertext";

// Unmarked v2 fixtures from the server tests at 846b09d4d.
const LEGACY_REQUEST: &str = concat!(
    "0202003c9ec4598db980c435d7ae52a59bb44a6d20e5cf2dd57d922a37fb4e4818873cbc41433296",
    "0cad4f50ef08ffad8a16c00345b602815b771afdde6d9aeede89789342851cd49e72b870a1501b46",
    "e6633796ea9654b35c99818b858b66dc2ceb374126c41d48f67848bcf1a9ad95bd3e29ff0899a617",
    "a103c046a20b3a991c9c55916fda1365a91980",
);

#[test]
fn gateway_transport_authenticates_exact_bytes_scope_and_the_rewrap_winner() {
    crate::test_utils::run_native_journey_test("gateway_transport_storage", || async {
        let f = SetupFixture::new(2).await;
        assert!(f.convert(None).await.is_err());
        let db = &f.state().db;
        let index = REGISTRY
            .iter()
            .position(|e| matches!(e.adapter, Adapter::GatewayTransport))
            .unwrap();
        let entry = &REGISTRY[index];
        let rows = db.collection::<Document>(entry.collection);
        let prepared = rows.find_one(doc! {}).await.unwrap().unwrap();
        let id = prepared.get_str("_id").unwrap();
        let owner = prepared.get_str("credential_owner_id").unwrap();
        let by_id = doc! {"_id": id};
        let mut config = crate::test_utils::test_app_config();
        let old = EncryptionKeys::from_config(&config);
        config.encryption_key_previous = config.encryption_key.replace("22".repeat(32));
        let new = EncryptionKeys::from_config(&config);

        // Re-serializing this JSON would change its hash and drop the duplicate member.
        let bytes = "{ \"name\":\"α\", \"name\":\"β\", \"credentials\":{} }\n".as_bytes();
        use sha2::{Digest, Sha256};
        let hash = format!("sha256:{:x}", Sha256::digest(bytes));
        let legacy = hex::decode(LEGACY_REQUEST).unwrap();
        let mut source = prepared.clone();
        source.remove("secret_context_versions");
        source.insert(FIELD, bson::to_bson(&legacy).unwrap());
        source.insert("request_hash", hash);
        source.insert("future_sibling", 42);
        let binary = |bytes| bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes,
        };
        for delta in [
            doc! {FIELD: binary(legacy.clone())},
            doc! {"revision": 8_i64},
            doc! {"state": "retired"},
            doc! {"setup_attempt_id": "replacement"},
            doc! {"setup_dispatch_deadline": Bson::Null},
            doc! {"request_hash": "new-hash"},
            doc! {"request_policy": {}},
            doc! {"provider_api_key_id": "replacement"},
            doc! {"route_api_key_id": "replacement"},
            doc! {"credential_owner_id": "successor"},
            doc! {"secret_context_versions": {}},
        ] {
            let mut winner = source.clone();
            winner.extend(delta);
            rows.replace_one(by_id.clone(), &winner).await.unwrap();
            assert!(!migrate(entry, db, &old, &source).await.unwrap());
            assert!(rows.find_one(by_id.clone()).await.unwrap().unwrap() == winner);
        }
        rows.replace_one(by_id.clone(), &source).await.unwrap();
        assert!(migrate(entry, db, &old, &source).await.unwrap());
        let previous = rows.find_one(by_id.clone()).await.unwrap().unwrap();
        assert!(verify(entry, db, &new, &previous).await.is_err());
        assert!(migrate(entry, db, &new, &previous).await.unwrap());
        let current = rows.find_one(by_id.clone()).await.unwrap().unwrap();
        super::swaps::assert_registry_swaps(entry, db, &new, &current, "credential_owner_id").await;
        assert!(!migrate(entry, db, &new, &current).await.unwrap());
        assert!(migrate(entry, db, &old, &current).await.is_err());
        let mut metadata = current.clone();
        metadata.remove("secret_context_versions");
        metadata.insert(FIELD, source.get(FIELD).unwrap());
        assert!(metadata == source);
        for (collection, field) in [
            (entry.collection, "other_field"),
            ("assistant_cma_creations", FIELD),
        ] {
            let ciphertext =
                crate::crypto::record_field::encrypt(&new, collection, id, owner, field, bytes)
                    .await
                    .unwrap();
            let mut swapped = current.clone();
            swapped.insert(FIELD, binary(ciphertext));
            assert!(verify(entry, db, &new, &swapped).await.is_err());
            assert!(migrate(entry, db, &new, &swapped).await.is_err());
        }
        let mut damaged = current.get_binary_generic(FIELD).unwrap().clone();
        *damaged.last_mut().unwrap() ^= 1;
        for delta in [
            doc! {"request_hash": "different"},
            doc! {FIELD: binary(damaged)},
            doc! {FIELD: binary(vec![])},
            doc! {FIELD: "malformed"},
            doc! {FIELD: Bson::Null},
            doc! {"secret_context_versions": {FIELD: 4}},
            doc! {"secret_context_versions": {"other": 3, FIELD: 3}},
            doc! {"secret_context_versions": {}},
        ] {
            let mut bad = current.clone();
            bad.extend(delta);
            assert!(verify(entry, db, &new, &bad).await.is_err());
            assert!(migrate(entry, db, &new, &bad).await.is_err());
        }

        // A restart after the successful CAS must verify the winner. Required
        // material missing on a physically retained retired row stays blocked.
        let mut missing = source;
        missing.remove(FIELD);
        missing.insert("_id", "retired-missing");
        missing.insert("profile_id", "retired-profile");
        missing.insert("provider_api_key_id", "retired-provider-key");
        missing.insert("route_api_key_id", "retired-route-key");
        missing.insert("state", "retired");
        rows.insert_one(missing).await.unwrap();
        let generation = start(db, &new, Mode::Migrate, None).await.unwrap();
        db.collection::<Document>(RUNS)
            .update_one(doc! {"_id": RUN_ID}, doc! {"$set": {"entry": index as i32}})
            .await
            .unwrap();
        assert!(advance(db, &new, "superseded").await.is_err());
        advance(db, &new, &generation).await.unwrap();
        let report = status(db, &new).await.unwrap().unwrap();
        assert_eq!(
            (report.scanned, report.rewritten, report.unresolved),
            (2, 0, 1)
        );
        let count = report
            .field_outcomes
            .iter()
            .find(|c| c.collection == entry.collection && c.field == FIELD)
            .unwrap();
        assert_eq!(
            (count.authenticated_current, count.verification_blocked),
            (1, 1)
        );
        f.close().await;
    });
}
