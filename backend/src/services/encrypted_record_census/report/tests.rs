use super::*;
use crate::services::encrypted_record_census::tests::with_history;
use crate::test_utils::{connect_transaction_test_database, test_app_config, test_encryption_keys};
use serde_json::Value;

async fn finish(db: &Database, keys: &EncryptionKeys, config: &AppConfig, id: &str) -> Status {
    for _ in 0..256 {
        let next = advance(db, keys, config, id).await.unwrap();
        if next.complete {
            return next;
        }
    }
    panic!("report did not publish")
}

async fn list(db: &Database, keys: &EncryptionKeys, id: &str, kind: &str) -> Vec<Value> {
    let mut after = None;
    let mut values = vec![];
    loop {
        let query = Query { after };
        let page = if kind == "families" {
            families(db, keys, id, query).await
        } else {
            findings(db, keys, id, query).await
        }
        .unwrap();
        values.extend(page.items);
        after = page.next;
        if after.is_none() {
            return values;
        }
    }
}

#[tokio::test]
async fn writers_off_zero_families_and_downgrade_namespace() {
    let db = connect_transaction_test_database("census_empty").await;
    let mut keys = test_encryption_keys();
    keys.set_contextual_record_writes(false);
    let config = test_app_config();
    with_history(&db, 120, async {
        let id = start(&db, &keys, &config).await.unwrap().generation;
        assert!(
            crate::services::encrypted_record_migration::advance(&db, &keys, &id)
                .await
                .is_err()
        );
        let result = finish(&db, &keys, &config, &id).await;
        let families = list(&db, &keys, &id, "families").await;
        assert_eq!(families.len(), observation::families().count());
        assert!(
            families.iter().all(|f| f["totals"]["rows_examined"] == 0
                && f["buckets"].as_array().unwrap().is_empty())
        );
        assert!(list(&db, &keys, &id, "findings").await.is_empty());
        assert_eq!(result.totals.unwrap().rows_examined, 0);
        assert!(!keys.contextual_record_writes());
        let state = serde_json::to_value(status(&db, &keys, &id).await.unwrap()).unwrap();
        assert_eq!(state["certification_enabled"], false);
        assert!(
            state["blockers"]
                .as_array()
                .unwrap()
                .contains(&"isolated_load_gate_unmet".into())
        );
    })
    .await;
    db.drop().await.unwrap();
}

async fn mixed_row(id: &str, current: &EncryptionKeys, previous: &EncryptionKeys) -> Document {
    let mut row = doc! {"_id":id,"user_id":"private-owner-value","secret_context_version":3};
    for (field, writer) in [
        ("username_encrypted", current),
        ("password_encrypted", previous),
    ] {
        let bytes = crate::crypto::record_field::encrypt(
            writer,
            "saved_logins",
            id,
            "private-owner-value",
            field,
            b"private-cleartext-value",
        )
        .await
        .unwrap();
        row.insert(
            field,
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes,
            },
        );
    }
    row
}

