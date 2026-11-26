use super::*;
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoClient, LagoEvent, LagoUsage, OwnerProvisionInput,
};
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::Mutex;

#[derive(Default)]
struct Provider {
    events: Mutex<Vec<LagoEvent>>,
    retry_first: bool,
    response: Option<LagoError>,
    require_prepared: Option<mongodb::Database>,
    usage: Option<serde_json::Value>,
    usage_reads: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl LagoApi for Provider {
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        unreachable!()
    }
    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        unreachable!()
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        if let Some(db) = &self.require_prepared {
            assert_eq!(db.collection::<Document>("billing_usage_deliveries")
                .count_documents(doc! {"transaction_id":&event.transaction_id,"event_second":event.timestamp,"state":"prepared"})
                .await.unwrap(), 1, "snapshot must be durable before sending");
        }
        let mut events = self.events.lock().unwrap();
        events.push(event.clone());
        if self.retry_first && events.len() == 1 {
            return Err(LagoError::retry("synthetic outage"));
        }
        if let Some(error) = &self.response {
            return Err(error.clone());
        }
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        unreachable!()
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        crate::services::billing::lago_client::require_subscription_id(subscription)?;
        if subscription.trim().is_empty() {
            return Err(AppError::BillingProviderUnavailable(
                "missing subscription".into(),
            ));
        }
        if let Some(raw) = &self.usage {
            self.usage_reads
                .lock()
                .unwrap()
                .push((customer.into(), subscription.into()));
            return Ok(LagoUsage {
                customer_id: customer.into(),
                subscription_id: subscription.into(),
                raw: raw.clone(),
            });
        }
        panic!("unready capture must not read provider usage")
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        unreachable!()
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        unreachable!()
    }
}

struct Fixture {
    db: mongodb::Database,
    provider: Arc<Provider>,
    worker: BillingReconciler,
    commands: Arc<Mutex<Vec<String>>>,
}

impl Fixture {
    async fn compare_usage(
        &self,
    ) -> AppResult<crate::services::billing::reconcile::ReconcileStats> {
        let runtime = crate::services::coordination_service::cluster_lease_runtime();
        let token = crate::services::coordination_service::LeaseStore::acquire(
            &self.db,
            RECONCILE_LEASE,
            &runtime.holder,
            runtime.ttl,
        )
        .await?
        .unwrap();
        let result = self.worker.compare_usage_for_test(&token).await;
        crate::services::coordination_service::LeaseStore::release(&self.db, &token).await?;
        result
    }

    async fn new(retry_first: bool) -> Self {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = commands.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "usage_capture",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event {
                    recorded.lock().unwrap().push(format!(
                        "{}:{}",
                        event.command_name,
                        event.command.get_str(&event.command_name).unwrap_or("")
                    ));
                }
            }),
        )
        .await;
        super::super::usage_delivery::ensure_indexes(&db)
            .await
            .unwrap();
        let now = bson::DateTime::now();
        db.collection::<Document>(BILLING_WALLET)
            .insert_one(doc! {
                "_id":"wallet-1", "owner_id":"owner-1", "lago_customer_id":"customer-1",
                "lago_subscription_id":"subscription-1", "plan_kind":"prepaid",
                "collection_state":"good", "balance_credits":Credits::from_whole(10),
                "balance_synced_at":now, "created_at":now, "updated_at":now,
            })
            .await
            .unwrap();
        let provider = Arc::new(Provider {
            retry_first,
            ..Default::default()
        });
        let worker = BillingReconciler::new(
            db.clone(),
            Some(provider.clone()),
            Arc::new(tests::billing_enabled_config()),
        );
        Self {
            db,
            provider,
            worker,
            commands,
        }
    }
}

#[tokio::test]
async fn usage_capture_persists_acknowledged_delivery() {
    let f = Fixture::new(false).await;
    let row = tests::finalized_row("capture-delivery");
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    f.commands.lock().unwrap().clear();
    assert_eq!(f.worker.push_usage_for_test().await.unwrap().pushed, 1);
    let commands = f.commands.lock().unwrap().clone();
    println!(
        "USAGE_CAPTURE_PUSH_COMMANDS {} {commands:?}",
        commands.len()
    );
    assert_eq!(commands.len(), 10);
    let receipt =
        f.db.collection::<Document>("billing_usage_deliveries")
            .find_one(doc! {"transaction_id":&row.transaction_id})
            .await
            .unwrap()
            .expect("provider-acknowledged delivery must have a durable receipt");
    assert_eq!(receipt.get_str("state"), Ok("acknowledged"));
    assert_eq!(receipt.get_str("subscription_id"), Ok("subscription-1"));
    assert_eq!(
        receipt.get_i64("event_second"),
        Ok(row.finalized_at.unwrap().timestamp())
    );
    assert_eq!(receipt.get_i64("acknowledgement_sequence"), Ok(1));
    assert_eq!(f.worker.push_usage_for_test().await.unwrap().pushed, 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_retry_preserves_missing_finalized_timestamp() {
    let f = Fixture::new(true).await;
    let mut row = tests::finalized_row("capture-stable-second");
    row.finalized_at = None;
    row.updated_at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    assert_eq!(f.worker.push_usage_outage_for_test().await.retried, 1);
    f.db.collection::<Document>(USAGE_METER)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"updated_at":bson::DateTime::from_millis(1_700_000_600_000)}},
        )
        .await
        .unwrap();
    assert_eq!(f.worker.push_usage_for_test().await.unwrap().pushed, 1);
    let events = f.provider.events.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0], events[1],
        "retries must reuse the exact recorded event"
    );
    assert_eq!(events[1].timestamp, 1_700_000_000);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_unready_fleet_never_calls_provider_or_compares() {
    let f = Fixture::new(false).await;
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(tests::finalized_row("capture-warming"))
        .await
        .unwrap();
    assert_eq!(f.compare_usage().await.unwrap().drift_alerts, 0);
    f.db.drop().await.unwrap();
}

