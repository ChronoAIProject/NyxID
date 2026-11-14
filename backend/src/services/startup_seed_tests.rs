use std::{collections::BTreeMap, sync::Arc, time::Duration};

use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{Document, doc},
    options::IndexOptions,
};
use tokio::sync::{Barrier, Mutex};

use crate::test_utils::{connect_transaction_test_database, test_encryption_keys};

type MissingKey = (String, String, String);
struct MissingGate {
    barrier: Barrier,
    arrivals: std::sync::atomic::AtomicUsize,
    resume: Option<tokio::sync::Notify>,
}
static MISSING: std::sync::LazyLock<Mutex<BTreeMap<MissingKey, Arc<MissingGate>>>> =
    std::sync::LazyLock::new(Mutex::default);

pub(super) async fn missing(db: &Database, family: &str, identity: &str) {
    let key = (db.name().into(), family.into(), identity.into());
    let barrier = MISSING.lock().await.get(&key).cloned();
    if let Some(gate) = barrier {
        if gate
            .arrivals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            >= 2
        {
            return;
        }
        tokio::time::timeout(Duration::from_secs(30), gate.barrier.wait())
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{family}/{identity}: missing barrier timed out after {} arrivals: {error}",
                    gate.arrivals.load(std::sync::atomic::Ordering::SeqCst)
                )
            });
        if let Some(resume) = &gate.resume {
            resume.notified().await;
        }
    }
}

async fn seed(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    family: &str,
) -> crate::errors::AppResult<()> {
    match family {
        "oauth" => super::oauth_client_service::seed_default_clients(db).await,
        "role" => super::role_service::seed_system_roles(db).await,
        "machine" => Box::pin(super::machine_access_service::ensure_indexes(db)).await,
        "provider" => Box::pin(super::provider_service::seed_default_providers(db, keys)).await,
        "service" | "requirement" => {
            Box::pin(super::provider_service::seed_default_services(db, keys)).await
        }
        _ => unreachable!(),
    }
}

