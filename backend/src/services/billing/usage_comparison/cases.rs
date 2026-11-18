use super::*;
use crate::models::billing_usage_delivery::COLLECTION_NAME as DELIVERIES;

#[tokio::test]
async fn usage_comparison_pinned_evidence_and_lease_fence_discard_races() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            for (collection, filter, update) in [
                (
                    COLLECTION_NAME,
                    doc! {"_id":"sub"},
                    doc! {"$inc":{"acknowledgement_sequence":1_i64}},
                ),
                (
                    COLLECTION_NAME,
                    doc! {"_id":"sub"},
                    doc! {"$inc":{"uncertainty_revision":1_i64}},
                ),
                (
                    CONTROL_COLLECTION,
                    doc! {"_id":COVERAGE_ID},
                    doc! {"$set":{"generation":"successor"}},
                ),
                (
                    crate::models::coordination::LEASE_COLLECTION_NAME,
                    doc! {},
                    doc! {"$set":{"lease_id":"successor"}},
                ),
            ] {
                let f = Fixture::new().await;
                f.target("sub", 0).await;
                f.provider.hooks.lock().unwrap().push(Hook::Update(
                    f.db.clone(),
                    collection,
                    filter,
                    update,
                ));
                let (stats, result) = f.pass().await;
                result.unwrap();
                assert_eq!(stats.drift_alerts, 0);
                let target =
                    f.db.collection::<Document>(COLLECTION_NAME)
                        .find_one(doc! {"_id":"sub"})
                        .await
                        .unwrap()
                        .unwrap();
                assert!(
                    !target.contains_key("comparison"),
                    "stale publication must not commit"
                );
                assert_eq!(f.provider.calls.lock().unwrap().len(), 1);
                f.close().await;
            }
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_period_rollover_and_non_sum_aggregation_never_report_drift() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 1).await;
            f.receipt("sub", 1, initial_now().to_chrono().timestamp(), "2")
                .await;
            let mut non_sum = usage("1");
            non_sum["customer_usage"]["charges_usage"][0]["billable_metric"]["aggregation_type"] =
                serde_json::json!("count_agg");
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(non_sum));
            f.pass().await.1.unwrap();
            assert_eq!(f.observation("sub").await.status, "unsupported");
            assert_eq!(
                f.observation("sub").await.reason,
                "non_sum_aggregation:count_agg"
            );
            assert!(
                diagnostics(&f.db)
                    .await
                    .unwrap()
                    .iter()
                    .any(|d| d.code == "billing_usage_unsupported")
            );
            let mut missing = usage("1");
            missing["customer_usage"]["charges_usage"] = serde_json::json!([]);
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(missing));
            f.advance(86400).await;
            f.pass().await.1.unwrap();
            assert_eq!(
                f.observation("sub").await.reason,
                "delivered_metric_missing"
            );
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(usage("1")));
            f.advance(86400).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.observation("sub").await.reason, "projection_backlog");
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.observation("sub").await.status, "pending");
            f.advance(600).await;
            let before = bson::to_document(&f.observation("sub").await).unwrap();
            let calls_before = f.provider.calls.lock().unwrap().len();
            let end = at("2026-12-01T00:00:00Z");
            f.provider.hooks.lock().unwrap().extend([
                Hook::Clock(end),
                Hook::Update(
                    f.db.clone(),
                    CONTROL_COLLECTION,
                    doc! {"_id":COVERAGE_ID},
                    doc! {"$set":{"observed_at":end}},
                ),
            ]);
            let (stats, result) = f.pass().await;
            result.unwrap();
            assert_eq!(stats.drift_alerts, 0);
            assert_eq!(f.provider.calls.lock().unwrap().len(), calls_before + 1);
            assert_eq!(
                bson::to_document(&f.observation("sub").await).unwrap(),
                before,
                "the database-time guard discards a publication after the period ends"
            );
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_rebuild_paginates_and_late_ack_uses_event_period() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            projection::TEST_PAGE_SIZE
                .scope(3, async {
                    let f = Fixture::new().await;
                    f.target("sub", 4).await;
                    let second = initial_now().to_chrono().timestamp();
                    for seq in 1..=4 {
                        f.receipt("sub", seq, second, "0.25").await;
                    }
                    f.provider
                        .replies
                        .lock()
                        .unwrap()
                        .insert("sub".into(), Reply::Usage(usage("1")));
                    f.pass().await.1.unwrap();
                    assert_eq!(f.observation("sub").await.reason, "projection_backlog");
                    f.advance(300).await;
                    f.pass().await.1.unwrap();
                    assert_eq!(f.observation("sub").await.status, "equal");
                    // A late acknowledgement belongs to its immutable event second, not its acknowledgement time.
                    f.receipt(
                        "sub",
                        5,
                        at("2026-10-31T23:59:59Z").to_chrono().timestamp(),
                        "50",
                    )
                    .await;
                    f.receipt(
                        "sub",
                        6,
                        at("2026-11-01T00:00:00Z").to_chrono().timestamp(),
                        "0.5",
                    )
                    .await;
                    f.db.collection::<Document>(COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":"sub"},
                            doc! {"$set":{"acknowledgement_sequence":6_i64}},
                        )
                        .await
                        .unwrap();
                    f.provider
                        .replies
                        .lock()
                        .unwrap()
                        .insert("sub".into(), Reply::Usage(usage("1.5")));
                    f.advance(6 * 3600).await;
                    f.pass().await.1.unwrap();
                    let observation = f.observation("sub").await;
                    assert_eq!(observation.status, "equal");
                    assert_eq!(observation.local["requests"], "1.5");
                    // Missing incremental history is one failed unit, never a zero quantity.
                    f.db.collection::<Document>(COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":"sub"},
                            doc! {"$set":{"acknowledgement_sequence":7_i64}},
                        )
                        .await
                        .unwrap();
                    f.advance(6 * 3600).await;
                    assert!(matches!(
                        f.pass().await.1,
                        Err(AppError::BillingRecoveryFailed {
                            failed_units: 1,
                            ..
                        })
                    ));
                    assert_eq!(
                        f.observation("sub").await.reason,
                        "receipt_projection_failed"
                    );
                    f.close().await;
                })
                .await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_only_sequenced_acknowledgements_enter_totals() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 1).await;
            for seq in 1..=4 {
                f.receipt("sub", seq, initial_now().to_chrono().timestamp(), "1")
                    .await;
            }
            for (seq, state) in [(2, "prepared"), (3, "rejected"), (4, "acknowledged")] {
                f.db.collection::<Document>(DELIVERIES)
                    .update_one(
                        doc! {"transaction_id":format!("sub:{seq}")},
                        doc! {"$set":{"state":state,"acknowledgement_sequence":Bson::Null}},
                    )
                    .await
                    .unwrap();
            }
            f.pass().await.1.unwrap();
            assert_eq!(
                f.observation("sub").await.reason,
                "unresolved_deliveries_in_period"
            );
            // A reopened rejection is still absent until the delivery acknowledgement assigns a sequence.
            f.db.collection::<Document>(DELIVERIES)
                .update_one(
                    doc! {"transaction_id":"sub:3"},
                    doc! {"$set":{"state":"prepared"}},
                )
                .await
                .unwrap();
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(
                f.observation("sub").await.reason,
                "unresolved_deliveries_in_period"
            );
            f.db.collection::<Document>(DELIVERIES)
                .update_one(
                    doc! {"transaction_id":"sub:2"},
                    doc! {"$set":{"state":"rejected"}},
                )
                .await
                .unwrap();
            f.db.collection::<Document>(DELIVERIES)
                .update_one(
                    doc! {"transaction_id":"sub:3"},
                    doc! {"$set":{"state":"acknowledged","acknowledgement_sequence":2_i64}},
                )
                .await
                .unwrap();
            f.db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":"sub"},
                    doc! {"$set":{"acknowledgement_sequence":2_i64}},
                )
                .await
                .unwrap();
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(usage("2")));
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.observation("sub").await.status, "equal");
            assert_eq!(f.observation("sub").await.local["requests"], "2");
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_uncertainty_warms_until_next_whole_period() {
    TEST_NOW.scope(std::cell::Cell::new(initial_now()), async {
        let f = Fixture::new().await;
        f.target("sub", 0).await;
        f.db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":"sub"}, doc! {"$set":{"coverage_unknown_reason":"delivery_acknowledged_outside_capture"}}).await.unwrap();
        f.pass().await.1.unwrap();
        assert_eq!(f.observation("sub").await.reason, "capture_uncertainty_in_period");
        let target = f.db.collection::<BillingUsageComparison>(COLLECTION_NAME).find_one(doc! {"_id":"sub"}).await.unwrap().unwrap();
        assert_eq!(target.uncertainty_revision, 1);
        assert_eq!(target.uncertain_through, Some(initial_now().to_chrono()));
        f.advance(20*86400).await;
        let mut next = usage("0");
        next["customer_usage"]["from_datetime"] = serde_json::json!("2026-12-01T00:00:00Z");
        next["customer_usage"]["to_datetime"] = serde_json::json!("2026-12-31T23:59:59Z");
        f.provider.replies.lock().unwrap().insert("sub".into(), Reply::Usage(next));
        f.pass().await.1.unwrap();
        assert_eq!(f.observation("sub").await.status, "equal");
        f.close().await;
    }).await;
}

