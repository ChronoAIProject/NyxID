use super::*;
use crate::{
    models::{
        service_change_event::{
            COLLECTION_NAME, HistoryActor, HistoryActorKind, HistoryContext, ServiceChangeEvent,
        },
        user::UserType,
        user_service::UserService,
    },
    services::org_service,
    test_utils::{
        connect_transaction_test_database, test_membership, test_user, test_user_service,
    },
};
use bson::{Document, doc};
use futures::TryStreamExt;

fn actor(id: &str, group: &str) -> HistoryContext {
    HistoryContext {
        actor: HistoryActor {
            kind: HistoryActorKind::Person,
            id: id.into(),
            name: "Editor".into(),
            person_id: Some(id.into()),
            api_key_id: None,
            app_id: None,
        },
        change_group_id: group.into(),
        operation: "test_management".into(),
    }
}
async fn events(db: &mongodb::Database, id: &str) -> Vec<ServiceChangeEvent> {
    db.collection::<ServiceChangeEvent>(COLLECTION_NAME)
        .find(doc! { "service_id": id })
        .sort(doc! { "service_sequence": 1 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn fixture() -> (mongodb::Database, UserService) {
    let db = connect_transaction_test_database("service_history").await;
    relay::ensure_indexes(&db).await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let s = test_user_service(
        &uuid::Uuid::new_v4().to_string(),
        &owner,
        "example",
        "endpoint",
        None,
        None,
    );
    db.collection::<Document>("user_endpoints").insert_one(doc! { "_id": "endpoint", "user_id": &owner, "label": "Original", "url": "https://example.com" }).await.unwrap();
    (db, s)
}

#[tokio::test]
async fn creation_unknown_fields_noop_and_safe_secret_redaction() {
    let (db, s) = fixture().await;
    let ctx = actor(&s.user_id, &uuid::Uuid::new_v4().to_string());
    context::scope(ctx.clone(), async {
        collection::<UserService>(&db, "user_services")
            .insert_one(&s)
            .await
            .unwrap();
    })
    .await;
    let created = db
        .collection::<UserService>("user_services")
        .find_one(doc! { "_id": &s.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(created.created_by.unwrap().actor.id, s.user_id);
    assert!(created.last_change.is_none());
    collection::<Document>(&db, "user_services").update_one(doc! { "_id": &s.id }, doc! { "$set": { "is_active": true, "credential_binding": "user", "updated_at": bson::DateTime::now() }, "$inc": { "state_version": 1 } }).await.unwrap();
    assert_eq!(events(&db, &s.id).await.len(), 1);
    context::scope(actor(&s.user_id, &uuid::Uuid::new_v4().to_string()), async {
        collection::<Document>(&db, "user_services").update_one(doc! { "_id": &s.id }, doc! { "$set": { "unreviewed_SECRET_FIELD": "SECRET_VALUE", "admin_only": true, "custom_user_agent": "SECRET_VALUE", "default_request_headers": [{ "name": "Authorization", "value": "SECRET_VALUE" }], "ws_frame_injections": [{ "template": "SECRET_VALUE" }] } }).await.unwrap();
    }).await;
    let events = events(&db, &s.id).await;
    assert_eq!(events.len(), 2);
    let event = events.last().unwrap();
    assert_eq!(event.action, "service.updated");
    assert!(event.additional_changes);
    assert!(event.changes.iter().any(|c| c.field == "admin_only"
        && c.before == Some(false.into())
        && c.after == Some(true.into())));
    assert!(!serde_json::to_string(event).unwrap().contains("SECRET"));
    assert!(
        !serde_json::to_string(&relay::mirror(event).unwrap())
            .unwrap()
            .contains("SECRET")
    );
    assert_eq!(event.actor.id, s.user_id);
}

#[tokio::test]
async fn caller_transaction_abort_and_plain_session_guard() {
    let (db, s) = fixture().await;
    db.collection::<UserService>("user_services")
        .insert_one(&s)
        .await
        .unwrap();
    // Plain ClientSession cannot satisfy Write::session's Transaction type.
    let result: mongodb::error::Result<()> = transaction::run(&db, async |session| {
        collection::<Document>(&db, "user_services")
            .update_one(
                doc! { "_id": &s.id },
                doc! { "$set": { "admin_only": true } },
            )
            .session(&mut *session)
            .await?;
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .session(&mut *session)
                .await?,
            1
        );
        Err(mongodb::error::Error::custom(
            "intentional abort after journal insertion",
        ))
    })
    .await;
    assert!(result.is_err());
    let live = db
        .collection::<UserService>("user_services")
        .find_one(doc! { "_id": &s.id })
        .await
        .unwrap()
        .unwrap();
    assert!(!live.admin_only);
    assert!(live.last_change.is_none());
    assert!(events(&db, &s.id).await.is_empty());
}

#[tokio::test]
async fn shared_endpoint_fanout_and_concurrent_before_after() {
    let (db, s) = fixture().await;
    let mut sibling = s.clone();
    sibling.id = uuid::Uuid::new_v4().to_string();
    sibling.slug = "sibling".into();
    db.collection::<UserService>("user_services")
        .insert_many([&s, &sibling])
        .await
        .unwrap();
    context::scope(actor(&s.user_id, &uuid::Uuid::new_v4().to_string()), async {
        collection::<Document>(&db, "user_endpoints").update_one(doc! { "_id": "endpoint" }, doc! { "$set": { "url": "https://SECRET_USER:SECRET_PASS@example.com/SECRET_PATH?SECRET_QUERY" } }).await.unwrap();
    }).await;
    for id in [&s.id, &sibling.id] {
        let e = events(&db, id).await;
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].changes[0].field, "url");
        assert!(!serde_json::to_string(&e).unwrap().contains("SECRET"));
    }
    let tasks = (1..=6).map(|_| {
        let db = db.clone();
        let id = s.id.clone();
        tokio::spawn(async move {
            collection::<Document>(&db, "user_services")
                .update_one(doc! { "_id": id }, doc! { "$inc": { "node_priority": 1 } })
                .await
                .unwrap();
        })
    });
    futures::future::join_all(tasks)
        .await
        .into_iter()
        .for_each(|r| r.unwrap());
    let e = events(&db, &s.id).await;
    let mut transitions: Vec<_> = e
        .iter()
        .flat_map(|e| &e.changes)
        .filter(|c| c.field == "node_priority")
        .map(|c| {
            (
                c.before.as_ref().unwrap().as_i64().unwrap(),
                c.after.as_ref().unwrap().as_i64().unwrap(),
            )
        })
        .collect();
    transitions.sort();
    assert_eq!(transitions, (0..6).map(|v| (v, v + 1)).collect::<Vec<_>>());
}

#[tokio::test]
async fn credential_replacement_refresh_removal_and_disabled_delete() {
    let (db, mut s) = fixture().await;
    s.api_key_id = Some("key".into());
    s.is_active = false;
    db.collection::<UserService>("user_services")
        .insert_one(&s)
        .await
        .unwrap();
    db.collection::<Document>("user_api_keys").insert_one(doc! { "_id": "key", "user_id": &s.user_id, "credential_type": "oauth2", "access_token_encrypted": "old" }).await.unwrap();
    collection::<Document>(&db, "user_api_keys").update_one(doc! { "_id": "key" }, doc! { "$set": { "access_token_encrypted": "SECRET_REFRESH", "token_scopes": "SECRET_SCOPE" } }).routine_refresh().await.unwrap();
    assert!(events(&db, &s.id).await.is_empty());
    collection::<Document>(&db, "user_api_keys").update_one(doc! { "_id": "key" }, doc! { "$set": { "access_token_encrypted": "SECRET_REPLACEMENT" }, "$inc": { "credential_epoch": 1 } }).await.unwrap();
    assert_eq!(
        events(&db, &s.id).await[0].action,
        "service.credential_replaced"
    );
    collection::<Document>(&db, "user_api_keys")
        .delete_one(doc! { "_id": "key" })
        .await
        .unwrap();
    assert_eq!(
        events(&db, &s.id).await.last().unwrap().action,
        "service.credential_removed"
    );
    crate::services::user_endpoint_service::delete_service_endpoint(
        &db,
        &s.user_id,
        "endpoint",
        std::slice::from_ref(&s.id),
    )
    .await
    .unwrap();
    assert!(
        events(&db, &s.id)
            .await
            .iter()
            .any(|e| e.action == "service.deleted")
    );
    let reader = read::Reader {
        actor_id: &s.user_id,
        allowed_service_ids: None,
    };
    assert!(
        read::list(&db, &reader, &s.id, None, &[], 20)
            .await
            .unwrap()
            .deleted
    );
}

#[tokio::test]
async fn shared_active_endpoint_cannot_be_deleted_under_sibling() {
    let (db, s) = fixture().await;
    let mut removed = s.clone();
    removed.id = uuid::Uuid::new_v4().to_string();
    removed.is_active = false;
    db.collection::<UserService>("user_services")
        .insert_many([&s, &removed])
        .await
        .unwrap();
    assert!(
        crate::services::user_endpoint_service::delete_service_endpoint(
            &db,
            &s.user_id,
            "endpoint",
            std::slice::from_ref(&removed.id)
        )
        .await
        .is_err()
    );
    assert!(
        db.collection::<Document>("user_endpoints")
            .find_one(doc! { "_id": "endpoint" })
            .await
            .unwrap()
            .is_some()
    );
    assert!(events(&db, &removed.id).await.is_empty());
}

#[tokio::test]
async fn archive_scope_roles_deleted_owner_and_group_pagination() {
    use crate::models::org_membership::OrgRole;
    let (db, mut s) = fixture().await;
    let person = s.user_id.clone();
    let org = uuid::Uuid::new_v4().to_string();
    s.user_id = org.clone();
    db.collection("users")
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection("org_memberships")
        .insert_one(test_membership(
            &org,
            &person,
            OrgRole::Admin,
            Some(vec![s.id.clone()]),
        ))
        .await
        .unwrap();
    let group = uuid::Uuid::new_v4().to_string();
    context::scope(actor(&person, &group), async {
        collection::<UserService>(&db, "user_services")
            .insert_one(&s)
            .await
            .unwrap();
        collection::<Document>(&db, "user_services")
            .update_one(
                doc! { "_id": &s.id },
                doc! { "$set": { "admin_only": true } },
            )
            .await
            .unwrap();
    })
    .await;
    collection::<UserService>(&db, "user_services")
        .delete_one(doc! { "_id": &s.id })
        .await
        .unwrap();
    let reader = read::Reader {
        actor_id: &person,
        allowed_service_ids: None,
    };
    let page = read::list(&db, &reader, &s.id, None, &[], 1).await.unwrap();
    assert!(page.deleted);
    assert_eq!(page.groups.len(), 1);
    let next = read::list(&db, &reader, &s.id, page.next_cursor.as_deref(), &[], 1)
        .await
        .unwrap();
    assert_eq!(next.groups[0].1.len(), 2);
    assert!(next.next_cursor.is_none());
    let empty_scope: Vec<String> = vec![];
    assert!(
        read::list(
            &db,
            &read::Reader {
                actor_id: &person,
                allowed_service_ids: Some(&empty_scope)
            },
            &s.id,
            None,
            &[],
            1
        )
        .await
        .is_err()
    );
    db.collection::<Document>("org_memberships")
        .update_many(
            doc! { "org_user_id": &org },
            doc! { "$set": { "role": "member" } },
        )
        .await
        .unwrap();
    assert!(!read::can_read(&db, &reader, &org, &s.id).await.unwrap());
    db.collection::<Document>("users")
        .delete_one(doc! { "_id": &org })
        .await
        .unwrap();
    assert!(!read::can_read(&db, &reader, &org, &s.id).await.unwrap());
    assert!(
        !org_service::resolve_owner_access(&db, &person, &org)
            .await
            .unwrap()
            .can_write()
    );
}

#[tokio::test]
async fn audit_append_crash_retry_and_conflicting_id() {
    let (db, s) = fixture().await;
    db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "seq": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "seq": { "$exists": true } })
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    collection::<UserService>(&db, "user_services")
        .insert_one(&s)
        .await
        .unwrap();
    let event = events(&db, &s.id).await.remove(0);
    crate::services::audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([7u8; 32]));
    let key = crate::services::audit_service::audit_chain_hmac_key().unwrap();
    let mirror = relay::mirror(&event).unwrap();
    crate::services::audit_chain_service::append_chained_entry(&db, mirror, key)
        .await
        .unwrap();
    relay::publish(&db, &event, key).await.unwrap();
    relay::publish(&db, &event, key).await.unwrap();
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(relay::verify_mirror(&db, &event).await.unwrap());
    let mut changed = event.clone();
    changed.operation = "different".into();
    assert!(relay::publish(&db, &changed, key).await.is_err());
    assert!(!relay::verify_mirror(&db, &changed).await.unwrap());
}

