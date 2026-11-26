use super::*;
use crate::{
    errors::AppResult,
    models::client_error_report::{COLLECTION_NAME, ClientErrorReport, ExportStatus},
    services::{analytics_consent_service, crash_report_service, shutdown::Runtime},
};
use mongodb::{
    Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};

pub async fn run(db: Database, config: Arc<CrashConfig>, shutdown: Runtime) {
    if config.interval.is_zero() || (config.web.is_none() && config.mobile.is_none()) {
        return;
    }
    let mut interval = tokio::time::interval(config.interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while let Some(_work) = shutdown.tick(&mut interval).await {
        if tick(&db, &config, &shutdown).await.is_err() {
            tracing::warn!(outcome = "worker_unavailable", "Crash export sweep");
        }
    }
}
async fn tick(db: &Database, config: &CrashConfig, shutdown: &Runtime) -> AppResult<()> {
    if config.interval.is_zero() {
        return Ok(());
    }
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(
            doc! {
                "export.status": "dispatching",
                "$expr": {
                    "$and": [
                        {"$lte": ["$export.lease_until", "$$NOW"]},
                        {"$gt": ["$expires_at", "$$NOW"]}
                    ]
                }
            },
            doc! {
                "$set": {
                    "export.status": "unknown",
                    "export.last_error_code": "dispatch_lease_expired",
                    "export.lease_id": null,
                    "export.lease_until": null
                }
            },
        )
        .await?;
    for _ in 0..16 {
        if shutdown.is_draining() {
            break;
        }
        let lease = uuid::Uuid::new_v4().to_string();
        let row = db
            .collection::<ClientErrorReport>(COLLECTION_NAME)
            .find_one_and_update(
                doc! {
                    "export.status": "pending",
                    "$expr": {"$and": [
                        {"$gt": ["$expires_at", "$$NOW"]},
                        {"$lte": ["$export.next_attempt_at", "$$NOW"]},
                        {"$lte": [
                            {"$ifNull": [
                                "$export.lease_until",
                                bson::DateTime::from_millis(0)
                            ]},
                            "$$NOW"
                        ]}
                    ]}
                },
                vec![doc! {
                    "$set": {
                        "export.lease_id": &lease,
                        "export.lease_until": {"$add": ["$$NOW", 60000_i64]}
                    }
                }],
            )
            .sort(doc! {"export.next_attempt_at": 1})
            .return_document(ReturnDocument::After)
            .await?;
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
    let mut fields = doc! {
        "export.status": status.as_str(),
        "export.last_error_code": code,
        "export.lease_id": null,
        "export.lease_until": null
    };
    if let Some(seconds) = retry {
        fields.insert(
            "export.next_attempt_at",
            doc! {"$add": ["$$NOW", (seconds.min(3600) * 1000) as i64]},
        );
    }
    if status == ExportStatus::Exported {
        fields.insert("export.exported_at", "$$NOW");
    }
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": id,
                "export.lease_id": lease,
                "$expr": {"$and": [
                    {"$gt": ["$export.lease_until", "$$NOW"]},
                    {"$gt": ["$expires_at", "$$NOW"]}
                ]}
            },
            vec![doc! {"$set": fields}],
        )
        .await?;
    Ok(())
}
async fn dispatch(
    db: &Database,
    config: &CrashConfig,
    row: ClientErrorReport,
    lease: &str,
) -> AppResult<()> {
    let permission =
        analytics_consent_service::permits_diagnostics(db, &row.user_id, &row.consent_revision);
    dispatch_with_consent(db, config, &row, lease, permission).await
}

