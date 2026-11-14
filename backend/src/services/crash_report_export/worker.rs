use super::*;
use crate::{
    errors::AppResult,
    models::client_error_report::{COLLECTION_NAME, ClientErrorReport, ExportStatus},
    services::{analytics_consent_service, crash_report_service},
};
use mongodb::{
    Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};

pub fn spawn(db: Database, config: Arc<CrashConfig>) {
    if config.interval.is_zero() {
        return;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if tick(&db, &config).await.is_err() {
                tracing::warn!(outcome = "worker_unavailable", "Crash export sweep");
            }
        }
    });
}
pub async fn tick(db: &Database, config: &CrashConfig) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME).update_many(doc! {
        "export.status": "dispatching", "$expr": {"$and": [{"$lte": ["$export.lease_until", "$$NOW"]}, {"$gt": ["$expires_at", "$$NOW"]}]}
    }, doc! {"$set": {"export.status": "unknown", "export.last_error_code": "dispatch_lease_expired", "export.lease_id": null, "export.lease_until": null}}).await?;
    for _ in 0..16 {
        let lease = uuid::Uuid::new_v4().to_string();
        let row = db.collection::<ClientErrorReport>(COLLECTION_NAME).find_one_and_update(doc! {
            "export.status": "pending", "$expr": {"$and": [
                {"$gt": ["$expires_at", "$$NOW"]}, {"$lte": ["$export.next_attempt_at", "$$NOW"]},
                {"$lte": [{"$ifNull": ["$export.lease_until", bson::DateTime::from_millis(0)]}, "$$NOW"]}
            ]}
        }, vec![doc! {"$set": {"export.lease_id": &lease, "export.lease_until": {"$add": ["$$NOW", 60000_i64]}}}]).sort(doc! {"export.next_attempt_at": 1}).return_document(ReturnDocument::After).await?;
        let Some(row) = row else {
            break;
        };
        dispatch(db, config, row, &lease).await?;
    }
    Ok(())
}
async fn settle(
    db: &Database,
    id: &str,
    lease: &str,
    status: ExportStatus,
    code: Option<&str>,
    retry: Option<u64>,
) -> AppResult<()> {
    let mut fields = doc! {"export.status": status.as_str(), "export.last_error_code": code, "export.lease_id": null, "export.lease_until": null};
    if let Some(seconds) = retry {
        fields.insert(
            "export.next_attempt_at",
            doc! {"$add": ["$$NOW", (seconds.min(3600) * 1000) as i64]},
        );
    }
    if status == ExportStatus::Exported {
        fields.insert("export.exported_at", "$$NOW");
    }
    db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id": id, "export.lease_id": lease, "$expr": {"$and": [{"$gt": ["$export.lease_until", "$$NOW"]}, {"$gt": ["$expires_at", "$$NOW"]}]}}, vec![doc! {"$set": fields}]).await?;
    Ok(())
}
async fn dispatch(
    db: &Database,
    config: &CrashConfig,
    row: ClientErrorReport,
    lease: &str,
) -> AppResult<()> {
    let destination = config
        .destination(row.report.platform)
        .filter(|d| Some(d.destination_id()) == row.export.destination_id.as_deref());
    let code = if row.export.deadline <= chrono::Utc::now() {
        Some("deadline")
    } else if destination.is_none() {
        Some("destination_unavailable")
    } else if !crash_report_service::available(db, &row.user_id).await {
        Some("reporting_unavailable")
    } else if !analytics_consent_service::permits_diagnostics(
        db,
        &row.user_id,
        &row.consent_revision,
    )
    .await
    {
        Some("consent_denied")
    } else {
        None
    };
    if let Some(code) = code {
        return settle(
            db,
            &row.id,
            lease,
            ExportStatus::Discarded,
            Some(code),
            None,
        )
        .await;
    }
    let changed = db.collection::<Document>(COLLECTION_NAME).update_one(doc! {
        "_id": &row.id, "export.status": "pending", "export.lease_id": lease,
        "$expr": {"$and": [{"$gt": ["$export.lease_until", "$$NOW"]}, {"$gt": ["$export.deadline", "$$NOW"]}, {"$gt": ["$expires_at", "$$NOW"]}]}
    }, doc! {"$set": {"export.status": "dispatching"}, "$inc": {"export.attempts": 1_i64}}).await?;
    if changed.modified_count != 1 {
        return Ok(());
    }
    let safe = SanitizedReport {
        report: &row.report,
        environment: &row.environment,
        severity: &row.severity,
    };
    let outcome = destination
        .expect("checked destination")
        .submit(&safe)
        .await;
    let (status, code, retry) = match outcome {
        SubmitOutcome::Exported => (ExportStatus::Exported, None, None),
        SubmitOutcome::Unknown => (ExportStatus::Unknown, Some("dispatch_unknown"), None),
        SubmitOutcome::Failed => (ExportStatus::Failed, Some("provider_rejected"), None),
        SubmitOutcome::Retry { after, code } => {
            let backoff = after
                .max(10_u64.saturating_mul(2_u64.saturating_pow(row.export.attempts.min(9))))
                .min(3600);
            if chrono::Utc::now() + chrono::Duration::seconds(backoff as i64) >= row.export.deadline
            {
                (ExportStatus::Discarded, Some("deadline"), None)
            } else {
                (ExportStatus::Pending, Some(code), Some(backoff))
            }
        }
    };
    settle(db, &row.id, lease, status, code, retry).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        analytics_consent_service as consent,
        crash_report_service::tests::{account, report},
        feature_flag_service::{self as flags, FlagTarget},
        security_readiness_service as readiness,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Exporter {
        calls: AtomicUsize,
        outcome: u8,
    }
    #[async_trait::async_trait]
    impl CrashReportExporter for Exporter {
        fn destination_id(&self) -> &str {
            "test-project"
        }
        async fn submit(&self, _: &SanitizedReport<'_>) -> SubmitOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.outcome {
                0 => SubmitOutcome::Exported,
                1 => SubmitOutcome::Retry {
                    after: 20,
                    code: "provider_retry",
                },
                2 => SubmitOutcome::Retry {
                    after: 10,
                    code: "connect_failed",
                },
                3 => SubmitOutcome::Unknown,
                _ => SubmitOutcome::Failed,
            }
        }
    }
    async fn setup(
        name: &str,
        outcome: u8,
    ) -> (Database, String, String, String, CrashConfig, Arc<Exporter>) {
        let (db, id, rev) = account(name).await;
        readiness::ready_test_replica(&db).await;
        flags::set_platform_override(
            &db,
            crash_report_service::FLAG,
            &FlagTarget::User(id.clone()),
            true,
            &id,
        )
        .await
        .unwrap();
        let exporter = Arc::new(Exporter {
            calls: AtomicUsize::new(0),
            outcome,
        });
        let config = CrashConfig {
            web: Some(exporter.clone()),
            ..Default::default()
        };
        let (_, report_id) =
            crash_report_service::insert(&db, &config, "test", &id, &rev, report())
                .await
                .unwrap();
        (db, id, rev, report_id, config, exporter)
    }
    async fn row(db: &Database, id: &str) -> ClientErrorReport {
        db.collection::<ClientErrorReport>(COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap()
    }
    #[tokio::test]
    async fn worker_settles_retry_connect_unknown_and_failed_without_resending() {
        for (outcome, expected) in [
            (0, ExportStatus::Exported),
            (1, ExportStatus::Pending),
            (2, ExportStatus::Pending),
            (3, ExportStatus::Unknown),
            (4, ExportStatus::Failed),
        ] {
            let (db, _, _, id, config, exporter) = setup("crash_worker_outcomes", outcome).await;
            tick(&db, &config).await.unwrap();
            let saved = row(&db, &id).await;
            assert_eq!(saved.export.status, expected);
            assert_eq!(saved.export.attempts, 1);
            if expected == ExportStatus::Pending {
                assert!(saved.export.next_attempt_at.unwrap() > chrono::Utc::now());
            }
            tick(&db, &config).await.unwrap();
            assert_eq!(exporter.calls.load(Ordering::SeqCst), 1);
        }
    }
    #[tokio::test]
    async fn withdrawal_regrant_deleted_account_kill_switch_and_deadline_discard_before_export() {
        for mode in ["withdraw", "regrant", "deleted", "kill", "deadline"] {
            let (db, actor, _, id, config, exporter) = setup("crash_worker_denials", 0).await;
            match mode {
                "withdraw" => {
                    consent::set_acknowledging(
                        &db,
                        &actor,
                        false,
                        consent::PolicyVersion::Diagnostics,
                    )
                    .await
                    .unwrap();
                }
                "regrant" => {
                    consent::set_acknowledging(
                        &db,
                        &actor,
                        true,
                        consent::PolicyVersion::Diagnostics,
                    )
                    .await
                    .unwrap();
                }
                "deleted" => {
                    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                        .delete_one(doc! {"_id":&actor})
                        .await
                        .unwrap();
                }
                "kill" => {
                    flags::set_platform_override(
                        &db,
                        crash_report_service::FLAG,
                        &FlagTarget::Global,
                        false,
                        &actor,
                    )
                    .await
                    .unwrap();
                }
                _ => {
                    db.collection::<Document>(COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":&id},
                            doc! {"$set":{"export.deadline":bson::DateTime::from_millis(0)}},
                        )
                        .await
                        .unwrap();
                }
            }
            tick(&db, &config).await.unwrap();
            assert_eq!(
                row(&db, &id).await.export.status,
                ExportStatus::Discarded,
                "{mode}"
            );
            assert_eq!(exporter.calls.load(Ordering::SeqCst), 0);
        }
    }
    #[tokio::test]
    async fn expired_dispatch_is_unknown_and_stale_fences_cannot_settle_or_send() {
        let (db, _, _, id, config, exporter) = setup("crash_worker_fence", 0).await;
        db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&id}, doc! {"$set":{"export.lease_id":"successor","export.lease_until":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(1))}}).await.unwrap();
        dispatch(&db, &config, row(&db, &id).await, "predecessor")
            .await
            .unwrap();
        settle(&db, &id, "predecessor", ExportStatus::Exported, None, None)
            .await
            .unwrap();
        assert_eq!(row(&db, &id).await.export.status, ExportStatus::Pending);
        assert_eq!(exporter.calls.load(Ordering::SeqCst), 0);
        db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&id}, doc! {"$set":{"export.status":"dispatching","export.lease_until":bson::DateTime::from_millis(0)}}).await.unwrap();
        tick(&db, &config).await.unwrap();
        tick(&db, &config).await.unwrap();
        assert_eq!(row(&db, &id).await.export.status, ExportStatus::Unknown);
        assert_eq!(exporter.calls.load(Ordering::SeqCst), 0);
    }
}
