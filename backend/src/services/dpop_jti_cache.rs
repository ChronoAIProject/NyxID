use std::time::Duration;

use crate::errors::AppResult;
use crate::services::coordination_service::ReplayStore;

pub const DPOP_JTI_CACHE_TTL_SECS: u64 = 600;
const DPOP_REPLAY_NAMESPACE: &str = "dpop-jti";

pub async fn claim_jti(db: &mongodb::Database, jti: &str) -> AppResult<bool> {
    ReplayStore::claim(
        db,
        DPOP_REPLAY_NAMESPACE,
        jti,
        Duration::from_secs(DPOP_JTI_CACHE_TTL_SECS),
    )
    .await
}
