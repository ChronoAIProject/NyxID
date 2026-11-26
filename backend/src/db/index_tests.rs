use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{Document, doc},
    event::{EventHandler, command::CommandEvent},
};

use super::ensure_indexes;

mod races;

const REPLACEMENTS: &[(&str, &str)] = &[
    ("users", "email_1"),
    ("users", "social_provider_1_social_provider_id_1"),
    ("downstream_services", "slug_1"),
    ("nodes", "user_id_1_name_1"),
    ("node_service_bindings", "node_id_1_service_id_1"),
    ("user_api_keys", "source_1_source_id_1"),
    ("user_services", "source_1_source_id_1"),
    ("org_invites", "expires_at_1"),
    (
        "coordination_leases",
        "coordination_leases_ephemeral_expiry_ttl",
    ),
];

type Drops = Arc<Mutex<Vec<(String, String)>>>;

fn record_drop(event: &CommandEvent, drops: &Drops) {
    if let CommandEvent::Started(event) = event
        && event.command_name == "dropIndexes"
    {
        let collection = event.command.get_str("dropIndexes").unwrap();
        let index = event.command.get_str("index").unwrap();
        if REPLACEMENTS.contains(&(collection, index))
            || matches!(collection, "nodes" | "node_service_bindings")
        {
            drops
                .lock()
                .unwrap()
                .push((collection.into(), index.into()));
        }
    }
}

async fn monitored_db() -> (Database, Drops) {
    let drops = Drops::default();
    let recorder = drops.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "idx_restart",
        EventHandler::callback(move |event| record_drop(&event, &recorder)),
    )
    .await
    .unwrap();
    (db, drops)
}

async fn definition(db: &Database, collection: &str, name: &str) -> IndexModel {
    db.collection::<Document>(collection)
        .list_indexes()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .find(|index| index.options.as_ref().and_then(|o| o.name.as_deref()) == Some(name))
        .expect("named index exists")
}

async fn identities(db: &Database) -> Vec<String> {
    let mut result = Vec::new();
    for &(collection, name) in REPLACEMENTS {
        // WiredTiger's table ident changes on rebuild; listIndexes alone cannot
        // distinguish the original index from a replacement with the same spec.
        let stats = db
            .run_command(doc! {"collStats": collection})
            .await
            .unwrap();
        result.push(
            stats
                .get_document("indexDetails")
                .unwrap()
                .get_document(name)
                .unwrap()
                .get_str("uri")
                .unwrap()
                .to_owned(),
        );
    }
    result
}

