use std::future::Future;
use std::time::Duration;

use mongodb::Database;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::errors::{AppError, AppResult};
use crate::models::coordination::CoordinationHolder;
use crate::services::coordination_service::{SlotGrant, SlotStore, SlotToken};

#[derive(Clone, Copy)]
enum SlotUse {
    Capacity,
    Exclusive,
}

#[derive(Clone)]
pub struct RenewableSlotManager {
    db: Database,
    holder: CoordinationHolder,
    ttl: Duration,
    renew_interval: Duration,
}

impl RenewableSlotManager {
    pub(crate) async fn transfer(
        &self,
        session: &mut mongodb::ClientSession,
        token: &SlotToken,
        namespace: &str,
        owner: &str,
        limit: u32,
    ) -> AppResult<Option<SlotGrant>> {
        SlotStore::transfer(
            &self.db,
            session,
            token,
            namespace,
            owner,
            &self.holder,
            self.ttl,
            limit,
        )
        .await
    }

    pub(crate) fn guard(&self, grant: SlotGrant) -> RenewableSlotGuard {
        RenewableSlotGuard::new(
            self.db.clone(),
            grant,
            self.ttl,
            self.renew_interval,
            SlotUse::Capacity,
        )
    }

    pub fn new(
        db: Database,
        holder: CoordinationHolder,
        ttl: Duration,
        renew_interval: Duration,
    ) -> Self {
        Self {
            db,
            holder,
            ttl,
            renew_interval,
        }
    }

    pub async fn acquire(
        &self,
        namespace: &str,
        scope: &str,
        limit: u32,
    ) -> AppResult<Option<RenewableSlotGuard>> {
        self.acquire_for(namespace, scope, limit, SlotUse::Capacity)
            .await
    }

    /// An exclusive operation must stop by the last confirmed lease deadline.
    pub async fn acquire_exclusive(
        &self,
        namespace: &str,
        scope: &str,
    ) -> AppResult<Option<RenewableSlotGuard>> {
        self.acquire_for(namespace, scope, 1, SlotUse::Exclusive)
            .await
    }

    async fn acquire_for(
        &self,
        namespace: &str,
        scope: &str,
        limit: u32,
        use_case: SlotUse,
    ) -> AppResult<Option<RenewableSlotGuard>> {
        let Some(grant) =
            SlotStore::acquire(&self.db, namespace, scope, limit, &self.holder, self.ttl).await?
        else {
            return Ok(None);
        };

        let guard = RenewableSlotGuard::new(
            self.db.clone(),
            grant,
            self.ttl,
            self.renew_interval,
            use_case,
        );
        // A delayed acquisition response can arrive after its confirmed lifetime.
        Ok((!guard.lost.is_cancelled()).then_some(guard))
    }
}

pub struct RenewableSlotGuard {
    db: Database,
    token: SlotToken,
    stop: CancellationToken,
    lost: CancellationToken,
}

impl RenewableSlotGuard {
    pub(crate) fn token(&self) -> &SlotToken {
        &self.token
    }

    fn new(
        db: Database,
        grant: SlotGrant,
        ttl: Duration,
        renew_interval: Duration,
        use_case: SlotUse,
    ) -> Self {
        let stop = CancellationToken::new();
        let lost = CancellationToken::new();
        let db_for_renewal = db.clone();
        let lease = grant.clone();
        let loss = lost.clone();
        let stopped = stop.clone();
        let confirmed_until = grant.dispatched_at + ttl;
        if Instant::now() >= confirmed_until {
            lost.cancel();
        } else {
            crate::services::shutdown::process().spawn_worker(async move {
                tokio::select! {
                    biased;
                    () = stopped.cancelled() => {}
                    () = Self::renew(&db_for_renewal, &lease, ttl, renew_interval, use_case) => loss.cancel(),
                }
            });
        }

        Self {
            db,
            token: grant.token,
            stop,
            lost,
        }
    }

    async fn renew(
        db: &Database,
        grant: &SlotGrant,
        ttl: Duration,
        interval: Duration,
        use_case: SlotUse,
    ) {
        // MongoDB measures its TTL from $$NOW during the write. Counting from
        // dispatch keeps a slow acknowledgement from extending local ownership.
        let lease = &grant.token;
        let mut deadline = grant.dispatched_at + ttl;
        let mut next = grant.dispatched_at + interval;
        let first_retry = interval.min(Duration::from_secs(1));
        let mut backoff = first_retry;
        let mut last_error = None;
        loop {
            sleep_until(next.min(deadline)).await;
            let started = Instant::now();
            if started >= deadline {
                break;
            }
            // A capacity renewal may already have matched on the primary while
            // its acknowledgement is outstanding. Wait at most its own TTL.
            let reply_deadline = match use_case {
                SlotUse::Exclusive => deadline,
                SlotUse::Capacity => started + ttl,
            };
            let result = tokio::select! {
                biased;
                () = sleep_until(reply_deadline) => break,
                result = SlotStore::renew(db, lease, ttl) => result,
            };
            match result {
                Ok(true) => {
                    deadline = started + ttl;
                    next = started + interval;
                    backoff = first_retry;
                    last_error = None;
                }
                Ok(false) => {
                    tracing::warn!(namespace = %lease.namespace, slot = lease.slot, "Cluster capacity slot ownership was lost");
                    return;
                }
                Err(error) => {
                    tracing::warn!(namespace = %lease.namespace, slot = lease.slot, %error, "Cluster capacity slot renewal failed, retrying within lease");
                    last_error = Some(error);
                    next = Instant::now() + backoff;
                    backoff = backoff.saturating_mul(2).min(interval);
                }
            }
        }
        tracing::error!(namespace = %lease.namespace, slot = lease.slot,
            error = last_error.as_ref().map(tracing::field::display),
            "Cluster capacity slot expired without a confirmed renewal");
    }

    pub async fn cancelled(&self) {
        self.lost.cancelled().await;
    }

    /// Whether this is the longest-held live slot of its namespace and scope.
    /// Owns its inputs, so it can run on another task.
    pub fn is_oldest(&self) -> impl Future<Output = AppResult<bool>> + Send + 'static {
        let (db, token) = (self.db.clone(), self.token.clone());
        async move { SlotStore::is_oldest_live(&db, &token).await }
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.lost.clone()
    }

    pub async fn run_until_lost<F, T, E>(&self, future: F) -> Result<T, E>
    where
        F: Future<Output = Result<T, E>>,
        E: From<AppError>,
    {
        tokio::select! {
            biased;
            () = self.cancelled() => Err(AppError::RateLimited.into()),
            result = future => result,
        }
    }
}

impl Drop for RenewableSlotGuard {
    fn drop(&mut self) {
        self.stop.cancel();
        let db = self.db.clone();
        let token = self.token.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            crate::services::shutdown::process().spawn_cleanup(async move {
                if let Err(error) = SlotStore::release(&db, &token).await {
                    tracing::warn!(
                        namespace = %token.namespace,
                        slot = token.slot,
                        %error,
                        "Failed to release cluster capacity slot"
                    );
                }
            });
        }
    }
}
