use super::*;
use crate::{
    models::user::{User, UserType},
    test_utils::{connect_test_database, test_user},
};
use serde_json::json;

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
    let revision = consent::set_acknowledging(&db, &id, true, consent::PolicyVersion::Diagnostics)
        .await
        .unwrap()
        .revision;
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
    assert_eq!(row.export.status, ExportStatus::Disabled);
    assert_eq!((row.expires_at - row.received_at).num_days(), 30);
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
    for policy in [
        consent::PolicyVersion::V1,
        consent::PolicyVersion::Diagnostics,
    ] {
        let disabled = consent::set_acknowledging(&db, &id, false, policy)
            .await
            .unwrap();
        assert!(!consent::permits_diagnostics(&db, &id, &disabled.revision).await);
        let enabled = consent::set_acknowledging(&db, &id, true, policy)
            .await
            .unwrap();
        assert!(consent::permits_dispatch(&db, &id, &enabled.revision).await);
        assert_eq!(
            consent::permits_diagnostics(&db, &id, &enabled.revision).await,
            matches!(policy, consent::PolicyVersion::Diagnostics)
        );
        assert!(!consent::permits_diagnostics(&db, &id, &rev).await);
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
        crate::test_utils::ensure_rate_window_headroom(
            Duration::from_secs(seconds),
            Duration::from_secs(10),
        )
        .await;
        for _ in 0..limit {
            assert!(
                super::super::coordination_service::RateWindowStore::admit(
                    &db,
                    namespace,
                    key,
                    limit,
                    Duration::from_secs(seconds)
                )
                .await
                .unwrap()
                .allowed
            );
        }
        assert!(
            matches!(rate_limits(&db,"actor",ip).await,Err(AppError::RateLimitedWithRetry{retry_after}) if retry_after>0)
        );
    }
}
#[tokio::test]
async fn admin_aggregation_pagination_and_window_filters_are_correct() {
    let (db, id, rev) = account("crash_summary").await;
    for kind in [Kind::JsError, Kind::RenderError, Kind::JsError] {
        let mut r = report();
        r.kind = kind;
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
    // Each build artifact is independent, even when its version/build match.
    assert_eq!(
        summary
            .versions
            .iter()
            .map(|d| d
                .get_i32("total")
                .unwrap_or_else(|_| d.get_i64("total").unwrap() as i32))
            .sum::<i32>(),
        3
    );
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
    assert!(first.iter().all(|r| r.id != second[0].id));
}
use futures::TryStreamExt;

#[tokio::test]
async fn diagnostics_gate_requires_a_policy_acknowledgement_without_changing_analytics() {
    let (db, id, _) = account("crash_policy_gate").await;
    let permission = consent::set_acknowledging(&db, &id, true, consent::PolicyVersion::V1)
        .await
        .unwrap();
    assert!(consent::permits_dispatch(&db, &id, &permission.revision).await);
    assert!(!consent::permits_diagnostics(&db, &id, &permission.revision).await);
    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&id},
            doc! {"$unset":{"profile_config.analytics.policy_version":""}},
        )
        .await
        .unwrap();
    assert!(consent::permits_dispatch(&db, &id, &permission.revision).await);
    assert!(!consent::permits_diagnostics(&db, &id, &permission.revision).await);
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
        ("1", false, false)
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
}