#[tokio::test]
async fn task_context_does_not_leak_to_parallel_requests_or_unscoped_spawn() {
    let first = actor("first", &uuid::Uuid::new_v4().to_string());
    let second = actor("second", &uuid::Uuid::new_v4().to_string());
    let (a, b) = tokio::join!(
        context::scope(first, async {
            tokio::task::yield_now().await;
            assert!(
                tokio::spawn(async { context::current() })
                    .await
                    .unwrap()
                    .is_none()
            );
            context::current().unwrap().actor.id
        }),
        context::scope(second, async {
            tokio::task::yield_now().await;
            context::current().unwrap().actor.id
        })
    );
    assert_eq!(a, "first");
    assert_eq!(b, "second");
    assert!(context::current().is_none());
}

#[tokio::test]
async fn persisted_default_null_is_not_delete_and_manual_keys_are_not_oauth() {
    let (db, mut s) = fixture().await;
    s.api_key_id = Some("key".into());
    let mut legacy = bson::to_document(&s).unwrap();
    legacy.remove("deleted_at");
    db.collection::<Document>("user_services")
        .insert_one(legacy)
        .await
        .unwrap();
    collection::<Document>(&db, "user_services")
        .update_one(
            doc! { "_id": &s.id },
            doc! { "$set": { "admin_only": true, "deleted_at": null } },
        )
        .await
        .unwrap();
    assert_eq!(events(&db, &s.id).await[0].action, "service.updated");
    db.collection::<Document>("user_api_keys").insert_one(doc! { "_id": "key", "user_id": &s.user_id, "credential_type": "api_key", "credential_encrypted": "old" }).await.unwrap();
    collection::<Document>(&db, "user_api_keys").update_one(doc! { "_id": "key" }, doc! { "$set": { "credential_encrypted": "new", "last_authorized_at": bson::DateTime::now() } }).await.unwrap();
    assert_eq!(
        events(&db, &s.id).await.last().unwrap().action,
        "service.credential_replaced"
    );
    collection::<Document>(&db, "user_api_keys").update_one(doc! { "_id": "key" }, doc! { "$set": { "credential_encrypted": null, "access_token_encrypted": null, "refresh_token_encrypted": null } }).await.unwrap();
    assert_eq!(
        events(&db, &s.id).await.last().unwrap().action,
        "service.credential_removed"
    );
    let before = events(&db, &s.id).await.len();
    crate::services::user_api_key_service::touch_last_used(&db, "key").await;
    assert_eq!(events(&db, &s.id).await.len(), before);
    assert!(
        db.collection::<Document>("user_api_keys")
            .find_one(doc! { "_id": "key" })
            .await
            .unwrap()
            .unwrap()
            .get_datetime("last_used_at")
            .is_ok()
    );
}

