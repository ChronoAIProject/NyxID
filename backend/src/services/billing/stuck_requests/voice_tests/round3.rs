use super::round2::{duration, window_done};
use super::*;
use crate::models::assistant_voice::WINDOWS;
use crate::models::assistant_voice_session::COLLECTION_NAME as SESSIONS;
use crate::services::billing::{grants, reconcile::BillingReconciler, voice as windows};

async fn poison(f: &Fixture, count: usize, malformed_window: bool) {
    let template =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    for i in 0..count {
        let id = format!("poison-{i:03}");
        let mut row = template.clone();
        row.insert("_id", &id);
        row.insert("billing_request_id", &id);
        row.insert("deadline", bson::DateTime::from_millis(-1));
        row.insert("sealed", true);
        if malformed_window {
            row.insert("observed_seconds", "bad");
        } else {
            f.db.collection::<Document>(METERS).insert_one(doc!{"_id":&id,"billing_request_id":&id,"transaction_id":&id,"layer":"platform","forwarded":true,"released":true,"metric":"invalid"}).await.unwrap();
        }
        f.db.collection::<Document>(WINDOWS)
            .insert_one(row)
            .await
            .unwrap();
    }
}

fn capture() -> (tempfile::NamedTempFile, tracing::subscriber::DefaultGuard) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(file.reopen().unwrap())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    (file, guard)
}

#[tokio::test]
async fn voice_r3_failed_window_does_not_abort_real_billing_pass() {
    let (f, _, id) = duration(true).await;
    poison(&f, 1, true).await;
    let grants = grants::issue_grants(
        &f.db,
        grants::IssueCreditGrantInput {
            amount_credits: 1,
            target_kind: crate::models::billing_target::BillingTargetKind::SelectedUsers,
            target_user_ids: vec![f.owner.clone()],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            expires_at: None,
            reason: None,
            granted_by: f.owner.clone(),
            amount: None,
            service_limits: vec![],
            request_id: None,
        },
    )
    .await
    .unwrap();
    f.db.collection::<Document>("credit_grants")
        .update_one(
            doc! {"_id":&grants[0].id},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let (file, _guard) = capture();
    let result = BillingReconciler::new(f.db.clone(), None, f.service.config.clone())
        .run_once()
        .await;
    let logs = std::fs::read_to_string(file.path()).unwrap();
    let stats = result.expect("one failed window must not abort the billing pass");
    assert_eq!(stats.grants_expired, 1);
    assert_eq!(stats.recovered_settlements, 1);
    let lines: Vec<_> = logs
        .lines()
        .filter(|s| s.contains("billing_voice_window_recovery_failed"))
        .collect();
    assert_eq!(lines.len(), 1, "{logs}");
    assert!(
        lines[0].contains("ERROR") && lines[0].contains("failed_windows=1"),
        "{logs}"
    );
    window_done(&f, &id).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_r3_101_failed_windows_rotate_and_retry_without_starvation() {
    let (f, _, id) = duration(true).await;
    poison(&f, 101, false).await;
    f.count();
    let first = windows::reconcile(&f.db).await.unwrap_or_default();
    let first_commands = f.count();
    let retried =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {"_id":"poison-000"})
            .await
            .unwrap()
            .unwrap();
    let first_at = retried.get_datetime("reconcile_failed_at").ok().copied();
    let second = windows::reconcile(&f.db).await.unwrap_or_default();
    let retried =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {"_id":"poison-000"})
            .await
            .unwrap()
            .unwrap();
    let second_at = retried.get_datetime("reconcile_failed_at").ok().copied();
    println!("VOICE_R3_FAILED_BATCH commands={first_commands} failures=100");
    assert_eq!(
        (first, second),
        (0, 1),
        "healthy window must finish by pass two"
    );
    assert!(
        first_at.is_some() && second_at > first_at,
        "failed windows must be retried"
    );
    window_done(&f, &id).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_r3_expected_waits_debug_actionable_failures_warn() {
    for reason in ["live", "pending", "registry", "malformed"] {
        let (f, sid, _) = fixture("responses", false).await;
        session(&f, &sid, true).await;
        let change = match reason {
            "live" => doc! {"lease_owner":"survivor","live_slot":true},
            "pending" => {
                doc! {"provider_session_id":"provider","final_usage_confirmed":false,"reconcile_deadline":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))}
            }
            "malformed" => doc! {"generation":"bad"},
            _ => doc! {},
        };
        f.db.collection::<Document>(SESSIONS)
            .update_one(doc! {}, doc! {"$set":change})
            .await
            .unwrap();
        if reason == "registry" {
            f.db.collection::<Document>("security_replicas")
                .delete_many(doc! {})
                .await
                .unwrap();
        }
        f.db.collection::<Document>(STATE)
            .insert_one(doc! {"_id":STATE_ID,"next_source":1})
            .await
            .unwrap();
        let (file, _guard) = capture();
        recover(&f.db).await.unwrap();
        let logs = std::fs::read_to_string(file.path()).unwrap();
        let lines: Vec<_> = logs
            .lines()
            .filter(|s| s.contains("Held request recovery deferred"))
            .collect();
        assert_eq!(lines.len(), 1, "{reason}: {logs}");
        assert!(
            lines[0].contains(if reason == "malformed" {
                "WARN"
            } else {
                "DEBUG"
            }),
            "{reason}: {logs}"
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn voice_r3_terminal_preserves_sealed_certain_window() {
    let (f, _, id) = duration(true).await;
    f.db.collection::<Document>(WINDOWS)
        .update_one(doc! {}, doc! {"$set":{"sealed":true,"uncertain":false}})
        .await
        .unwrap();
    let before =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let rows = terminal::publish(&f.db, &id).await.unwrap();
    assert!(!rows.is_empty());
    let after =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    for (key, value) in &before {
        assert_eq!(after.get(key), Some(value), "sealed field {key}");
    }
    assert_ne!(before, after, "fence must be a real write");
    crate::services::billing::meter::settle_persisted(&f.db, rows)
        .await
        .unwrap();
    windows::finalize(&f.db, &id, true).await.unwrap();
    window_done(&f, &id).await;
    f.db.drop().await.unwrap();
}
