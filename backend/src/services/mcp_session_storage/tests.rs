use super::*;
use crate::models::mcp_session::McpSessionStore;
use crate::test_utils::connect_transaction_test_database;
use serde_json::json;

async fn marker(db: &Database, when: DateTime<Utc>) {
    db.collection::<SecurityCapabilityActivation>(ROLLOUTS)
        .insert_one(SecurityCapabilityActivation {
            id: uuid::Uuid::new_v4().to_string(),
            record_kind: RECORD_KIND.into(),
            capability: READINESS.into(),
            active_since: when,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn raw_reader_and_dual_reader_share_pre_activation_sessions() {
    let db = connect_transaction_test_database("mcp_storage_compat").await;
    let store = McpSessionStore::with_db(db.clone());
    let raw = store
        .create_with_parent_session("owner", true, true, Some("login"))
        .await
        .unwrap()
        .unwrap();
    let rows = db.collection::<McpSessionRecord>(SESSIONS);
    // The previous binary queries by the literal header and the sliding expiry.
    let old = rows
        .find_one(doc! {"_id": &raw, "expires_at":{"$gt": bson::DateTime::now()}})
        .await
        .unwrap()
        .unwrap();
    assert!(old.session_fallback_allowed);
    assert!(
        store
            .get_for_auth(&raw)
            .await
            .unwrap()
            .unwrap()
            .session_fallback_allowed
    );
    assert_eq!(migrate_batch(&db, &mut None).await.unwrap(), 0);

    marker(&db, Utc::now()).await;
    let wire = store
        .create_with_parent_session("owner", true, true, Some("login"))
        .await
        .unwrap()
        .unwrap();
    let key = hash(&wire);
    assert!(rows.find_one(doc! {"_id": &wire}).await.unwrap().is_none());
    assert!(
        store
            .get_for_auth(&wire)
            .await
            .unwrap()
            .unwrap()
            .session_fallback_allowed
    );
    assert!(store.get_for_auth(&key).await.unwrap().is_none());
    assert!(!store.touch(&key).await.unwrap());
    assert!(
        store
            .activate_services_and_notify(&wire, &["service".into()], json!({"changed":true}))
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .notifications_after(&wire, 0)
            .await
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    assert!(
        db.collection::<Document>(SLOTS)
            .find_one(doc! {"session_id": &key})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.collection::<Document>(NOTIFICATIONS)
            .find_one(doc! {"_id": format!("{key}:1"), "session_id": &key})
            .await
            .unwrap()
            .is_some()
    );
    for collection in [SESSIONS, SLOTS, NOTIFICATIONS] {
        let docs: Vec<Document> = db
            .collection(collection)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(docs.iter().all(|row| !row.to_string().contains(&wire)));
    }
    store.remove(&key).await.unwrap();
    assert!(store.validate(&wire, "owner").await.unwrap());
    store.remove(&wire).await.unwrap();
    assert!(store.get_for_auth(&wire).await.unwrap().is_none());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn migration_racing_a_touch_preserves_parent_notification_and_absolute_end() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::Arc;
    let activity = Arc::new(tokio::sync::Notify::new());
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let writes = attempts.clone();
    let signal = activity.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(command) = event
            && command.command.get_str("findAndModify") == Ok(SESSIONS)
            && command
                .command
                .get_document("update")
                .and_then(|u| u.get_document("$set"))
                .is_ok_and(|fields| fields.contains_key("last_active_at"))
        {
            writes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            signal.notify_one();
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "mcp_storage_race",
        handler,
    )
    .await;
    let store = McpSessionStore::with_db(db.clone());
    let wire = store
        .create_with_parent_session("owner", true, true, Some("parent-login"))
        .await
        .unwrap()
        .unwrap();
    store
        .send_notification(&wire, json!({"before":1}))
        .await
        .unwrap();
    let at = bson::DateTime::now().to_chrono() - Duration::hours(1);
    marker(&db, at).await;
    let rows = db.collection::<Document>(SESSIONS);
    let before = bson::DateTime::from_chrono(at - Duration::hours(1));
    rows.update_one(
        doc! {"_id":&wire},
        doc! {"$set":{"created_at":before,"last_active_at":before}},
    )
    .await
    .unwrap();
    let end = at + GRACE;
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert!(migrate_one(&db, &mut tx, &wire, end).await.unwrap());
    // Another client can still use the committed row until migration commits.
    assert!(store.validate(&wire, "owner").await.unwrap());
    let attempt = activity.notified();
    let renewal = store.touch(&wire);
    let commit = async {
        tokio::time::timeout(std::time::Duration::from_secs(10), attempt)
            .await
            .unwrap();
        tx.commit_transaction().await.unwrap();
    };
    let (renewed, ()) = tokio::join!(renewal, commit);
    assert!(renewed.unwrap());
    assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    let key = hash(&wire);
    let saved = rows.find_one(doc! {"_id":&key}).await.unwrap().unwrap();
    assert_eq!(saved.get_datetime("expires_at").unwrap().to_chrono(), end);
    assert_eq!(
        saved
            .get_datetime("absolute_expires_at")
            .unwrap()
            .to_chrono(),
        end
    );
    assert!(*saved.get_datetime("last_active_at").unwrap() > before);
    assert_eq!(saved.get_str("parent_session_id"), Ok("parent-login"));
    let slot = db
        .collection::<Document>(SLOTS)
        .find_one(doc! {"session_id": &key})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        slot.get_datetime("expires_at"),
        saved.get_datetime("expires_at")
    );
    store
        .send_notification(&wire, json!({"after":2}))
        .await
        .unwrap();
    let messages = store.notifications_after(&wire, 0).await.unwrap().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id, format!("{key}:1"));
    assert_eq!(messages[0].payload, json!({"before":1}));
    assert_eq!(messages[1].sequence, 2);
    assert!(
        store
            .get_for_auth(&wire)
            .await
            .unwrap()
            .unwrap()
            .session_fallback_allowed
    );
    for name in [SESSIONS, SLOTS, NOTIFICATIONS] {
        let docs: Vec<Document> = db
            .collection(name)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(docs.iter().all(|row| !row.to_string().contains(&wire)));
    }
    assert_eq!(migrate_batch(&db, &mut None).await.unwrap(), 0);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn seven_day_end_applies_before_and_after_sweep() {
    let db = connect_transaction_test_database("mcp_storage_deadline").await;
    let store = McpSessionStore::with_db(db.clone());
    let wire = store
        .create_with_proxy_access("owner", true, true)
        .await
        .unwrap()
        .unwrap();
    let at = Utc::now() - GRACE - Duration::seconds(1);
    db.collection::<Document>(SESSIONS)
        .update_one(
            doc! {"_id": &wire},
            doc! {"$set":{"created_at":bson::DateTime::from_chrono(at-Duration::days(1))}},
        )
        .await
        .unwrap();
    marker(&db, at).await;
    for migrated in [false, true] {
        if migrated {
            assert_eq!(migrate_batch(&db, &mut None).await.unwrap(), 1);
        }
        assert!(store.get_for_auth(&wire).await.unwrap().is_none());
        assert!(!store.validate(&wire, "owner").await.unwrap());
        assert!(!store.touch(&wire).await.unwrap());
        assert!(store.notifications_after(&wire, 0).await.unwrap().is_none());
    }
    let new_session = store.create("owner").await.unwrap().unwrap();
    assert!(store.touch(&new_session).await.unwrap());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_resolves_migrated_parent_before_backfill_and_cleanup() {
    let db = connect_transaction_test_database("mcp_migrated_recovery").await;
    let store = McpSessionStore::with_db(db.clone());
    let scanned = store.create("owner").await.unwrap().unwrap();
    store
        .send_notification(&scanned, json!({"n":1}))
        .await
        .unwrap();
    db.collection::<Document>(SLOTS)
        .delete_one(doc! {"session_id":&scanned})
        .await
        .unwrap();
    marker(&db, Utc::now()).await;
    assert_eq!(migrate_batch(&db, &mut None).await.unwrap(), 1);
    let current = recover(&db, &scanned, true).await.unwrap().unwrap();
    assert_eq!(current.id, hash(&scanned));
    assert!(
        db.collection::<Document>(SLOTS)
            .find_one(doc! {"session_id":&current.id})
            .await
            .unwrap()
            .is_some()
    );
    assert!(recover(&db, &scanned, false).await.unwrap().is_none());
    for name in [SESSIONS, SLOTS, NOTIFICATIONS] {
        assert_eq!(
            db.collection::<Document>(name)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    db.drop().await.unwrap();
}
