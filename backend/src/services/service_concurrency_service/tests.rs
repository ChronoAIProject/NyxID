use super::*;
use crate::models::service_concurrency::ConcurrencyOverride;

async fn db() -> Database {
    let db = crate::test_utils::connect_transaction_test_database("service_concurrency").await;
    ensure_indexes(&db).await.unwrap();
    db
}
fn service(limit: Option<u32>) -> DownstreamService {
    let mut service = crate::test_utils::test_auto_connected_catalog_service();
    service.concurrency_policy = Some(ServiceConcurrencyPolicy {
        service_id: service.id.clone(),
        default_limit: limit,
        users: vec![],
        orgs: vec![],
    });
    service
}
async fn released(db: &Database, service: &str, person: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = db
                .collection::<Document>(COLLECTION_NAME)
                .find_one(scope(service, person))
                .await
                .unwrap();
            if row.is_none_or(|r| r.get_array("leases").unwrap().is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn at_limit_minus_one_exactly_one_concurrent_claim_commits() {
    let db = db().await;
    let held = claim(&db, "service", "person", 2).await.unwrap();
    let results =
        futures::future::join_all((0..12).map(|_| claim(&db, "service", "person", 2))).await;
    assert_eq!(results.iter().filter(|v| v.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|v| v.as_ref().err())
            .all(|e| matches!(e, AppError::ServiceConcurrencyLimited))
    );
    drop(results);
    drop(held);
    released(&db, "service", "person").await;
}

#[tokio::test]
async fn expiry_is_reclaimed_without_ttl_sweep_and_stale_release_cannot_remove_successor() {
    let db = db().await;
    let old = claim(&db, "service", "person", 1).await.unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            scope("service", "person"),
            doc! {"$set":{"leases.0.expires_at":bson::DateTime::from_millis(1)}},
        )
        .await
        .unwrap();
    assert!(!renew(&db, &old.0.filter, &old.0.lease_id).await.unwrap());
    let current = claim(&db, "service", "person", 1).await.unwrap();
    drop(old);
    assert!(matches!(
        claim(&db, "service", "person", 1).await,
        Err(AppError::ServiceConcurrencyLimited)
    ));
    drop(current);
    released(&db, "service", "person").await;
}

#[tokio::test]
async fn lowering_a_limit_counts_all_existing_slots_and_actors_are_independent() {
    let db = db().await;
    let a = claim(&db, "service", "person", 3).await.unwrap();
    let b = claim(&db, "service", "person", 3).await.unwrap();
    assert!(matches!(
        claim(&db, "service", "person", 1).await,
        Err(AppError::ServiceConcurrencyLimited)
    ));
    let other = claim(&db, "service", "other", 1).await.unwrap();
    let other_service = claim(&db, "other", "person", 1).await.unwrap();
    drop((a, b, other, other_service));
}

#[tokio::test]
async fn bodies_hold_slots_until_completion_error_or_disconnect() {
    let db = db().await;
    for mode in ["completion", "error", "disconnect"] {
        let lease = claim(&db, "service", mode, 1).await.unwrap();
        let body = match mode {
            "completion" => Body::from("hello"),
            "error" => Body::from_stream(futures::stream::once(async {
                Err::<bytes::Bytes, _>(std::io::Error::other("failed"))
            })),
            _ => Body::from_stream(futures::stream::pending::<
                Result<bytes::Bytes, std::io::Error>,
            >()),
        };
        let response = lease.hold_response(Response::new(body));
        assert!(matches!(
            claim(&db, "service", mode, 1).await,
            Err(AppError::ServiceConcurrencyLimited)
        ));
        if mode == "disconnect" {
            drop(response);
        } else {
            let result = response.into_body().collect().await;
            assert_eq!(result.is_ok(), mode == "completion");
        }
        released(&db, "service", mode).await;
    }
}

#[tokio::test]
async fn cancellation_panic_and_renewal_loss_release_slots() {
    let db = db().await;
    let lease = claim(&db, "service", "person", 1).await.unwrap();
    let lost = lease.0.lost.clone();
    let task =
        tokio::spawn(async move { lease.run(std::future::pending::<AppResult<()>>()).await });
    lost.cancel();
    assert!(matches!(
        task.await.unwrap(),
        Err(AppError::ServiceConcurrencyLimited)
    ));
    released(&db, "service", "person").await;
    let lease = claim(&db, "service", "person", 1).await.unwrap();
    let task = tokio::spawn(async move {
        let _lease = lease;
        std::future::pending::<()>().await
    });
    task.abort();
    let _ = task.await;
    released(&db, "service", "person").await;
    let lease = claim(&db, "service", "person", 1).await.unwrap();
    let task = tokio::spawn(async move {
        let _lease = lease;
        panic!("test unwind")
    });
    assert!(task.await.unwrap_err().is_panic());
    released(&db, "service", "person").await;
}

#[tokio::test]
async fn precedence_uses_live_active_memberships_and_explicit_unlimited() {
    let db = db().await;
    for (id, active) in [("a", true), ("b", true), ("inactive", false)] {
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .insert_one(doc! {"_id":id,"user_type":"org","is_active":active})
            .await
            .unwrap();
        db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .insert_one(
                doc! {"member_user_id":"person","org_user_id":id,"revoked_at":bson::Bson::Null},
            )
            .await
            .unwrap();
    }
    let mut p = service(Some(4)).concurrency_policy.unwrap();
    p.orgs = vec![
        ConcurrencyOverride {
            id: "a".into(),
            limit: Some(2),
        },
        ConcurrencyOverride {
            id: "b".into(),
            limit: Some(8),
        },
        ConcurrencyOverride {
            id: "inactive".into(),
            limit: None,
        },
    ];
    assert_eq!(effective_limit(&db, &p, "person").await.unwrap(), Some(8));
    assert_eq!(
        effective_limit(&db, &p, "nonmember").await.unwrap(),
        Some(4)
    );
    p.users.push(ConcurrencyOverride {
        id: "person".into(),
        limit: Some(1),
    });
    assert_eq!(effective_limit(&db, &p, "person").await.unwrap(), Some(1));
    p.users[0].limit = None;
    assert_eq!(effective_limit(&db, &p, "person").await.unwrap(), None);
    p.users.clear();
    p.orgs[0].limit = None;
    assert_eq!(effective_limit(&db, &p, "person").await.unwrap(), None);
    db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_many(
            doc! {"member_user_id":"person"},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_eq!(effective_limit(&db, &p, "person").await.unwrap(), Some(4));
}

#[tokio::test]
async fn absent_policy_issues_zero_database_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let commands = Arc::new(AtomicUsize::new(0));
    let observed = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "concurrency_dormant",
        EventHandler::callback(move |event| {
            if matches!(event, CommandEvent::Started(_)) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await
    .unwrap();
    let mut svc = service(Some(1));
    svc.concurrency_policy = None;
    commands.store(0, Ordering::SeqCst);
    assert!(acquire(&db, &svc, "actor").await.unwrap().is_none());
    assert_eq!(commands.load(Ordering::SeqCst), 0);
}

#[test]
fn legacy_rows_and_validation_and_typed_error() {
    use axum::response::IntoResponse;
    let service = crate::test_utils::test_auto_connected_catalog_service();
    let mut row = bson::to_document(&service).unwrap();
    row.remove("concurrency_policy");
    assert!(
        bson::from_document::<DownstreamService>(row)
            .unwrap()
            .concurrency_policy
            .is_none()
    );
    let mut p = super::tests::service(Some(0)).concurrency_policy.unwrap();
    assert!(validate(&p).is_err());
    p.default_limit = Some(1);
    p.users.push(ConcurrencyOverride {
        id: "bad".into(),
        limit: None,
    });
    assert!(validate(&p).is_err());
    p.users[0].id = uuid::Uuid::new_v4().to_string();
    assert!(validate(&p).is_ok());
    p.orgs = p.users.clone();
    assert!(validate(&p).is_err());
    let response = AppError::ServiceConcurrencyLimited.into_response();
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "1");
}

#[tokio::test]
async fn renewal_extends_only_live_claim_and_loss_closes_an_unpolled_body() {
    let db = db().await;
    let lease = claim(&db, "service", "person", 1).await.unwrap();
    db.collection::<Document>(COLLECTION_NAME).update_one(scope("service","person"),doc! {"$set":{"leases.0.expires_at":bson::DateTime::from_millis(chrono::Utc::now().timestamp_millis()+5_000)}}).await.unwrap();
    assert!(
        renew(&db, &lease.0.filter, &lease.0.lease_id)
            .await
            .unwrap()
    );
    let row = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(scope("service", "person"))
        .await
        .unwrap()
        .unwrap();
    let until = row.get_array("leases").unwrap()[0]
        .as_document()
        .unwrap()
        .get_datetime("expires_at")
        .unwrap();
    assert!(until.timestamp_millis() > chrono::Utc::now().timestamp_millis() + 30_000);
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    struct OnDrop(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let probe = OnDrop(closed.clone());
    let body = Body::from_stream(async_stream::stream! {
        let _probe=probe;
        std::future::pending::<()>().await;
        yield Ok::<_,std::io::Error>(bytes::Bytes::new());
    });
    let lost = lease.0.lost.clone();
    let response = lease.hold_response(Response::new(body));
    lost.cancel();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !closed.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(response);
    released(&db, "service", "person").await;
}