#[test]
fn usage_capture_parser_recognizes_nested_provider_metric() {
    let response = serde_json::json!({"customer_usage":{
        "from_datetime":"2026-10-01T00:00:00Z", "to_datetime":"2026-10-31T23:59:59Z",
        "charges_usage":[{"billable_metric":{"code":"platform_requests","aggregation_type":"sum_agg"},"units":"2.5"}]
    }});
    assert_eq!(
        super::super::lago_client::current_usage::parse(&response)
            .unwrap()
            .metric("platform_requests")
            .ok(),
        Some(2_500_000)
    );
}

#[tokio::test]
async fn usage_capture_client_refuses_blank_before_transport() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let received = calls.clone();
    let app = axum::Router::new().fallback(move || {
        received.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { axum::Json(serde_json::json!({"customer_usage":{}})) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client =
        LagoClient::new(format!("http://{address}/api/v1"), "synthetic-key".into()).unwrap();
    for subscription in ["", " \t\n"] {
        assert!(
            client
                .current_usage("synthetic-owner", subscription)
                .await
                .is_err()
        );
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn usage_capture_benefits_have_no_delivered_receipt() {
    let f = Fixture::new(false).await;
    let mut row = tests::finalized_row("benefits-only");
    row.funding = Some(crate::models::usage_meter::UsageFunding {
        settled: true,
        lago_billable_quantity_micros: Some(0),
        ..Default::default()
    });
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    assert_eq!(f.worker.push_usage_for_test().await.unwrap().pushed, 0);
    assert!(f.provider.events.lock().unwrap().is_empty());
    assert_eq!(
        f.db.collection::<Document>("billing_usage_deliveries")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_rejection_and_throttling_never_acknowledge() {
    for rejected in [false, true] {
        let mut f = Fixture::new(false).await;
        let mut error = LagoError::retry("synthetic-response-private-marker");
        error.status = Some(reqwest::StatusCode::TOO_MANY_REQUESTS);
        if rejected {
            error = LagoError::dead_letter("invalid", "synthetic-response-private-marker");
        }
        f.provider = Arc::new(Provider {
            response: Some(error),
            ..Default::default()
        });
        f.worker = BillingReconciler::new(
            f.db.clone(),
            Some(f.provider.clone()),
            Arc::new(tests::billing_enabled_config()),
        );
        let row = tests::finalized_row("refused-event");
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let stats = f.worker.push_usage_for_test().await.unwrap();
        assert_eq!(stats.pushed, 0);
        assert_eq!(stats.dead_lettered, u64::from(rejected));
        assert_eq!(stats.retried, u64::from(!rejected));
        let receipt =
            f.db.collection::<Document>("billing_usage_deliveries")
                .find_one(doc! {})
                .await
                .unwrap()
                .expect("attempt has a receipt");
        assert_eq!(
            receipt.get_str("state"),
            Ok(if rejected { "rejected" } else { "prepared" })
        );
        if rejected {
            let expires_at = receipt.get_datetime("expires_at").unwrap().to_chrono();
            assert_eq!(
                expires_at - receipt.get_datetime("resolved_at").unwrap().to_chrono(),
                chrono::Duration::days(100)
            );
        } else {
            assert!(receipt.get_datetime("expires_at").is_err());
        }
        assert!(!format!("{receipt:?}").contains("synthetic-response-private-marker"));
        assert_eq!(
            f.db.collection::<Document>("billing_usage_comparisons")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_capture_snapshot_is_durable_before_provider_send() {
    let mut f = Fixture::new(false).await;
    f.provider = Arc::new(Provider {
        require_prepared: Some(f.db.clone()),
        ..Default::default()
    });
    f.worker = BillingReconciler::new(
        f.db.clone(),
        Some(f.provider.clone()),
        Arc::new(tests::billing_enabled_config()),
    );
    let mut row = tests::finalized_row("prepared-before-send");
    row.api_key_id = Some("private-key-marker".into());
    row.actor_user_id = "private-actor-marker".into();
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(row)
        .await
        .unwrap();
    assert_eq!(f.worker.push_usage_for_test().await.unwrap().pushed, 1);
    let receipt =
        f.db.collection::<Document>("billing_usage_deliveries")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert!(!format!("{receipt:?}").contains("private-"));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_acknowledgement_is_atomic_and_replay_safe() {
    use crate::models::billing_usage_delivery::BillingUsageDelivery;
    let f = Fixture::new(false).await;
    let row = tests::finalized_row("atomic-delivery");
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    let receipt = super::super::usage_delivery::prepare(
        &f.db,
        &row,
        "customer-1",
        LagoEvent::from_usage_row(&row, Some("subscription-1".into())).unwrap(),
    )
    .await
    .unwrap();
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    ack_in_session(
        &f.db,
        &mut session,
        &row.id,
        Some(&(receipt.clone(), false)),
    )
    .await
    .unwrap();
    session.abort_transaction().await.unwrap();
    assert!(
        !f.db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
            .lago_acked
    );
    assert_eq!(
        f.db.collection::<Document>("billing_usage_comparisons")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    mark_delivered(&f.db, &row.id, Some((receipt.clone(), true)))
        .await
        .unwrap();
    mark_delivered(&f.db, &row.id, Some((receipt, false)))
        .await
        .unwrap();
    let state =
        f.db.collection::<Document>("billing_usage_comparisons")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_i64("acknowledgement_sequence"), Ok(1));
    assert_eq!(state.get_bool("coverage_verified"), Ok(false));
    let saved =
        f.db.collection::<BillingUsageDelivery>("billing_usage_deliveries")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert!(
        saved.duplicate_acknowledgement,
        "duplicate ids alone do not prove the original payload"
    );
    let expiry = saved
        .expires_at
        .expect("acknowledged delivery resolves retention");
    assert_eq!(
        expiry - saved.resolved_at.unwrap(),
        chrono::Duration::days(100)
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_receipt_states_never_block_meter_acknowledgement() {
    for state in [
        "prepared",
        "rejected",
        "legacy_rejected",
        "acknowledged",
        "missing",
        "mismatch",
        "different_row",
        "old_replica",
        "unknown",
    ] {
        let f = Fixture::new(false).await;
        let row = tests::finalized_row(&format!("ack-state-{state}"));
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let mut receipt = super::super::usage_delivery::prepare(
            &f.db,
            &row,
            "customer-1",
            LagoEvent::from_usage_row(&row, Some("subscription-1".into())).unwrap(),
        )
        .await
        .unwrap();
        match state {
            "rejected" => super::super::usage_delivery::reject(&f.db, &receipt)
                .await
                .unwrap(),
            "legacy_rejected" => {
                f.db.collection::<Document>("billing_usage_deliveries").update_one(
                    doc! {"_id":&receipt.id}, doc! {"$set":{"state":"rejected","resolved_at":bson::DateTime::from_millis(1_700_000_000_000)},
                        "$unset":{"rejection_count":"","last_rejected_at":""}}
                ).await.unwrap();
            }
            "acknowledged" => {
                mark_delivered(&f.db, &row.id, Some((receipt.clone(), false)))
                    .await
                    .unwrap();
                f.db.collection::<Document>(USAGE_METER)
                    .update_one(doc! {"_id": &row.id}, doc! {"$set": {"lago_acked": false}})
                    .await
                    .unwrap();
            }
            "missing" => {
                f.db.collection::<Document>("billing_usage_deliveries")
                    .delete_one(doc! {"_id": &receipt.id})
                    .await
                    .unwrap();
            }
            "mismatch" => {
                f.db.collection::<Document>("billing_usage_deliveries")
                    .update_one(
                        doc! {"_id": &receipt.id},
                        doc! {"$set": {"transaction_id": "different-row"}},
                    )
                    .await
                    .unwrap();
            }
            "different_row" => {
                receipt.transaction_id = "another-row".into();
            }
            "unknown" => {
                f.db.collection::<Document>("billing_usage_deliveries")
                    .update_one(
                        doc! {"_id":&receipt.id},
                        doc! {"$set":{"state":"future_state"}},
                    )
                    .await
                    .unwrap();
            }
            "old_replica" => {
                f.db.collection::<Document>("billing_usage_deliveries")
                    .update_one(doc! {"_id": &receipt.id}, doc! {"$unset": {"state": ""}})
                    .await
                    .unwrap();
            }
            "prepared" => {}
            _ => unreachable!(),
        }
        let generation_before =
            f.db.collection::<Document>(BILLING_WALLET)
                .find_one(doc! {"_id": "wallet-1"})
                .await
                .unwrap()
                .and_then(|wallet| wallet.get_i64("snapshot_generation").ok())
                .unwrap_or_default();
        mark_delivered(&f.db, &row.id, Some((receipt.clone(), false)))
            .await
            .unwrap();
        mark_delivered(&f.db, &row.id, Some((receipt.clone(), false)))
            .await
            .unwrap();
        let saved_row =
            f.db.collection::<UsageMeterRow>(USAGE_METER)
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .unwrap();
        assert!(
            saved_row.lago_acked,
            "receipt state {state} blocked the meter"
        );
        let generation_after =
            f.db.collection::<Document>(BILLING_WALLET)
                .find_one(doc! {"_id": "wallet-1"})
                .await
                .unwrap()
                .unwrap()
                .get_i64("snapshot_generation")
                .unwrap();
        assert_eq!(generation_after, generation_before + 1, "state {state}");
        if state == "missing" {
            let comparison =
                f.db.collection::<Document>("billing_usage_comparisons")
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(comparison.get_bool("coverage_verified"), Ok(false));
            assert_eq!(
                comparison.get_str("coverage_unknown_reason"),
                Ok("delivery_receipt_missing")
            );
        } else {
            let saved =
                f.db.collection::<Document>("billing_usage_deliveries")
                    .find_one(doc! {"_id": &receipt.id})
                    .await
                    .unwrap();
            if matches!(
                state,
                "prepared" | "rejected" | "legacy_rejected" | "acknowledged"
            ) {
                assert_eq!(
                    saved.as_ref().and_then(|doc| doc.get_str("state").ok()),
                    Some("acknowledged")
                );
            } else if matches!(state, "mismatch" | "different_row") {
                assert_eq!(
                    saved
                        .as_ref()
                        .and_then(|doc| doc.get_str("reconciliation_issue").ok()),
                    Some("delivery_receipt_mismatch")
                );
            } else if matches!(state, "unknown" | "old_replica") {
                assert_eq!(
                    saved
                        .as_ref()
                        .and_then(|doc| doc.get_str("reconciliation_issue").ok()),
                    Some("delivery_receipt_state_unknown")
                );
            }
        }
        if matches!(
            state,
            "missing" | "mismatch" | "different_row" | "unknown" | "old_replica"
        ) {
            let comparison =
                f.db.collection::<Document>("billing_usage_comparisons")
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(comparison.get_bool("coverage_verified"), Ok(false));
            assert!(comparison.get_str("coverage_unknown_reason").is_ok());
            assert_eq!(comparison.get_i64("acknowledgement_sequence"), Ok(0));
        }
        if matches!(state, "unknown" | "old_replica") {
            let saved =
                f.db.collection::<Document>("billing_usage_deliveries")
                    .find_one(doc! {"_id":&receipt.id})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(saved.get_str("state"), Ok("acknowledged"));
            assert!(saved.get_i64("acknowledgement_sequence").is_err());
            assert!(saved.get_datetime("expires_at").is_ok());
        }
        if matches!(state, "mismatch" | "different_row") {
            let saved =
                f.db.collection::<Document>("billing_usage_deliveries")
                    .find_one(doc! {"_id":&receipt.id})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(saved.get_str("state"), Ok("prepared"));
            assert!(saved.get_datetime("expires_at").is_err());
        }
        if state == "legacy_rejected" {
            let saved =
                f.db.collection::<Document>("billing_usage_deliveries")
                    .find_one(doc! {"_id":&receipt.id})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(saved.get_i64("rejection_count"), Ok(1));
            assert_eq!(
                saved.get_datetime("last_rejected_at"),
                Ok(&bson::DateTime::from_millis(1_700_000_000_000))
            );
        }
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_capture_rejected_receipt_does_not_block_repaired_delivery() {
    let mut f = Fixture::new(false).await;
    let rejected = Arc::new(Provider {
        response: Some(LagoError::dead_letter("invalid", "synthetic rejection")),
        ..Default::default()
    });
    f.provider = rejected.clone();
    f.worker = BillingReconciler::new(
        f.db.clone(),
        Some(rejected.clone()),
        Arc::new(tests::billing_enabled_config()),
    );
    let row = tests::finalized_row("repaired-delivery");
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    let first = f.worker.push_usage_for_test().await.unwrap();
    assert_eq!(first.dead_lettered, 1);
    assert_eq!(
        f.worker.push_usage_for_test().await.unwrap().dead_lettered,
        0
    );
    assert_eq!(
        rejected.events.lock().unwrap().len(),
        1,
        "provider dead letters have no automatic retry"
    );
    f.db.collection::<Document>(USAGE_METER)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {
                "status": "finalized",
                "released": true,
                "updated_at": bson::DateTime::from_millis(0),
            }},
        )
        .await
        .unwrap();
    let accepting = Arc::new(Provider {
        retry_first: true,
        ..Default::default()
    });
    f.provider = accepting.clone();
    f.worker = BillingReconciler::new(
        f.db.clone(),
        Some(accepting.clone()),
        Arc::new(tests::billing_enabled_config()),
    );
    assert_eq!(f.worker.push_usage_outage_for_test().await.retried, 1);
    let pending =
        f.db.collection::<Document>("billing_usage_deliveries")
            .find_one(doc! {"transaction_id":&row.transaction_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(pending.get_str("state"), Ok("prepared"));
    assert!(pending.get_datetime("expires_at").is_err());
    assert_eq!(pending.get_i64("rejection_count"), Ok(1));
    f.db.collection::<Document>(USAGE_METER)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"updated_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let second = f.worker.push_usage_for_test().await.unwrap();
    assert_eq!(second.pushed, 1);
    let retried = accepting.events.lock().unwrap().clone();
    assert_eq!(retried.len(), 2);
    assert!(
        retried
            .iter()
            .all(|event| event == &rejected.events.lock().unwrap()[0]),
        "repair retries the same event"
    );
    let saved =
        f.db.collection::<Document>("billing_usage_deliveries")
            .find_one(doc! {"transaction_id": &row.transaction_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.get_str("state"), Ok("acknowledged"));
    assert_eq!(saved.get_i64("rejection_count"), Ok(1));
    assert!(saved.get_datetime("last_rejected_at").is_ok());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_reconciles_orphaned_prepared_receipts() {
    let f = Fixture::new(false).await;
    let cases = [
        "prepared-acked-meter",
        "prepared-missing-meter",
        "prepared-live-meter",
    ];
    let mut receipts = Vec::new();
    for name in cases {
        let mut row = tests::finalized_row(name);
        row.updated_at = Utc::now() + Duration::days(1);
        let subscription = format!("subscription-{name}");
        let receipt = super::super::usage_delivery::prepare(
            &f.db,
            &row,
            "customer-1",
            LagoEvent::from_usage_row(&row, Some(subscription)).unwrap(),
        )
        .await
        .unwrap();
        if name != "prepared-missing-meter" {
            f.db.collection::<UsageMeterRow>(USAGE_METER)
                .insert_one(&row)
                .await
                .unwrap();
            if name == "prepared-acked-meter" {
                f.db.collection::<Document>(USAGE_METER)
                    .update_one(doc! {"_id": &row.id}, doc! {"$set": {"lago_acked": true}})
                    .await
                    .unwrap();
            }
        }
        if name != "prepared-live-meter" {
            f.db.collection::<Document>("billing_usage_deliveries")
                .update_one(
                    doc! {"_id": &receipt.id},
                    doc! {"$unset": {
                        "rejection_count": "",
                        "resolved_at": "",
                        "last_rejected_at": "",
                        "reconciliation_issue": "",
                        "expires_at": "",
                    }},
                )
                .await
                .unwrap();
        }
        receipts.push((name, receipt.id));
    }
    f.worker.push_usage_for_test().await.unwrap();
    for (name, id) in receipts {
        let saved =
            f.db.collection::<Document>("billing_usage_deliveries")
                .find_one(doc! {"_id": &id})
                .await
                .unwrap()
                .unwrap();
        if name == "prepared-live-meter" {
            assert_eq!(saved.get_str("state"), Ok("prepared"));
            assert!(saved.get_datetime("expires_at").is_err());
        } else {
            assert_eq!(saved.get_str("state"), Ok("acknowledged"));
            assert!(saved.get_i64("acknowledgement_sequence").is_err());
            assert!(saved.get_datetime("acknowledged_at").is_err());
            assert_eq!(
                saved.get_str("reconciliation_issue"),
                Ok(if name == "prepared-acked-meter" {
                    "prepared_receipt_meter_already_acknowledged"
                } else {
                    "prepared_receipt_meter_missing"
                })
            );
            assert_eq!(
                saved.get_datetime("expires_at").unwrap().to_chrono()
                    - saved.get_datetime("resolved_at").unwrap().to_chrono(),
                Duration::days(100)
            );
        }
    }
    let comparisons =
        f.db.collection::<Document>("billing_usage_comparisons")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
    assert_eq!(comparisons.len(), 2);
    assert!(comparisons.iter().all(|comparison| {
        comparison.get_bool("coverage_verified") == Ok(false)
            && comparison
                .get_str("coverage_unknown_reason")
                .is_ok_and(|reason| reason.starts_with("prepared_receipt_meter_"))
    }));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_retention_uses_ttl_and_resolution_time_once() {
    use crate::models::billing_usage_delivery::BillingUsageDelivery;
    let f = Fixture::new(false).await;
    let receipts =
        f.db.collection::<BillingUsageDelivery>("billing_usage_deliveries");
    let indexes: Vec<_> = receipts
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(indexes.iter().any(|index| {
        index.keys == doc! {"expires_at":1}
            && index
                .options
                .as_ref()
                .and_then(|options| options.expire_after)
                == Some(std::time::Duration::ZERO)
    }));
    for acknowledged in [true, false] {
        let row = tests::finalized_row(if acknowledged {
            "retained-ack"
        } else {
            "retained-rejection"
        });
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let receipt = super::super::usage_delivery::prepare(
            &f.db,
            &row,
            "customer-1",
            LagoEvent::from_usage_row(&row, Some("subscription-1".into())).unwrap(),
        )
        .await
        .unwrap();
        assert!(receipt.expires_at.is_none());
        assert!(receipt.resolved_at.is_none());
        if acknowledged {
            mark_delivered(&f.db, &row.id, Some((receipt.clone(), false)))
                .await
                .unwrap();
        } else {
            super::super::usage_delivery::reject(&f.db, &receipt)
                .await
                .unwrap();
        }
        let saved = receipts
            .find_one(doc! {"_id":&receipt.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.expires_at.unwrap() - saved.resolved_at.unwrap(),
            Duration::days(100)
        );
        // Move the saved resolution one day back without sleeping. A repeated
        // resolution must preserve the original deadline rather than extend it.
        let resolved_at = saved.resolved_at.unwrap() - Duration::days(1);
        let expires_at = resolved_at + Duration::days(100);
        f.db.collection::<Document>("billing_usage_deliveries")
            .update_one(
                doc! {"_id":&receipt.id},
                doc! {"$set":{"resolved_at":bson::DateTime::from_chrono(resolved_at),
                "expires_at":bson::DateTime::from_chrono(expires_at)}},
            )
            .await
            .unwrap();
        if acknowledged {
            mark_delivered(&f.db, &row.id, Some((receipt.clone(), true)))
                .await
                .unwrap();
        } else {
            super::super::usage_delivery::reject(&f.db, &receipt)
                .await
                .unwrap();
        }
        let replayed = receipts
            .find_one(doc! {"_id":&receipt.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replayed.resolved_at, Some(resolved_at));
        assert_eq!(replayed.expires_at, Some(expires_at));
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_retry_keeps_original_target_and_properties() {
    let f = Fixture::new(true).await;
    let row = tests::finalized_row("frozen-target");
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    f.worker.push_usage_outage_for_test().await;
    f.db.collection::<Document>(BILLING_WALLET).update_one(doc! {"_id":"wallet-1"},
        doc! {"$set":{"lago_customer_id":"replacement-customer","lago_subscription_id":"replacement-subscription"}}).await.unwrap();
    f.db.collection::<Document>(USAGE_METER).update_one(doc! {"_id":&row.id},
        doc! {"$set":{"quantity":99_i64,"model":"changed-model","updated_at":bson::DateTime::from_millis(0)}}).await.unwrap();
    f.worker.push_usage_for_test().await.unwrap();
    let events = f.provider.events.lock().unwrap().clone();
    assert_eq!(events[0], events[1]);
    let saved =
        f.db.collection::<Document>("billing_usage_deliveries")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.get_str("customer_id"), Ok("customer-1"));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_success_clears_old_failure_without_obsolete_warming_diagnostic() {
    use crate::models::coordination::CoordinationHolder;
    let f = Fixture::new(false).await;
    let holder = CoordinationHolder {
        instance_id: "capture-test".into(),
        generation_id: "generation-1".into(),
    };
    let old_time = bson::DateTime::from_millis(1_700_000_000_000);
    let previous = Some(bson::Bson::Document(
        doc! {"version":1_i64,"failures":{"usage_comparison":{
            "consecutive_passes":3_i64,"first_failed_at":old_time,"last_failed_at":old_time,"error_code":11302_i64,"failing_units":6_i64
        }}},
    ));
    let (token, _) = LeaseStore::acquire_with_checkpoint(
        &f.db,
        RECONCILE_LEASE,
        &holder,
        std::time::Duration::from_secs(300),
    )
    .await
    .unwrap()
    .unwrap();
    let mut pass = Pass::new(previous);
    pass.record("usage_comparison", Ok(()));
    assert!(pass.release(&f.db, &token).await.unwrap());
    let first = super::super::reconcile_status::diagnostics(&f.db).await;
    assert!(first.is_empty());
    let (token, checkpoint) = LeaseStore::acquire_with_checkpoint(
        &f.db,
        RECONCILE_LEASE,
        &holder,
        std::time::Duration::from_secs(300),
    )
    .await
    .unwrap()
    .unwrap();
    let mut pass = Pass::new(checkpoint);
    pass.record("usage_comparison", Ok(()));
    assert!(pass.release(&f.db, &token).await.unwrap());
    let second = super::super::reconcile_status::diagnostics(&f.db).await;
    assert!(second.is_empty());
    assert!(f.provider.events.lock().unwrap().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_prepared_scan_rotates_past_live_receipts() {
    let f = Fixture::new(false).await;
    let receipts = f.db.collection::<Document>("billing_usage_deliveries");
    for n in 0..65 {
        let mut row = tests::finalized_row(&format!("rotation-{n:03}"));
        row.updated_at = Utc::now() + Duration::days(1);
        let receipt = super::super::usage_delivery::prepare(
            &f.db,
            &row,
            "customer-1",
            LagoEvent::from_usage_row(&row, Some("subscription-1".into())).unwrap(),
        )
        .await
        .unwrap();
        receipts
            .update_one(
                doc! {"_id":&receipt.id},
                doc! {"$set":{"last_checked_at":bson::DateTime::from_millis(n)}},
            )
            .await
            .unwrap();
        if n < 64 {
            f.db.collection::<UsageMeterRow>(USAGE_METER)
                .insert_one(row)
                .await
                .unwrap();
        }
    }
    f.commands.lock().unwrap().clear();
    f.worker.push_usage_for_test().await.unwrap();
    let commands = f.commands.lock().unwrap().clone();
    assert_eq!(
        commands.len(),
        130,
        "one receipt scan, 64 meter reads, 64 rotation writes, one push selection"
    );
    assert_eq!(
        receipts
            .count_documents(doc! {"state":"acknowledged"})
            .await
            .unwrap(),
        0
    );
    f.worker.push_usage_for_test().await.unwrap();
    assert_eq!(
        receipts
            .count_documents(doc! {"state":"acknowledged","transaction_id":"rotation-064"})
            .await
            .unwrap(),
        1
    );
    assert!(f.provider.events.lock().unwrap().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_success_keeps_previous_coverage_uncertainty() {
    let f = Fixture::new(false).await;
    for missing in [true, false] {
        let row = tests::finalized_row(if missing {
            "lost-receipt"
        } else {
            "later-success"
        });
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let receipt = super::super::usage_delivery::prepare(
            &f.db,
            &row,
            "customer-1",
            LagoEvent::from_usage_row(&row, Some("subscription-1".into())).unwrap(),
        )
        .await
        .unwrap();
        if missing {
            f.db.collection::<Document>("billing_usage_deliveries")
                .delete_one(doc! {"_id":&receipt.id})
                .await
                .unwrap();
        }
        mark_delivered(&f.db, &row.id, Some((receipt, false)))
            .await
            .unwrap();
    }
    let state =
        f.db.collection::<Document>("billing_usage_comparisons")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_bool("coverage_verified"), Ok(false));
    assert_eq!(
        state.get_str("coverage_unknown_reason"),
        Ok("delivery_receipt_missing")
    );
    assert_eq!(state.get_i64("acknowledgement_sequence"), Ok(1));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_receipt_repair_failure_does_not_block_push() {
    let f = Fixture::new(false).await;
    f.db.collection::<Document>("billing_usage_deliveries")
        .insert_one(doc! {"_id":"malformed","transaction_id":"unrelated","state":"prepared"})
        .await
        .unwrap();
    let row = tests::finalized_row("healthy-delivery-after-repair-failure");
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    assert_eq!(f.worker.push_usage_for_test().await.unwrap().pushed, 1);
    assert!(
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
            .lago_acked
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_comparison_lost_response_retry_projects_equal_without_uncertainty() {
    let mut f = Fixture::new(false).await;
    crate::services::security_readiness_service::ready_test_replica(&f.db).await;
    crate::services::security_readiness_service::stop_test_replica(&f.db).await;
    let now = Utc::now();
    let start = now - Duration::days(10);
    let end = now + Duration::days(20);
    f.provider = Arc::new(Provider {
        retry_first: true,
        response: Some(LagoError::duplicate("already accepted")),
        usage: Some(serde_json::json!({"customer_usage":{
            "from_datetime":start.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "to_datetime":end.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "charges_usage":[{"billable_metric":{"code":"platform_requests","aggregation_type":"sum_agg"},"units":"1"}]
        }})),
        ..Default::default()
    });
    f.worker = BillingReconciler::new(
        f.db.clone(),
        Some(f.provider.clone()),
        Arc::new(tests::billing_enabled_config()),
    );
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(tests::finalized_row("compare-real-receipt"))
        .await
        .unwrap();
    assert_eq!(f.worker.push_usage_outage_for_test().await.retried, 1);
    f.db.collection::<Document>(USAGE_METER)
        .update_many(
            doc! {},
            doc! {"$set":{"updated_at":bson::DateTime::from_chrono(now-Duration::hours(1))}},
        )
        .await
        .unwrap();
    assert_eq!(f.worker.push_usage_for_test().await.unwrap().pushed, 1);
    let events = f.provider.events.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0], events[1],
        "lost response retries immutable payload"
    );
    f.db.collection::<Document>("billing_usage_comparisons")
        .update_one(
            doc! {},
            doc! {"$set":{
                "last_acknowledged_at":bson::DateTime::from_chrono(now-Duration::minutes(11))
            }},
        )
        .await
        .unwrap();
    f.db.collection::<Document>("billing_usage_comparison_control").replace_one(
        doc! {"_id":"5b543627-0ed6-464e-b6a8-94e99c4d1773"},
        doc! {"_id":"5b543627-0ed6-464e-b6a8-94e99c4d1773", "kind":"coverage", "ready":true,
            "generation":uuid::Uuid::new_v4().to_string(), "observed_at":bson::DateTime::from_chrono(now),
            "continuous_since":bson::DateTime::from_chrono(start-Duration::days(1))},
    ).upsert(true).await.unwrap();
    f.commands.lock().unwrap().clear();
    let stats = f.compare_usage().await.unwrap();
    println!("USAGE_COMPARISON_COMMANDS {:?}", f.commands.lock().unwrap());
    assert_eq!(
        *f.provider.usage_reads.lock().unwrap(),
        vec![("customer-1".to_owned(), "subscription-1".to_owned())]
    );
    assert_eq!(stats.drift_alerts, 0);
    let target =
        f.db.collection::<Document>("billing_usage_comparisons")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert!(!target.contains_key("coverage_unknown_reason"));
    let comparison = target
        .get_document("comparison")
        .expect("a complete observation is durable");
    assert_eq!(comparison.get_str("status"), Ok("equal"));
    assert_eq!(
        comparison
            .get_document("local")
            .unwrap()
            .get_str("platform_requests"),
        Ok("1")
    );
    crate::services::security_readiness_service::stop_test_replica(&f.db).await;
    f.db.collection::<Document>("billing_wallet")
        .delete_many(doc! {})
        .await
        .unwrap();
    f.db.collection::<Document>("billing_usage_comparisons")
        .delete_many(doc! {})
        .await
        .unwrap();
    f.commands.lock().unwrap().clear();
    f.compare_usage().await.unwrap();
    assert_eq!(
        f.commands.lock().unwrap().len(),
        10,
        "ready empty comparison includes the legacy finding lookup and test lease acquire/release"
    );
    f.db.collection::<Document>(crate::models::security_capability_activation::COLLECTION_NAME)
        .delete_one(
            doc! {"capability":crate::services::billing::usage_comparison::coverage::READINESS},
        )
        .await
        .unwrap();
    f.commands.lock().unwrap().clear();
    f.compare_usage().await.unwrap();
    assert_eq!(
        f.commands.lock().unwrap().len(),
        8,
        "unready empty comparison plus test lease acquire/release"
    );
    println!(
        "USAGE_COMPARISON_EMPTY ready=8 unready=6 commands excluding test lease acquire/release"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_comparison_future_ack_keeps_delivery_but_invalidates_coverage() {
    {
        let f = Fixture::new(false).await;
        let mut row = tests::finalized_row("uncertain-ack");
        row.finalized_at = Some(Utc::now() + Duration::days(101));
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let receipt = super::super::usage_delivery::prepare(
            &f.db,
            &row,
            "customer-1",
            LagoEvent::from_usage_row(&row, Some("subscription-1".into())).unwrap(),
        )
        .await
        .unwrap();
        mark_delivered(&f.db, &row.id, Some((receipt.clone(), false)))
            .await
            .unwrap();
        let saved =
            f.db.collection::<Document>(USAGE_METER)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(saved.get_bool("lago_acked"), Ok(true));
        let target =
            f.db.collection::<Document>("billing_usage_comparisons")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(target.get_i64("acknowledgement_sequence"), Ok(1));
        assert_eq!(target.get_i64("uncertainty_revision"), Ok(1));
        assert_eq!(
            target.get_str("coverage_unknown_reason"),
            Ok("future_delivery_event")
        );
        assert!(
            target
                .get_datetime("uncertain_through")
                .unwrap()
                .timestamp_millis()
                >= receipt.event_second * 1000
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_comparison_round2_duplicate_ack_does_not_discard_period() {
    let f = Fixture::new(false).await;
    let row = tests::finalized_row("retry-provenance");
    f.db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .unwrap();
    let receipt = super::super::usage_delivery::prepare(
        &f.db,
        &row,
        "customer-1",
        LagoEvent::from_usage_row(&row, Some("subscription-1".into())).unwrap(),
    )
    .await
    .unwrap();
    mark_delivered(&f.db, &row.id, Some((receipt.clone(), true)))
        .await
        .unwrap();
    let target =
        f.db.collection::<Document>("billing_usage_comparisons")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert!(!target.contains_key("coverage_unknown_reason"));
    let saved =
        f.db.collection::<Document>("billing_usage_deliveries")
            .find_one(doc! {"_id":receipt.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        saved
            .get_datetime("usage_created_at")
            .unwrap()
            .timestamp_millis(),
        row.created_at.timestamp_millis()
    );
    f.db.drop().await.unwrap();
}