async fn race(family: &str, identity: &str) {
    let db = connect_transaction_test_database(&format!("startup_seed_{family}")).await;
    for (collection, keys, partial) in [
        ("provider_configs", doc! {"slug":1}, None),
        ("roles", doc! {"slug":1}, None),
        (
            "downstream_services",
            doc! {"slug":1},
            Some(doc! {"is_active":true}),
        ),
        (
            "service_provider_requirements",
            doc! {"service_id":1,"provider_config_id":1},
            None,
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(partial)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
    }
    let keys = test_encryption_keys();
    if matches!(family, "service" | "requirement") {
        Box::pin(super::provider_service::seed_default_providers(&db, &keys))
            .await
            .unwrap();
    }
    if matches!(family, "service" | "requirement") {
        Box::pin(super::provider_service::seed_default_services(&db, &keys))
            .await
            .unwrap();
        let service = db
            .collection::<Document>("downstream_services")
            .find_one(doc! {"slug":identity})
            .await
            .unwrap()
            .unwrap();
        db.collection::<Document>("service_provider_requirements")
            .delete_many(doc! {"service_id":service.get_str("_id").unwrap()})
            .await
            .unwrap();
        if family == "service" {
            db.collection::<Document>("downstream_services")
                .delete_one(doc! {"_id":service.get_str("_id").unwrap()})
                .await
                .unwrap();
        }
    }
    let key = (db.name().into(), family.into(), identity.into());
    let gate = Arc::new(MissingGate {
        barrier: Barrier::new(2),
        arrivals: Default::default(),
        resume: None,
    });
    MISSING.lock().await.insert(key.clone(), gate.clone());
    let (a, b) = tokio::join!(
        Box::pin(seed(&db, &keys, family)),
        Box::pin(seed(&db, &keys, family))
    );
    MISSING.lock().await.remove(&key);
    assert!(
        gate.arrivals.load(std::sync::atomic::Ordering::SeqCst) >= 2,
        "both callers must observe the seed missing"
    );
    let duplicate = |result: &crate::errors::AppResult<()>| {
        matches!(result, Err(crate::errors::AppError::DatabaseError(error))
            if super::telegram_new_service::is_duplicate_key_error(error))
    };
    assert!(
        a.is_ok() && b.is_ok(),
        "{family}: first={a:?} (duplicate_key_11000={}); second={b:?} (duplicate_key_11000={})",
        duplicate(&a),
        duplicate(&b)
    );
    let collection = match family {
        "oauth" => "oauth_clients",
        "role" => "roles",
        "machine" => "platform_settings",
        "provider" => "provider_configs",
        "service" | "requirement" => "downstream_services",
        _ => unreachable!(),
    };
    let identity_field = if matches!(family, "oauth" | "machine") {
        "_id"
    } else {
        "slug"
    };
    assert_eq!(
        db.collection::<Document>(collection)
            .count_documents(doc! {identity_field:identity})
            .await
            .unwrap(),
        1
    );
    let single = connect_transaction_test_database(&format!("single_start_{family}")).await;
    if matches!(family, "service" | "requirement") {
        Box::pin(seed(&single, &keys, "provider")).await.unwrap();
    }
    Box::pin(seed(&single, &keys, family)).await.unwrap();
    let mut identities = Vec::new();
    for database in [&db, &single] {
        let rows: Vec<Document> = database
            .collection::<Document>(collection)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let mut names: Vec<String> = rows
            .iter()
            .map(|row| row.get_str(identity_field).unwrap().into())
            .collect();
        names.sort();
        identities.push(names);
    }
    assert_eq!(
        identities[0], identities[1],
        "two starts produce exactly one start's seed identities"
    );
    if matches!(family, "service" | "requirement") {
        let services: Vec<Document> = db
            .collection::<Document>("downstream_services")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let requirements: Vec<Document> = db
            .collection::<Document>("service_provider_requirements")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        for service in &services {
            let rows: Vec<_> = requirements
                .iter()
                .filter(|r| r.get_str("service_id").unwrap() == service.get_str("_id").unwrap())
                .collect();
            if service.get_str("auth_method").unwrap() == "none" {
                assert_eq!(
                    rows.len(),
                    1,
                    "one complete service/provider pair: {}",
                    service.get_str("slug").unwrap()
                );
                assert_eq!(
                    rows[0].get_str("provider_config_id"),
                    service.get_str("provider_config_id")
                );
            } else {
                assert!(rows.is_empty(), "direct-auth services have no requirement");
            }
        }
        assert!(requirements.iter().all(|r| {
            services
                .iter()
                .any(|s| s.get_str("_id") == r.get_str("service_id"))
        }));
        assert_eq!(
            requirements.len() as u64,
            single
                .collection::<Document>("service_provider_requirements")
                .count_documents(doc! {})
                .await
                .unwrap()
        );
    }
    single.drop().await.unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn simultaneous_missing_oauth_seed() {
    Box::pin(race("oauth", "nyx-mcp")).await;
}

#[tokio::test]
async fn simultaneous_missing_role_seed() {
    Box::pin(race("role", "admin")).await;
}

#[tokio::test]
async fn simultaneous_missing_provider_seed() {
    Box::pin(race("provider", "ifttt")).await;
}

#[tokio::test]
async fn simultaneous_missing_service_seed() {
    Box::pin(race("service", "llm-openai")).await;
}

#[tokio::test]
async fn simultaneous_missing_requirement_seed() {
    Box::pin(race("requirement", "llm-openai")).await;
}

#[tokio::test]
async fn simultaneous_missing_machine_separation_seed() {
    Box::pin(race(
        "machine",
        crate::models::machine_access::SEPARATION_CUTOVER,
    ))
    .await;
}

#[tokio::test]
async fn failed_requirement_seed_does_not_publish_half_a_service_pair() {
    Box::pin(async {
        let db = connect_transaction_test_database("seed_pair_abort").await;
        let keys = test_encryption_keys();
        super::provider_service::seed_default_providers(&db, &keys)
            .await
            .unwrap();
        db.create_collection("service_provider_requirements")
            .validator(doc! {"required":false})
            .await
            .unwrap();
        let result = super::provider_service::seed_default_services(&db, &keys).await;
        assert!(matches!(
            result,
            Err(crate::errors::AppError::DatabaseError(_))
        ));
        let half = db
            .collection::<Document>("downstream_services")
            .count_documents(doc! {"auth_method":"none"})
            .await
            .unwrap();
        db.drop().await.unwrap();
        assert_eq!(
            half, 0,
            "a rejected requirement must roll back its new service"
        );
    })
    .await;
}

#[tokio::test]
async fn seed_collision_preserves_winner_and_rejects_wrong_identity() {
    let db = connect_transaction_test_database("seed_winner_validation").await;
    let rows = db.collection::<Document>("seed_winners");
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"slug":1})
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await
    .unwrap();
    let winner = doc! {"_id":"winner", "slug":"default", "created_by":"operator", "name":"Admin name", "is_active":false};
    rows.insert_one(&winner).await.unwrap();
    let candidate = doc! {"_id":"loser", "slug":"default", "created_by":"system", "name":"Default name", "is_active":true};
    let saved =
        super::startup_seed_service::insert_or_read(&rows, &candidate, doc! {"slug":"default"})
            .await
            .unwrap();
    assert_eq!(saved, Some(winner.clone()));
    assert_eq!(
        rows.find_one(doc! {"_id":"winner"}).await.unwrap().unwrap(),
        winner
    );
    assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 1);

    for filter in [doc! {"slug":"other"}, doc! {"_id":"absent"}] {
        let error = super::startup_seed_service::insert_or_read(&rows, &candidate, filter)
            .await
            .unwrap_err();
        assert!(
            matches!(error, crate::errors::AppError::DatabaseError(error)
            if super::telegram_new_service::is_duplicate_key_error(&error))
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn seed_insert_propagates_non_duplicate_errors() {
    let db = connect_transaction_test_database("seed_validation_failure").await;
    db.create_collection("seed_winners")
        .validator(doc! {"accepted":true})
        .await
        .unwrap();
    let rows = db.collection::<Document>("seed_winners");
    let error = super::startup_seed_service::insert_or_read(
        &rows,
        &doc! {"_id":"seed", "accepted":false},
        doc! {"_id":"seed"},
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, crate::errors::AppError::DatabaseError(error)
        if !super::telegram_new_service::is_duplicate_key_error(&error))
    );
    assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 0);
    db.drop().await.unwrap();
}

