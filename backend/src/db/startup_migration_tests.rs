use super::*;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

const WRITES: &[&str] = &[
    "insert",
    "update",
    "delete",
    "findAndModify",
    "commitTransaction",
];

#[derive(Clone, Debug)]
struct Write {
    command: String,
    succeeded: bool,
    failed: bool,
}

#[derive(Default)]
struct Trace {
    enabled: bool,
    writes: Vec<Write>,
    requests: HashMap<i32, usize>,
}

#[derive(Clone, Copy, Debug)]
enum Migration {
    NodeCreate,
    NodeUpdate,
    DuplicateCleanup,
}

impl Migration {
    async fn run(self, db: &Database) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::NodeCreate | Self::NodeUpdate => migrate_node_service_bindings(db).await,
            Self::DuplicateCleanup => cleanup_duplicate_migration_services(db).await,
        }
    }
}

struct Fixture {
    db: Database,
    owner: String,
    catalog: String,
    node: String,
    base: Option<String>,
}

async fn fixture(migration: Migration) -> Fixture {
    let db = crate::test_utils::connect_transaction_test_database("startup_migration_write").await;
    crate::services::service_history::relay::ensure_indexes(&db)
        .await
        .unwrap();
    for name in [USER_SERVICES, USER_API_KEYS] {
        db.collection::<Document>(name)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"source": 1, "source_id": 1})
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! {"source_id": {"$type": "string"}})
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
    }
    db.collection::<Document>(USER_SERVICES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "slug": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"is_active": true})
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    db.collection::<Document>(USER_ENDPOINTS)
        .create_index(IndexModel::builder().keys(doc! {"user_id": 1}).build())
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = uuid::Uuid::new_v4().to_string();
    catalog.slug = "migration-fixture".into();
    catalog.base_url = "https://example.invalid".into();
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    let node = uuid::Uuid::new_v4().to_string();
    let mut base = None;
    if matches!(migration, Migration::DuplicateCleanup) {
        for suffix in ["", "-2"] {
            let endpoint_id = uuid::Uuid::new_v4().to_string();
            let key_id = uuid::Uuid::new_v4().to_string();
            let now = bson::DateTime::now();
            db.collection::<Document>(USER_ENDPOINTS)
                .insert_one(doc! {
                    "_id": &endpoint_id, "user_id": &owner, "label": "Fixture",
                    "url": "https://example.invalid", "created_at": now, "updated_at": now,
                })
                .await
                .unwrap();
            db.collection::<Document>(USER_API_KEYS)
                .insert_one(doc! {
                    "_id": &key_id, "user_id": &owner, "label": "Fixture",
                    "credential_type": "node_managed", "status": "active",
                    "created_at": now, "updated_at": now,
                })
                .await
                .unwrap();
            let mut service = crate::test_utils::test_user_service(
                &uuid::Uuid::new_v4().to_string(),
                &owner,
                &format!("{}{suffix}", catalog.slug),
                &endpoint_id,
                Some(&catalog.id),
                None,
            );
            service.api_key_id = Some(key_id);
            if suffix.is_empty() {
                base = Some(service.id.clone());
            } else {
                service.source = Some("migration_connection".into());
                service.source_id = Some(uuid::Uuid::new_v4().to_string());
            }
            db.collection(USER_SERVICES)
                .insert_one(service)
                .await
                .unwrap();
        }
    } else {
        let now = Utc::now();
        db.collection(NODE_SERVICE_BINDINGS)
            .insert_one(NodeServiceBinding {
                id: uuid::Uuid::new_v4().to_string(),
                node_id: node.clone(),
                user_id: owner.clone(),
                service_id: catalog.id.clone(),
                is_active: true,
                priority: 7,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        if matches!(migration, Migration::NodeUpdate) {
            migrate_node_service_bindings(&db).await.unwrap();
            db.collection::<Document>(USER_SERVICES)
                .update_one(
                    doc! {},
                    doc! {"$set": {"node_id": bson::Bson::Null, "node_priority": 0}},
                )
                .await
                .unwrap();
        }
    }
    Fixture {
        db,
        owner,
        catalog: catalog.id,
        node,
        base,
    }
}

async fn monitored(db: &Database) -> (Database, String, Arc<Mutex<Trace>>) {
    let mut options = ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let app_name = uuid::Uuid::new_v4().to_string();
    options.app_name = Some(app_name.clone());
    let trace = Arc::new(Mutex::new(Trace {
        enabled: true,
        ..Default::default()
    }));
    let recorder = trace.clone();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        let mut trace = recorder.lock().unwrap();
        if !trace.enabled {
            return;
        }
        match event {
            CommandEvent::Started(event) if WRITES.contains(&event.command_name.as_str()) => {
                let index = trace.writes.len();
                trace.requests.insert(event.request_id, index);
                trace.writes.push(Write {
                    command: event.command_name,
                    succeeded: false,
                    failed: false,
                });
            }
            CommandEvent::Succeeded(event) => {
                if let Some(index) = trace.requests.remove(&event.request_id) {
                    trace.writes[index].succeeded = true;
                }
            }
            CommandEvent::Failed(event) => {
                if let Some(index) = trace.requests.remove(&event.request_id) {
                    trace.writes[index].failed = true;
                }
            }
            _ => {}
        }
    }));
    (
        Client::with_options(options).unwrap().database(db.name()),
        app_name,
        trace,
    )
}