#[tokio::test]
async fn correct_indexes_survive_two_startups_without_drop_commands() {
    let (db, drops) = monitored_db().await;
    ensure_indexes(&db).await.unwrap();
    let before = identities(&db).await;
    drops.lock().unwrap().clear();
    for _ in 0..2 {
        ensure_indexes(&db).await.unwrap();
        let after = identities(&db).await;
        let recorded = drops.lock().unwrap().clone();
        assert!(
            recorded.is_empty(),
            "unexpected dropIndexes commands: {recorded:?}"
        );
        assert_eq!(after, before, "startup must retain the same index objects");
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_definitions_convert_once_and_keep_active_uniqueness() {
    let (db, drops) = monitored_db().await;
    ensure_indexes(&db).await.unwrap();
    let mut wanted = Vec::new();
    for &(collection, name) in REPLACEMENTS {
        let current = definition(&db, collection, name).await;
        let mut legacy = current.clone();
        let options = legacy.options.as_mut().unwrap();
        options.partial_filter_expression = None;
        if matches!(collection, "user_api_keys" | "user_services")
            || name.starts_with("social_provider")
        {
            options.sparse = Some(true);
        }
        if collection == "org_invites" {
            options.expire_after = Some(std::time::Duration::ZERO);
        }
        db.collection::<Document>(collection)
            .drop_index(name)
            .await
            .unwrap();
        db.collection::<Document>(collection)
            .create_index(legacy)
            .await
            .unwrap();
        wanted.push(current);
    }
    let old_objects = identities(&db).await;
    drops.lock().unwrap().clear();
    ensure_indexes(&db).await.unwrap();
    let new_objects = identities(&db).await;
    for (i, &(collection, name)) in REPLACEMENTS.iter().enumerate() {
        assert_ne!(
            old_objects[i], new_objects[i],
            "legacy {collection}.{name} replaced"
        );
        assert_eq!(
            drops
                .lock()
                .unwrap()
                .iter()
                .filter(|(c, n)| c == collection && n == name)
                .count(),
            1
        );
        let actual = definition(&db, collection, name).await;
        assert_eq!(
            bson::to_document(&actual).unwrap(),
            bson::to_document(&wanted[i]).unwrap()
        );
    }
    for (collection, key) in [
        (
            "nodes",
            doc! {"user_id":"fixture-owner", "name":"fixture-node"},
        ),
        (
            "node_service_bindings",
            doc! {"node_id":"fixture-node", "service_id":"fixture-service"},
        ),
    ] {
        let coll = db.collection::<Document>(collection);
        let row = |active| {
            let mut row = key.clone();
            row.insert("_id", uuid::Uuid::new_v4().to_string());
            row.insert("is_active", active);
            row
        };
        coll.insert_one(row(true)).await.unwrap();
        assert!(
            coll.insert_one(row(true)).await.is_err(),
            "active duplicate rejected"
        );
        coll.insert_one(row(false)).await.unwrap();
        coll.insert_one(row(false)).await.unwrap();
    }
    drops.lock().unwrap().clear();
    ensure_indexes(&db).await.unwrap();
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(identities(&db).await, new_objects);
    db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_startup_preserves_correct_indexes() {
    let drops = Drops::default();
    let recorder = drops.clone();
    let armed = Arc::new(AtomicBool::new(false));
    let gate = armed.clone();
    let (reached_tx, reached_rx) = tokio::sync::oneshot::channel();
    let reached_tx = Mutex::new(Some(reached_tx));
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let release_rx = Mutex::new(release_rx);
    let first = crate::test_utils::connect_test_database_with_command_handler(
        "idx_overlap",
        EventHandler::callback(move |event| {
            record_drop(&event, &recorder);
            if let CommandEvent::Started(event) = event
                && matches!(event.command_name.as_str(), "listIndexes" | "dropIndexes")
                && event.command.get_str(&event.command_name) == Ok("nodes")
                && gate.swap(false, Ordering::SeqCst)
            {
                reached_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                // The synchronous monitor holds the first client's command.
                // The other runtime worker can complete the second startup.
                release_rx.lock().unwrap().recv().unwrap();
            }
        }),
    )
    .await
    .unwrap();
    ensure_indexes(&first).await.unwrap();
    let before = identities(&first).await;
    let second_recorder = drops.clone();
    let second_fixture = crate::test_utils::connect_test_database_with_command_handler(
        "idx_peer",
        EventHandler::callback(move |event| record_drop(&event, &second_recorder)),
    )
    .await
    .unwrap();
    let second = second_fixture.client().database(first.name());
    drops.lock().unwrap().clear();
    armed.store(true, Ordering::SeqCst);
    let running_db = first.clone();
    let mut running = tokio::spawn(async move { ensure_indexes(&running_db).await });
    tokio::select! {
        reached = reached_rx => reached.unwrap(),
        result = &mut running => panic!("first startup did not reach its command gate: {result:?}"),
    }
    let second_result = ensure_indexes(&second).await;
    release_tx.send(()).unwrap();
    let first_result = running.await.unwrap();
    second_result.expect("overlapping second startup succeeds");
    first_result.expect("first startup resumes successfully");
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(identities(&first).await, before);
    first.drop().await.unwrap();
}

#[tokio::test]
async fn same_name_with_wrong_keys_or_constraints_is_replaced() {
    let (db, drops) = monitored_db().await;
    ensure_indexes(&db).await.unwrap();
    let collection = db.collection::<Document>("nodes");
    let name = "user_id_1_name_1";
    let wanted = definition(&db, "nodes", name).await;
    let mut reversed = wanted.clone();
    reversed.keys = doc! {"name": 1, "user_id": 1};
    let mut nonunique = wanted.clone();
    nonunique.options.as_mut().unwrap().unique = Some(false);
    let mut wrong_filter = wanted.clone();
    wrong_filter
        .options
        .as_mut()
        .unwrap()
        .partial_filter_expression = Some(doc! {"is_active": false});
    for wrong in [reversed, nonunique, wrong_filter] {
        collection.drop_index(name).await.unwrap();
        collection.create_index(wrong).await.unwrap();
        drops.lock().unwrap().clear();
        ensure_indexes(&db).await.unwrap();
        assert_eq!(*drops.lock().unwrap(), vec![("nodes".into(), name.into())]);
        let actual = definition(&db, "nodes", name).await;
        assert!(actual.keys.iter().eq(wanted.keys.iter()));
        assert_eq!(
            bson::to_document(&actual).unwrap(),
            bson::to_document(&wanted).unwrap()
        );
    }
    db.drop().await.unwrap();
}