#[tokio::test]
async fn usage_comparison_changed_totals_failure_and_same_pass_cannot_confirm() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 0).await;
            f.pass().await.1.unwrap();
            f.advance(600).await;
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(usage("2")));
            assert_eq!(f.pass().await.0.drift_alerts, 0);
            f.advance(600).await;
            assert_eq!(f.pass().await.0.drift_alerts, 1);
            f.advance(600).await;
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Error);
            assert!(f.pass().await.1.is_err());
            assert!(f.observation("sub").await.stale_drift);
            f.advance(600).await;
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(usage("2")));
            assert_eq!(
                f.pass().await.0.drift_alerts,
                0,
                "failure breaks confirmation"
            );
            let target =
                f.db.collection::<BillingUsageComparison>(COLLECTION_NAME)
                    .find_one(doc! {"_id":"sub"})
                    .await
                    .unwrap()
                    .unwrap();
            let coverage = coverage::refresh(&f.db).await.unwrap();
            let parsed =
                super::super::super::lago_client::current_usage::parse(&usage("2")).unwrap();
            let previous = target.comparison.as_ref().unwrap();
            let same = observation::complete(
                &target,
                &coverage,
                target.projection.as_ref().unwrap(),
                &parsed,
                previous.observed_at + chrono::Duration::minutes(20),
                &previous.pass_id,
            )
            .unwrap();
            assert_eq!(same.status, "pending");
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_no_subscription_no_read_and_discovery_rotates() {
    TEST_NOW.scope(std::cell::Cell::new(initial_now()), async {
        let f = Fixture::new().await;
        f.pass().await.1.unwrap();
        assert!(f.provider.calls.lock().unwrap().is_empty());
        let wallets = f.db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME);
        for n in 0..101 {
            wallets.insert_one(doc! {"_id":format!("wallet-{n:03}"),"owner_id":"owner","lago_customer_id":"customer", "lago_subscription_id":if n == 100 { "last" } else { " " }}).await.unwrap();
        }
        assert!(schedule::discover(&f.db).await.unwrap().is_empty());
        assert_eq!(f.db.collection::<Document>(COLLECTION_NAME).count_documents(doc! {}).await.unwrap(), 0);
        assert!(schedule::discover(&f.db).await.unwrap().is_empty());
        assert_eq!(f.db.collection::<Document>(COLLECTION_NAME).count_documents(doc! {}).await.unwrap(), 1);
        f.close().await;
    }).await;
}