#[tokio::test]
async fn bulk_updates_over_previous_ceiling_and_transactional_batch_abort() {
    let (db, s) = fixture().await;
    let rows = (0..10_010)
        .map(|i| doc! { "_id": format!("bulk-{i:06}"), "user_id": &s.user_id, "label": "original" })
        .collect::<Vec<_>>();
    db.collection::<Document>("user_endpoints")
        .insert_many(rows)
        .await
        .unwrap();
    let result = collection::<Document>(&db, "user_endpoints")
        .update_many(
            doc! { "_id": { "$regex": "^bulk-" } },
            doc! { "$set": { "label": "changed" } },
        )
        .await
        .unwrap();
    assert_eq!(result.modified_count, 10_010);
    let services = (0..130)
        .map(|i| {
            let mut row = s.clone();
            row.id = uuid::Uuid::new_v4().to_string();
            row.slug = format!("batch-{i}");
            row
        })
        .collect::<Vec<_>>();
    db.collection::<UserService>("user_services")
        .insert_many(&services)
        .await
        .unwrap();
    let aborted: mongodb::error::Result<()> = transaction::run(&db, async |session| {
        let result = collection::<Document>(&db, "user_services")
            .update_many(doc! {}, doc! { "$set": { "admin_only": true } })
            .session(session)
            .await?;
        assert_eq!(result.modified_count, 130);
        Err(mongodb::error::Error::custom("abort all batches"))
    })
    .await;
    assert!(aborted.is_err());
    assert_eq!(
        db.collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>("user_services")
            .count_documents(doc! { "admin_only": true })
            .await
            .unwrap(),
        0
    );
    let group = uuid::Uuid::new_v4().to_string();
    context::scope(actor(&s.user_id, &group), async {
        collection::<Document>(&db, "user_services")
            .update_many(doc! {}, doc! { "$set": { "admin_only": true } })
            .await
            .unwrap();
    })
    .await;
    assert_eq!(
        db.collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! { "change_group_id": &group })
            .await
            .unwrap(),
        130
    );
}

