use super::*;
use crate::{
    models::user::{User, UserType},
    test_utils::{connect_test_database, connect_test_database_with_command_handler, test_user},
};
use serde_json::json;

// Integration assertions exercise production admission once, after establishing
// the healthy majority-I/O precondition its bounded consent gate requires.
pub(super) async fn insert(
    db: &Database,
    config: &CrashConfig,
    environment: &str,
    actor: &str,
    revision: &str,
    report: CrashReportV1,
) -> AppResult<(bool, String)> {
    crate::test_utils::await_responsive_majority_io(db).await;
    super::insert(db, config, environment, actor, revision, report).await
}

pub(crate) fn report() -> CrashReportV1 {
    serde_json::from_value(json!({"schema_version":1,"event_id":uuid::Uuid::new_v4().to_string(),"captured_at":Utc::now().to_rfc3339(),"platform":"web","kind":"js_error","exception_class":"TypeError","app_version":"1.2.3","app_build":"abc123","artifact_id":uuid::Uuid::new_v4().to_string(),"os_version":"18.1","runtime":{"name":"safari","major":18},"js_frames":[{"asset":"assets/app-a1.js","line":12,"column":4}]})).unwrap()
}
pub(crate) async fn account(name: &str) -> (Database, String, String) {
    let db = connect_test_database(name).await.expect("MongoDB required");
    ensure_indexes(&db).await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&id, UserType::Person))
        .await
        .unwrap();
    // Seed an existing policy-2 account independently of rollout. Production
    // acknowledgements are tested through the offered-only service/HTTP path.
    let revision = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>(crate::models::user::COLLECTION_NAME).update_one(doc! {"_id":&id},doc! {"$set":{"profile_config.analytics":{"enabled":true,"policy_version":"2","revision":&revision,"changed_at":bson::DateTime::now()}}}).await.unwrap();
    (db, id, revision)
}
#[test]
fn closed_schema_rejects_sentinels_at_every_nested_boundary() {
    let sentinel = "Bearer SECRET_SENTINEL person@example.test eyJhbGciOiJSUzI1NiJ9.secret /Users/person/private https://user:pass@evil.test/?token=secret#email";
    for path in [
        "message",
        "reason",
        "stack",
        "cause",
        "registers",
        "customData",
        "url",
        "function",
        "headers",
    ] {
        let mut value = serde_json::to_value(report()).unwrap();
        value[path] = json!(sentinel);
        assert!(serde_json::from_value::<CrashReportV1>(value).is_err());
    }
    for field in ["asset", "function", "url", "message"] {
        let mut value = serde_json::to_value(report()).unwrap();
        value["js_frames"][0][field] = json!(sentinel);
        let parsed = serde_json::from_value::<CrashReportV1>(value);
        assert!(parsed.is_err() || validation::validate(&parsed.unwrap(), Utc::now()).is_err());
    }
    let mut value = serde_json::to_value(report()).unwrap();
    value["runtime"]["userAgent"] = json!(sentinel);
    assert!(serde_json::from_value::<CrashReportV1>(value).is_err());
    assert!(!format!("{:?}", report()).contains("app-a1"));
}
#[test]
fn validation_bounds_time_versions_platforms_frames_and_native_indexes() {
    let now = Utc::now();
    let original = report();
    validation::validate(&original, now).unwrap();
    let mut cases = Vec::new();
    let mut r = original.clone();
    r.schema_version = 2;
    cases.push(r);
    let mut r = original.clone();
    r.event_id = uuid::Uuid::nil().to_string();
    cases.push(r);
    let mut r = original.clone();
    r.captured_at = (now - chrono::Duration::hours(25)).to_rfc3339();
    cases.push(r);
    let mut r = original.clone();
    r.captured_at = (now + chrono::Duration::minutes(6)).to_rfc3339();
    cases.push(r);
    let mut r = original.clone();
    r.app_version = "secret@example.test".into();
    cases.push(r);
    let mut r = original.clone();
    r.os_version = "18.1.2".into();
    cases.push(r);
    let mut r = original.clone();
    r.js_frames[0].line = 10_000_001;
    cases.push(r);
    let mut r = original.clone();
    r.js_frames = vec![r.js_frames[0].clone(); 65];
    cases.push(r);
    let mut r = original.clone();
    r.kind = Kind::NativeCrash;
    r.native_signal = Some(NativeSignal::Abrt);
    cases.push(r);
    for r in cases {
        assert!(validation::validate(&r, now).is_err());
    }
    let mut native = original;
    native.platform = Platform::Ios;
    native.kind = Kind::NativeCrash;
    native.native_signal = Some(NativeSignal::Bus);
    native.runtime = None;
    native.js_frames.clear();
    native.native_images.push(NativeImage {
        uuid: uuid::Uuid::new_v4().to_string(),
        name: "NyxIDMobile".into(),
        size: 4096,
    });
    native.native_frames.push(NativeFrame {
        image: Some(0),
        offset: 32,
    });
    validation::validate(&native, now).unwrap();
    native.native_frames[0].image = Some(1);
    assert!(validation::validate(&native, now).is_err());
}
#[tokio::test]
async fn consent_matrix_storage_duplicate_ttl_and_expired_reads() {
    let (db, id, rev) = account("crash_storage").await;
    let config = CrashConfig::default();
    let r = report();
    let (duplicate, report_id) = insert(&db, &config, "test", &id, &rev, r.clone())
        .await
        .unwrap();
    assert!(!duplicate);
    assert_eq!(
        insert(&db, &config, "test", &id, &rev, r).await.unwrap(),
        (true, report_id.clone())
    );
    let row = admin::detail(&db, &report_id).await.unwrap();
    assert_eq!(row.row.export.status, ExportStatus::Disabled);
    assert_eq!((row.row.expires_at - row.row.received_at).num_days(), 30);
    let stored = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": &report_id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.get_datetime("received_at").is_ok());
    assert!(!format!("{stored:?}").contains("SECRET_SENTINEL"));
    let indexes: Vec<_> = db
        .collection::<Document>(COLLECTION_NAME)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        indexes.iter().any(|i| i
            .options
            .as_ref()
            .is_some_and(|o| o.name.as_deref() == Some("crash_ttl")
                && o.expire_after == Some(Duration::ZERO)))
    );
    super::super::security_readiness_service::ready_test_replica(&db).await;
    super::super::feature_flag_service::set_platform_override(
        &db,
        FLAG,
        &super::super::feature_flag_service::FlagTarget::User(id.clone()),
        true,
        &id,
    )
    .await
    .unwrap();
    for policy in [
        consent::PolicyVersion::V1,
        consent::PolicyVersion::Diagnostics,
    ] {
        let disabled = consent::set_acknowledging(&db, &id, false, policy)
            .await
            .unwrap();
        assert!(
            !(consent::permits_diagnostics(&db, &id, &disabled.revision).await
                == consent::ConsentPermission::Permitted(()))
        );
        let enabled = consent::set_acknowledging(&db, &id, true, policy)
            .await
            .unwrap();
        crate::test_utils::await_responsive_majority_io(&db).await;
        assert!(consent::permits_dispatch(&db, &id, &enabled.revision).await);
        assert_eq!(
            (consent::permits_diagnostics(&db, &id, &enabled.revision).await
                == consent::ConsentPermission::Permitted(())),
            matches!(policy, consent::PolicyVersion::Diagnostics)
        );
        assert!(
            !(consent::permits_diagnostics(&db, &id, &rev).await
                == consent::ConsentPermission::Permitted(()))
        );
    }
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &report_id},
            doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(admin::detail(&db, &report_id).await.is_err());
    assert!(
        admin::list(&db, &admin::ReportQuery::default())
            .await
            .unwrap()
            .0
            .is_empty()
    );
    assert!(
        admin::summary(
            &db,
            &admin::SummaryQuery {
                window: Some("30d".into()),
                platform: None
            }
        )
        .await
        .unwrap()
        .versions
        .is_empty()
    );
    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .delete_one(doc! {"_id": &id})
        .await
        .unwrap();
    assert!(
        consent::permitted_diagnostics_revision(&db, &id)
            .await
            .is_none()
    );
}
#[tokio::test]
async fn all_cluster_limits_produce_retry_after_before_json_parsing() {
    for (namespace, key, limit, seconds, ip) in [
        ("crash:global", "all", 600, 60, None),
        ("crash:ip", "127.0.0.1", 30, 60, Some("127.0.0.1")),
        ("crash:account-minute", "actor", 5, 60, None),
        ("crash:account-day", "actor", 50, 86400, None),
    ] {
        let db = connect_test_database("crash_rates").await.unwrap();
        crate::test_utils::await_responsive_majority_io(&db).await;
        crate::test_utils::ensure_rate_window_headroom(
            Duration::from_secs(seconds),
            Duration::from_secs(10),
        )
        .await;
        use super::super::coordination_service::RateWindowStore;
        assert!(
            RateWindowStore::admit(&db, namespace, key, limit, Duration::from_secs(seconds))
                .await
                .unwrap()
                .allowed
        );
        // Seed only the warm-up count. Exercise both real boundary admissions
        // without hundreds of writes that can cross the minute under load.
        let seeded = db
            .collection::<Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME)
            .update_one(
                doc! {"namespace":namespace},
                doc! {"$set":{"count":(limit - 1) as i64}},
            )
            .await
            .unwrap();
        assert_eq!(seeded.matched_count, 1);
        let last = RateWindowStore::admit(&db, namespace, key, limit, Duration::from_secs(seconds))
            .await
            .unwrap();
        assert!(last.allowed);
        assert_eq!(last.remaining, 0);
        assert!(
            matches!(rate_limits(&db,"actor",ip).await,Err(AppError::RateLimitedWithRetry{retry_after}) if retry_after>0)
        );
    }
}
#[tokio::test]
async fn admin_aggregation_pagination_and_window_filters_are_correct() {
    let (db, id, rev) = account("crash_summary").await;
    let mut artifacts = Vec::new();
    for kind in [Kind::JsError, Kind::RenderError, Kind::JsError] {
        let mut r = report();
        r.kind = kind;
        artifacts.push(r.artifact_id.clone().unwrap());
        register_report_release(&db, &r).await;
        insert(&db, &CrashConfig::default(), "test", &id, &rev, r)
            .await
            .unwrap();
    }
    let summary = admin::summary(
        &db,
        &admin::SummaryQuery {
            window: None,
            platform: Some(Platform::Web),
        },
    )
    .await
    .unwrap();
    artifacts.sort();
    artifacts.dedup();
    assert_eq!(summary.versions.len(), 1);
    let version = &summary.versions[0];
    assert_eq!(version.total, 3);
    assert_eq!(version.counts["js_error"], 2);
    assert_eq!(version.counts["render_error"], 1);
    assert_eq!(version.artifact_ids, artifacts);
    let query = admin::ReportQuery {
        limit: Some(2),
        ..Default::default()
    };
    let (first, cursor) = admin::list(&db, &query).await.unwrap();
    assert_eq!(first.len(), 2);
    let (second, next) = admin::list(
        &db,
        &admin::ReportQuery {
            cursor,
            limit: Some(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(second.len(), 1);
    assert!(next.is_none());
    assert!(first.iter().all(|r| r.row.id != second[0].row.id));
}

#[tokio::test]
async fn admin_aggregation_combines_native_and_js_for_the_same_version_build() {
    let (db, id, rev) = account("crash_mixed_artifacts").await;
    let artifact = uuid::Uuid::new_v4().to_string();
    for kind in [Kind::NativeCrash, Kind::JsFatal, Kind::UnhandledRejection] {
        let mut r = report();
        r.platform = Platform::Ios;
        r.app_version = "1.0.4".into();
        r.app_build = "45".into();
        r.kind = kind;
        r.runtime = None;
        if kind == Kind::NativeCrash {
            r.artifact_id = None;
            r.native_signal = Some(NativeSignal::Segv);
            r.js_frames.clear();
        } else {
            r.artifact_id = Some(artifact.clone());
            r.js_frames[0].asset = "main.jsbundle".into();
        }
        register_report_release(&db, &r).await;
        insert(&db, &CrashConfig::default(), "test", &id, &rev, r)
            .await
            .unwrap();
    }
    let query = admin::SummaryQuery {
        window: None,
        platform: Some(Platform::Ios),
    };
    let summary = admin::summary(&db, &query).await.unwrap();
    assert_eq!(summary.versions.len(), 1);
    let version = &summary.versions[0];
    assert_eq!(
        (
            &*version.platform,
            &*version.environment,
            &*version.app_version,
            &*version.app_build
        ),
        ("ios", "test", "1.0.4", "45")
    );
    assert_eq!(version.total, 3);
    for kind in ["native_crash", "js_fatal", "unhandled_rejection"] {
        assert_eq!(version.counts[kind], 1);
    }
    assert_eq!(version.counts["js_error"], 0);
    assert_eq!(version.artifact_ids, vec![artifact]);
    let (reports, _) = admin::list(
        &db,
        &admin::ReportQuery {
            platform: Some(Platform::Ios),
            app_version: Some("1.0.4".into()),
            app_build: Some("45".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(reports.len(), 3);

    // Mobile records without any artifact, including older absent fields, yield an empty list.
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$unset": {"report.artifact_id": ""}})
        .await
        .unwrap();
    let summary = admin::summary(&db, &query).await.unwrap();
    assert_eq!(summary.versions.len(), 1);
    assert_eq!(summary.versions[0].total, 3);
    assert!(summary.versions[0].artifact_ids.is_empty());
}

#[tokio::test]
async fn admin_reports_label_verified_and_unverified_web_and_ios_releases() {
    let (db, id, rev) = account("crash_report_verification").await;
    let mut verified_web = report();
    let unverified_web = report();
    register_report_release(&db, &verified_web).await;
    let (_, verified_web_id) = insert(
        &db,
        &CrashConfig::default(),
        "test",
        &id,
        &rev,
        verified_web.clone(),
    )
    .await
    .unwrap();
    let (_, unverified_web_id) = insert(
        &db,
        &CrashConfig::default(),
        "test",
        &id,
        &rev,
        unverified_web,
    )
    .await
    .unwrap();

    verified_web.platform = Platform::Ios;
    verified_web.event_id = uuid::Uuid::new_v4().to_string();
    verified_web.artifact_id = None;
    verified_web.app_version = "2.0.0".into();
    verified_web.app_build = "200".into();
    verified_web.kind = Kind::NativeCrash;
    verified_web.native_signal = Some(NativeSignal::Segv);
    verified_web.runtime = None;
    verified_web.js_frames.clear();
    let mut unverified_ios = verified_web.clone();
    unverified_ios.event_id = uuid::Uuid::new_v4().to_string();
    unverified_ios.app_build = "201".into();
    register_report_release(&db, &verified_web).await;
    let (_, verified_ios_id) = insert(
        &db,
        &CrashConfig::default(),
        "test",
        &id,
        &rev,
        verified_web,
    )
    .await
    .unwrap();
    let (_, unverified_ios_id) = insert(
        &db,
        &CrashConfig::default(),
        "test",
        &id,
        &rev,
        unverified_ios,
    )
    .await
    .unwrap();

    let (rows, _) = admin::list(&db, &admin::ReportQuery::default())
        .await
        .unwrap();
    let verified = rows
        .iter()
        .map(|row| (&row.row.id, row.release_verified))
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(verified.get(&verified_web_id), Some(&true));
    assert_eq!(verified.get(&unverified_web_id), Some(&false));
    assert_eq!(verified.get(&verified_ios_id), Some(&true));
    assert_eq!(verified.get(&unverified_ios_id), Some(&false));
    assert!(
        admin::detail(&db, &verified_web_id)
            .await
            .unwrap()
            .release_verified
    );
    assert!(
        !admin::detail(&db, &unverified_ios_id)
            .await
            .unwrap()
            .release_verified
    );
}

#[tokio::test]
async fn admin_report_list_uses_one_batched_registry_query_per_page() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let release_reads = Arc::new(AtomicUsize::new(0));
    let observed = release_reads.clone();
    let db = connect_test_database_with_command_handler(
        "crash_report_registry_reads",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command.get_str("find").ok()
                    == Some(crate::models::client_release_registry::COLLECTION_NAME)
            {
                observed.fetch_add(1, Ordering::Relaxed);
            }
        }),
    )
    .await
    .unwrap();
    let now = Utc::now();
    let stored = (0..4)
        .map(|index| {
            let mut row = report();
            row.app_version = format!("1.2.{index}");
            row.app_build = format!("build-{index}");
            row.artifact_id = Some(uuid::Uuid::new_v4().to_string());
            ClientErrorReport {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: uuid::Uuid::new_v4().to_string(),
                event_id: row.event_id.clone(),
                consent_revision: uuid::Uuid::new_v4().to_string(),
                severity: validation::severity(row.kind).into(),
                fingerprint: validation::fingerprint(&row),
                report: row,
                environment: "test".into(),
                received_at: now + chrono::Duration::milliseconds(i64::from(index)),
                expires_at: now + chrono::Duration::days(30),
                export: Export {
                    status: ExportStatus::Disabled,
                    destination_id: None,
                    attempts: 0,
                    lease_id: None,
                    lease_until: None,
                    next_attempt_at: None,
                    deadline: now + chrono::Duration::hours(24),
                    last_error_code: None,
                    exported_at: None,
                },
            }
        })
        .collect::<Vec<_>>();
    db.collection::<ClientErrorReport>(COLLECTION_NAME)
        .insert_many(stored)
        .await
        .unwrap();
    release_reads.store(0, Ordering::Relaxed);
    let (rows, _) = admin::list(&db, &admin::ReportQuery::default())
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(release_reads.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn admin_version_artifact_ids_are_distinct_sorted_and_bounded() {
    let (db, id, rev) = account("crash_artifact_bound").await;
    let artifacts: Vec<String> = (0..12)
        .map(|i| format!("00000000-0000-4000-8000-{i:012x}"))
        .collect();
    for artifact in artifacts.iter().rev().chain(artifacts.iter().take(2)) {
        let mut r = report();
        r.artifact_id = Some(artifact.clone());
        register_report_release(&db, &r).await;
        insert(&db, &CrashConfig::default(), "test", &id, &rev, r)
            .await
            .unwrap();
    }
    let summary = admin::summary(
        &db,
        &admin::SummaryQuery {
            window: None,
            platform: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(summary.versions.len(), 1);
    assert_eq!(summary.versions[0].total, 14);
    assert_eq!(
        summary.versions[0].artifact_ids,
        artifacts.iter().take(10).cloned().collect::<Vec<_>>()
    );
}
use futures::TryStreamExt;

#[tokio::test]
async fn diagnostics_gate_requires_a_policy_acknowledgement_without_changing_analytics() {
    let (db, id, _) = account("crash_policy_gate").await;
    let permission = consent::set_acknowledging(&db, &id, true, consent::PolicyVersion::V1)
        .await
        .unwrap();
    crate::test_utils::await_responsive_majority_io(&db).await;
    assert!(consent::permits_dispatch(&db, &id, &permission.revision).await);
    assert!(
        !(consent::permits_diagnostics(&db, &id, &permission.revision).await
            == consent::ConsentPermission::Permitted(()))
    );
    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&id},
            doc! {"$unset":{"profile_config.analytics.policy_version":""}},
        )
        .await
        .unwrap();
    crate::test_utils::await_responsive_majority_io(&db).await;
    assert!(consent::permits_dispatch(&db, &id, &permission.revision).await);
    assert!(
        !(consent::permits_diagnostics(&db, &id, &permission.revision).await
            == consent::ConsentPermission::Permitted(()))
    );
    assert!(!available(&db, &id).await);
}

#[tokio::test]
async fn simultaneous_first_counter_writes_preserve_every_outcome() {
    let (db, _, _) = account("crash_counters").await;
    futures::future::join_all((0..32).map(|_| count(&db, Some(Platform::Web), "accepted"))).await;
    let rows: Vec<Document> = db
        .collection(COUNTERS)
        .find(doc! {"platform": "web", "outcome": "accepted"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.get_i64("count").unwrap())
            .sum::<i64>(),
        32
    );
}

#[tokio::test]
async fn policy_status_timeout_fails_closed_without_failing_consent() {
    let start = std::time::Instant::now();
    assert_eq!(
        bounded_policy_status(std::future::pending()).await,
        DiagnosticsStatus::Unknown
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
}

pub(crate) async fn register_report_release(db: &Database, r: &CrashReportV1) {
    releases::register(
        db,
        &releases::RegisterRelease {
            platform: r.platform,
            app_version: r.app_version.clone(),
            app_build: r.app_build.clone(),
            artifact_id: r.artifact_id.clone(),
        },
        crate::models::client_release_registry::ReleaseSource::Admin,
        None,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn transient_policy_failures_are_unknown_and_only_definitive_denial_is_13400() {
    assert_eq!(
        bounded_policy_status(async { Err(AppError::SecurityReadinessUnavailable) }).await,
        DiagnosticsStatus::Unknown
    );
    assert!(matches!(
        DiagnosticsStatus::Unknown.admission(),
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        DiagnosticsStatus::NotOffered.admission(),
        Err(AppError::CrashReportingUnavailable)
    ));
    let (db, id, _) = account("crash_eligibility").await;
    assert_eq!(eligibility(&db, &id).await, DiagnosticsStatus::NotOffered);
    super::super::security_readiness_service::ready_test_replica(&db).await;
    super::super::feature_flag_service::set_platform_override(
        &db,
        FLAG,
        &super::super::feature_flag_service::FlagTarget::User(id.clone()),
        true,
        &id,
    )
    .await
    .unwrap();
    assert_eq!(
        eligibility(&db, &id).await,
        DiagnosticsStatus::ConsentRequired
    );
    // A malformed matching override simulates resolution/decoding failure, not denial.
    db.collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .update_one(
            doc! {"flag_key":FLAG},
            doc! {"$set":{"target_kind":"broken"}},
        )
        .await
        .unwrap();
    assert_eq!(eligibility(&db, &id).await, DiagnosticsStatus::Unknown);
}
#[tokio::test]
async fn release_verification_is_live_and_needs_no_report_backfill() {
    let (db, id, rev) = account("crash_registry_aggregation").await;
    let mut r = report();
    r.platform = Platform::Ios;
    r.runtime = None;
    r.js_frames.clear();
    r.artifact_id = None;
    insert(&db, &CrashConfig::default(), "test", &id, &rev, r.clone())
        .await
        .unwrap();
    let q = admin::SummaryQuery {
        window: None,
        platform: None,
    };
    let before = admin::summary(&db, &q).await.unwrap();
    assert!(before.versions.is_empty());
    assert_eq!(before.unverified_releases[0].total, 1);
    register_report_release(&db, &r).await;
    let after = admin::summary(&db, &q).await.unwrap();
    assert_eq!(after.versions[0].total, 1);
    assert!(after.unverified_releases.is_empty());
}