#[tokio::test]
async fn mixed_keys_replace_at_cut_and_partial_sibling_is_counted_and_redacted() {
    let db = connect_transaction_test_database("census_mixed").await;
    let mut config = test_app_config();
    let mut old = config.clone();
    old.encryption_key = Some("5a".repeat(32));
    config.encryption_key_previous = old.encryption_key.clone();
    let keys = EncryptionKeys::from_config(&config);
    let previous = EncryptionKeys::from_config(&old);
    let coll = db.collection::<Document>("saved_logins");
    let one = mixed_row("sensitive-mixed-id", &keys, &previous).await;
    let mut two = mixed_row("sensitive-corrupt-id", &keys, &previous).await;
    two.insert("password_encrypted", "private-malformed-value");
    coll.insert_one(one).await.unwrap();
    with_history(&db, 120, async {
        let id = start(&db, &keys, &config).await.unwrap().generation;
        coll.insert_one(two).await.unwrap();
        let result = finish(&db, &keys, &config, &id).await;
        let totals = result.totals.unwrap();
        assert_eq!(
            (totals.rows_examined, totals.mixed, totals.unresolved),
            (2, 1, 1)
        );
        let all = list(&db, &keys, &id, "families").await;
        let report = all
            .iter()
            .find(|f| f["collection"] == "saved_logins")
            .unwrap();
        let buckets: Vec<_> = report["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["field"].is_null() && b["envelope_version"] == "V3")
            .collect();
        assert_eq!(buckets.len(), 2);
        assert_ne!(buckets[0]["key_ref"], buckets[1]["key_ref"]);
        assert_eq!(
            buckets
                .iter()
                .map(|b| b["material_instances"].as_i64().unwrap())
                .sum::<i64>(),
            3
        );
        let findings = list(&db, &keys, &id, "findings").await;
        assert_eq!(findings.len(), 1);
        let response = serde_json::to_string(&(all, findings)).unwrap();
        for forbidden in [
            "private-owner-value",
            "sensitive-",
            "private-cleartext-value",
            "private-malformed-value",
            "proofs",
            "envelope\":",
            "arn:",
        ] {
            assert!(!response.contains(forbidden));
        }
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lost_lease_or_expiry_after_staging_cannot_commit_progress() {
    let db = connect_transaction_test_database("census_cas").await;
    let keys = test_encryption_keys();
    let config = test_app_config();
    with_history(&db, 120, async {
        for expire in [false, true] {
            let id = start(&db, &keys, &config).await.unwrap().generation;
            let store = Store {
                db: &db,
                keys: &keys,
                generation: &id,
            };
            let (doc, mut state) = store.load().await.unwrap();
            let census = Census::resume(&db, &keys, &id, state.scope.clone())
                .await
                .unwrap();
            let claim = store.claim(&doc).await.unwrap();
            run_page(&census, &store, &mut state).await.unwrap();
            let mutation = if expire {
                doc! {"$currentDate":{"expires_at":true}}
            } else {
                doc! {"$set":{"claim":"successor"}}
            };
            store
                .collection()
                .update_one(doc! {"_id":&id}, mutation)
                .await
                .unwrap();
            assert!(
                store
                    .commit(&census, &claim, &doc, &mut state)
                    .await
                    .is_err()
            );
            let frozen = store
                .collection()
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(frozen.get("checkpoint"), doc.get("checkpoint"));
            assert_eq!(frozen.get_i64("revision"), Ok(0));
        }
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn corrupted_partition_refuses_publication_and_historical_digest_keeps_dated_totals() {
    let db = connect_transaction_test_database("census_integrity").await;
    let keys = test_encryption_keys();
    let config = test_app_config();
    with_history(&db, 120, async {
        let id = start(&db, &keys, &config).await.unwrap().generation;
        let store = Store {
            db: &db,
            keys: &keys,
            generation: &id,
        };
        loop {
            advance(&db, &keys, &config, &id).await.unwrap();
            let (_, state) = store.load().await.unwrap();
            if state.phase == Phase::Verify {
                let key = doc! {"_id":{"generation":&id,"hash":&state.published[0]}};
                let original = store
                    .collection()
                    .find_one(key.clone())
                    .await
                    .unwrap()
                    .unwrap();
                store
                    .collection()
                    .update_one(key.clone(), doc! {"$set":{"partition":"damaged"}})
                    .await
                    .unwrap();
                assert!(advance(&db, &keys, &config, &id).await.is_err());
                assert!(!status(&db, &keys, &id).await.unwrap().complete);
                store.collection().delete_one(key.clone()).await.unwrap();
                assert!(advance(&db, &keys, &config, &id).await.is_err());
                store.collection().insert_one(original).await.unwrap();
                break;
            }
        }
        finish(&db, &keys, &config, &id).await;
        let (_, mut state) = store.load().await.unwrap();
        state.scope.registry_digest = "previous-registry".into();
        let sealed = store.seal("checkpoint", &state).await.unwrap();
        store
            .collection()
            .update_one(doc! {"_id":&id}, doc! {"$set":{"checkpoint":sealed}})
            .await
            .unwrap();
        let response = serde_json::to_value(status(&db, &keys, &id).await.unwrap()).unwrap();
        assert_eq!(response["complete"], false);
        assert_eq!(response["totals"]["rows_examined"], 0);
        assert!(
            response["blockers"]
                .as_array()
                .unwrap()
                .contains(&"registry_digest_mismatch".into())
        );
        assert_eq!(list(&db, &keys, &id, "families").await.len(), 51);
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn snapshot_history_expiry_on_get_more_discards_the_partial_page() {
    let _guard = crate::tests::BROKER_FAILPOINT.lock().await;
    let fixture = connect_transaction_test_database("census_expiry").await;
    let mut options =
        mongodb::options::ClientOptions::parse(crate::test_utils::selected_test_database_uri())
            .await
            .unwrap();
    options.app_name = Some(fixture.name().into());
    let db = mongodb::Client::with_options(options)
        .unwrap()
        .database(fixture.name());
    let family = observation::families().next().unwrap();
    let rows = db.collection::<Document>(family.collection());
    let padding = "x".repeat(6 * 1024 * 1024);
    for id in ["a", "b", "c"] {
        rows.insert_one(doc! {"_id":id,"unused":&padding})
            .await
            .unwrap();
    }
    let config = test_app_config();
    let keys = test_encryption_keys();
    with_history(&db,120,async{
        let id=start(&db,&keys,&config).await.unwrap().generation;
        let store=Store{db:&db,keys:&keys,generation:&id};
        let (_,mut state)=store.load().await.unwrap();
        let census=Census::resume(&db,&keys,&id,state.scope.clone()).await.unwrap();
        let cut=state.start.clone();
        let at=census.snapshot(&cut).await.unwrap();
        let admin=fixture.client().database("admin");
        admin.run_command(doc!{"configureFailPoint":"failCommand","mode":{"times":1},"data":{"failCommands":["getMore"],"appName":fixture.name(),"errorCode":239}}).await.unwrap();
        let result=scan::source_page(&store,&mut state,&at,&family).await;
        admin.run_command(doc!{"configureFailPoint":"failCommand","mode":"off"}).await.unwrap();
        assert!(result.is_err());
        assert!(state.walk.cursor.is_some());
        assert!(store.load().await.unwrap().1.walk.cursor.is_none());
        assert_eq!(census.log().find_one(doc!{"_id":&id}).await.unwrap().unwrap().get_str("invalid"),Ok("snapshot_unavailable"));
        assert!(advance(&db,&keys,&config,&id).await.is_err());
    }).await;
    db.drop().await.unwrap();
}