#[tokio::test]
async fn audit_legacy_corruption_and_poison_backoff_do_not_starve_new_events() {
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT};
    let (db, s) = fixture().await;
    crate::services::audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([7; 32]));
    let key = crate::services::audit_service::audit_chain_hmac_key().unwrap();
    collection::<UserService>(&db, "user_services")
        .insert_one(&s)
        .await
        .unwrap();
    let poisoned = events(&db, &s.id).await.remove(0);
    db.collection::<AuditLog>(AUDIT)
        .insert_one(relay::mirror(&poisoned).unwrap())
        .await
        .unwrap();
    assert!(relay::publish(&db, &poisoned, key).await.is_err());
    assert!(!relay::verify_mirror(&db, &poisoned).await.unwrap());
    collection::<Document>(&db, "user_services")
        .update_one(
            doc! { "_id": &s.id },
            doc! { "$set": { "admin_only": true } },
        )
        .await
        .unwrap();
    relay::sweep(&db).await.unwrap();
    let rows = events(&db, &s.id).await;
    let failed = rows.iter().find(|e| e.id == poisoned.id).unwrap();
    assert_eq!(failed.audit_attempts, 1);
    assert!(failed.next_audit_attempt_at.is_some());
    let published = rows.iter().find(|e| e.id != poisoned.id).unwrap();
    assert!(published.audited_at.is_some());
    assert!(relay::verify_mirror(&db, published).await.unwrap());
    db.collection::<Document>(AUDIT)
        .update_one(
            doc! { "_id": &published.id },
            doc! { "$set": { "entry_hash": "corrupt" } },
        )
        .await
        .unwrap();
    assert!(!relay::verify_mirror(&db, published).await.unwrap());
    assert!(relay::publish(&db, published, key).await.is_err());
}

