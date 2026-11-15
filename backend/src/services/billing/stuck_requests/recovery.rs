use super::*;
use std::{collections::BTreeSet, time::Duration};
const BATCH_ROWS: i64 = 100;
const BATCH_REQUESTS: usize = 50;
const BUDGET: Duration = Duration::from_secs(5);

pub(crate) async fn recover(db: &mongodb::Database) -> AppResult<()> {
    // A cancelled terminal transaction is atomic. A committed terminal quantity
    // resumes through ordinary settlement, including its financial lock bridges.
    match tokio::time::timeout(BUDGET, run(db)).await {
        Ok(result) => result,
        Err(_) => Ok(()),
    }
}
async fn run(db: &mongodb::Database) -> AppResult<()> {
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
        return Ok(());
    };
    drop(tx);
    let rows: Vec<Document> = db
        .collection::<Document>(METERS)
        .find(source.page_filter(after))
        .hint(mongodb::options::Hint::Name(source.index().to_owned()))
        .sort(doc! {"_id": 1})
        .limit(BATCH_ROWS)
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
        cursor = row.get_str("_id").map_err(|_| unavailable())?.to_owned();
        visited += 1;
        if !matches!(selection::has_reservation(row), Ok(true)) {
            continue;
        }
        let Ok(c) = classify(row, Some(&registry)) else {
            continue;
        };
        if !c.eligible {
            continue;
        }
        let request = row
            .get_str("billing_request_id")
            .map_err(|_| unavailable())?;
        if !seen.insert(request.to_owned()) {
            continue;
        }
        match terminal::publish(db, request).await {
            Ok(rows) => {
                super::super::meter::settle_persisted(db, rows).await?;
            }
            Err(_) => tracing::warn!(
                billing_request_id = request,
                "Unknown request exit deferred; durable state will be rechecked"
            ),
        }
    }
    if visited == rows.len() && rows.len() < BATCH_ROWS as usize {
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
