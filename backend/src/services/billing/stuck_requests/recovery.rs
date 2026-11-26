use super::*;
use crate::services::billing::recovery_batch::{self, Failures};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
const BATCH_ROWS: i64 = 100;
const BATCH_REQUESTS: usize = 50;
const BUDGET: Duration = Duration::from_secs(5);

pub(crate) async fn recover(db: &mongodb::Database) -> AppResult<()> {
    // A cancelled terminal transaction is atomic. A committed terminal quantity
    // resumes through ordinary settlement, including its financial lock bridges.
    let mut deferred = BTreeMap::new();
    let mut failures = Failures::default();
    let result = match tokio::time::timeout(BUDGET, run(db, &mut deferred, &mut failures)).await {
        Ok(result) => result,
        Err(_) => {
            *deferred.entry("pass_time_budget").or_insert(0_u64) += 1;
            Ok(())
        }
    };
    if result.is_err() {
        *deferred
            .entry("pass_read_or_settlement_failed")
            .or_insert(0_u64) += 1;
    }
    if !deferred.is_empty() {
        log_deferred(&deferred);
    }
    result?;
    failures.finish(())
}
async fn run(
    db: &mongodb::Database,
    deferred: &mut BTreeMap<&'static str, u64>,
    failures: &mut Failures,
) -> AppResult<()> {
    if !super::super::reconcile::continue_reconcile() {
        return Ok(());
    }
    let state = db
        .collection::<Document>(STATE)
        .find_one(doc! {"_id":STATE_ID})
        .max_time(Duration::from_secs(2))
        .await?;
    let source_index = state
        .as_ref()
        .and_then(|s| s.get_i32("next_source").ok())
        .filter(|i| (0..3).contains(i))
        .unwrap_or(0) as usize;
    let source = selection::Source::ALL[source_index];
    let cursor_key = format!("cursor_{source_index}");
    let after = state
        .as_ref()
        .and_then(|s| s.get_str(&cursor_key).ok())
        .unwrap_or("");
    let mut tx = db.client().start_session().await?;
    let Ok(registry) = registry(db, &mut tx).await else {
        *deferred.entry("registry_unavailable").or_default() += 1;
        return Ok(());
    };
    drop(tx);
    let rows: Vec<Document> = db
        .collection::<Document>(METERS)
        .find(source.page_filter(after))
        .hint(mongodb::options::Hint::Name(source.index().to_owned()))
        .sort(doc! {"_id": 1})
        .limit(recovery_batch::limit(BATCH_ROWS))
        .max_time(Duration::from_secs(2))
        .await?
        .try_collect()
        .await?;
    let mut seen = BTreeSet::new();
    let mut cursor = after.to_owned();
    let mut visited = 0;
    for row in &rows {
        if !super::super::reconcile::continue_reconcile() || seen.len() == BATCH_REQUESTS {
            break;
        }
        visited += 1;
        let Ok(id) = row.get_str("_id") else {
            failures.note(unavailable());
            *deferred.entry("meter_row_malformed").or_default() += 1;
            continue;
        };
        cursor = id.to_owned();
        if !matches!(selection::has_reservation(row), Ok(true)) {
            continue;
        }
        let Ok(request) = row.get_str("billing_request_id") else {
            failures.note(unavailable());
            *deferred.entry("meter_row_malformed").or_default() += 1;
            continue;
        };
        if !seen.insert(request.to_owned()) {
            continue;
        }
        let c = match classify(row, Some(&registry)) {
            Ok(c) => c,
            Err(error) => {
                failures.note(error);
                *deferred
                    .entry(if voice::is_voice(row) {
                        "voice_malformed_evidence"
                    } else {
                        "meter_row_malformed"
                    })
                    .or_default() += 1;
                continue;
            }
        };
        if !c.eligible {
            if voice::is_voice(row) {
                *deferred.entry(c.label).or_default() += 1;
            }
            continue;
        }
        match terminal::publish_outcome(db, request).await {
            Ok(terminal::Outcome::Published(rows)) => {
                if let Err(error) =
                    recovery_batch::work(super::super::meter::settle_persisted(db, rows)).await
                {
                    failures.note(error);
                    *deferred
                        .entry("pass_read_or_settlement_failed")
                        .or_default() += 1;
                }
            }
            Ok(terminal::Outcome::Deferred(reason)) => {
                *deferred.entry(reason).or_default() += 1;
            }
            Err(error) => {
                let reason = publication_reason(&error);
                *deferred.entry(reason).or_default() += 1;
                if reason != "publication_conflict" {
                    failures.note(error);
                }
            }
        }
    }
    if visited == rows.len() && rows.len() < recovery_batch::limit(BATCH_ROWS) as usize {
        cursor.clear();
    }
    db.collection::<Document>(STATE)
        .update_one(
            doc! {"_id":STATE_ID},
            doc! {"$set":{cursor_key:cursor,"next_source":((source_index+1)%3) as i32}},
        )
        .upsert(true)
        .await?;
    Ok(())
}

pub(super) fn publication_reason(error: &AppError) -> &'static str {
    match error {
        AppError::Conflict(_) => "publication_conflict",
        AppError::DatabaseError(error)
            if error.contains_label("TransientTransactionError")
                && !error.contains_label("UnknownTransactionCommitResult") =>
        {
            "publication_conflict"
        }
        _ => "request_read_or_publication_failed",
    }
}
fn log_deferred(deferred: &BTreeMap<&'static str, u64>) {
    let actionable = deferred.keys().any(|reason| {
        matches!(
            *reason,
            "voice_malformed_evidence"
                | "excluded_voice_unknown_shape"
                | "voice_actor_mismatch"
                | "meter_row_malformed"
                | "request_rows_unavailable"
                | "policy_quantity_unavailable"
                | "request_read_or_publication_failed"
                | "pass_read_or_settlement_failed"
        )
    });
    if actionable {
        tracing::warn!(?deferred, "Held request recovery deferred");
    } else {
        tracing::debug!(?deferred, "Held request recovery deferred");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_r3_conflict_and_failure_summary_levels() {
        let conflict = publication_reason(&AppError::Conflict("changed".into()));
        let failure = publication_reason(&AppError::DatabaseError(mongodb::error::Error::custom(
            "read failed",
        )));
        assert_eq!(conflict, "publication_conflict");
        assert_eq!(failure, "request_read_or_publication_failed");
        for (reasons, level) in [
            (
                vec![conflict, "pass_time_budget", "registry_unavailable"],
                "DEBUG",
            ),
            (vec![conflict, failure], "WARN"),
            (vec!["pass_read_or_settlement_failed"], "WARN"),
            (
                vec![
                    "excluded_voice_unknown_shape",
                    "voice_actor_mismatch",
                    "meter_row_malformed",
                ],
                "WARN",
            ),
        ] {
            let capture = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(capture.reopen().unwrap())
                .finish();
            let _guard = tracing::subscriber::set_default(subscriber);
            tracing::callsite::rebuild_interest_cache();
            log_deferred(&reasons.into_iter().map(|r| (r, 1)).collect());
            let logs = std::fs::read_to_string(capture.path()).unwrap();
            assert_eq!(logs.lines().count(), 1, "{logs}");
            assert!(logs.contains(level), "{logs}");
        }
    }
}
