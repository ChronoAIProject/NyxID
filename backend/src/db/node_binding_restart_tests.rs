use super::*;
use crate::{
    handlers::{assistant_action_effects_services, keys, user_services_handler},
    test_utils::{test_app_state, test_auth_user},
};
use axum::{
    Json,
    extract::{Path, State},
};
use tracing::instrument::WithSubscriber;

async fn consent_snapshots(db: &Database, owner: &str) -> HashMap<String, String> {
    crate::services::auth_agent_key_login_service::options_for_actor(db, owner)
        .await
        .unwrap()
        .connections
        .into_iter()
        .map(|c| (c.id, c.permission_snapshot))
        .collect()
}

#[tokio::test]
async fn matching_node_bindings_issue_no_writes_on_startup() {
    let fixture = fixture(Migration::NodeCreate).await;
    let db = fixture.db;
    let catalogs = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    let bindings = db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS);
    let mut catalog = catalogs.find_one(doc! {}).await.unwrap().unwrap();
    let mut binding = bindings.find_one(doc! {}).await.unwrap().unwrap();
    for i in 1..12 {
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = format!("migration-fixture-{i}");
        catalogs.insert_one(&catalog).await.unwrap();
        binding.id = uuid::Uuid::new_v4().to_string();
        binding.service_id = catalog.id.clone();
        bindings.insert_one(&binding).await.unwrap();
    }
    migrate_node_service_bindings(&db).await.unwrap();
    let services = db.collection::<Document>(USER_SERVICES);
    let fixed_time = bson::DateTime::from_millis(1000);
    let setup = services
        .update_many(doc! {}, doc! {"$set":{"updated_at": fixed_time}})
        .await
        .unwrap();
    assert_eq!(setup.matched_count, 12);
    let reviewed = consent_snapshots(&db, &fixture.owner).await;
    assert_eq!(reviewed.len(), 12);
    let before = snapshot(&db).await;

    // A marker after the migration bounds the stream without a timeout or sleep.
    let mut stream = db.watch().await.unwrap();
    let (replica, _, trace) = monitored(&db).await;
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(log.reopen().unwrap())
        .finish();
    migrate_node_service_bindings(&replica)
        .with_subscriber(subscriber)
        .await
        .unwrap();
    let summary = std::fs::read_to_string(log.path()).unwrap();
    db.collection::<Document>("migration_test_barrier")
        .insert_one(doc! {"_id":"done"})
        .await
        .unwrap();
    let mut updates = 0;
    loop {
        let event = stream
            .try_next()
            .await
            .unwrap()
            .expect("post-migration marker");
        let collection = event.ns.unwrap().coll.unwrap();
        if collection == "migration_test_barrier" {
            break;
        }
        assert_eq!(collection, USER_SERVICES);
        assert_eq!(
            event
                .update_description
                .unwrap()
                .updated_fields
                .keys()
                .collect::<Vec<_>>(),
            ["updated_at"]
        );
        updates += 1;
    }
    let after = snapshot(&db).await;
    let commands = trace.lock().unwrap().commands.clone();
    let current = consent_snapshots(&db, &fixture.owner).await;
    let stale = reviewed
        .iter()
        .filter(|(id, digest)| current.get(*id) != Some(*digest))
        .count();
    println!(
        "12 matching routes: {commands:?}, {updates} timestamp-only change stream events, {stale} stale consent snapshots"
    );
    if !summary.is_empty() {
        println!("{}", summary.trim());
    }
    assert_eq!(before[3..], after[3..], "history changed");
    assert!(
        trace.lock().unwrap().writes.is_empty(),
        "matching routes issued MongoDB writes: {commands:?}"
    );
    assert!(
        !summary.contains("Migrated node service bindings to unified collections"),
        "{summary}"
    );
    assert_eq!((updates, stale), (0, 0));
    assert_eq!(before, after);
    db.drop().await.unwrap();
}

async fn insert_node(f: &Fixture, node: &str) {
    let now = bson::DateTime::now();
    f.db.collection::<Document>(crate::models::node::COLLECTION_NAME)
        .insert_one(doc! {
            "_id":node, "user_id":&f.owner, "name":"Fixture", "status":"offline",
            "auth_token_hash":"unused", "is_active":true, "created_at":now, "updated_at":now,
        })
        .await
        .unwrap();
}

async fn service(f: &Fixture) -> UserService {
    f.db.collection(USER_SERVICES)
        .find_one(doc! {"user_id":&f.owner, "catalog_service_id":&f.catalog})
        .await
        .unwrap()
        .unwrap()
}

