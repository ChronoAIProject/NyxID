use super::*;
use crate::services::admin_usage_service::{self as usage, AdminUsageQuery};
use crate::test_utils::connect_test_database;

fn row(at: DateTime<Utc>) -> Document {
    let id = uuid::Uuid::new_v4().to_string();
    doc! {
        "_id": &id, "transaction_id": format!("{id}:platform"), "billing_request_id": &id,
        "actor_user_id": "actor", "billing_owner_id": "owner", "service_id": "service", "service_slug": "service",
        "layer": "platform", "credential_class": "user_owned", "metric": "tokens", "lago_metric_code": "tokens",
        "status": "finalized", "forwarded": true, "quantity": 100_i64,
        "created_at": bson::DateTime::from_chrono(at), "token_breakdown": { "prompt_tokens": 70_i64, "completion_tokens": 30_i64 },
    }
}
async fn read(db: &Database, from: DateTime<Utc>, to: DateTime<Utc>) -> usage::AdminUsageResponse {
    usage::get_usage(
        db,
        AdminUsageQuery {
            from: Some(from.to_rfc3339()),
            to: Some(to.to_rfc3339()),
            ..Default::default()
        }
        .validate(to)
        .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn crash_after_increment_replays_without_double_count_on_standalone_protocol() {
    let db = connect_test_database("rollup_crash")
        .await
        .expect("MongoDB required");
    ensure_indexes(&db).await.unwrap();
    let now = hour(Utc::now());
    db.collection::<Document>(METERS)
        .insert_many([
            row(now - chrono::Duration::hours(2)),
            row(now - chrono::Duration::hours(1)),
        ])
        .await
        .unwrap();
    initialize(&db).await.unwrap();
    let batch = claim(&db, &state(&db).await.unwrap().unwrap(), now)
        .await
        .unwrap()
        .unwrap();
    apply(&db, &batch, None).await.unwrap();
    // Simulated process death: increments durable, source markers absent. A
    // fresh replica resumes solely from MongoDB, with no in-memory claim state.
    assert_eq!(
        db.collection::<Document>(METERS)
            .count_documents(doc! { "rollup_pending": false })
            .await
            .unwrap(),
        0
    );
    fold_once(&db, now).await.unwrap();
    fold_once(&db, now).await.unwrap();
    let result = read(&db, now - chrono::Duration::days(1), now).await;
    assert_eq!(result.totals.events, 2);
    assert_eq!(result.totals.requests, 2);
    assert_eq!(result.totals.quantities["tokens"], 200);
    assert_eq!(result.freshness.tail_rows, 0);
    // Raw TTL expiry must not erase completed-hour operational history.
    db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        2
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_replicas_fold_once_and_handle_edges_live_rows_and_late_settlement() {
    let db = connect_test_database("rollup_edges")
        .await
        .expect("MongoDB required");
    ensure_indexes(&db).await.unwrap();
    let end = hour(Utc::now());
    let start = end - chrono::Duration::hours(3);
    let mut rows = vec![];
    for minutes in [0, 10, 59, 60, 90, 120, 179, 180] {
        rows.push(row(start + chrono::Duration::minutes(minutes)));
    }
    let mut dead = row(start + chrono::Duration::minutes(95));
    dead.insert("status", "dead_letter");
    dead.insert("forwarded", false);
    let dead_id = dead.get_str("_id").unwrap().to_owned();
    rows.push(dead);
    let mut settling = row(start + chrono::Duration::minutes(100));
    settling.insert("wallet_id", "wallet");
    settling.insert("released", false);
    settling.insert("funding", doc! { "settled": false });
    let settling_id = settling.get_str("_id").unwrap().to_owned();
    rows.push(settling);
    db.collection::<Document>(METERS)
        .insert_many(rows)
        .await
        .unwrap();
    let (a, b) = tokio::join!(fold_once(&db, end), fold_once(&db, end));
    a.unwrap();
    b.unwrap();
    let result = read(&db, start + chrono::Duration::minutes(10), end).await;
    assert_eq!(result.totals.events, 7);
    assert_eq!(result.freshness.tail_rows, 1); // unsettled charged row remains live
    assert_eq!(
        db.collection::<Document>(METERS)
            .find_one(doc! { "_id": &settling_id })
            .await
            .unwrap()
            .unwrap()
            .get_bool("rollup_pending")
            .ok(),
        None
    );
    db.collection::<Document>(METERS)
        .update_one(
            doc! { "_id": dead_id },
            doc! { "$set": { "forwarded": true } },
        )
        .await
        .unwrap();
    db.collection::<Document>(METERS).update_one(doc! { "_id": settling_id }, doc! { "$set": { "released": true, "lago_acked": true, "funding": {
        "settled": true, "total_charge_micros": 123_i64, "wallet_funded_micros": 100_i64, "grant_funded_micros": 23_i64,
    } } }).await.unwrap();
    fold_once(&db, end).await.unwrap();
    fold_once(&db, end).await.unwrap();
    let result = read(&db, start + chrono::Duration::minutes(10), end).await;
    assert_eq!(result.totals.events, 8);
    assert_eq!(result.totals.gross_cost_micros, Some(123));
    assert_eq!(result.freshness.tail_rows, 0);
    assert_eq!(
        read(
            &db,
            start + chrono::Duration::minutes(11),
            start + chrono::Duration::minutes(59)
        )
        .await
        .totals
        .events,
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn aborted_transaction_leaves_sources_and_increments_uncommitted() {
    let db = crate::test_utils::connect_transaction_test_database("rollup_transaction_abort").await;
    ensure_indexes(&db).await.unwrap();
    let now = hour(Utc::now());
    db.collection::<Document>(METERS)
        .insert_one(row(now - chrono::Duration::hours(1)))
        .await
        .unwrap();
    initialize(&db).await.unwrap();
    let batch = claim(&db, &state(&db).await.unwrap().unwrap(), now)
        .await
        .unwrap()
        .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    apply(&db, &batch, Some(&mut session)).await.unwrap();
    // A reader during the active transaction sees one coherent source: the
    // original live row, never its uncommitted summary as well.
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        1
    );
    session.abort_transaction().await.unwrap();
    assert_eq!(
        db.collection::<Document>(ROLLUPS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    fold_once(&db, now).await.unwrap();
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        1
    );
    db.drop().await.unwrap();
}
