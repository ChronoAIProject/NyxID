//! Process-local discovery hint. This never grants diagnostics authority.
use super::{admin, cache_key};
use crate::{errors::AppResult, services::shutdown::Runtime};
use mongodb::Database;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
// Local admin responses never wait for manifest/network work or a slow primary.
const READ_DEADLINE: Duration = Duration::from_millis(200);

#[derive(Default)]
struct Hint {
    offered: bool,
    generation: u64,
}
static HINTS: LazyLock<Mutex<HashMap<String, Hint>>> = LazyLock::new(Default::default);

/// Only an in-process lookup: no database commands or person-specific state.
pub fn offered(db: &Database) -> bool {
    HINTS
        .lock()
        .expect("public crash hint")
        .get(&cache_key(db))
        .is_some_and(|hint| hint.offered)
}

pub(super) async fn invalidate(db: &Database) {
    HINTS
        .lock()
        .expect("public crash hint")
        .entry(cache_key(db))
        .or_default()
        .generation += 1;
    refresh(db).await;
}

async fn refresh(db: &Database) {
    refresh_from(db, admin::rollout(db)).await;
}

async fn refresh_from(db: &Database, read: impl Future<Output = AppResult<admin::Rollout>>) {
    let key = cache_key(db);
    let generation = HINTS
        .lock()
        .expect("public crash hint")
        .entry(key.clone())
        .or_default()
        .generation;
    if let Ok(Ok(rollout)) = tokio::time::timeout(READ_DEADLINE, read).await {
        let mut hints = HINTS.lock().expect("public crash hint");
        let hint = hints.entry(key).or_default();
        // Do not overwrite a newer local flag write with an older background read.
        if hint.generation == generation {
            hint.offered = matches!(rollout, admin::Rollout::Global | admin::Rollout::Cohort);
        }
    }
    // Failure retains the last value. This only gates a client's preference GET;
    // the real offer, readiness and consent are still decided on every request.
}

pub async fn run(db: Database, shutdown: Runtime) {
    run_every(db, shutdown, REFRESH_INTERVAL).await;
}

async fn run_every(db: Database, shutdown: Runtime, cadence: Duration) {
    let mut interval = tokio::time::interval(cadence);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Tokio's first tick is immediate, including on a default-off deployment.
    while let Some(_work) = shutdown.tick(&mut interval).await {
        refresh(&db).await;
    }
}

#[cfg(test)]
mod tests;