#[tokio::test]
async fn unknown_nested_fields_survive_default_normalization_without_disclosure() {
    let (db, s) = fixture().await;
    let mut row = bson::to_document(&s).unwrap();
    row.insert("ws_frame_injections", vec![doc! { "trigger": "first_frame_from_downstream", "template": "SECRET_PAYLOAD", "future_SECRET_FIELD": "SECRET_BEFORE" }]);
    db.collection::<Document>("user_services")
        .insert_one(row)
        .await
        .unwrap();
    collection::<Document>(&db,"user_services").update_one(doc! { "_id": &s.id },doc! { "$set": { "ws_frame_injections.0.direction": "downstream", "ws_frame_injections.0.consume_trigger": true, "ws_frame_injections.0.frame_kind": "text" } }).await.unwrap();
    assert!(events(&db, &s.id).await.is_empty());
    collection::<Document>(&db, "user_services")
        .update_one(
            doc! { "_id": &s.id },
            doc! { "$set": { "ws_frame_injections.0.future_SECRET_FIELD": "SECRET_AFTER" } },
        )
        .await
        .unwrap();
    let rows = events(&db, &s.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "service.updated");
    assert!(!serde_json::to_string(&rows).unwrap().contains("SECRET"));
}

#[tokio::test]
async fn retried_caller_transaction_keeps_event_uuid_and_atomic_sequence() {
    let (db, s) = fixture().await;
    db.collection::<UserService>("user_services")
        .insert_one(&s)
        .await
        .unwrap();
    let observed = std::sync::Mutex::new(Vec::new());
    context::scope(
        actor(&s.user_id, &uuid::Uuid::new_v4().to_string()),
        async {
            transaction::run(&db, async |session| {
                collection::<Document>(&db, "user_services")
                    .update_one(
                        doc! { "_id": &s.id },
                        doc! { "$inc": { "node_priority": 1 } },
                    )
                    .session(&mut *session)
                    .await?;
                let event = db
                    .collection::<ServiceChangeEvent>(COLLECTION_NAME)
                    .find_one(doc! { "service_id": &s.id })
                    .session(&mut *session)
                    .await?
                    .unwrap();
                let mut observed = observed.lock().unwrap();
                observed.push(event.id);
                if observed.len() == 1 {
                    return Err(mongodb::error::Error::custom(
                        transaction::ConcurrentHeadCreation,
                    ));
                }
                Ok(())
            })
            .await
            .unwrap();
        },
    )
    .await;
    {
        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0], observed[1]);
    }
    let rows = events(&db, &s.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].service_sequence, 1);
}

