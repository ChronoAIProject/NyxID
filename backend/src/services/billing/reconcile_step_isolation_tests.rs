//! Recovery selection contains poison and independent healthy work on real MongoDB.
use super::*;
use crate::models::{
    credits::Credits,
    usage_meter::{UsageFunding, UsageStatus},
};
use crate::services::billing;
use mongodb::{
    Database,
    bson::Bson,
    event::{EventHandler, command::CommandEvent},
};

#[derive(Clone, Copy, Debug)]
enum Step {
    Model,
    Managed,
    Abandon,
    Funding,
    Expiry,
    Schedule,
    Ledger,
    Settlement,
    Topup,
}
impl Step {
    const ALL: [Self; 9] = [
        Self::Model,
        Self::Managed,
        Self::Abandon,
        Self::Funding,
        Self::Expiry,
        Self::Schedule,
        Self::Ledger,
        Self::Settlement,
        Self::Topup,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Model => "model_recovery",
            Self::Managed => "managed_recovery",
            Self::Abandon => "abandon_unforwarded",
            Self::Funding => "funding_releases",
            Self::Expiry => "grant_expiry",
            Self::Schedule => "schedules",
            Self::Ledger => "grant_ledger",
            Self::Settlement => "settlement_recovery",
            Self::Topup => "topup_expiry",
        }
    }
    fn collection(self) -> &'static str {
        match self {
            Self::Model => "platform_model_attempts",
            Self::Managed => "managed_executions",
            Self::Abandon | Self::Funding | Self::Settlement => USAGE,
            Self::Expiry | Self::Ledger => "credit_grants",
            Self::Schedule => "credit_schedules",
            Self::Topup => "billing_wallet",
        }
    }
    async fn run(self, db: &Database) -> AppResult<()> {
        let now = Utc::now();
        match self {
            Self::Model => crate::services::platform_model_execution::recover(db)
                .await
                .map(|_| ()),
            Self::Managed => crate::services::managed_execution_service::reconcile(db)
                .await
                .map(|_| ()),
            Self::Abandon => {
                billing::reservation::abandon_stale_unforwarded(db, now - Duration::minutes(1))
                    .await
                    .map(|_| ())
            }
            Self::Funding => billing::funding::recover_terminal_releases(db)
                .await
                .map(|_| ()),
            Self::Expiry => billing::grants::expire_due_grants(db, now)
                .await
                .map(|_| ()),
            Self::Schedule => billing::schedules::disburse_due(db, now, 5000)
                .await
                .map(|_| ()),
            Self::Ledger => billing::grants::recover_unledgered_events(db, now)
                .await
                .map(|_| ()),
            Self::Settlement => billing::reservation::recover_retryable_settlements(db)
                .await
                .map(|_| ()),
            Self::Topup => {
                billing::topup_expiry::expire_purchased_credits(db, &Provider::default(), now)
                    .await
                    .map(|_| ())
            }
        }
    }
    fn poison(self, id: usize) -> Document {
        let old = bson::DateTime::from_millis(0);
        let mut d = match self {
            Self::Model => doc! {"_id":id as i64,"state":"dispatched","lease_until":old},
            Self::Managed => {
                doc! {"state":"evidence_ready","expires_at":old,"next_reconcile_at":old}
            }
            Self::Abandon => {
                doc! {"status":"reserved","forwarded":false,"released":false,"updated_at":old}
            }
            Self::Funding => {
                doc! {"status":"abandoned","released":true,"funding":{"settled":false},"updated_at":old}
            }
            Self::Expiry => {
                doc! {"status":"active","reserved":Credits::ZERO,"expires_at":old,"issued_ledgered_at":old}
            }
            Self::Schedule => doc! {"is_active":true},
            Self::Ledger => doc! {"status":"active","created_at":old},
            Self::Settlement => {
                doc! {"status":"failed","released":false,"quantity":1_i64,"finalized_at":old}
            }
            Self::Topup => doc! {"lago_wallet_id":format!("poison-wallet-{id}"),"updated_at":old},
        };
        if !matches!(self, Self::Model) {
            d.insert("_id", format!("a-bad-{id:05}"));
        }
        d
    }
}
fn grant(id: &str) -> Document {
    let now = bson::DateTime::now();
    doc! {
        "_id":id,"batch_id":"batch","recipient_user_id":"owner-1","target_kind":"selected_users","amount_credits":2_i64,
        "amount":Credits::from_whole(2),"remaining":Credits::from_whole(2),"reserved":Credits::ZERO,
        "scope":{"all_services":true,"service_ids":[],"service_slugs":[]},"granted_by":"admin-fixture","status":"active",
        "created_at":now,"updated_at":now,"issued_ledgered_at":now,
    }
}
async fn healthy(db: &Database, step: Step) {
    let now = bson::DateTime::now();
    let old = bson::DateTime::from_chrono(Utc::now() - Duration::hours(2));
    let raw = match step {
        Step::Model => doc! {"_id":"z-good","state":"dispatched","lease_until":old},
        Step::Managed => {
            doc! {"_id":"z-good","billing_request_id":"managed-request","billing_owner_id":"owner-1","actor_user_id":"actor-1",
            "catalog_service_id":"catalog","ingress":"proxy","request_bytes":0_i64,"request_digest":"digest","catalog_digest":"digest","route_digest":"digest",
            "billing":bson::to_bson(&crate::models::service_billing::ServiceBilling::default()).unwrap(),
            "state":"dispatch_admitted","maximum":Credits::ZERO,"meter_row_ids":[],"expires_at":old,"next_reconcile_at":old,"created_at":old,"updated_at":old}
        }
        Step::Schedule => {
            doc! {"_id":"z-good","amount_credits":2_i64,"amount":Credits::from_whole(2),"recurrence":"daily","expiry":{"kind":"never"},
            "target_kind":"selected_users","target_user_ids":["owner-1"],"scope":{"all_services":true,"service_ids":[],"service_slugs":[]},
            "is_active":true,"created_by":"admin-fixture","created_at":old,"updated_at":old}
        }
        Step::Expiry | Step::Ledger => {
            let mut row = grant("z-good");
            if matches!(step, Step::Expiry) {
                row.insert("expires_at", old);
            } else {
                row.insert("issued_ledgered_at", Bson::Null);
            }
            row
        }
        Step::Abandon | Step::Funding | Step::Settlement => {
            let mut row = finalized_row("z-good");
            row.id = "z-good".into();
            row.quantity = Some(0);
            row.released = false;
            row.status = UsageStatus::Failed;
            if matches!(step, Step::Abandon) {
                row.status = UsageStatus::Reserved;
                row.forwarded = false;
                row.quantity = None;
                row.reserved_credits = Credits::from_whole(1);
                db.collection::<Document>("billing_wallet")
                    .update_one(
                        doc! {"_id":"wallet-1"},
                        doc! {"$set":{"reserved_credits":Credits::from_whole(1)}},
                    )
                    .await
                    .unwrap();
            }
            if matches!(step, Step::Funding) {
                row.status = UsageStatus::Abandoned;
                row.released = true;
                row.quantity = None;
                row.funding = Some(UsageFunding::default());
            }
            bson::to_document(&row).unwrap()
        }
        Step::Topup => {
            doc! {"_id":"z-good","owner_id":"topup-owner","lago_customer_id":"customer","lago_subscription_id":"subscription",
            "lago_wallet_id":"wallet","plan_kind":"prepaid","collection_state":"good","balance_synced_at":now,"created_at":now,"updated_at":now}
        }
    };
    db.collection::<Document>(step.collection())
        .insert_one(raw)
        .await
        .unwrap();
}
async fn healthy_done(db: &Database, step: Step) -> bool {
    let row = db
        .collection::<Document>(step.collection())
        .find_one(doc! {"_id":"z-good"})
        .await
        .unwrap()
        .unwrap();
    match step {
        Step::Model => row.get_str("state").ok() == Some("pending_review"),
        Step::Managed => row.get_str("state").ok() == Some("pending"),
        Step::Abandon | Step::Settlement => row.get_bool("released").ok() == Some(true),
        Step::Funding => {
            row.get_document("funding")
                .unwrap()
                .get_bool("settled")
                .ok()
                == Some(true)
        }
        Step::Expiry => row.get_str("status").ok() == Some("expired"),
        Step::Ledger => row.get_datetime("issued_ledgered_at").is_ok(),
        Step::Schedule => row.get_datetime("last_disbursed_at").is_ok(),
        Step::Topup => row.get_datetime("topup_expiry_checked_at").is_ok(),
    }
}
async fn db() -> (Database, Arc<Mutex<Vec<String>>>) {
    db_recording(Arc::new(Mutex::new(Vec::new()))).await
}
async fn db_recording(reads: Arc<Mutex<Vec<Document>>>) -> (Database, Arc<Mutex<Vec<String>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let seen = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "step_isolation",
        EventHandler::callback(move |e| {
            if let CommandEvent::Started(e) = e {
                if matches!(e.command_name.as_str(), "find" | "findAndModify") {
                    reads.lock().unwrap().push(e.command.clone());
                }
                seen.lock().unwrap().push(format!(
                    "{}:{}",
                    e.command_name,
                    e.command.get_str(&e.command_name).unwrap_or("")
                ));
            }
        }),
    )
    .await;
    billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    db.collection::<Document>("billing_wallet").insert_one(doc!{"_id":"wallet-1","owner_id":"owner-1","lago_customer_id":"customer-1",
        "lago_subscription_id":"subscription-1","plan_kind":"prepaid","collection_state":"good","balance_credits":Credits::from_whole(10),
        "balance_synced_at":bson::DateTime::now(),"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    // The held-request sublane uses these same production partial indexes.
    db.collection::<Document>(USAGE)
        .create_indexes(billing::stuck_requests::indexes())
        .await
        .unwrap();
    (db, commands)
}
async fn prove(step: Step) {
    let (db, _) = db().await;
    let poison = step.poison(0);
    db.collection::<Document>(step.collection())
        .insert_one(poison.clone())
        .await
        .unwrap();
    healthy(&db, step).await;
    let result = step.run(&db).await;
    assert!(
        healthy_done(&db, step).await,
        "{step:?}: bad unit blocked healthy work"
    );
    assert!(
        result.is_err(),
        "{step:?}: failed unit must still fail the step"
    );
    let mut pass = billing::reconcile_status::Pass::new(None);
    pass.record(step.name(), result);
    let runtime = lease_runtime("unit-status", 30_000);
    let token = runtime
        .acquire(&db, super::super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    pass.release(&db, &token).await.unwrap();
    let status = db
        .collection::<Document>("coordination_leases")
        .find_one(doc! {"_id":super::super::RECONCILE_LEASE})
        .await
        .unwrap()
        .unwrap();
    let failure = status
        .get_document("checkpoint")
        .unwrap()
        .get_document("failures")
        .unwrap()
        .get_document(step.name())
        .unwrap();
    assert_eq!(failure.get_i64("failing_units").unwrap(), 1, "{step:?}");
    let retained = db
        .collection::<Document>(step.collection())
        .find_one(doc! {"_id":poison.get("_id")})
        .await
        .unwrap()
        .unwrap();
    for field in [
        "quantity",
        "released",
        "funding",
        "reserved",
        "remaining",
        "active_settlement",
        "active_topup_expiry",
    ] {
        assert_eq!(
            retained.get(field),
            poison.get(field),
            "{step:?} mutated durable {field}"
        );
    }
    // Expire existing claims without waiting. Retry must retain the bad unit.
    if matches!(step, Step::Model | Step::Managed) {
        let field = if matches!(step, Step::Model) {
            "lease_until"
        } else {
            "next_reconcile_at"
        };
        db.collection::<Document>(step.collection())
            .update_one(
                doc! {"_id":poison.get("_id")},
                doc! {"$set":{field:bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
    }
    assert!(
        step.run(&db).await.is_err(),
        "{step:?}: poison must be retried"
    );
    let diagnostics = billing::reconcile_status::diagnostics(&db).await;
    assert!(diagnostics[0].detail.contains("failing_units=1"));
    db.drop().await.unwrap();
}
macro_rules! regression {
    ($name:ident,$step:ident) => {
        #[tokio::test]
        async fn $name() {
            prove(Step::$step).await;
        }
    };
}
regression!(billing_step_model_isolates_bad_unit, Model);
regression!(billing_step_managed_isolates_bad_unit, Managed);
regression!(billing_step_abandon_isolates_bad_unit, Abandon);
regression!(billing_step_funding_isolates_bad_unit, Funding);
regression!(billing_step_expiry_isolates_bad_unit, Expiry);
regression!(billing_step_schedule_isolates_bad_unit, Schedule);
regression!(billing_step_ledger_isolates_bad_unit, Ledger);
regression!(billing_step_settlement_isolates_bad_unit, Settlement);
regression!(billing_step_topup_isolates_bad_unit, Topup);

#[tokio::test]
async fn billing_step_clean_command_counts() {
    for step in Step::ALL {
        let (db, commands) = db().await;
        commands.lock().unwrap().clear();
        step.run(&db).await.unwrap();
        let idle = commands.lock().unwrap().clone();
        healthy(&db, step).await;
        commands.lock().unwrap().clear();
        step.run(&db).await.unwrap();
        let work = commands.lock().unwrap().clone();
        assert!(
            healthy_done(&db, step).await,
            "invalid healthy fixture: {step:?}"
        );
        let expected = match step {
            Step::Model => (1, 7),
            Step::Managed => (1, 2),
            Step::Abandon => (1, 4),
            Step::Funding => (3, 4),
            Step::Expiry => (3, 12),
            Step::Schedule => (4, 19),
            Step::Ledger => (2, 10),
            Step::Settlement => (10, 23),
            Step::Topup => (2, 2),
        };
        assert_eq!(
            (idle.len(), work.len()),
            expected,
            "{step:?}: clean commands changed"
        );
        println!(
            "BILLING_STEP_CLEAN {} idle={} healthy={} idle_commands={idle:?} healthy_commands={work:?}",
            step.name(),
            idle.len(),
            work.len()
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn billing_step_overfull_batches_keep_existing_bounds() {
    // With the test-only two-unit cap, three bad candidates put good work
    // outside this pass. Only the three pre-existing retry/rotation protocols
    // promise progress on the next pass; other capped selections may repeat.
    for step in Step::ALL
        .into_iter()
        .filter(|s| !matches!(s, Step::Abandon))
    {
        let (db, _) = db().await;
        db.collection::<Document>(step.collection())
            .insert_many((0..3).map(|i| step.poison(i)))
            .await
            .unwrap();
        healthy(&db, step).await;
        for pass in 1..=2 {
            let result = billing::recovery_batch::TEST_BATCH_LIMIT
                .scope(2, step.run(&db))
                .await;
            assert!(result.is_err(), "{step:?} pass={pass}");
            if pass == 1 {
                assert!(!healthy_done(&db, step).await, "{step:?}");
            }
        }
        assert_eq!(
            healthy_done(&db, step).await,
            matches!(step, Step::Model | Step::Managed | Step::Topup),
            "{step:?}"
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn billing_step_grant_expiry_excludes_failures_across_chunks() {
    let (db, _) = db().await;
    // One more failure than the unchanged 500-row selection; the 10,000
    // examined-unit pass cap still has room for the good grant after them.
    db.collection::<Document>("credit_grants")
        .insert_many((0..501).map(|i| Step::Expiry.poison(i)))
        .await
        .unwrap();
    healthy(&db, Step::Expiry).await;
    assert!(matches!(
        Step::Expiry.run(&db).await,
        Err(AppError::BillingRecoveryFailed {
            failed_units: 501,
            ..
        })
    ));
    assert!(healthy_done(&db, Step::Expiry).await);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_step_query_failure_still_fails_selection() {
    let (db, _) = db().await;
    // A real command failure is different from a bad selected unit: no made-up
    // count is published for rows the query could not enumerate.
    db.collection::<Document>("bad_selection_source")
        .insert_one(doc! {"_id":1,"zero":0,"is_active":true})
        .await
        .unwrap();
    db.create_collection("credit_schedules")
        .view_on("bad_selection_source")
        .pipeline(vec![doc! {"$set":{"amount":{"$divide":[1,"$zero"]}}}])
        .await
        .unwrap();
    let error = Step::Schedule.run(&db).await.unwrap_err();
    assert!(!matches!(error, AppError::BillingRecoveryFailed { .. }));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_step_processing_error_isolated_after_decode() {
    for step in [
        Step::Model,
        Step::Managed,
        Step::Abandon,
        Step::Funding,
        Step::Expiry,
        Step::Ledger,
        Step::Topup,
    ] {
        let (db, _) = db().await;
        healthy(&db, step).await;
        let mut bad = db
            .collection::<Document>(step.collection())
            .find_one(doc! {"_id":"z-good"})
            .await
            .unwrap()
            .unwrap();
        bad.insert("_id", "a-processing");
        if matches!(step, Step::Abandon | Step::Funding) {
            bad.insert("transaction_id", "processing-transaction");
        }
        let blocked = match step {
            Step::Model => doc! {"state":{"$ne":"pending_review"}},
            Step::Managed => doc! {"state":{"$ne":"pending"}},
            Step::Abandon => doc! {"status":{"$ne":"abandoned"}},
            Step::Funding => doc! {"funding.settled":{"$ne":true}},
            Step::Expiry => doc! {"status":{"$ne":"expired"}},
            Step::Ledger => doc! {"issued_ledgered_at":Bson::Null},
            Step::Topup => doc! {"topup_expiry_checked_at":Bson::Null},
            _ => unreachable!(),
        };
        if matches!(step, Step::Model) {
            bad.insert("lease_until", bson::DateTime::from_millis(0));
        }
        if matches!(step, Step::Managed) {
            bad.insert("next_reconcile_at", bson::DateTime::from_millis(0));
        }
        if matches!(step, Step::Expiry) {
            bad.insert("expires_at", bson::DateTime::from_millis(0));
        }
        bad.insert("created_at", bson::DateTime::from_millis(0));
        bad.insert("updated_at", bson::DateTime::from_millis(0));
        db.collection::<Document>(step.collection())
            .insert_one(bad.clone())
            .await
            .unwrap();
        if matches!(step, Step::Abandon) {
            db.collection::<Document>("billing_wallet")
                .update_one(
                    doc! {"_id":"wallet-1"},
                    doc! {"$set":{"reserved_credits":Credits::from_whole(2)}},
                )
                .await
                .unwrap();
        }
        db.run_command(doc!{"collMod":step.collection(),"validator":{"$or":[{"_id":{"$ne":"a-processing"}},blocked]}}).await.unwrap();
        let result = step.run(&db).await;
        assert!(
            healthy_done(&db, step).await,
            "{step:?}: processing failure blocked healthy work"
        );
        assert!(
            matches!(
                result,
                Err(AppError::BillingRecoveryFailed {
                    failed_units: 1,
                    ..
                })
            ),
            "{step:?}: {result:?}"
        );
        let retained = db
            .collection::<Document>(step.collection())
            .find_one(doc! {"_id":"a-processing"})
            .await
            .unwrap()
            .unwrap();
        for field in [
            "released",
            "reserved_credits",
            "remaining",
            "active_settlement",
            "active_topup_expiry",
        ] {
            assert_eq!(retained.get(field), bad.get(field), "{step:?}: {field}");
        }
        db.run_command(doc! {"collMod":step.collection(),"validator":{}})
            .await
            .unwrap();
        if matches!(step, Step::Model | Step::Managed) {
            let field = if matches!(step, Step::Model) {
                "lease_until"
            } else {
                "next_reconcile_at"
            };
            db.collection::<Document>(step.collection())
                .update_one(
                    doc! {"_id":"a-processing"},
                    doc! {"$set":{field:bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
        }
        if matches!(step, Step::Abandon) {
            let wallet = db
                .collection::<crate::models::billing_wallet::BillingWallet>("billing_wallet")
                .find_one(doc! {"_id":"wallet-1"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                wallet.reserved_credits,
                Credits::from_whole(1),
                "failed release retains exactly its own hold"
            );
        }
        step.run(&db).await.unwrap();
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn billing_step_funding_locks_and_pending_reclaims_do_not_block_later_lanes() {
    for (step, collection, poison) in [
        (
            Step::Funding,
            "usage_allowance_periods",
            doc! {"_id":"poison-lock","active_settlement":{"operation_id":"keep","applied":true}},
        ),
        (
            Step::Funding,
            "credit_grants",
            doc! {"_id":"poison-lock","active_settlement":{"operation_id":"keep","applied":true}},
        ),
        (
            Step::Expiry,
            "credit_grants",
            doc! {"_id":"poison-reclaim","status":"active","revoke_requested_at":bson::DateTime::now(),"reserved":Credits::ZERO,"active_settlement":Bson::Null},
        ),
        (
            Step::Schedule,
            "credit_schedule_periods",
            doc! {"_id":"poison-period","status":"disbursing","period_end":bson::DateTime::from_millis(0)},
        ),
    ] {
        let (db, _) = db().await;
        db.collection::<Document>(collection)
            .insert_one(poison.clone())
            .await
            .unwrap();
        healthy(&db, step).await;
        let result = step.run(&db).await;
        assert!(
            healthy_done(&db, step).await,
            "{step:?}/{collection} blocked later lane"
        );
        assert!(
            matches!(
                result,
                Err(AppError::BillingRecoveryFailed {
                    failed_units: 1,
                    ..
                })
            ),
            "{result:?}"
        );
        let retained = db
            .collection::<Document>(collection)
            .find_one(doc! {"_id":poison.get("_id")})
            .await
            .unwrap()
            .unwrap();
        for field in ["active_settlement", "reserved", "status", "period_end"] {
            assert_eq!(retained.get(field), poison.get(field));
        }
        assert!(step.run(&db).await.is_err());
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn billing_step_settlement_sublanes_do_not_starve_final_quantities() {
    let old = bson::DateTime::from_millis(0);
    for poison in [
        doc! {"_id":"a-checkpoint","status":"forwarded","released":false,"usage_checkpoint":{"final_report":true,"recovered":false,"recover_after":old}},
        doc! {"_id":"a-realtime","status":"forwarded","released":false,"realtime_session":{"closed":false,"recover_after":old}},
        doc! {"_id":"a-components","pending_platform_usage":{}},
        doc! {"_id":"a-resale","layer":"platform","pending_resale_quantity":1},
        doc! {"_id":"a-dead","status":"dead_letter","released":false,"quantity":1,"last_error":"billing settlement retries exhausted","updated_at":old},
    ] {
        let (db, _) = db().await;
        db.collection::<Document>(USAGE)
            .insert_one(poison.clone())
            .await
            .unwrap();
        healthy(&db, Step::Settlement).await;
        let result = Step::Settlement.run(&db).await;
        assert!(
            healthy_done(&db, Step::Settlement).await,
            "{:?} blocked final quantity",
            poison.get("_id")
        );
        assert!(
            matches!(
                result,
                Err(AppError::BillingRecoveryFailed {
                    failed_units: 1,
                    ..
                })
            ),
            "{result:?}"
        );
        let saved = db
            .collection::<Document>(USAGE)
            .find_one(doc! {"_id":poison.get("_id")})
            .await
            .unwrap()
            .unwrap();
        for field in [
            "quantity",
            "status",
            "released",
            "usage_checkpoint",
            "realtime_session",
            "pending_platform_usage",
            "pending_resale_quantity",
        ] {
            assert_eq!(saved.get(field), poison.get(field));
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn billing_step_model_claim_rejection_excludes_failed_ids_in_pass() {
    let (db, _) = db().await;
    for i in 0..3 {
        db.collection::<Document>("platform_model_attempts").insert_one(doc!{
            "_id":format!("a-claim-{i}"),"state":"dispatched","lease_until":bson::DateTime::from_millis(0),"reject_claim":true
        }).await.unwrap();
    }
    healthy(&db, Step::Model).await;
    db.run_command(doc!{"collMod":"platform_model_attempts","validator":{"$or":[{"reject_claim":{"$ne":true}},{"state":{"$ne":"recovering"}}]}}).await.unwrap();
    assert!(matches!(
        billing::recovery_batch::TEST_BATCH_LIMIT
            .scope(4, Step::Model.run(&db))
            .await,
        Err(AppError::BillingRecoveryFailed {
            failed_units: 3,
            ..
        })
    ));
    assert!(healthy_done(&db, Step::Model).await);
    for i in 0..3 {
        let bad = db
            .collection::<Document>("platform_model_attempts")
            .find_one(doc! {"_id":format!("a-claim-{i}")})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bad.get_str("state").unwrap(), "dispatched");
        assert!(!bad.contains_key("recovery_token"));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_step_schedule_period_processing_failure_retains_progress() {
    let (db, _) = db().await;
    healthy(&db, Step::Schedule).await;
    let mut bad = db
        .collection::<Document>("credit_schedules")
        .find_one(doc! {"_id":"z-good"})
        .await
        .unwrap()
        .unwrap();
    bad.insert("_id", "a-processing");
    db.collection::<Document>("credit_schedules")
        .insert_one(bad)
        .await
        .unwrap();
    db.create_collection("credit_schedule_periods")
        .validator(doc! {"schedule_id":{"$ne":"a-processing"}})
        .await
        .unwrap();
    assert!(matches!(
        Step::Schedule.run(&db).await,
        Err(AppError::BillingRecoveryFailed {
            failed_units: 1,
            ..
        })
    ));
    assert!(healthy_done(&db, Step::Schedule).await);
    assert_eq!(
        db.collection::<Document>("credit_grants")
            .count_documents(doc! {"schedule_origin.schedule_id":"a-processing"})
            .await
            .unwrap(),
        0
    );
    db.run_command(doc! {"collMod":"credit_schedule_periods","validator":{}})
        .await
        .unwrap();
    Step::Schedule.run(&db).await.unwrap();
    assert_eq!(
        db.collection::<Document>("credit_grants")
            .count_documents(doc! {"schedule_origin.schedule_id":"a-processing"})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

async fn assert_wait_success(db: &Database, step: Step, result: AppResult<()>) {
    assert!(
        result.is_ok(),
        "normal wait failed {}: {result:?}",
        step.name()
    );
    let mut pass = billing::reconcile_status::Pass::new(None);
    pass.record(step.name(), result);
    let runtime = lease_runtime("wait-status", 30_000);
    let token = runtime
        .acquire(db, super::super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    pass.release(db, &token).await.unwrap();
    assert!(billing::reconcile_status::diagnostics(db).await.is_empty());
}

#[tokio::test]
async fn billing_step_busy_claim_and_backoff_are_waits() {
    let (db, commands) = db().await;
    healthy(&db, Step::Settlement).await;
    let funding = UsageFunding {
        settlement_claim_id: Some("live-owner".into()),
        settlement_claimed_at: Some(Utc::now()),
        ..Default::default()
    };
    db.collection::<Document>(USAGE)
        .update_one(
            doc! {"_id":"z-good"},
            doc! {"$set":{"funding":bson::to_bson(&funding).unwrap()}},
        )
        .await
        .unwrap();
    assert_wait_success(&db, Step::Settlement, Step::Settlement.run(&db).await).await;
    let row = db
        .collection::<Document>(USAGE)
        .find_one(doc! {"_id":"z-good"})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.get_bool("released").unwrap());
    assert!(row.get_datetime("settlement_next_retry_at").unwrap() > &bson::DateTime::now());
    commands.lock().unwrap().clear();
    assert_wait_success(&db, Step::Settlement, Step::Settlement.run(&db).await).await;
    assert!(
        !commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == "findAndModify:usage_meter")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_step_pending_grant_journals_are_waits() {
    for step in [Step::Expiry, Step::Ledger, Step::Schedule] {
        let (db, _) = db().await;
        healthy(&db, step).await;
        // Rejection keeps the existing durable pending-journal protocol alive.
        db.create_collection("billing_ledger")
            .validator(doc! {"reject_all":{"$exists":true}})
            .await
            .unwrap();
        assert_wait_success(&db, step, step.run(&db).await).await;
        db.drop().await.unwrap();
    }
    let (db, _) = db().await;
    let mut row = grant("reclaim");
    row.insert("revoke_requested_at", bson::DateTime::now());
    db.collection::<Document>("credit_grants")
        .insert_one(row)
        .await
        .unwrap();
    db.create_collection("billing_ledger")
        .validator(doc! {"reject_all":{"$exists":true}})
        .await
        .unwrap();
    assert_wait_success(&db, Step::Expiry, Step::Expiry.run(&db).await).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_step_failures_write_no_retry_fields() {
    for step in Step::ALL {
        let (db, _) = db().await;
        let poison = step.poison(0);
        db.collection::<Document>(step.collection())
            .insert_one(poison.clone())
            .await
            .unwrap();
        assert!(step.run(&db).await.is_err());
        let saved = db
            .collection::<Document>(step.collection())
            .find_one(doc! {"_id":poison.get("_id")})
            .await
            .unwrap()
            .unwrap();
        assert!(
            !saved
                .keys()
                .any(|k| k.starts_with("recovery_") && k.ends_with("_failed_at")),
            "{} wrote a retry field: {saved:?}",
            step.name()
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn billing_step_ambiguous_expiry_and_quiet_period_are_waits() {
    let (db, _) = db().await;
    healthy(&db, Step::Topup).await;
    let now = bson::DateTime::now();
    db.collection::<Document>("billing_wallet").update_one(doc!{"_id":"z-good"},doc!{"$set":{
        "active_topup_expiry":{"operation_id":"quiet-operation","processing_token":"prior","lease_until":bson::DateTime::from_millis(0),
            "amount":Credits::from_whole(1),"items":[],"created_at":now,"updated_at":now},
        "pending_topup_expiry_credits":Credits::from_whole(1)
    }}).await.unwrap();
    let provider = Provider::default();
    for _ in 0..2 {
        let result = billing::topup_expiry::expire_purchased_credits(&db, &provider, Utc::now())
            .await
            .map(|_| ());
        assert_wait_success(&db, Step::Topup, result).await;
        assert_eq!(provider.expiry_posts.load(Ordering::SeqCst), 1);
        let row = db
            .collection::<Document>("billing_wallet")
            .find_one(doc! {"_id":"z-good"})
            .await
            .unwrap()
            .unwrap();
        assert!(
            row.get_document("active_topup_expiry")
                .unwrap()
                .get_datetime("dispatch_started_at")
                .is_ok()
        );
    }
    db.drop().await.unwrap();
}

struct HistoryOutage;
#[async_trait]
impl LagoApi for HistoryOutage {
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        unreachable!()
    }
    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        unreachable!()
    }
    async fn record_event(&self, _: &LagoEvent) -> Result<LagoAck, LagoError> {
        unreachable!()
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        unreachable!()
    }
    async fn current_usage(&self, _: &str, subscription_id: &str) -> AppResult<LagoUsage> {
        crate::services::billing::lago_client::require_subscription_id(subscription_id)?;
        unreachable!()
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        unreachable!()
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        unreachable!()
    }
    async fn wallet_transactions(&self, _: &str) -> AppResult<Vec<LagoWalletTransaction>> {
        Err(AppError::BillingProviderUnavailable(
            "test history outage".into(),
        ))
    }
}
#[tokio::test]
async fn billing_step_provider_outage_is_a_failure() {
    let (db, _) = db().await;
    healthy(&db, Step::Topup).await;
    let result =
        billing::topup_expiry::expire_purchased_credits(&db, &HistoryOutage, Utc::now()).await;
    assert!(matches!(
        result,
        Err(AppError::BillingRecoveryFailed {
            failed_units: 1,
            ..
        })
    ));
    let mut pass = billing::reconcile_status::Pass::new(None);
    pass.record(Step::Topup.name(), result.map(|_| ()));
    let runtime = lease_runtime("outage-status", 30_000);
    let token = runtime
        .acquire(&db, super::super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    pass.release(&db, &token).await.unwrap();
    let diagnostic = billing::reconcile_status::diagnostics(&db).await;
    assert_eq!(diagnostic.len(), 1);
    assert!(diagnostic[0].detail.contains("topup_expiry"));
    assert!(diagnostic[0].detail.contains("failing_units=1"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_step_error_annotation_is_batch_only_and_client_safe() {
    let source = || AppError::Internal("private-error-sentinel".into());
    assert!(matches!(
        billing::recovery_batch::waiting(source()),
        AppError::Internal(_)
    ));
    let error: AppResult<()> =
        billing::recovery_batch::work(async { Err(billing::recovery_batch::waiting(source())) })
            .await;
    assert!(matches!(
        error,
        Err(AppError::BillingRecoveryWaiting { .. })
    ));
    let safe = serde_json::to_value(source().response_body()).unwrap();
    for error in [
        error.unwrap_err(),
        AppError::BillingRecoveryFailed {
            failed_units: 2,
            source: Box::new(source()),
        },
    ] {
        assert_eq!(error.status_code(), source().status_code());
        assert_eq!(error.error_code(), source().error_code());
        assert_eq!(serde_json::to_value(error.response_body()).unwrap(), safe);
        assert!(
            !serde_json::to_string(&error.response_body())
                .unwrap()
                .contains("private-error-sentinel")
        );
    }
    // A caught wait cannot suppress an unrelated error later in the same unit.
    let mut failures = billing::recovery_batch::Failures::default();
    failures
        .unit(async {
            let _ = billing::recovery_batch::waiting(source());
            Err::<(), _>(AppError::ValidationError("actual failure".into()))
        })
        .await;
    assert!(matches!(
        failures.finish(()),
        Err(AppError::BillingRecoveryFailed {
            failed_units: 1,
            ..
        })
    ));
}

#[tokio::test]
async fn billing_step_consumption_journal_preserves_lock_without_failure() {
    let (db, _) = db().await;
    healthy(&db, Step::Settlement).await;
    let mut grant = grant("consumption");
    grant.insert(
        "active_settlement",
        doc! {"operation_id":"consume-op","usage_row_id":"z-good","reserved":Credits::ZERO,
        "consume":Credits::from_whole(1),"applied":true,"updated_at":bson::DateTime::now()},
    );
    db.collection::<Document>("credit_grants")
        .insert_one(grant)
        .await
        .unwrap();
    db.collection::<Document>(USAGE)
        .update_one(
            doc! {"_id":"z-good"},
            doc! {"$set":{"funding":bson::to_bson(&UsageFunding::default()).unwrap()}},
        )
        .await
        .unwrap();
    db.create_collection("billing_ledger")
        .validator(doc! {"reject_all":{"$exists":true}})
        .await
        .unwrap();
    assert_wait_success(&db, Step::Funding, Step::Funding.run(&db).await).await;
    let saved = db
        .collection::<Document>("credit_grants")
        .find_one(doc! {"_id":"consumption"})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.get_document("active_settlement").is_ok());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_step_selection_command_shapes() {
    fn canonical(value: &mut Bson) {
        match value {
            Bson::DateTime(_) => *value = Bson::String("<clock>".into()),
            Bson::Document(d) => {
                for (_, value) in d.iter_mut() {
                    canonical(value);
                }
            }
            Bson::Array(a) => {
                for value in a {
                    canonical(value);
                }
            }
            _ => {}
        }
    }
    for step in Step::ALL {
        let reads = Arc::new(Mutex::new(Vec::<Document>::new()));
        let (db, _) = db_recording(reads.clone()).await;
        reads.lock().unwrap().clear();
        step.run(&db).await.unwrap();
        let shapes = reads
            .lock()
            .unwrap()
            .iter()
            .map(|c| {
                let mut shape = Document::new();
                for field in [
                    "find",
                    "findAndModify",
                    "filter",
                    "query",
                    "sort",
                    "limit",
                    "hint",
                ] {
                    if let Some(value) = c.get(field) {
                        let mut value = value.clone();
                        canonical(&mut value);
                        shape.insert(field, value);
                    }
                }
                shape
            })
            .collect::<Vec<_>>();
        println!(
            "BILLING_SELECTION {} {}",
            step.name(),
            serde_json::to_string(&shapes).unwrap()
        );
        db.drop().await.unwrap();
    }
}
