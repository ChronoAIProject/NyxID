//! Expensive fixture kept out of ordinary CI. Run explicitly with nextest's
//! `--run-ignored all -E 'test(stuck_request_inventory_production_scale)'`.
use super::*;

#[tokio::test]
#[ignore = "two million historical rows and six thousand unfinished rows"]
async fn stuck_request_inventory_production_scale() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    let raw =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    for batch in 0..200 {
        let rows: Vec<_> = (batch * 10_000..(batch + 1) * 10_000)
            .map(|i| {
                doc! {
                    "_id":uuid::Uuid::new_v4().to_string(), "transaction_id":format!("history-{i}"),
                    "billing_request_id":uuid::Uuid::new_v4().to_string(), "status":"finalized",
                    "forwarded":true, "released":true, "quantity":1000_i64,
                    "billing_owner_id":&f.owner, "reserved_credits":Credits::ZERO,
                    "metric":"tokens", "created_at":bson::DateTime::from_millis(0),
                    "padding":"x".repeat(400),
                }
            })
            .collect();
        f.db.collection::<Document>(METERS)
            .insert_many(rows)
            .await
            .unwrap();
    }
    f.db.collection::<Document>("credit_grants")
        .insert_many(
            (0..46).map(|i| doc! {"_id":format!("grant-{i}"),"recipient_user_id":&f.owner}),
        )
        .await
        .unwrap();
    f.db.collection::<Document>("usage_allowance_periods")
        .insert_many((0..1127).map(|i| doc! {"_id":format!("period-{i}"),"owner_user_id":&f.owner}))
        .await
        .unwrap();
    let rows: Vec<_> = (0..6000)
        .map(|i| {
            let mut row = raw.clone();
            row.insert("_id", uuid::Uuid::new_v4().to_string());
            row.insert("transaction_id", format!("unfinished-{i}"));
            row.insert("billing_request_id", format!("unfinished-{i}"));
            row.insert("created_at", bson::DateTime::from_millis(0));
            row.remove("forwarded_owner");
            if i % 2 == 0 {
                row.insert(
                    "forwarded_owner",
                    doc! {
                        "generation_id": if i % 4 == 0 { "live" } else { "dead" },
                        "ingress": "proxy", "marked_at": bson::DateTime::from_millis(0),
                    },
                );
            }
            row.insert(
                "reserved_credits",
                Credits::from_whole(if i % 3 == 0 { 1 } else { 0 }),
            );
            let grants = if i % 3 == 1 {
                vec![doc! {"grant_id":format!("grant-{}",i%46),"amount":Credits::from_whole(1)}]
            } else {
                vec![]
            };
            let allowances = if i % 3 == 2 {
                vec![doc! {
                    "allowance_id":"allowance", "period_id":format!("period-{}",i%1127),
                    "quantity":1000_i64,
                }]
            } else {
                vec![]
            };
            row.insert(
                "funding",
                doc! {
                    "settled": false, "grant_reservations": grants,
                    "allowance_reservations": allowances,
                },
            );
            row
        })
        .collect();
    f.db.collection::<Document>(METERS)
        .insert_many(rows)
        .await
        .unwrap();
    roster(
        &f,
        "live",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    let plan =
        f.db.run_command(doc! {
            "explain": {
                "aggregate": METERS, "pipeline": inventory::pipeline(),
                "hint": inventory::INDEX, "cursor": {},
            },
            "verbosity": "executionStats",
        })
        .await
        .unwrap();
    let json = serde_json::to_string(&plan).unwrap();
    assert!(json.contains(inventory::INDEX));
    assert!(!json.contains("COLLSCAN"), "{json}");
    println!("SCALE_PLAN {json}");
    for attempt in 0..3 {
        let start = std::time::Instant::now();
        let churn = async {
            loop {
                f.db.collection::<Document>("security_replicas")
                    .update_one(
                        doc! {"_id":"live"},
                        vec![doc! {"$set":{"observed_at":"$$NOW"}}],
                    )
                    .await
                    .unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        };
        let result = tokio::select! {
            result = diagnostics(&f.db) => result.unwrap(),
            () = churn => unreachable!(),
        };
        println!(
            "SCALE_RUST attempt={attempt} elapsed_ms={} detail={}",
            start.elapsed().as_millis(),
            result[0].detail
        );
        assert!(
            result[0].detail.starts_with("complete=true"),
            "{}",
            result[0].detail
        );
        assert!(
            result[0]
                .detail
                .contains("rows=6000 distinct_requests=6000"),
            "{}",
            result[0].detail
        );
    }
    f.db.drop().await.unwrap();
}