#[tokio::test]
async fn archived_discovery_respects_current_roles_scopes_and_uuid_identity() {
    use crate::models::org_membership::OrgRole;
    let (db, mut first) = fixture().await;
    let actor_id = first.user_id.clone();
    let org = uuid::Uuid::new_v4().to_string();
    first.user_id = org.clone();
    db.collection("users")
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection("org_memberships")
        .insert_one(test_membership(&org, &actor_id, OrgRole::Admin, None))
        .await
        .unwrap();
    let mut second = first.clone();
    second.id = uuid::Uuid::new_v4().to_string();
    for service in [&first, &second] {
        collection::<UserService>(&db, "user_services")
            .insert_one(service)
            .await
            .unwrap();
        collection::<UserService>(&db, "user_services")
            .delete_one(doc! { "_id": &service.id })
            .await
            .unwrap();
    }
    let reader = read::Reader {
        actor_id: &actor_id,
        allowed_service_ids: None,
    };
    let page = read::archived(&db, &reader, None, 1).await.unwrap();
    assert_eq!(page.services.len(), 1);
    let next = read::archived(&db, &reader, page.next_cursor.as_deref(), 1)
        .await
        .unwrap();
    assert_eq!(next.services.len(), 1);
    assert_ne!(page.services[0].service_id, next.services[0].service_id);
    assert_eq!(page.services[0].service_slug, next.services[0].service_slug);
    db.collection::<Document>("org_memberships")
        .update_one(
            doc! { "org_user_id": &org },
            doc! { "$set": { "allowed_service_ids": [&first.id] } },
        )
        .await
        .unwrap();
    assert_eq!(
        read::archived(&db, &reader, None, 20)
            .await
            .unwrap()
            .services[0]
            .service_id,
        first.id
    );
    assert_eq!(
        read::archived(&db, &reader, None, 20)
            .await
            .unwrap()
            .services
            .len(),
        1
    );
    let denied_ids = vec![second.id.clone()];
    let restricted = read::Reader {
        actor_id: &actor_id,
        allowed_service_ids: Some(&denied_ids),
    };
    assert!(
        read::archived(&db, &restricted, None, 20)
            .await
            .unwrap()
            .services
            .is_empty()
    );
    let allowed_ids = vec![first.id.clone()];
    let permitted = read::Reader {
        actor_id: &actor_id,
        allowed_service_ids: Some(&allowed_ids),
    };
    assert_eq!(
        read::archived(&db, &permitted, None, 20)
            .await
            .unwrap()
            .services
            .len(),
        1
    );
    db.collection::<Document>("users")
        .update_one(
            doc! { "_id": &org },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert!(
        read::archived(&db, &reader, None, 20)
            .await
            .unwrap()
            .services
            .is_empty()
    );
    db.collection::<Document>("users")
        .update_one(doc! { "_id": &org }, doc! { "$set": { "is_active": true } })
        .await
        .unwrap();
    for role in ["member", "viewer"] {
        db.collection::<Document>("org_memberships")
            .update_one(
                doc! { "org_user_id": &org },
                doc! { "$set": { "role": role } },
            )
            .await
            .unwrap();
        assert!(
            read::archived(&db, &reader, None, 20)
                .await
                .unwrap()
                .services
                .is_empty()
        );
    }
    db.collection::<Document>("org_memberships")
        .update_one(
            doc! { "org_user_id": &org },
            doc! { "$set": { "role": "admin", "revoked_at": bson::DateTime::now() } },
        )
        .await
        .unwrap();
    assert!(
        read::archived(&db, &reader, None, 20)
            .await
            .unwrap()
            .services
            .is_empty()
    );
}

#[tokio::test]
async fn independent_backing_writes_race_first_history_head() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (db, mut s) = fixture().await;
    s.api_key_id = Some("key".into());
    db.collection::<UserService>("user_services")
        .insert_one(&s)
        .await
        .unwrap();
    db.collection::<Document>("user_api_keys")
        .insert_one(doc! { "_id": "key", "user_id": &s.user_id, "label": "Before" })
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let jobs = [("user_endpoints", "endpoint"), ("user_api_keys", "key")].map(|(entity, id)| {
        let db = db.clone();
        let service_id = s.id.clone();
        let barrier = barrier.clone();
        async move {
            let first = AtomicBool::new(true);
            transaction::run(&db, async |session| {
                if first.swap(false, Ordering::SeqCst) {
                    let head = db
                        .collection::<Document>(
                            crate::models::service_change_event::HEADS_COLLECTION_NAME,
                        )
                        .find_one(doc! { "_id": &service_id })
                        .session(&mut *session)
                        .await?;
                    assert!(head.is_none());
                    barrier.wait().await;
                }
                collection::<Document>(&db, entity)
                    .update_one(doc! { "_id": id }, doc! { "$set": { "label": "After" } })
                    .session(&mut *session)
                    .await?;
                Ok(())
            })
            .await
            .unwrap();
        }
    });
    let [endpoint, key] = jobs;
    tokio::join!(endpoint, key);
    let rows = events(&db, &s.id).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().map(|e| e.service_sequence).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_ne!(rows[0].entity_type, rows[1].entity_type);
    assert!(rows.iter().all(|e| e.action == "service.updated"));
}

