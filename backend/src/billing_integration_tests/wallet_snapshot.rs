use super::*;
use crate::models::credits::Credits;
fn request(owner: &str, id: &str) -> BillingRouteContext {
    route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "wallet_snapshot",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Requests,
        },
        id,
        owner,
    )
}
async fn fixture(label: &str) -> (mongodb::Database, String, Arc<FakeLago>, BillingService) {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    fixture_on_db(db).await
}
async fn fixture_on_db(
    db: mongodb::Database,
) -> (mongodb::Database, String, Arc<FakeLago>, BillingService) {
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let owner = insert_owner(&db).await;
    let lago = Arc::new(FakeLago {
        tax_rate: Some("9".into()),
        wallet_balance: Some(Credits::from_whole(100)),
        ..Default::default()
    });
    let service = billing_service(&db, lago.clone(), 1);
    service.ensure_wallet(&owner).await.unwrap();
    (db, owner, lago, service)
}

#[tokio::test]
async fn snapshot_order_wallet_command_counts() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorded = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            let collection = event.command.get_str(&event.command_name).unwrap_or("");
            recorded
                .lock()
                .unwrap()
                .push(format!("{}:{}", event.command_name, collection));
        }
    });
    let db =
        crate::test_utils::connect_test_database_with_command_handler("snapshot_counts", handler)
            .await
            .unwrap();
    let (db, owner, lago, service) = fixture_on_db(db).await;
    let warm = service.open(&request(&owner, "warm")).await.unwrap();
    service.fail(&warm, "warmup").await.unwrap();
    let saved = wallet(&db, &owner).await;
    let take = |label| {
        let values = std::mem::take(&mut *commands.lock().unwrap());
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for value in values {
            *counts.entry(value).or_default() += 1;
        }
        if label == "admission_warm" {
            assert_eq!(
                counts.values().sum::<usize>(),
                18,
                "warm admission command budget changed: {counts:?}"
            );
        }
        println!("SNAPSHOT_COMMANDS {label}: {counts:?}");
    };
    commands.lock().unwrap().clear();
    let meter = service.open(&request(&owner, "measured")).await.unwrap();
    take("admission_warm");
    service.fail(&meter, "cleanup").await.unwrap();
    commands.lock().unwrap().clear();
    crate::services::billing::webhook::handle_lago_webhook_event(
        &db,
        Some(lago.as_ref()),
        "wallet.updated",
        &serde_json::json!({"wallet":{"external_customer_id":saved.lago_customer_id}}),
        900,
    )
    .await
    .unwrap();
    take("wallet_refresh_with_adjustment");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn snapshot_order_wallet_settlements_do_not_starve_refresh_or_break_accounts() {
    use crate::services::billing::{account_reconciliation, webhook};
    let (db, owner, _, service) = fixture("snapshot_busy_wallet").await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let provider = FakeLago {
        balance_read_gate: Some((entered.clone(), resume.clone())),
        ..Default::default()
    };
    let payload = serde_json::json!({"wallet":{"external_customer_id":&owner}});
    let (result, ()) = tokio::join!(
        webhook::handle_lago_webhook_event(&db, Some(&provider), "wallet.updated", &payload, 900),
        async {
            entered.notified().await;
            let generation = wallet(&db, &owner).await.snapshot_generation;
            for n in 0..8 {
                let meter = service
                    .open(&request(&owner, &format!("busy-{n}")))
                    .await
                    .unwrap();
                service.mark_forwarded(&meter).await.unwrap();
                service
                    .settle(&meter, PlatformUsage::single_request(0), None, None)
                    .await
                    .unwrap();
                assert_eq!(wallet(&db, &owner).await.snapshot_generation, generation);
            }
            resume.notify_one();
        }
    );
    assert_eq!(
        result.unwrap().action,
        webhook::LagoWebhookAction::WalletRefreshed
    );
    let saved = wallet(&db, &owner).await;
    assert_eq!(saved.balance_credits, Credits::from_whole(10_000));
    assert_eq!(saved.pending_lago_debits, "8.72".parse().unwrap());
    assert_eq!(
        db.collection::<bson::Document>("billing_wallet_refresh_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    account_reconciliation::run_once(&db).await.unwrap();
    let status = db
        .collection::<crate::models::chain_verify_status::ChainVerifyStatus>("chain_verify_status")
        .find_one(doc! {"_id":"billing_accounts"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.outcome,
        crate::models::chain_verify_status::ChainVerifyOutcome::Ok,
        "{:?}",
        status.break_detail
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tax_ordering_inline_command_counts() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorded = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            let collection = event.command.get_str(&event.command_name).unwrap_or("");
            recorded
                .lock()
                .unwrap()
                .push(format!("{}:{}", event.command_name, collection));
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "tax_order_counts",
        handler,
    )
    .await;
    let (db, owner, lago, service) = fixture_on_db(db).await;
    let warm = service
        .open(&request(&owner, "warmup-tax-count"))
        .await
        .unwrap();
    service.fail(&warm, "warmup").await.unwrap();
    for state in ["warm", "stale", "cold"] {
        let profiles =
            db.collection::<bson::Document>(crate::models::billing_tax_profile::COLLECTION_NAME);
        if state == "stale" {
            profiles
                .update_many(
                    doc! {},
                    doc! {"$set":{"fetched_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
        } else if state == "cold" {
            profiles.delete_many(doc! {}).await.unwrap();
        }
        commands.lock().unwrap().clear();
        lago.tax_fetches.store(0, Ordering::SeqCst);
        let meter = service.open(&request(&owner, state)).await.unwrap();
        let captured = std::mem::take(&mut *commands.lock().unwrap());
        println!("TAX_COMMANDS {state}: {captured:?}");
        assert_eq!(
            lago.tax_fetches.load(Ordering::SeqCst),
            usize::from(state != "warm")
        );
        let expected = match state {
            "warm" => 18,
            "stale" => 20,
            "cold" => 23,
            _ => unreachable!(),
        };
        assert_eq!(captured.len(), expected, "{state}: {captured:?}");
        service.fail(&meter, "cleanup").await.unwrap();
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tax_ordering_retry_admission_command_counts() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorded = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            let collection = event.command.get_str(&event.command_name).unwrap_or("");
            recorded
                .lock()
                .unwrap()
                .push(format!("{}:{}", event.command_name, collection));
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "tax_retry_counts",
        handler,
    )
    .await;
    let (db, owner, _, initial_service) = fixture_on_db(db).await;
    let warm = initial_service
        .open(&request(&owner, "retry-warmup"))
        .await
        .unwrap();
    initial_service.fail(&warm, "cleanup").await.unwrap();
    for (state, expected) in [("stale", 25), ("cold", 30)] {
        let profiles = db.collection::<bson::Document>("billing_tax_profiles");
        let plan = profiles
            .find_one(doc! {"owner_id":&owner})
            .await
            .unwrap()
            .unwrap();
        let plan = plan.get_str("plan_code").unwrap().to_owned();
        if state == "stale" {
            profiles
                .update_many(
                    doc! {},
                    doc! {"$set":{"fetched_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
        } else {
            profiles.delete_many(doc! {}).await.unwrap();
        }
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let lago = Arc::new(FakeLago {
            tax_rate: Some("9".into()),
            tax_read_gate: Some((entered.clone(), resume.clone())),
            ..Default::default()
        });
        let service = billing_service(&db, lago.clone(), 1);
        commands.lock().unwrap().clear();
        let ctx = request(&owner, state);
        let (result, before_invalidation) = tokio::join!(service.open(&ctx), async {
            entered.notified().await;
            let before = std::mem::take(&mut *commands.lock().unwrap());
            // Admission is paused at its provider read: exclude only the
            // administrator's two invalidation commands from its count.
            crate::services::billing::taxes::invalidate_plan(&db, &plan)
                .await
                .unwrap();
            let invalidation = std::mem::take(&mut *commands.lock().unwrap());
            assert_eq!(invalidation.len(), 2, "{invalidation:?}");
            resume.notify_one();
            before
        });
        let mut captured = before_invalidation;
        captured.extend(std::mem::take(&mut *commands.lock().unwrap()));
        let calls = lago.tax_fetches.load(Ordering::SeqCst);
        println!("TAX_RETRY_COMMANDS {state}: provider_reads={calls}, {captured:?}");
        if let Ok(meter) = &result {
            service.fail(meter, "cleanup").await.unwrap();
        }
        if result.is_err() || calls != 2 || captured.len() != expected {
            db.drop().await.unwrap();
        }
        result.expect("admission must survive one tax invalidation");
        assert_eq!(calls, 2);
        assert_eq!(captured.len(), expected, "{state}: {captured:?}");
    }
    db.drop().await.unwrap();
}