async fn concurrent_winner_matches_sequential(family: &str, identity: &str) {
    let db = connect_transaction_test_database("seed_sequential_parity").await;
    let keys = test_encryption_keys();
    super::provider_service::seed_default_providers(&db, &keys)
        .await
        .unwrap();
    super::provider_service::seed_default_services(&db, &keys)
        .await
        .unwrap();
    seed(&db, &keys, family).await.unwrap();
    let (collection, filter, index, changes) = match family {
        "oauth" => (
            "oauth_clients",
            doc! {"_id":identity},
            None,
            doc! {"client_type":"confidential", "client_secret_hash":"existing-hash", "created_by":"operator", "allowed_scopes":"openid"},
        ),
        "role" => (
            "roles",
            doc! {"slug":identity},
            Some(doc! {"slug":1}),
            doc! {"permissions":["old:permission"], "is_default":true, "name":"Old role"},
        ),
        "provider" => (
            "provider_configs",
            doc! {"slug":identity},
            Some(doc! {"slug":1}),
            doc! {"provider_type":"oauth2", "created_by":"operator", "name":"Operator provider"},
        ),
        "service" => {
            let mut changes = doc! {"created_by":"operator", "service_type":"websocket", "name":"Operator service"};
            if identity == "api-google" {
                changes.insert("provider_config_id", "operator-provider");
            }
            (
                "downstream_services",
                doc! {"slug":identity},
                Some(doc! {"slug":1}),
                changes,
            )
        }
        "requirement" => {
            let service = db
                .collection::<Document>("downstream_services")
                .find_one(doc! {"slug":identity})
                .await
                .unwrap()
                .unwrap();
            (
                "service_provider_requirements",
                doc! {"service_id":service.get_str("_id").unwrap()},
                Some(doc! {"service_id":1,"provider_config_id":1}),
                doc! {"required":false, "scopes":["operator-scope"], "injection_key":"Operator-Header"},
            )
        }
        _ => unreachable!(),
    };
    let rows = db.collection::<Document>(collection);
    let existing_requirement = if family == "service" {
        let service = rows.find_one(filter.clone()).await.unwrap().unwrap();
        let requirements = db.collection::<Document>("service_provider_requirements");
        let query = doc! {"service_id": service.get_str("_id").unwrap()};
        requirements
            .update_one(
                query.clone(),
                doc! {"$set": {"provider_config_id":"operator-provider"}},
            )
            .await
            .unwrap();
        Some(requirements.find_one(query).await.unwrap().unwrap())
    } else {
        None
    };
    if let Some(index) = index {
        let partial = (family == "service").then(|| doc! {"is_active":true});
        rows.create_index(
            IndexModel::builder()
                .keys(index)
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(partial)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    }
    rows.update_one(filter.clone(), doc! {"$set":changes})
        .await
        .unwrap();
    let winner = rows.find_one(filter.clone()).await.unwrap().unwrap();
    seed(&db, &keys, family).await.unwrap();
    let mut sequential = rows.find_one(filter.clone()).await.unwrap().unwrap();
    sequential.remove("updated_at");
    rows.delete_one(filter.clone()).await.unwrap();
    let gate_key = (db.name().into(), family.into(), identity.into());
    let gate = Arc::new(MissingGate {
        barrier: Barrier::new(2),
        arrivals: Default::default(),
        resume: Some(tokio::sync::Notify::new()),
    });
    MISSING.lock().await.insert(gate_key.clone(), gate.clone());
    let (result, ()) = tokio::join!(Box::pin(seed(&db, &keys, family)), async {
        gate.barrier.wait().await;
        rows.insert_one(&winner).await.unwrap();
        gate.resume.as_ref().unwrap().notify_one();
    });
    MISSING.lock().await.remove(&gate_key);
    assert!(
        result.is_ok(),
        "{family}: a concurrent winner must follow the existing-row branch: {result:?}"
    );
    let mut concurrent = rows.find_one(filter.clone()).await.unwrap().unwrap();
    concurrent.remove("updated_at");
    assert_eq!(
        concurrent, sequential,
        "{family}: concurrent and sequential second starts agree"
    );
    assert_eq!(rows.count_documents(filter).await.unwrap(), 1);
    if let Some(requirement) = existing_requirement {
        assert_eq!(
            db.collection::<Document>("service_provider_requirements")
                .find_one(doc! {"_id": requirement.get_str("_id").unwrap()})
                .await
                .unwrap()
                .unwrap(),
            requirement
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn oauth_seed_winner_matches_sequential_start() {
    Box::pin(concurrent_winner_matches_sequential("oauth", "nyx-mcp")).await;
}
#[tokio::test]
async fn role_seed_winner_matches_sequential_start() {
    Box::pin(concurrent_winner_matches_sequential("role", "admin")).await;
}
#[tokio::test]
async fn provider_seed_winner_matches_sequential_start() {
    Box::pin(concurrent_winner_matches_sequential("provider", "ifttt")).await;
}
#[tokio::test]
async fn service_seed_winner_matches_sequential_start() {
    for slug in ["llm-openai", "api-google"] {
        Box::pin(concurrent_winner_matches_sequential("service", slug)).await;
    }
}
#[tokio::test]
async fn requirement_seed_winner_matches_sequential_start() {
    Box::pin(concurrent_winner_matches_sequential(
        "requirement",
        "llm-openai",
    ))
    .await;
}

#[tokio::test]
async fn old_starter_service_without_requirement_is_backfilled() {
    Box::pin(async {
        for timing in ["before_start", "before_lookup", "after_missing"] {
            let db = connect_transaction_test_database("old_seed_pair_bridge").await;
            let keys = test_encryption_keys();
            super::provider_service::seed_default_providers(&db, &keys)
                .await
                .unwrap();
            super::provider_service::seed_default_services(&db, &keys)
                .await
                .unwrap();
            let services = db.collection::<Document>("downstream_services");
            services
                .create_index(
                    IndexModel::builder()
                        .keys(doc! {"slug":1})
                        .options(
                            IndexOptions::builder()
                                .unique(true)
                                .partial_filter_expression(doc! {"is_active":true})
                                .build(),
                        )
                        .build(),
                )
                .await
                .unwrap();
            let filter = doc! {"slug":"llm-openai"};
            let service = services.find_one(filter.clone()).await.unwrap().unwrap();
            let id = service.get_str("_id").unwrap();
            let requirements = db.collection::<Document>("service_provider_requirements");
            requirements
                .create_index(
                    IndexModel::builder()
                        .keys(doc! {"service_id":1,"provider_config_id":1})
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                )
                .await
                .unwrap();
            requirements
                .delete_many(doc! {"service_id":id})
                .await
                .unwrap();
            if timing != "before_start" {
                services.delete_one(filter.clone()).await.unwrap();
                let stage = if timing == "before_lookup" {
                    "service_lookup"
                } else {
                    "service"
                };
                let key = (db.name().into(), stage.into(), "llm-openai".into());
                let gate = Arc::new(MissingGate {
                    barrier: Barrier::new(2),
                    arrivals: Default::default(),
                    resume: Some(tokio::sync::Notify::new()),
                });
                MISSING.lock().await.insert(key.clone(), gate.clone());
                let (result, ()) = tokio::join!(Box::pin(seed(&db, &keys, "service")), async {
                    gate.barrier.wait().await;
                    services.insert_one(&service).await.unwrap();
                    gate.resume.as_ref().unwrap().notify_one();
                });
                MISSING.lock().await.remove(&key);
                result.unwrap();
            } else {
                seed(&db, &keys, "service").await.unwrap();
            }
            assert_eq!(services.count_documents(filter).await.unwrap(), 1);
            let pair: Vec<_> = requirements
                .find(doc! {"service_id":id})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(pair.len(), 1);
            assert_eq!(
                pair[0].get_str("provider_config_id").unwrap(),
                service.get_str("provider_config_id").unwrap()
            );
            assert_eq!(
                services
                    .find_one(doc! {"_id":id})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_str("_id")
                    .unwrap(),
                id
            );
            db.drop().await.unwrap();
        }
    })
    .await;
}