async fn dispatch_with_consent(
    db: &Database,
    config: &CrashConfig,
    row: &ClientErrorReport,
    lease: &str,
    permission: impl std::future::Future<Output = analytics_consent_service::ConsentPermission>,
) -> AppResult<()> {
    let destination = config
        .destination(row.report.platform)
        .filter(|d| Some(d.destination_id()) == row.export.destination_id.as_deref());
    let eligibility = if row.export.deadline > chrono::Utc::now() && destination.is_some() {
        crash_report_service::eligibility(db, &row.user_id).await
    } else {
        crash_report_service::DiagnosticsStatus::Unknown
    };
    if row.export.deadline > chrono::Utc::now() {
        if destination.is_none() {
            return settle(
                db,
                &row.id,
                lease,
                ExportStatus::Pending,
                Some("destination_unavailable"),
                Some(10),
            )
            .await;
        }
        if eligibility == crash_report_service::DiagnosticsStatus::Unknown {
            return settle(
                db,
                &row.id,
                lease,
                ExportStatus::Pending,
                Some("eligibility_unknown"),
                Some(10),
            )
            .await;
        }
    }
    let code = if row.export.deadline <= chrono::Utc::now() {
        Some("deadline")
    } else if destination.is_none() {
        Some("destination_unavailable")
    } else if !eligibility.offered() {
        Some("reporting_unavailable")
    } else {
        match permission.await {
            analytics_consent_service::ConsentPermission::Permitted(()) => None,
            analytics_consent_service::ConsentPermission::Denied => Some("consent_denied"),
            analytics_consent_service::ConsentPermission::Unknown => {
                return settle(
                    db,
                    &row.id,
                    lease,
                    ExportStatus::Pending,
                    Some("consent_unknown"),
                    Some(10),
                )
                .await;
            }
        }
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
    let changed = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": &row.id,
                "export.status": "pending",
                "export.lease_id": lease,
                "$expr": {"$and": [
                    {"$gt": ["$export.lease_until", "$$NOW"]},
                    {"$gt": ["$export.deadline", "$$NOW"]},
                    {"$gt": ["$expires_at", "$$NOW"]}
                ]}
            },
            doc! {
                "$set": {"export.status": "dispatching"},
                "$inc": {"export.attempts": 1_i64}
            },
        )
        .await?;
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

    #[tokio::test]
    async fn crash_worker_without_destinations_returns_without_database_commands() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        let commands = Arc::new(AtomicUsize::new(0));
        let observed = commands.clone();
        let db = crate::test_utils::connect_test_database_with_command_handler(
            "crash_no_destinations",
            EventHandler::callback(move |event| {
                if matches!(event, CommandEvent::Started(_)) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            }),
        )
        .await
        .unwrap();
        commands.store(0, Ordering::SeqCst);
        tokio::time::timeout(
            Duration::from_secs(1),
            run(
                db.clone(),
                Arc::new(CrashConfig::default()),
                Runtime::default(),
            ),
        )
        .await
        .expect("an unconfigured worker returns before its first tick");
        assert_eq!(commands.load(Ordering::SeqCst), 0);
        db.drop().await.unwrap();
    }

    async fn tick(db: &Database, config: &CrashConfig) -> AppResult<()> {
        let shutdown = Runtime::default();
        let _work = shutdown.background().unwrap();
        super::tick(db, config, &shutdown).await
    }

    struct BlockingExporter {
        calls: AtomicUsize,
        started: tokio::sync::Notify,
        release: tokio::sync::Semaphore,
    }

    #[async_trait::async_trait]
    impl CrashReportExporter for BlockingExporter {
        fn destination_id(&self) -> &str {
            "test-project"
        }

        async fn submit(&self, _: &SanitizedReport<'_>) -> SubmitOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.release.acquire().await.unwrap().forget();
            SubmitOutcome::Exported
        }
    }

    async fn blocking_setup(
        name: &str,
    ) -> (
        Database,
        String,
        String,
        String,
        CrashConfig,
        Arc<BlockingExporter>,
    ) {
        let (db, actor, revision, id, mut config, _) = setup(name, 0).await;
        let exporter = Arc::new(BlockingExporter {
            calls: AtomicUsize::new(0),
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
        });
        config.web = Some(exporter.clone());
        config.interval = Duration::from_secs(3600);
        (db, actor, revision, id, config, exporter)
    }

    #[tokio::test]
    async fn drain_counts_inflight_export_settles_it_and_stops_before_the_next_claim() {
        let (db, actor, revision, first, config, exporter) = blocking_setup("crash_drain").await;
        crate::test_utils::await_responsive_majority_io(&db).await;
        let (_, second) =
            crash_report_service::insert(&db, &config, "test", &actor, &revision, report())
                .await
                .unwrap();
        // Keep both rows due regardless of clock skew, with deterministic claim order.
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &second},
                doc! {"$set": {"export.next_attempt_at": bson::DateTime::from_millis(1)}},
            )
            .await
            .unwrap();
        let shutdown = Runtime::default();
        let worker = shutdown.spawn_worker(run(db.clone(), Arc::new(config), shutdown.clone()));
        tokio::time::timeout(Duration::from_secs(5), exporter.started.notified())
            .await
            .unwrap();
        let dispatching = row(&db, &first).await;
        assert_eq!(dispatching.export.status, ExportStatus::Dispatching);
        assert_eq!(dispatching.export.attempts, 1);
        assert!(dispatching.export.lease_id.is_some());
        assert!(dispatching.export.lease_until.is_some());
        assert_eq!(shutdown.begin_drain().unwrap().background, 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), shutdown.idle())
                .await
                .is_err()
        );
        let pending = row(&db, &second).await;
        assert_eq!(pending.export.status, ExportStatus::Pending);
        assert!(pending.export.lease_id.is_none());
        exporter.release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row(&db, &first).await.export.status, ExportStatus::Exported);
        let pending = row(&db, &second).await;
        assert_eq!(pending.export.status, ExportStatus::Pending);
        assert!(pending.export.lease_id.is_none());
        assert_eq!(exporter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(shutdown.counts().background, 0);
        assert_eq!(shutdown.tracked_units(), 0);
    }

    #[tokio::test]
    async fn drain_deadline_keeps_dispatch_intent_and_expired_lease_is_unknown_without_resend() {
        let (db, _, _, id, config, exporter) = blocking_setup("crash_drain_deadline").await;
        let shutdown = Runtime::default();
        let worker =
            shutdown.spawn_worker(run(db.clone(), Arc::new(config.clone()), shutdown.clone()));
        tokio::time::timeout(Duration::from_secs(5), exporter.started.notified())
            .await
            .unwrap();
        assert_eq!(shutdown.begin_drain().unwrap().background, 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), shutdown.idle())
                .await
                .is_err()
        );
        // The production hard deadline aborts tracked tasks through Runtime::stop.
        shutdown.stop();
        assert!(worker.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(5), shutdown.tasks_stopped())
            .await
            .unwrap();
        let abandoned = row(&db, &id).await;
        assert_eq!(abandoned.export.status, ExportStatus::Dispatching);
        let stale_lease = abandoned.export.lease_id.unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &id},
                doc! {"$set": {"export.lease_until": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        tick(&db, &config).await.unwrap();
        settle(&db, &id, &stale_lease, ExportStatus::Exported, None, None)
            .await
            .unwrap();
        tick(&db, &config).await.unwrap();
        let unknown = row(&db, &id).await;
        assert_eq!(unknown.export.status, ExportStatus::Unknown);
        assert_eq!(
            unknown.export.last_error_code.as_deref(),
            Some("dispatch_lease_expired")
        );
        assert!(unknown.export.lease_id.is_none());
        assert_eq!(exporter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(shutdown.tracked_units(), 0);
    }

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
        crate::test_utils::await_responsive_majority_io(&db).await;
        let (_, report_id) =
            crash_report_service::insert(&db, &config, "test", &id, &rev, report())
                .await
                .unwrap();
        // Backdate claims so tests are independent of client/server clock skew.
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &report_id},
                doc! {"$set":{"export.next_attempt_at":bson::DateTime::from_millis(0)}},
            )
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
    async fn crash_worker_consent_permitted_sends_denied_discards_and_unknown_retries() {
        for outcome in ["permitted", "denied", "unknown"] {
            let (db, actor, revision, id, config, exporter) = setup("crash_consent_read", 0).await;
            let lease = uuid::Uuid::new_v4().to_string();
            db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &id},
                    vec![doc! {"$set": {
                        "export.lease_id": &lease,
                        "export.lease_until": {"$add": ["$$NOW", 60000_i64]}
                    }}],
                )
                .await
                .unwrap();
            crate::test_utils::await_responsive_majority_io(&db).await;
            let permission = async {
                match outcome {
                    "permitted" => consent::permits_diagnostics(&db, &actor, &revision).await,
                    "denied" => consent::permits_diagnostics(&db, &actor, "stale").await,
                    _ => consent::bounded_consent_read(std::future::pending(), Duration::ZERO)
                        .await
                        .for_revision(&revision),
                }
            };
            dispatch_with_consent(&db, &config, &row(&db, &id).await, &lease, permission)
                .await
                .unwrap();
            let saved = row(&db, &id).await;
            let (status, code, calls) = match outcome {
                "permitted" => (ExportStatus::Exported, None, 1),
                "denied" => (ExportStatus::Discarded, Some("consent_denied"), 0),
                _ => (ExportStatus::Pending, Some("consent_unknown"), 0),
            };
            assert_eq!(saved.export.status, status);
            assert_eq!(saved.export.last_error_code.as_deref(), code);
            assert_eq!(exporter.calls.load(Ordering::SeqCst), calls);
            assert!(saved.export.lease_id.is_none());
            if outcome == "unknown" {
                assert_eq!(saved.export.attempts, 0);
                assert!(saved.export.next_attempt_at.unwrap() > saved.received_at);
            }
        }
    }

    #[tokio::test]
    async fn mixed_destination_replicas_release_pending_rows_and_matching_replica_exports() {
        let (db, _, _, id, config, exporter) = setup("crash_mixed_dsn", 0).await;
        tick(&db, &CrashConfig::default()).await.unwrap();
        let pending = row(&db, &id).await;
        assert_eq!(pending.export.status, ExportStatus::Pending);
        assert!(pending.export.lease_id.is_none());
        assert_eq!(pending.export.attempts, 0);
        assert!(pending.export.next_attempt_at.unwrap() > chrono::Utc::now());
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"export.next_attempt_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        crate::test_utils::await_responsive_majority_io(&db).await;
        tick(&db, &config).await.unwrap();
        assert_eq!(row(&db, &id).await.export.status, ExportStatus::Exported);
        assert_eq!(exporter.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn zero_export_interval_disables_new_rows_and_never_works_pending_rows() {
        let (db, actor, rev, id, mut config, exporter) = setup("crash_zero_interval", 0).await;
        config.interval = Duration::ZERO;
        crate::test_utils::await_responsive_majority_io(&db).await;
        let (_, disabled) =
            crash_report_service::insert(&db, &config, "test", &actor, &rev, report())
                .await
                .unwrap();
        assert_eq!(
            row(&db, &disabled).await.export.status,
            ExportStatus::Disabled
        );
        tick(&db, &config).await.unwrap();
        assert_eq!(row(&db, &id).await.export.status, ExportStatus::Pending);
        assert_eq!(exporter.calls.load(Ordering::SeqCst), 0);
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
            crate::test_utils::await_responsive_majority_io(&db).await;
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
            crate::test_utils::await_responsive_majority_io(&db).await;
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