#[tokio::test]
async fn usage_comparison_quiet_retention_and_daily_clock_boundaries() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 0).await;
            let now = initial_now();
            f.db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":"sub"},
                    doc! {"$set":{"last_acknowledged_at":now}},
                )
                .await
                .unwrap();
            f.pass().await.1.unwrap();
            assert_eq!(
                f.observation("sub").await.reason,
                "acknowledgements_not_quiet"
            );
            f.advance(600).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.observation("sub").await.status, "pending");
            let target =
                f.db.collection::<BillingUsageComparison>(COLLECTION_NAME)
                    .find_one(doc! {"_id":"sub"})
                    .await
                    .unwrap()
                    .unwrap();
            let coverage = coverage::refresh(&f.db).await.unwrap();
            let mut period =
                super::super::super::lago_client::current_usage::parse(&usage("1")).unwrap();
            period.start_second = now.to_chrono().timestamp() - 100 * 86400;
            let mut old_coverage = coverage.clone();
            old_coverage.continuous_since = Some(now.to_chrono() - chrono::Duration::days(101));
            assert_eq!(
                coverage::eligible(&target, &old_coverage, &period, now.to_chrono()),
                Err("receipt_history_outside_retention")
            );
            period.start_second += 1;
            assert!(coverage::eligible(&target, &old_coverage, &period, now.to_chrono()).is_ok());
            // UTC day rollover allows the next day's budget; backward time does not.
            let requested = 1;
            f.db.collection::<Document>(CONTROL_COLLECTION)
                .update_one(
                    doc! {"_id":SCHEDULE_ID},
                    doc! {"$set":{"day_reads":5760_i64}},
                )
                .await
                .unwrap();
            assert_eq!(schedule::reserve(&f.db, requested).await.unwrap(), 0);
            TEST_NOW.with(|clock| clock.set(at("2026-11-16T00:00:00Z")));
            assert_eq!(schedule::reserve(&f.db, requested).await.unwrap(), 1);
            f.db.collection::<Document>(CONTROL_COLLECTION)
                .update_one(
                    doc! {"_id":SCHEDULE_ID},
                    doc! {"$set":{"day_reads":5760_i64}},
                )
                .await
                .unwrap();
            TEST_NOW.with(|clock| clock.set(initial_now()));
            assert_eq!(schedule::reserve(&f.db, requested).await.unwrap(), 0);
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_http_contract_caps_body_and_classifies_wait() {
    use super::super::super::lago_client::{LagoClient, current_usage};
    use axum::{Router, routing::get};
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let app = Router::new().route(
        "/api/v1/customers/customer/current_usage",
        get(
            move |axum::extract::Query(query): axum::extract::Query<BTreeMap<String, String>>| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let subscription = query
                        .get("external_subscription_id")
                        .expect("subscription query parameter");
                    match subscription.as_str() {
                        "wait" => (axum::http::StatusCode::TOO_MANY_REQUESTS, String::new()),
                        "fail" => (axum::http::StatusCode::SERVICE_UNAVAILABLE, String::new()),
                        "missing" => (axum::http::StatusCode::NOT_FOUND, String::new()),
                        "rejected" => (axum::http::StatusCode::UNPROCESSABLE_ENTITY, String::new()),
                        "large" => (axum::http::StatusCode::OK, "x".repeat(1024 * 1024 + 1)),
                        "valid" => (axum::http::StatusCode::OK, usage("1.25").to_string()),
                        _ => panic!("unexpected subscription"),
                    }
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = LagoClient::new(format!("http://{address}"), "fixture-key".into()).unwrap();
    assert!(
        client
            .current_usage_for_comparison("customer", " ")
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        client
            .current_usage_for_comparison("customer", "wait")
            .await,
        Err(AppError::RateLimited)
    ));
    assert!(matches!(
        client
            .current_usage_for_comparison("customer", "fail")
            .await,
        Err(AppError::BillingProviderUnavailable(_))
    ));
    for (subscription, expected) in [
        ("missing", Finding::SubscriptionUnavailable),
        ("rejected", Finding::RequestRejected),
    ] {
        let response = client
            .current_usage_for_comparison("customer", subscription)
            .await
            .unwrap();
        assert_eq!(current_usage::parse(&response.raw).unwrap_err(), expected);
    }
    let large = client
        .current_usage_for_comparison("customer", "large")
        .await
        .unwrap();
    assert_eq!(
        current_usage::parse(&large.raw).unwrap_err(),
        Finding::ResponseTooLarge
    );
    let valid = client
        .current_usage_for_comparison("customer", "valid")
        .await
        .unwrap();
    assert_eq!(
        current_usage::parse(&valid.raw).unwrap().quantities_micros["requests"],
        1_250_000
    );
    assert_eq!(calls.load(Ordering::SeqCst), 6, "no retry request");
    server.abort();
}
