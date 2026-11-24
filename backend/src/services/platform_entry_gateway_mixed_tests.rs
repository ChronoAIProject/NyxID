//! Both revisions serve the same retained attempt at its original loopback address.
//! Exchange the fixture rows as BSON because each test process owns its database.
use super::*;
use crate::services::{channel_bot_record, encrypted_record_migration as migration};
use futures::TryStreamExt;

#[test]
fn mixed_gateway_setup() {
    let Ok(step) = std::env::var("NYXID_RECORD_FIXTURE_STEP") else {
        return;
    };
    crate::test_utils::run_native_journey_test("mixed_gateway_setup", move || async move {
        let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE").unwrap();
        let writes = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
        let legacy = std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() == Ok("true");
        let mut keys = crate::test_utils::test_encryption_keys();
        keys.set_contextual_record_writes(writes);
        keys.set_allow_legacy_record_reads(legacy);
        let (mut f, mut receipt) = if step == "write" {
            let f = SetupFixture::new(2).await;
            // Prepare the independent Discord credential for strict-mode admission on either revision.
            let bot = f
                .state()
                .db
                .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .find_one(doc! {"_id": &f.bot.id})
                .await
                .unwrap()
                .unwrap();
            channel_bot_record::migrate(&f.state().db, &f.state().encryption_keys, &bot)
                .await
                .unwrap();
            let receipt = doc! {
                "address": f.harness.base.strip_prefix("http://").unwrap(),
                "bot": &f.bot.id,
                "funding": f.funding(),
            };
            (f, receipt)
        } else {
            let receipt: Document = bson::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            let address: std::net::SocketAddr =
                receipt.get_str("address").unwrap().parse().unwrap();
            assert!(address.ip().is_loopback());
            let listener = tokio::net::TcpListener::bind(address).await.unwrap();
            let db = crate::test_utils::connect_transaction_test_database("gateway_mixed").await;
            for (name, value) in receipt.get_document("collections").unwrap() {
                let rows = db.collection::<Document>(name);
                let documents = value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_document().unwrap());
                rows.delete_many(doc! {}).await.unwrap();
                rows.insert_many(documents).await.unwrap();
            }
            let bot = db
                .collection(crate::models::channel_bot::COLLECTION_NAME)
                .find_one(doc! {"_id": receipt.get_str("bot").unwrap()})
                .await
                .unwrap()
                .unwrap();
            let mut state = crate::test_utils::test_app_state(db);
            state.config.jwt_issuer = "https://nyxid.example".into();
            (
                SetupFixture::serve(
                    listener,
                    state,
                    bot,
                    receipt.get_str("funding").unwrap().into(),
                    2,
                ),
                receipt,
            )
        };
        if step != "write" {
            *f.harness.commands.lock().unwrap() =
                bson::from_bson(receipt["commands"].clone()).unwrap();
            *f.harness.records.lock().unwrap() =
                bson::from_bson(receipt["gateway_records"].clone()).unwrap();
        }
        f.harness.state.encryption_keys = Arc::new(keys);
        let db = &f.state().db;
        let rows = db.collection::<Document>(GATEWAYS);
        if step != "write" {
            let id = receipt.get_str("transport").unwrap();
            let expiry = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(180));
            rows.update_one(
                doc! {"_id": id},
                doc! {"$set": {"setup_dispatch_deadline": expiry}},
            )
            .await
            .unwrap();
        }
        match step.as_str() {
            "retire" | "replace" => {
                let review = admin::review_lifecycle(f.state(), f.actor(), &f.bot.id)
                    .await
                    .unwrap();
                with_transport_for_test(
                    f.transport.clone(),
                    admin::reserve_reviewed_update(
                        f.state(),
                        f.actor(),
                        f.funding(),
                        NativeTransportChoice::Gateway,
                        &review,
                        None,
                        None,
                    ),
                )
                .await
                .unwrap();
                let old = rows
                    .find_one(
                        doc! {"_id": receipt.get_str("transport").unwrap(), "state": "retired"},
                    )
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    old.get_str("request_hash").unwrap(),
                    receipt.get_str("hash").unwrap()
                );
                assert!(old.get_binary_generic("request_ciphertext").is_ok());
            }
            "migrate" => {
                let keys = &f.state().encryption_keys;
                let generation = migration::start(db, keys, migration::Mode::Migrate, None)
                    .await
                    .unwrap();
                loop {
                    migration::advance(db, keys, &generation).await.unwrap();
                    let report = migration::status(db, keys).await.unwrap().unwrap();
                    if let Some(counts) = report
                        .field_outcomes
                        .iter()
                        .find(|c| c.collection == GATEWAYS && c.authenticated_current > 0)
                    {
                        assert_eq!(counts.verification_blocked, 0);
                        break;
                    }
                    assert_eq!(report.phase, "running");
                }
            }
            "read" | "reject" => {
                let NativeConversionOutcome::GatewayRecovery(review) =
                    f.convert(None).await.unwrap()
                else {
                    panic!("expected retained review")
                };
                assert_eq!(review.request_hash, receipt.get_str("hash").unwrap());
                // The mock returns a reserved, incomplete record after every successful POST.
                assert!(f.convert(Some(&review)).await.is_err());
                let row = rows
                    .find_one(doc! {"_id": &review.record_id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_ne!(row.get_str("setup_attempt_id").unwrap(), review.attempt_id);
                let requests = f.harness.received.lock().unwrap();
                if step == "reject" {
                    assert!(requests.is_empty());
                } else {
                    assert_eq!(requests.len(), 1);
                    assert_eq!(sha256(&requests[0]), review.request_hash);
                }
            }
            "write" => {}
            _ => panic!("unknown gateway fixture step"),
        }
        if matches!(step.as_str(), "write" | "replace") {
            assert!(f.convert(None).await.is_err());
            let row = rows
                .find_one(doc! {"state": "prepared"})
                .await
                .unwrap()
                .unwrap();
            let requests = f.harness.received.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let hash = sha256(&requests[0]);
            assert_eq!(row.get_str("request_hash").unwrap(), hash);
            assert!(row.get_binary_generic("request_ciphertext").is_ok());
            if writes {
                assert_eq!(
                    row.get_document("secret_context_versions")
                        .unwrap()
                        .get_i32("request_ciphertext"),
                    Ok(3)
                );
            } else {
                assert!(!row.contains_key("secret_context_versions"));
            }
            assert_ne!(receipt.get_str("transport").ok(), row.get_str("_id").ok());
            receipt.insert("transport", row.get_str("_id").unwrap());
            receipt.insert("hash", hash);
        }
        if matches!(step.as_str(), "retire" | "migrate") {
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 0);
        }
        let mut collections = Document::new();
        for name in db.list_collection_names().await.unwrap() {
            let docs: Vec<Document> = db
                .collection(&name)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            if !docs.is_empty() {
                collections.insert(name, docs);
            }
        }
        receipt.insert(
            "commands",
            bson::to_bson(&*f.harness.commands.lock().unwrap()).unwrap(),
        );
        receipt.insert(
            "gateway_records",
            bson::to_bson(&*f.harness.records.lock().unwrap()).unwrap(),
        );
        receipt.insert("collections", collections);
        std::fs::write(file, bson::to_vec(&receipt).unwrap()).unwrap();
        f.close().await;
    });
}
