use super::*;
use std::time::{Duration as StdDuration, Instant};

struct InvalidatingProvider {
    db: Database,
    calls: AtomicUsize,
    invalidations: usize,
    invalidated_plan: &'static str,
    owner_invalidation: bool,
    first_delay: StdDuration,
    second_delay: StdDuration,
    fail_on: Option<usize>,
}
impl InvalidatingProvider {
    fn new(db: &Database, invalidations: usize) -> Self {
        Self {
            db: db.clone(),
            calls: AtomicUsize::new(0),
            invalidations,
            invalidated_plan: "standard",
            owner_invalidation: false,
            first_delay: StdDuration::ZERO,
            second_delay: StdDuration::ZERO,
            fail_on: None,
        }
    }
}
#[async_trait::async_trait]
impl LagoApi for InvalidatingProvider {
    async fn tax_config(&self, _: &str, _: &str) -> AppResult<LagoTaxConfig> {
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
        let rate = if attempt == 0 { "5" } else { "9" };
        let config = Provider::new(rate).config();
        let delay = if attempt == 0 {
            self.first_delay
        } else {
            self.second_delay
        };
        tokio::time::sleep(delay).await;
        if self.fail_on == Some(attempt) {
            return Err(AppError::BillingProviderUnavailable(
                "test tax outage".into(),
            ));
        }
        // The read has captured its response, but has not returned to the
        // publication path. This is the price-save interleaving from T1.
        if attempt < self.invalidations {
            if self.owner_invalidation {
                invalidate_owner(&self.db, "owner").await?;
            } else {
                invalidate_plan(&self.db, self.invalidated_plan).await?;
            }
        }
        Ok(config)
    }
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
    async fn current_usage(&self, _: &str, _: &str) -> AppResult<LagoUsage> {
        unreachable!()
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        unreachable!()
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        unreachable!()
    }
}

async fn inline_retry_succeeds(existing: bool, owner_invalidation: bool) {
    let (db, wallet) = fixture(existing).await;
    let provider = InvalidatingProvider {
        owner_invalidation,
        invalidated_plan: if existing { "standard" } else { "another-plan" },
        ..InvalidatingProvider::new(&db, 1)
    };
    let result = ensure_fresh(&db, &provider, &wallet, 900).await;
    let saved = profile(&db).await;
    db.drop().await.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let accepted = result.expect("inline ordering loss must get one fresh attempt");
    assert_eq!(rate(&accepted), "9".parse().unwrap());
    let saved = saved.unwrap();
    assert_eq!(rate(&saved), rate(&accepted));
    assert!(fresh(&saved, &wallet, 900));
    assert!(saved.refresh_failed_at.is_none());
}

#[tokio::test]
async fn tax_ordering_retry_stale_admission_after_plan_invalidation() {
    inline_retry_succeeds(true, false).await;
}

#[tokio::test]
async fn tax_ordering_retry_cold_admission_after_plan_epoch_change() {
    inline_retry_succeeds(false, false).await;
}

#[tokio::test]
async fn tax_ordering_retry_recaptures_cold_owner_counter() {
    inline_retry_succeeds(false, true).await;
}

#[tokio::test]
async fn tax_ordering_retry_two_losses_fail_closed_without_backoff() {
    for existing in [true, false] {
        let (db, wallet) = fixture(existing).await;
        let provider = InvalidatingProvider::new(&db, 2);
        let result = ensure_fresh(&db, &provider, &wallet, 900).await;
        let saved = profile(&db).await;
        db.drop().await.unwrap();
        assert!(result.is_err());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        assert!(saved.is_none_or(|p| {
            p.refresh_failed_at.is_none() && p.fetched_at == chrono::DateTime::UNIX_EPOCH
        }));
    }
}

#[tokio::test]
async fn tax_ordering_retry_provider_failure_never_retries_and_keeps_backoff() {
    for existing in [true, false] {
        let (db, wallet) = fixture(existing).await;
        let failure = InvalidatingProvider {
            fail_on: Some(0),
            ..InvalidatingProvider::new(&db, 0)
        };
        let result = ensure_fresh(&db, &failure, &wallet, 900).await;
        let saved = profile(&db).await.unwrap();
        let blocked = Provider::new("9");
        let backoff = ensure_fresh(&db, &blocked, &wallet, 900).await;
        db.drop().await.unwrap();
        assert!(result.is_err());
        assert!(backoff.is_err());
        assert!(saved.refresh_failed_at.is_some());
        assert_eq!(failure.calls.load(Ordering::SeqCst), 1);
        assert_eq!(blocked.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn tax_ordering_retry_second_provider_failure_records_backoff() {
    let (db, wallet) = fixture(true).await;
    let provider = InvalidatingProvider {
        fail_on: Some(1),
        ..InvalidatingProvider::new(&db, 1)
    };
    let result = ensure_fresh(&db, &provider, &wallet, 900).await;
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert!(result.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert!(saved.refresh_failed_at.is_some());
}

#[tokio::test]
async fn tax_ordering_retry_uses_remaining_provider_deadline() {
    let (db, wallet) = fixture(true).await;
    let provider = InvalidatingProvider {
        first_delay: StdDuration::from_millis(4200),
        second_delay: StdDuration::from_secs(2),
        ..InvalidatingProvider::new(&db, 1)
    };
    let started = Instant::now();
    let result = ensure_fresh(&db, &provider, &wallet, 900).await;
    let elapsed = started.elapsed();
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert!(
        result.is_err(),
        "the second read must not get another five seconds"
    );
    assert!(
        saved.refresh_failed_at.is_some(),
        "provider timeout retains backoff"
    );
    assert!(elapsed >= StdDuration::from_millis(4900), "{elapsed:?}");
    assert!(elapsed < StdDuration::from_millis(6100), "{elapsed:?}");
    println!("TAX_RETRY_ELAPSED {elapsed:?}");
}

#[tokio::test]
async fn tax_ordering_retry_forced_refresh_remains_one_attempt() {
    for existing in [true, false] {
        let (db, wallet) = fixture(existing).await;
        let provider = InvalidatingProvider::new(&db, 1);
        let result = refresh(&db, &provider, &wallet, 900).await;
        let saved = profile(&db).await;
        db.drop().await.unwrap();
        assert!(result.is_err());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert!(saved.is_none_or(|p| p.refresh_failed_at.is_none()));
    }
}

#[tokio::test]
async fn tax_ordering_retry_deadline_also_bounds_publication_wait() {
    let (db, wallet) = fixture(true).await;
    let provider = Provider::new("5");
    let gate = Gate::new(Phase::BeforePublication);
    let started = Instant::now();
    let result = tokio::time::timeout(
        StdDuration::from_secs(7),
        GATE.scope(gate.clone(), ensure_fresh(&db, &provider, &wallet, 900)),
    )
    .await;
    let elapsed = started.elapsed();
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert!(gate.used.load(Ordering::SeqCst));
    assert!(
        result
            .expect("inline publication wait must respect its overall bound")
            .is_err()
    );
    assert!(elapsed >= StdDuration::from_millis(5900), "{elapsed:?}");
    assert!(elapsed < StdDuration::from_millis(6200), "{elapsed:?}");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(saved.refresh_failed_at.is_none());
    assert_eq!(saved.fetched_at, chrono::DateTime::UNIX_EPOCH);
}
