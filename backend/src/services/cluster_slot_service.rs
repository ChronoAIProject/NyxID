use std::future::Future;
use std::time::Duration;

use mongodb::Database;
use tokio_util::sync::CancellationToken;

use crate::errors::{AppError, AppResult};
use crate::models::coordination::CoordinationHolder;
use crate::services::coordination_service::{SlotStore, SlotToken};

#[derive(Clone)]
pub struct RenewableSlotManager {
    db: Database,
    holder: CoordinationHolder,
    ttl: Duration,
    renew_interval: Duration,
}

impl RenewableSlotManager {
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
        let Some(token) =
            SlotStore::acquire(&self.db, namespace, scope, limit, &self.holder, self.ttl).await?
        else {
            return Ok(None);
        };

        Ok(Some(RenewableSlotGuard::new(
            self.db.clone(),
            token,
            self.ttl,
            self.renew_interval,
        )))
    }
}

pub struct RenewableSlotGuard {
    db: Database,
    token: SlotToken,
    stop: CancellationToken,
    lost: CancellationToken,
}

impl RenewableSlotGuard {
    fn new(db: Database, token: SlotToken, ttl: Duration, renew_interval: Duration) -> Self {
        let stop = CancellationToken::new();
        let lost = CancellationToken::new();
        let renewal_db = db.clone();
        let renewal_token = token.clone();
        let renewal_stop = stop.clone();
        let renewal_lost = lost.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = renewal_stop.cancelled() => return,
                    () = tokio::time::sleep(renew_interval) => {}
                }

                match SlotStore::renew(&renewal_db, &renewal_token, ttl).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(
                            namespace = %renewal_token.namespace,
                            slot = renewal_token.slot,
                            "Cluster capacity slot ownership was lost"
                        );
                        renewal_lost.cancel();
                        return;
                    }
                    Err(error) => {
                        tracing::error!(
                            namespace = %renewal_token.namespace,
                            slot = renewal_token.slot,
                            %error,
                            "Cluster capacity slot renewal failed"
                        );
                        renewal_lost.cancel();
                        return;
                    }
                }
            }
        });

        Self {
            db,
            token,
            stop,
            lost,
        }
    }

    pub async fn cancelled(&self) {
        self.lost.cancelled().await;
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.lost.clone()
    }

    pub async fn run_until_lost<F, T>(&self, future: F) -> AppResult<T>
    where
        F: Future<Output = AppResult<T>>,
    {
        tokio::select! {
            biased;
            () = self.cancelled() => Err(AppError::RateLimited),
            result = future => result,
        }
    }
}

impl Drop for RenewableSlotGuard {
    fn drop(&mut self) {
        self.stop.cancel();
        let db = self.db.clone();
        let token = self.token.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
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
