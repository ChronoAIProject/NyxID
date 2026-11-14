//! First-party, policy-2 diagnostic admission and owner-linked local storage.
pub mod admin;
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

pub async fn global_killed(db: &Database) -> AppResult<bool> {
    Ok(db.collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME).find_one(doc! {
        "flag_key": FLAG, "definition_id": null, "org_user_id": null, "target_kind": "global", "target_key": null, "enabled": false
    }).await?.is_some())
}
pub async fn available(db: &Database, actor: &str) -> bool {
    if !matches!(global_killed(db).await, Ok(false)) {
        return false;
    }
    if !matches!(
        super::feature_flag_service::personal_flag_enabled(db, actor, FLAG).await,
        Ok(true)
    ) {
        return false;
    }
    super::security_readiness_service::require_serving(db, FLAG)
        .await
        .is_ok()
}
pub async fn policy_status(
    db: &Database,
    actor: &str,
    value: &crate::models::user::AnalyticsConsent,
) -> (&'static str, bool) {
    let serving = super::security_readiness_service::require_serving(db, FLAG)
        .await
        .is_ok();
    let diagnostics = serving
        && value.enabled
        && value.policy_version.as_deref() == Some(consent::POLICY_VERSION_DIAGNOSTICS)
        && available(db, actor).await;
    (if serving { "2" } else { "1" }, diagnostics)
}
pub async fn rate_limits(db: &Database, actor: &str, ip: Option<&str>) -> AppResult<()> {
    let limits = [
        ("crash:global", "all", 600, 60),
        ("crash:ip", ip.unwrap_or("unattributed"), 30, 60),
        ("crash:account-minute", actor, 5, 60),
        ("crash:account-day", actor, 50, 86400),
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
    let mut result = db.collection::<Document>(COUNTERS).update_one(doc! {"bucket": bucket, "platform": platform, "outcome": outcome}, doc! {
        "$inc": {"count": 1_i64}, "$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "received_at": bson::DateTime::from_millis(bucket * 3600 * 1000), "expires_at": bson::DateTime::from_chrono(now + chrono::Duration::days(30))}
    }).upsert(true).await;
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
        tracing::warn!(outcome = "counter_unavailable", "Crash ingestion health");
    }
    tracing::debug!(outcome, platform, "Crash ingestion outcome");
}
pub async fn insert(
    db: &Database,
    config: &CrashConfig,
    environment: &str,
    actor: &str,
    revision: &str,
    report: CrashReportV1,
) -> AppResult<(bool, String)> {
    let now = Utc::now();
    validation::validate(&report, now)?;
    if !consent::permits_diagnostics(db, actor, revision).await {
        return Err(AppError::CrashReportConsentRequired);
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
            status: if destination.is_some() {
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
            if let Some(existing) = db.collection::<ClientErrorReport>(COLLECTION_NAME).find_one(doc! {"user_id": actor, "event_id": &row.event_id, "expires_at": {"$gt": bson::DateTime::from_chrono(now)}}).await? { Ok((true, existing.id)) } else { Err(error) }
        }
    }
}
pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    let specs = [
        (
            doc! {"user_id": 1, "event_id": 1},
            "crash_owner_event",
            true,
            false,
        ),
        (
            doc! {"report.platform": 1, "environment": 1, "report.app_version": 1, "report.app_build": 1, "received_at": -1},
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