#[tokio::test]
async fn journal_insert_failure_aborts_business_change_and_summary() {
    let (db, s) = fixture().await;
    db.collection::<UserService>("user_services")
        .insert_one(&s)
        .await
        .unwrap();
    db.run_command(doc! { "collMod": COLLECTION_NAME, "validator": { "schema_version": -1 } })
        .await
        .unwrap();
    assert!(
        collection::<Document>(&db, "user_services")
            .update_one(
                doc! { "_id": &s.id },
                doc! { "$set": { "admin_only": true } }
            )
            .await
            .is_err()
    );
    let live = db
        .collection::<UserService>("user_services")
        .find_one(doc! { "_id": &s.id })
        .await
        .unwrap()
        .unwrap();
    assert!(!live.admin_only);
    assert!(live.last_change.is_none());
    assert!(events(&db, &s.id).await.is_empty());
}

#[tokio::test]
async fn backing_active_field_is_not_service_lifecycle() {
    let (db, s) = fixture().await;
    db.collection::<UserService>("user_services")
        .insert_one(&s)
        .await
        .unwrap();
    collection::<Document>(&db, "user_endpoints")
        .update_one(
            doc! { "_id": "endpoint" },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert_eq!(events(&db, &s.id).await[0].action, "service.updated");
}

#[tokio::test]
async fn oversized_operation_remains_whole_and_late_commits_do_not_move_page_anchor() {
    let (db, s) = fixture().await;
    collection::<UserService>(&db, "user_services")
        .insert_one(&s)
        .await
        .unwrap();
    let template = events(&db, &s.id).await.remove(0);
    let mut rows = Vec::new();
    for sequence in 2..=10003 {
        let mut event = template.clone();
        event.id = uuid::Uuid::new_v4().to_string();
        event.service_sequence = sequence;
        rows.push(event);
    }
    let mut newer = template.clone();
    newer.id = uuid::Uuid::new_v4().to_string();
    newer.service_sequence = 10004;
    newer.change_group_id = uuid::Uuid::new_v4().to_string();
    rows.push(newer.clone());
    db.collection::<ServiceChangeEvent>(COLLECTION_NAME)
        .insert_many(rows)
        .await
        .unwrap();
    let reader = read::Reader {
        actor_id: &s.user_id,
        allowed_service_ids: None,
    };
    let page = read::list(&db, &reader, &s.id, None, &[], 1).await.unwrap();
    assert_eq!(page.groups[0].0, newer.change_group_id);
    let mut late = template.clone();
    late.id = uuid::Uuid::new_v4().to_string();
    late.service_sequence = 10005;
    db.collection::<ServiceChangeEvent>(COLLECTION_NAME)
        .insert_one(late)
        .await
        .unwrap();
    let older = read::list(&db, &reader, &s.id, page.next_cursor.as_deref(), &[], 1)
        .await
        .unwrap();
    assert_eq!(older.groups.len(), 1);
    assert_eq!(older.groups[0].1.len(), 10004);
    assert!(older.next_cursor.is_none());
}

#[tokio::test]
async fn concurrent_reference_creation_and_backing_edit_are_serialized() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (db, s) = fixture().await;
    let inserted = tokio::sync::Notify::new();
    let edited = tokio::sync::Notify::new();
    let first = AtomicBool::new(true);
    let creation = transaction::run(&db, async |session| {
        // Establish the snapshot before the endpoint edit; the first write must
        // conflict with that edit, retry, and create against its final state.
        let endpoint = db
            .collection::<Document>("user_endpoints")
            .find_one(doc! { "_id": "endpoint" })
            .session(&mut *session)
            .await?
            .unwrap();
        if first.swap(false, Ordering::SeqCst) {
            inserted.notify_one();
            edited.notified().await;
        } else {
            assert_eq!(endpoint.get_str("label").unwrap(), "Updated concurrently");
        }
        collection::<UserService>(&db, "user_services")
            .insert_one(&s)
            .session(&mut *session)
            .await?;
        Ok(())
    });
    let edit = async {
        inserted.notified().await;
        collection::<Document>(&db, "user_endpoints")
            .update_one(
                doc! { "_id": "endpoint" },
                doc! { "$set": { "label": "Updated concurrently" } },
            )
            .await
            .unwrap();
        edited.notify_one();
    };
    let (result, ()) = tokio::join!(creation, edit);
    result.unwrap();
    let rows = events(&db, &s.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "service.created");
    let endpoint = db
        .collection::<Document>("user_endpoints")
        .find_one(doc! { "_id": "endpoint" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(endpoint.get_i64("service_history_ref_epoch").unwrap(), 1);
}

#[tokio::test]
async fn new_and_rebound_references_are_included_in_retried_backing_fanout() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for create in [true, false] {
        for (field, backing) in [
            ("endpoint_id", "user_endpoints"),
            ("api_key_id", "user_api_keys"),
        ] {
            let (db, s) = fixture().await;
            db.collection::<Document>(backing)
                .insert_one(doc! { "_id": "new-backing", "user_id": &s.user_id, "label": "Before" })
                .await
                .unwrap();
            if !create {
                db.collection::<UserService>("user_services")
                    .insert_one(&s)
                    .await
                    .unwrap();
            }
            let snapshot_ready = tokio::sync::Notify::new();
            let reference_ready = tokio::sync::Notify::new();
            let first = AtomicBool::new(true);
            let edit = transaction::run(&db, async |session| {
                db.collection::<Document>(backing)
                    .find_one(doc! { "_id": "new-backing" })
                    .session(&mut *session)
                    .await?;
                if first.swap(false, Ordering::SeqCst) {
                    snapshot_ready.notify_one();
                    reference_ready.notified().await;
                }
                collection::<Document>(&db, backing)
                    .update_one(
                        doc! { "_id": "new-backing" },
                        doc! { "$set": { "label": "After" } },
                    )
                    .session(&mut *session)
                    .await?;
                Ok(())
            });
            let reference = async {
                snapshot_ready.notified().await;
                if create {
                    let mut service = bson::to_document(&s).unwrap();
                    service.insert(field, "new-backing");
                    collection::<Document>(&db, "user_services")
                        .insert_one(service)
                        .await
                        .unwrap();
                } else {
                    collection::<Document>(&db, "user_services")
                        .update_one(
                            doc! { "_id": &s.id },
                            doc! { "$set": { field: "new-backing" } },
                        )
                        .await
                        .unwrap();
                }
                reference_ready.notify_one();
            };
            let (result, ()) = tokio::join!(edit, reference);
            result.unwrap();
            let rows = events(&db, &s.id).await;
            assert_eq!(
                rows.len(),
                2,
                "one reference mutation and one backing change: {field}, create={create}"
            );
            assert_eq!(rows[1].entity_type, backing);
            let service = db
                .collection::<UserService>("user_services")
                .find_one(doc! { "_id": &s.id })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                service.last_change.unwrap().change_group_id,
                rows[1].change_group_id
            );
            assert_eq!(service.created_by.is_some(), create);
        }
    }
}
