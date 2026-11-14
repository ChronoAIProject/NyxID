//! First-party, policy-2 diagnostic admission and owner-linked local storage.
pub mod admin;
pub(crate) mod logging;
pub mod public_config;
pub mod releases;
pub mod validation;
use crate::{
    errors::{AppError, AppResult},
    models::client_error_report::*,
    services::{analytics_consent_service as consent, crash_report_export::CrashConfig},
};
use chrono::{DateTime, Utc};
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};
use std::time::Duration;

pub const FLAG: &str = super::feature_flag_service::CRASH_REPORTS_FLAG_KEY;

/// Only definitive denials may destroy a client's pending reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticsStatus {
    Available,
    ConsentRequired,
    NotOffered,
    Unknown,
}
impl DiagnosticsStatus {
    pub fn offered(self) -> bool {
        matches!(self, Self::Available | Self::ConsentRequired)
    }
    pub fn admission(self) -> AppResult<()> {
        match self {
            Self::Available | Self::ConsentRequired => Ok(()),
            Self::NotOffered => Err(AppError::CrashReportingUnavailable),
            Self::Unknown => Err(AppError::SecurityReadinessUnavailable),
        }
    }
}
// Absence only: a positive rollout is always resolved live. This process serves
// one application database; isolated test databases have unique names.
static NEGATIVE_SCOPES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(Default::default);
fn cache_key(db: &Database) -> String {
    db.name().to_owned()
}
pub async fn invalidate_rollout_cache(db: &Database) {
    NEGATIVE_SCOPES
        .lock()
        .expect("rollout cache")
        .remove(&cache_key(db));
    public_config::invalidate(db).await;
}
async fn has_enabled_scope(db: &Database) -> AppResult<bool> {
    let key = cache_key(db);
    if NEGATIVE_SCOPES
        .lock()
        .expect("rollout cache")
        .get(&key)
        .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
    {
        return Ok(false);
    }
    let enabled = db
        .collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .find_one(doc! {"flag_key": FLAG, "definition_id": null, "enabled": true})
        .projection(doc! {"_id":1})
        .await?
        .is_some();
    if !enabled {
        let mut cache = NEGATIVE_SCOPES.lock().expect("rollout cache");
        cache.retain(|_, at| at.elapsed() < Duration::from_secs(5));
        if cache.len() < 1024 {
            cache.insert(key, std::time::Instant::now());
        }
    }
    Ok(enabled)
}
async fn offered_status(db: &Database, actor: &str) -> AppResult<DiagnosticsStatus> {
    if !has_enabled_scope(db).await? {
        return Ok(DiagnosticsStatus::NotOffered);
    }
    let (enabled, killed) = super::feature_flag_service::resolve_crash_reporting(db, actor).await?;
    if killed || !enabled {
        return Ok(DiagnosticsStatus::NotOffered);
    }
    super::security_readiness_service::require_serving(db, FLAG).await?;
    Ok(DiagnosticsStatus::ConsentRequired)
}
async fn bounded_policy_status(
    check: impl std::future::Future<Output = AppResult<DiagnosticsStatus>>,
) -> DiagnosticsStatus {
    tokio::time::timeout(Duration::from_millis(250), check)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(DiagnosticsStatus::Unknown)
}
pub async fn eligibility(db: &Database, actor: &str) -> DiagnosticsStatus {
    bounded_policy_status(offered_status(db, actor)).await
}
#[cfg(test)]
pub async fn available(db: &Database, actor: &str) -> bool {
    eligibility(db, actor).await.offered()
}
pub async fn policy_status(
    db: &Database,
    actor: &str,
    value: &crate::models::user::AnalyticsConsent,
) -> DiagnosticsStatus {
    let status = eligibility(db, actor).await;
    if status.offered()
        && value.enabled
        && value.policy_version.as_deref() == Some(consent::POLICY_VERSION_DIAGNOSTICS)
    {
        DiagnosticsStatus::Available
    } else {
        status
    }
}
pub async fn rate_limits(db: &Database, actor: &str, ip: Option<&str>) -> AppResult<()> {
    let limits = [
        ("crash:account-minute", actor, 5, 60),
        ("crash:account-day", actor, 50, 86400),
        ("crash:ip", ip.unwrap_or("unattributed"), 30, 60),
        ("crash:global", "all", 600, 60),
    ];
    for (namespace, key, limit, seconds) in limits {
        let admission = super::coordination_service::RateWindowStore::admit(
            db,
            namespace,
            key,
            limit,
            Duration::from_secs(seconds),
        )
        .await?;
        if !admission.allowed {
            return Err(AppError::RateLimitedWithRetry {
                retry_after: (admission.reset_at - Utc::now()).num_seconds().max(1) as u64,
            });
        }
    }
    Ok(())
}
pub async fn count(db: &Database, platform: Option<Platform>, outcome: &'static str) {
    let now = Utc::now();
    let platform = platform.map(Platform::as_str).unwrap_or("unknown");
    let bucket = now.timestamp() / 3600;
    let mut result = db
        .collection::<Document>(COUNTERS)
        .update_one(
            doc! {"bucket": bucket, "platform": platform, "outcome": outcome},
            doc! {
                "$inc": {"count": 1_i64},
                "$setOnInsert": {
                    "_id": uuid::Uuid::new_v4().to_string(),
                    "received_at": bson::DateTime::from_millis(bucket * 3600 * 1000),
                    "expires_at": bson::DateTime::from_chrono(
                        now + chrono::Duration::days(30)
                    )
                }
            },
        )
        .upsert(true)
        .await;
    // Concurrent first writes can race the unique bucket upsert. Retry only a
    // proven duplicate; an ambiguous transport failure must not double-count.
    if result.as_ref().is_err_and(|e| match e.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => command.code == 11000,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) => {
            write.code == 11000
        }
        _ => false,
    }) {
        result = db
            .collection::<Document>(COUNTERS)
            .update_one(
                doc! {"bucket": bucket, "platform": platform, "outcome": outcome},
                doc! {"$inc": {"count": 1_i64}},
            )
            .await;
    }
    if result.is_err() {
        logging::counter_unavailable();
    }
    logging::ingestion(outcome, platform);
}
pub async fn insert(
    db: &Database,
    config: &CrashConfig,
    environment: &str,
    actor: &str,
    revision: &str,
    report: CrashReportV1,
) -> AppResult<(bool, String)> {
    insert_with_consent(
        db,
        config,
        environment,
        actor,
        revision,
        report,
        consent::permits_diagnostics(db, actor, revision),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_with_consent(
    db: &Database,
    config: &CrashConfig,
    environment: &str,
    actor: &str,
    revision: &str,
    mut report: CrashReportV1,
    permission: impl std::future::Future<Output = consent::ConsentPermission>,
) -> AppResult<(bool, String)> {
    let now = Utc::now();
    validation::validate(&report, now)?;
    report.captured_at = validation::captured_at(&report.captured_at)?.to_rfc3339();
    match permission.await {
        consent::ConsentPermission::Permitted(()) => {}
        consent::ConsentPermission::Denied => return Err(AppError::CrashReportConsentRequired),
        consent::ConsentPermission::Unknown => return Err(AppError::SecurityReadinessUnavailable),
    }
    let destination = config.destination(report.platform);
    let row = ClientErrorReport {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: actor.into(),
        event_id: report.event_id.clone(),
        consent_revision: revision.into(),
        severity: validation::severity(report.kind).into(),
        fingerprint: validation::fingerprint(&report),
        report,
        environment: environment.into(),
        received_at: now,
        expires_at: now + chrono::Duration::days(i64::from(config.retention_days)),
        export: Export {
            status: if destination.is_some() && !config.interval.is_zero() {
                ExportStatus::Pending
            } else {
                ExportStatus::Disabled
            },
            destination_id: destination.map(|d| d.destination_id().to_owned()),
            attempts: 0,
            lease_id: None,
            lease_until: None,
            next_attempt_at: Some(now),
            deadline: now + chrono::Duration::hours(24),
            last_error_code: None,
            exported_at: None,
        },
    };
    match super::erasure_admission::insert_owned(db, COLLECTION_NAME, actor, &row).await {
        Ok(_) => Ok((false, row.id)),
        Err(error) => {
            // Unique owner/event index makes retries idempotent. Never resurrect an expired report.
            if let Some(existing) = db
                .collection::<ClientErrorReport>(COLLECTION_NAME)
                .find_one(doc! {
                    "user_id": actor,
                    "event_id": &row.event_id,
                    "expires_at": {"$gt": bson::DateTime::from_chrono(now)}
                })
                .await?
            {
                Ok((true, existing.id))
            } else {
                Err(error)
            }
        }
    }
}
pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    releases::ensure_indexes(db).await?;
    db.collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"flag_key":1,"enabled":1})
                .options(
                    IndexOptions::builder()
                        .name("crash_enabled_scope".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    let specs = [
        (
            doc! {"user_id": 1, "event_id": 1},
            "crash_owner_event",
            true,
            false,
        ),
        (
            doc! {
                "report.platform": 1,
                "environment": 1,
                "report.app_version": 1,
                "report.app_build": 1,
                "received_at": -1
            },
            "crash_versions",
            false,
            false,
        ),
        (
            doc! {"fingerprint": 1, "received_at": -1},
            "crash_fingerprint",
            false,
            false,
        ),
        (
            doc! {"export.status": 1, "export.next_attempt_at": 1},
            "crash_export_due",
            false,
            false,
        ),
        (
            doc! {"received_at": -1, "_id": -1},
            "crash_page",
            false,
            false,
        ),
        (doc! {"expires_at": 1}, "crash_ttl", false, true),
    ];
    for (keys, name, unique, ttl) in specs {
        let mut options = IndexOptions::builder()
            .name(name.to_owned())
            .unique(unique)
            .build();
        if ttl {
            options.expire_after = Some(Duration::ZERO);
        }
        db.collection::<Document>(COLLECTION_NAME)
            .create_index(IndexModel::builder().keys(keys).options(options).build())
            .await?;
    }
    for (keys, name, ttl) in [
        (
            doc! {"bucket": 1, "platform": 1, "outcome": 1},
            "crash_counter_bucket",
            false,
        ),
        (doc! {"expires_at": 1}, "crash_counter_ttl", true),
    ] {
        let mut options = IndexOptions::builder()
            .name(name.to_owned())
            .unique(!ttl)
            .build();
        if ttl {
            options.expire_after = Some(Duration::ZERO);
        }
        db.collection::<Document>(COUNTERS)
            .create_index(IndexModel::builder().keys(keys).options(options).build())
            .await?;
    }
    Ok(())
}
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod review_tests;