async fn active_bindings(f: &Fixture) -> Vec<(String, i32)> {
    f.db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(doc! {"user_id":&f.owner,"service_id":&f.catalog,"is_active":true})
        .await
        .unwrap()
        .map_ok(|b| (b.node_id, b.priority))
        .try_collect()
        .await
        .unwrap()
}

async fn put_user_service(f: &Fixture, id: &str, fields: serde_json::Value) {
    let _ = user_services_handler::update_user_service(
        State(test_app_state(f.db.clone())),
        test_auth_user(&f.owner),
        Default::default(),
        Path(id.to_owned()),
        Json(serde_json::from_value(fields).unwrap()),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn startup_preserves_assistant_route_clear_and_owner_priority() {
    let f = fixture(Migration::NodeCreate).await;
    migrate_node_service_bindings(&f.db).await.unwrap();
    let id = service(&f).await.id;
    assert_eq!(active_bindings(&f).await, [(f.node.clone(), 7)]);
    let fallback = uuid::Uuid::new_v4().to_string();
    insert_node(&f, &fallback).await;
    crate::services::node_service::create_binding(&f.db, &f.owner, &fallback, &f.catalog)
        .await
        .unwrap();
    let new_node = uuid::Uuid::new_v4().to_string();
    insert_node(&f, &new_node).await;
    let state = test_app_state(f.db.clone());
    for node in [Some(new_node), None] {
        let _ = assistant_action_effects_services::route_service(
            State(state.clone()),
            test_auth_user(&f.owner),
            Json(
                assistant_action_effects_services::RouteAssistantServiceRequest {
                    action_request_id: uuid::Uuid::new_v4().to_string(),
                    user_service_id: id.clone(),
                    via_node_id: node.clone(),
                },
            ),
        )
        .await
        .unwrap();
        put_user_service(&f, &id, serde_json::json!({"node_priority": 23})).await;
        let edited = service(&f).await;
        assert_eq!(
            (edited.node_id.as_ref(), edited.node_priority),
            (node.as_ref(), 23)
        );
        let mut expected = vec![(fallback.clone(), 0)];
        expected.extend(node.map(|primary| (primary, 0)));
        expected.sort();
        let mut active = active_bindings(&f).await;
        active.sort();
        assert_eq!(active, expected);
        let before = snapshot(&f.db).await;
        let (replica, _, trace) = monitored(&f.db).await;
        migrate_node_service_bindings(&replica).await.unwrap();
        assert!(
            trace.lock().unwrap().writes.is_empty(),
            "a mismatched connection was written"
        );
        assert_eq!(before, snapshot(&f.db).await);
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn unified_route_edits_sync_node_but_keep_binding_priority_separate() {
    let f = fixture(Migration::NodeCreate).await;
    // A no-auth catalog connection permits changing the node through both PUTs.
    let endpoint = crate::test_utils::test_user_endpoint(
        &uuid::Uuid::new_v4().to_string(),
        &f.owner,
        "Fixture",
        "https://127.0.0.1",
        None,
        Some(&f.catalog),
    );
    f.db.collection::<UserEndpoint>(USER_ENDPOINTS)
        .insert_one(&endpoint)
        .await
        .unwrap();
    f.db.collection::<UserService>(USER_SERVICES)
        .insert_one(crate::test_utils::test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &f.owner,
            "fixture",
            &endpoint.id,
            Some(&f.catalog),
            Some(&f.node),
        ))
        .await
        .unwrap();
    migrate_node_service_bindings(&f.db).await.unwrap();
    let id = service(&f).await.id;
    let new_node = uuid::Uuid::new_v4().to_string();
    insert_node(&f, &new_node).await;
    put_user_service(
        &f,
        &id,
        serde_json::json!({"node_id":new_node,"node_priority":23}),
    )
    .await;
    assert_eq!(active_bindings(&f).await, [(new_node.clone(), 0)]);
    assert_eq!(service(&f).await.node_priority, 23);

    let binding =
        f.db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .find_one(doc! {"is_active":true})
            .await
            .unwrap()
            .unwrap();
    crate::services::node_service::update_binding_priority(
        &f.db,
        &f.owner,
        &new_node,
        &binding.id,
        3,
    )
    .await
    .unwrap();
    assert_eq!(service(&f).await.node_priority, 23);
    migrate_node_service_bindings(&f.db).await.unwrap();
    let applied = service(&f).await;
    assert_eq!(
        (applied.node_id, applied.node_priority),
        (Some(new_node), 23)
    );

    let _ = keys::update_key(
        State(test_app_state(f.db.clone())),
        test_auth_user(&f.owner),
        Path(id),
        Json(serde_json::from_value(serde_json::json!({"node_id":""})).unwrap()),
    )
    .await
    .unwrap();
    assert!(active_bindings(&f).await.is_empty());
    migrate_node_service_bindings(&f.db).await.unwrap();
    assert_eq!(service(&f).await.node_id, None);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn binding_leaves_both_existing_siblings_untouched() {
    let f = fixture(Migration::NodeCreate).await;
    let services = f.db.collection::<UserService>(USER_SERVICES);
    for slug in ["first-connection", "second-connection"] {
        services
            .insert_one(crate::test_utils::test_user_service(
                &uuid::Uuid::new_v4().to_string(),
                &f.owner,
                slug,
                "fixture-endpoint",
                Some(&f.catalog),
                None,
            ))
            .await
            .unwrap();
    }
    services
        .update_one(
            doc! {"slug": "second-connection"},
            doc! {"$set": {"node_id": "other-node", "node_priority": 29}},
        )
        .await
        .unwrap();
    let before = snapshot(&f.db).await;
    let (replica, _, trace) = monitored(&f.db).await;
    migrate_node_service_bindings(&replica).await.unwrap();
    assert!(trace.lock().unwrap().writes.is_empty());
    assert_eq!(snapshot(&f.db).await, before);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn missing_legacy_priority_already_agrees_with_zero() {
    let f = fixture(Migration::NodeCreate).await;
    migrate_node_service_bindings(&f.db).await.unwrap();
    f.db.collection::<Document>(USER_SERVICES)
        .update_one(doc! {}, doc! {"$unset":{"node_priority":""}})
        .await
        .unwrap();
    f.db.collection::<Document>(NODE_SERVICE_BINDINGS)
        .update_one(doc! {}, doc! {"$set":{"priority":0}})
        .await
        .unwrap();
    let (replica, _, trace) = monitored(&f.db).await;
    migrate_node_service_bindings(&replica).await.unwrap();
    assert!(trace.lock().unwrap().writes.is_empty());
    assert_eq!(service(&f).await.node_priority, 0);
    f.db.drop().await.unwrap();
}

async fn during_startup_read(db: &Database, collection: &str, operation: impl Future<Output = ()>) {
    // Hold the first replica's completed collection read. Use a separate thread so
    // the driver's synchronous command callback cannot block the owner's client.
    let mut options = ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (read, scanned) = tokio::sync::oneshot::channel();
    let (resume, wait) = std::sync::mpsc::channel();
    let namespace = format!("{}.{collection}", db.name());
    let gate = Mutex::new(Some((read, wait)));
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Succeeded(e) = event
            && e.command_name == "find"
            && e.reply.get_document("cursor").and_then(|c| c.get_str("ns"))
                == Ok(namespace.as_str())
            && let Some((read, wait)) = gate.lock().unwrap().take()
        {
            read.send(()).unwrap();
            wait.recv().unwrap();
        }
    }));
    let name = db.name().to_owned();
    let replica = tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let db = Client::with_options(options).unwrap().database(&name);
                migrate_node_service_bindings(&db).await.unwrap();
            });
    });
    scanned.await.unwrap();
    operation.await;
    resume.send(()).unwrap();
    replica.await.unwrap();
}

