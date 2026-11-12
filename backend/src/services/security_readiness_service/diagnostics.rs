use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use mongodb::Database;
use sha2::{Digest, Sha256};
use tracing::instrument::WithSubscriber;

use super::Readiness;
use crate::services::{audit_service, coordination_service::RateWindowStore, transaction_service};

pub(super) const EVENT: &str = "security_readiness_serving_with_roster_warnings";
pub(super) const INTERVAL: Duration = Duration::from_secs(3600);
const RETRY_DELAY: Duration = Duration::from_secs(60);
const MAX_LOCAL_CONDITIONS: usize = 256;
static THROTTLE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

fn condition_key(report: &Readiness) -> String {
    let mut warnings: Vec<_> = report.roster_warnings.iter().collect();
    warnings.sort();
    warnings.dedup();
    let mut digest = Sha256::new();
    for warning in warnings {
        digest.update((warning.len() as u64).to_be_bytes());
        digest.update(warning.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn admit_local(throttle: &mut HashMap<String, Instant>, key: String, now: Instant) -> bool {
    if throttle
        .get(&key)
        .is_some_and(|not_before| now < *not_before)
    {
        return false;
    }
    if throttle.len() >= MAX_LOCAL_CONDITIONS
        && !throttle.contains_key(&key)
        && let Some(oldest) = throttle
            .iter()
            .min_by_key(|(_, at)| *at)
            .map(|(key, _)| key.clone())
    {
        throttle.remove(&oldest);
    }
    throttle.insert(key, now + INTERVAL);
    true
}

fn retry_local(key: &str, admitted_until: Instant, now: Instant) {
    let mut throttle = THROTTLE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(not_before) = throttle.get_mut(key)
        && *not_before == admitted_until
    {
        // An evicted/re-admitted condition belongs to its newer task.
        *not_before = now + RETRY_DELAY;
    }
}

/// Diagnostic state only. Serving authority always comes from MongoDB.
/// Admit locally before spawning or doing I/O; do not wait for a busy lock.
pub(super) fn warn(
    db: &Database,
    capability: &str,
    report: &Readiness,
) -> Option<tokio::task::JoinHandle<()>> {
    warn_at(db, capability, report, Instant::now())
}

fn warn_at(
    db: &Database,
    capability: &str,
    report: &Readiness,
    now: Instant,
) -> Option<tokio::task::JoinHandle<()>> {
    if !report.serving_with_roster_warnings {
        return None;
    }
    let condition = condition_key(report);
    let key = condition.clone();
    #[cfg(test)]
    let key = format!("{}:{key}", db.name());
    {
        let Ok(mut throttle) = THROTTLE.get_or_init(Mutex::default).try_lock() else {
            return None;
        };
        if !admit_local(&mut throttle, key.clone(), now) {
            return None;
        }
    }
    let db = db.clone();
    let capability = capability.to_owned();
    let warnings = report.roster_warnings.clone();
    let active_since = report.active_since;
    let live_generations = report.live_generations;
    let expected_replicas = report.expected_replicas.clone();
    Some(tokio::spawn(
        async move {
            // A stuck diagnostic must not accumulate unbounded work or hold a
            // request open. Failed diagnostics retry after a short local delay.
            let work = async {
                let key = audit_service::audit_chain_hmac_key().ok_or_else(|| {
                    crate::errors::AppError::Internal("Audit chain not initialized".into())
                })?;
                let event_data = serde_json::json!({
                    "first_observed_capability": capability,
                    "roster_warnings": warnings,
                    "active_since": active_since,
                    "live_generations": live_generations,
                    "expected_replicas": expected_replicas,
                });
                // An audit failure must not consume the shared hour. Both facts
                // commit together, so a short retry can actually emit the event.
                let context = (db.clone(), condition.clone(), key, event_data);
                let emitted = transaction_service::run_db_only(
                    &db,
                    &context,
                    |session, (db, condition, key, event_data)| {
                        Box::pin(async move {
                            if !RateWindowStore::admit_in_session(
                                db, EVENT, condition, 1, INTERVAL, session,
                            )
                            .await?
                            {
                                return Ok(false);
                            }
                            audit_service::log_system_event_in_session(
                                db,
                                session,
                                key,
                                EVENT,
                                event_data.clone(),
                            )
                            .await?;
                            Ok(true)
                        })
                    },
                )
                .await?;
                if emitted {
                    tracing::warn!(
                        first_observed_capability = capability,
                        ?warnings,
                        ?active_since,
                        live_generations,
                        ?expected_replicas,
                        "Serving an activated security capability with roster warnings"
                    );
                }
                Ok::<_, crate::errors::AppError>(())
            };
            match tokio::time::timeout(Duration::from_secs(5), work).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    retry_local(&key, now + INTERVAL, Instant::now());
                    tracing::debug!(%error, "Serving readiness diagnostic unavailable")
                }
                Err(_) => {
                    retry_local(&key, now + INTERVAL, Instant::now());
                    tracing::debug!("Serving readiness diagnostic timed out");
                }
            }
        }
        .with_current_subscriber(),
    ))
}

#[cfg(test)]
pub(super) fn reset_local(db: &Database, report: &Readiness) {
    THROTTLE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .remove(&format!("{}:{}", db.name(), condition_key(report)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn local_conditions_expire_and_evict_oldest_at_the_bound() {
        let mut throttle = HashMap::new();
        let start = Instant::now();
        for i in 0..MAX_LOCAL_CONDITIONS + 1 {
            assert!(admit_local(
                &mut throttle,
                i.to_string(),
                start + Duration::from_nanos(i as u64)
            ));
        }
        assert_eq!(throttle.len(), MAX_LOCAL_CONDITIONS);
        assert!(!throttle.contains_key("0"));
        assert!(!admit_local(
            &mut throttle,
            "1".into(),
            start + Duration::from_secs(3599)
        ));
        assert!(admit_local(
            &mut throttle,
            "1".into(),
            start + INTERVAL + Duration::from_secs(1)
        ));
    }

    #[tokio::test]
    async fn failed_admission_retries_after_one_minute_without_a_retry_storm() {
        use super::super::{CAPABILITIES, controlled_ready_test_replica, overlapping_test_replica};
        use crate::models::{audit_log, coordination::RATE_WINDOW_COLLECTION_NAME};
        let db = crate::test_utils::connect_test_database("roster_diagnostic_retry")
            .await
            .unwrap();
        let heartbeat = controlled_ready_test_replica(&db).await;
        heartbeat.abort();
        overlapping_test_replica(&db).await;
        let report = super::super::report(&db, CAPABILITIES[0]).await.unwrap();
        db.create_collection(RATE_WINDOW_COLLECTION_NAME)
            .validator(doc! {"namespace": {"$ne": EVENT}})
            .await
            .unwrap();
        warn(&db, CAPABILITIES[0], &report).unwrap().await.unwrap();
        let key = format!("{}:{}", db.name(), condition_key(&report));
        let retry_at = THROTTLE.get().unwrap().lock().unwrap()[&key];
        assert!(retry_at <= Instant::now() + RETRY_DELAY);
        assert!(retry_at > Instant::now() + Duration::from_secs(55));
        let audits = db.collection::<mongodb::bson::Document>(audit_log::COLLECTION_NAME);
        assert_eq!(
            audits
                .count_documents(doc! {"event_type": EVENT})
                .await
                .unwrap(),
            0
        );
        db.run_command(doc! {"collMod": RATE_WINDOW_COLLECTION_NAME, "validator": {}})
            .await
            .unwrap();
        for _ in 0..16 {
            assert!(
                warn_at(
                    &db,
                    CAPABILITIES[0],
                    &report,
                    retry_at - Duration::from_secs(1)
                )
                .is_none()
            );
        }
        warn_at(&db, CAPABILITIES[0], &report, retry_at)
            .unwrap()
            .await
            .unwrap();
        assert_eq!(
            audits
                .count_documents(doc! {"event_type": EVENT})
                .await
                .unwrap(),
            1
        );
        assert!(warn_at(&db, CAPABILITIES[0], &report, retry_at + RETRY_DELAY).is_none());

        let mut changed = report;
        changed
            .roster_warnings
            .push("undeclared_replica:new".into());
        db.run_command(doc! {"collMod": audit_log::COLLECTION_NAME,
        "validator": {"event_type": {"$ne": EVENT}}})
            .await
            .unwrap();
        warn(&db, CAPABILITIES[0], &changed).unwrap().await.unwrap();
        let key = format!("{}:{}", db.name(), condition_key(&changed));
        let retry_at = THROTTLE.get().unwrap().lock().unwrap()[&key];
        assert_eq!(
            audits
                .count_documents(doc! {"event_type": EVENT})
                .await
                .unwrap(),
            1
        );
        db.run_command(doc! {"collMod": audit_log::COLLECTION_NAME, "validator": {}})
            .await
            .unwrap();
        assert!(
            warn_at(
                &db,
                CAPABILITIES[0],
                &changed,
                retry_at - Duration::from_secs(1)
            )
            .is_none()
        );
        warn_at(&db, CAPABILITIES[0], &changed, retry_at)
            .unwrap()
            .await
            .unwrap();
        assert_eq!(
            audits
                .count_documents(doc! {"event_type": EVENT})
                .await
                .unwrap(),
            2
        );
    }
}
