use super::*;
use crate::test_utils::with_manual_clock;
use std::time::Duration as StdDuration;

#[derive(Clone)]
enum TaxRead {
    TimedReads(Arc<Mutex<Vec<String>>>),
    TimeoutFirst,
    LoseLease(mongodb::Database),
}

tokio::task_local! { static TAX_READ: TaxRead; }

pub(super) async fn on_tax_read(customer: &str) {
    match TAX_READ.try_with(Clone::clone) {
        Ok(TaxRead::TimedReads(commands)) => {
            commands.lock().unwrap().push("tax_config".into());
            tokio::time::advance(StdDuration::from_millis(249)).await;
        }
        Ok(TaxRead::TimeoutFirst) if customer == "a-bad" => {
            tokio::time::advance(StdDuration::from_secs(6)).await;
            std::future::pending().await
        }
        Ok(TaxRead::TimeoutFirst) => tokio::time::advance(StdDuration::from_millis(3500)).await,
        Ok(TaxRead::LoseLease(db)) => {
            with_manual_clock(async {
                let changed = db
                    .collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":RECONCILE_LEASE},
                        doc! {"$set":{"lease_id":"replaced-token"}},
                    )
                    .await
                    .unwrap();
                assert_eq!(changed.modified_count, 1);
                tokio::time::advance(StdDuration::from_secs(2)).await;
                LEASE_LOSS_OBSERVED.with(Arc::clone).notified().await;
            })
            .await;
        }
        Err(_) => {}
    }
}

#[tokio::test]
async fn tax_refresh_budget_resumes_after_prices_in_the_same_pass() {
    let f = Fixture::new().await;
    let profiles =
        f.db.collection::<crate::models::billing_tax_profile::BillingTaxProfile>(
            crate::models::billing_tax_profile::COLLECTION_NAME,
        );
    let mut stale = Vec::new();
    let mut expected = Vec::new();
    for age_order in 0..85 {
        // The oldest profile has the highest wallet ID.
        let id = format!("wallet-{:02}", 84 - age_order);
        f.wallet(&id).await;
        let mut profile = billing::taxes::resolve_config(
            &id,
            &format!("sub-{id}"),
            &billing::taxes::zero_config(),
        )
        .unwrap();
        profile.fetched_at = chrono::DateTime::from_timestamp_millis(age_order).unwrap();
        stale.push(profile);
        expected.push(format!("tax:{id}"));
    }
    profiles.insert_many(stale).await.unwrap();
    f.commands.lock().unwrap().clear();
    let started = bson::DateTime::now();

    let runtime = cluster_lease_runtime();
    // Database lease time is independent of the simulated provider clock.
    let token = LeaseStore::acquire(
        &f.db,
        RECONCILE_LEASE,
        &runtime.holder,
        StdDuration::from_secs(3600),
    )
    .await
    .unwrap()
    .unwrap();
    let mut pass = Pass::new(None);
    // Wallet 81 crosses the 20-second boundary (80 * 249 ms = 19.920 s).
    let result = with_manual_clock(TAX_READ.scope(
        TaxRead::TimedReads(f.commands.clone()),
        f.reconciler.run_steps(&mut pass, &token),
    ))
    .await;
    assert!(pass.release(&f.db, &token).await.unwrap());
    let result = pass.result(result);
    let diagnostics = billing::reconcile_status::diagnostics(&f.db).await;
    if let Some(d) = diagnostics
        .iter()
        .find(|d| d.code == "billing_reconcile_steps")
    {
        panic!("{}: {}", d.code, d.detail);
    }
    result.unwrap();
    let calls: Vec<_> = f
        .provider
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.starts_with("tax:"))
        .cloned()
        .collect();
    assert_eq!(calls, expected);

    let commands = f.commands.lock().unwrap().clone();
    let reads_per_sweep: Vec<_> = commands
        .split(|c| c == "aggregate:billing_wallet")
        .map(|batch| batch.iter().filter(|c| *c == "tax_config").count())
        .filter(|n| *n > 0)
        .collect();
    assert_eq!(reads_per_sweep, [81, 4]);
    assert_eq!(
        profiles
            .count_documents(
                doc! {"revision":1_i64, "fetched_at":{"$gte":started}, "refresh_failed_at":null}
            )
            .await
            .unwrap(),
        85
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn tax_refresh_budget_keeps_provider_timeout_and_finishes_later_wallet() {
    let f = Fixture::new().await;
    f.wallet("a-bad").await;
    let healthy = ["b-good", "c-good", "d-good", "e-good"];
    for id in healthy {
        f.wallet(id).await;
    }
    f.wallet("z-next").await;
    // One provider expiry and four successes reach the budget exactly.
    let result =
        with_manual_clock(TAX_READ.scope(TaxRead::TimeoutFirst, Step::Taxes.run(&f))).await;
    assert!(
        matches!(&result, Err(AppError::BillingRecoveryFailed { failed_units: 1, source })
        if matches!(source.as_ref(), AppError::BillingProviderUnavailable(_)) && source.error_code() == 11302),
        "{result:?}"
    );
    for id in healthy {
        assert!(f.healthy_done(Step::Taxes, id).await);
    }
    assert!(!f.healthy_done(Step::Taxes, "z-next").await);
    f.status(Step::Taxes, result, true).await;
    let diagnostics = billing::reconcile_status::diagnostics(&f.db).await;
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "billing_reconcile_steps"
                && d.detail.contains("step=taxes_before_recovery ")
                && d.detail.contains("error_code=11302 failing_units=1"))
    );
    Step::Taxes.run(&f).await.unwrap();
    assert!(f.healthy_done(Step::Taxes, "a-bad").await);
    assert!(f.healthy_done(Step::Taxes, "z-next").await);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn tax_refresh_budget_stops_after_current_wallet_on_lease_loss() {
    let f = Fixture::new().await;
    f.wallet("a-first").await;
    f.wallet("z-next").await;
    let lease = ClusterLeaseRuntime::new(
        cluster_lease_runtime().holder.clone(),
        StdDuration::from_secs(30),
        StdDuration::from_secs(1),
    );
    LEASE_LOSS_OBSERVED
        .scope(
            Arc::new(tokio::sync::Notify::new()),
            TAX_READ.scope(
                TaxRead::LoseLease(f.db.clone()),
                f.reconciler.run_once_with_runtime(&lease),
            ),
        )
        .await
        .unwrap();
    assert!(f.healthy_done(Step::Taxes, "a-first").await);
    assert!(!f.healthy_done(Step::Taxes, "z-next").await);
    assert_eq!(f.provider.calls.lock().unwrap().as_slice(), ["tax:a-first"]);
    f.db.drop().await.unwrap();
}