#[tokio::test]
async fn stale_start_cannot_restore_a_deactivated_binding_after_a_second_start() {
    let f = fixture(Migration::NodeCreate).await;
    migrate_node_service_bindings(&f.db).await.unwrap();
    let id = service(&f).await.id;
    let new_node = uuid::Uuid::new_v4().to_string();
    insert_node(&f, &new_node).await;

    during_startup_read(&f.db, NODE_SERVICE_BINDINGS, async {
        put_user_service(
            &f,
            &id,
            serde_json::json!({"node_id":new_node,"node_priority":0}),
        )
        .await;
        assert_eq!(active_bindings(&f).await, [(new_node.clone(), 0)]);
        migrate_node_service_bindings(&f.db).await.unwrap();
        assert_eq!(service(&f).await.node_id.as_ref(), Some(&new_node));
    })
    .await;

    let kept = service(&f).await;
    assert_eq!(
        (kept.node_id, kept.node_priority),
        (Some(new_node.clone()), 0)
    );
    assert_eq!(active_bindings(&f).await, [(new_node, 0)]);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn two_failover_bindings_never_rewrite_an_existing_primary() {
    let f = fixture(Migration::NodeCreate).await;
    migrate_node_service_bindings(&f.db).await.unwrap();
    let other = uuid::Uuid::new_v4().to_string();
    insert_node(&f, &other).await;
    crate::services::node_service::create_binding(&f.db, &f.owner, &other, &f.catalog)
        .await
        .unwrap();
    assert_eq!(service(&f).await.node_id.as_ref(), Some(&f.node));

    let before = snapshot(&f.db).await;
    let (replica, _, trace) = monitored(&f.db).await;
    for _ in 0..2 {
        migrate_node_service_bindings(&replica).await.unwrap();
        assert_eq!(snapshot(&f.db).await, before);
    }
    assert!(trace.lock().unwrap().writes.is_empty());
    assert_eq!(active_bindings(&f).await.len(), 2);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn startup_cannot_apply_a_route_to_a_service_edited_after_selection() {
    let f = fixture(Migration::NodeCreate).await;
    migrate_node_service_bindings(&f.db).await.unwrap();
    f.db.collection::<Document>(USER_SERVICES)
        .update_one(
            doc! {},
            doc! {"$unset":{"node_id":""},"$set":{"node_priority":0}},
        )
        .await
        .unwrap();
    let id = service(&f).await.id;
    during_startup_read(&f.db, USER_SERVICES, async {
        put_user_service(&f, &id, serde_json::json!({"node_priority":23})).await;
    })
    .await;
    let kept = service(&f).await;
    assert_eq!((kept.node_id, kept.node_priority), (None, 23));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn startup_imports_only_the_lowest_priority_then_binding_id_for_every_source() {
    for migration in [
        Migration::NodeCreate,
        Migration::ProviderToken,
        Migration::ServiceConnection,
    ] {
        let f = fixture(migration).await;
        let bindings = f.db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS);
        let initial = bindings.find_one(doc! {}).await.unwrap().unwrap();
        let winner_id = "00000000-0000-4000-8000-000000000002";
        let default_id = "00000000-0000-4000-8000-000000000001";
        for (id, priority) in [
            ("00000000-0000-4000-8000-000000000003", -1),
            (winner_id, -1),
            (default_id, 0),
        ] {
            let mut binding = initial.clone();
            binding.id = id.into();
            binding.node_id = uuid::Uuid::new_v4().to_string();
            binding.priority = priority;
            bindings.insert_one(binding).await.unwrap();
        }
        bindings
            .update_one(doc! {"_id":default_id}, doc! {"$unset":{"priority":""}})
            .await
            .unwrap();
        let winner = bindings
            .find_one(doc! {"_id":winner_id})
            .await
            .unwrap()
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        migrate_to_unified_collections(&f.db, &keys).await.unwrap();
        let imported = service(&f).await;
        assert_eq!(
            (imported.node_id, imported.node_priority),
            (Some(winner.node_id), -1),
            "{migration:?}"
        );
        if matches!(migration, Migration::NodeCreate) {
            assert_eq!(imported.source_id.as_deref(), Some(winner.id.as_str()));
        }
        let before = snapshot(&f.db).await;
        let (replica, _, trace) = monitored(&f.db).await;
        migrate_to_unified_collections(&replica, &keys)
            .await
            .unwrap();
        assert!(trace.lock().unwrap().writes.is_empty());
        assert_eq!(snapshot(&f.db).await, before);
        assert_eq!(
            bindings
                .count_documents(doc! {"is_active":true})
                .await
                .unwrap(),
            4
        );
        if matches!(migration, Migration::NodeCreate) {
            f.db.collection::<Document>(USER_SERVICES)
                .update_one(doc! {"_id":&imported.id}, doc! {"$set":{"is_active":false}})
                .await
                .unwrap();
            f.db.collection::<Document>(USER_API_KEYS)
                .delete_one(doc! {"_id": imported.api_key_id.as_deref().unwrap()})
                .await
                .unwrap();
            migrate_node_service_bindings(&replica).await.unwrap();
            assert!(
                trace.lock().unwrap().writes.is_empty(),
                "a fallback must not resurrect a consumed import"
            );
        } else {
            f.db.collection::<Document>(USER_SERVICES)
                .update_one(
                    doc! {"_id":&imported.id},
                    doc! {"$unset":{"node_id":""},"$set":{"node_priority":41}},
                )
                .await
                .unwrap();
            let cleared = snapshot(&f.db).await;
            migrate_to_unified_collections(&replica, &keys)
                .await
                .unwrap();
            assert!(trace.lock().unwrap().writes.is_empty());
            assert_eq!(snapshot(&f.db).await, cleared);
        }
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn concurrent_first_imports_publish_one_node_connection() {
    let f = fixture(Migration::NodeCreate).await;
    during_startup_read(&f.db, USER_SERVICES, async {
        migrate_node_service_bindings(&f.db).await.unwrap();
    })
    .await;
    assert_converged(&f, Migration::NodeCreate).await;
    f.db.drop().await.unwrap();
}