async fn snapshot(db: &Database) -> Vec<Vec<Document>> {
    let mut rows = Vec::new();
    for collection in [
        USER_ENDPOINTS,
        USER_API_KEYS,
        USER_SERVICES,
        "service_change_events",
        "service_history_heads",
    ] {
        rows.push(
            db.collection::<Document>(collection)
                .find(doc! {})
                .sort(doc! {"_id": 1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap(),
        );
    }
    rows
}

async fn assert_converged(fixture: &Fixture, migration: Migration) -> Vec<String> {
    let db = &fixture.db;
    for collection in [USER_ENDPOINTS, USER_API_KEYS, USER_SERVICES] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1,
            "{migration:?}: incomplete item or orphan in {collection}"
        );
    }
    let service = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(service.user_id, fixture.owner);
    assert_eq!(
        service.catalog_service_id.as_deref(),
        Some(fixture.catalog.as_str())
    );
    let endpoint = db
        .collection::<Document>(USER_ENDPOINTS)
        .find_one(doc! {"_id": &service.endpoint_id})
        .await
        .unwrap()
        .unwrap();
    let key = db
        .collection::<Document>(USER_API_KEYS)
        .find_one(doc! {"_id": service.api_key_id.as_deref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(endpoint.get_str("user_id").unwrap(), fixture.owner);
    assert_eq!(key.get_str("user_id").unwrap(), fixture.owner);
    if let Some(base) = &fixture.base {
        assert_eq!(&service.id, base, "cleanup removed the surviving service");
    } else {
        assert_eq!(service.node_id.as_deref(), Some(fixture.node.as_str()));
        assert_eq!(service.node_priority, 7);
        assert_eq!(service.source.as_deref(), Some("migration_node_binding"));
        assert_eq!(service.source_id.as_deref(), key.get_str("source_id").ok());
    }
    let mut actions: Vec<String> = db
        .collection::<Document>("service_change_events")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get_str("action").unwrap().to_owned())
        .collect();
    actions.sort();
    assert!(!actions.is_empty(), "migration lost its journal");
    actions
}

async fn restart_at_every_write(migration: Migration) {
    let reference = fixture(migration).await;
    let (db, _, trace) = monitored(&reference.db).await;
    migration.run(&db).await.unwrap();
    let baseline = trace.lock().unwrap().writes.clone();
    assert!(
        baseline
            .iter()
            .all(|write| write.succeeded && !write.failed)
    );
    assert!(
        baseline
            .iter()
            .any(|write| write.command == "commitTransaction")
    );
    let expected_actions = assert_converged(&reference, migration).await;
    // Each cutoff rejects the next write and all subsequent writes, leaving a
    // durable prefix. The final cutoff covers a stop after acknowledged commit.
    for cutoff in 0..=baseline.len() {
        let fixture = fixture(migration).await;
        let before = snapshot(&fixture.db).await;
        let (db, app_name, trace) = monitored(&fixture.db).await;
        let admin = fixture.db.client().database("admin");
        let mode = if cutoff == 0 {
            bson::Bson::String("alwaysOn".into())
        } else {
            bson::Bson::Document(doc! {"skip": cutoff as i32})
        };
        admin
            .run_command(doc! {
                "configureFailPoint": "failCommand", "mode": mode,
                "data": {"failCommands": WRITES, "appName": app_name, "errorCode": 2},
            })
            .await
            .unwrap();
        let interrupted = migration.run(&db).await;
        admin
            .run_command(doc! {"configureFailPoint": "failCommand", "mode": "off"})
            .await
            .unwrap();
        let writes = trace.lock().unwrap().writes.clone();
        let first_failure = writes.iter().position(|write| write.failed);
        assert_eq!(
            first_failure,
            (cutoff < baseline.len()).then_some(cutoff),
            "{migration:?} cutoff {cutoff}: failure did not hit the requested write: {writes:?}"
        );
        let after_interruption = snapshot(&fixture.db).await;
        // A fresh client cannot retain the interrupted invocation's session or state.
        let restarted = Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap()
            .database(fixture.db.name());
        migration.run(&restarted).await.unwrap();
        assert_eq!(
            assert_converged(&fixture, migration).await,
            expected_actions,
            "{migration:?} cutoff {cutoff}: journal changed on restart"
        );
        migration.run(&restarted).await.unwrap();
        assert_eq!(
            assert_converged(&fixture, migration).await,
            expected_actions,
            "{migration:?} cutoff {cutoff}: second restart duplicated history"
        );
        if cutoff < baseline.len() {
            assert_eq!(
                after_interruption, before,
                "{migration:?} cutoff {cutoff}: published part of an item before commit"
            );
        } else {
            interrupted.unwrap();
        }
        println!(
            "{migration:?}: restart converged at write boundary {cutoff}/{}",
            baseline.len()
        );
        fixture.db.drop().await.unwrap();
    }
    reference.db.drop().await.unwrap();
}

#[tokio::test]
async fn startup_migration_node_binding_recovers_after_every_write() {
    restart_at_every_write(Migration::NodeCreate).await;
}

#[tokio::test]
async fn startup_migration_existing_node_route_recovers_after_every_write() {
    restart_at_every_write(Migration::NodeUpdate).await;
}

#[tokio::test]
async fn startup_migration_duplicate_cleanup_recovers_after_every_write() {
    restart_at_every_write(Migration::DuplicateCleanup).await;
}
