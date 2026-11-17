//! Final quantities are never written off. Exhausting the short retry burst
//! moves them to a slower, bounded recovery lane instead of stranding holds.
use super::*;

const DEAD_LETTER_RETRY_MILLIS: i64 = 60 * 60 * 1000;
const DEAD_LETTER_BATCH: i64 = 25;
const DEAD_LETTER_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

fn due() -> Document {
    doc! {
        "status": "dead_letter", "released": false,
        "quantity": {"$ne": Bson::Null},
        "last_error": "billing settlement retries exhausted",
        "$expr": {"$lte": [
            {"$ifNull": ["$settlement_next_retry_at", {"$add": ["$updated_at", DEAD_LETTER_RETRY_MILLIS]}]},
            "$$NOW",
        ]},
    }
}

pub(super) async fn recover_dead_letters(
    db: &mongodb::Database,
) -> AppResult<SettlementRecoveryStats> {
    // A cancelled money step retains its existing durable claim/lock bridge;
    // cancellation cannot turn recorded quantities into unknown consumption.
    let mut failures = Failures::default();
    let result = match tokio::time::timeout(DEAD_LETTER_BUDGET, recover(db, &mut failures)).await {
        Ok(result) => result,
        Err(_) => Ok(SettlementRecoveryStats::default()),
    };
    failures.finish(result?)
}

async fn recover(
    db: &mongodb::Database,
    failures: &mut Failures,
) -> AppResult<SettlementRecoveryStats> {
    let rows: Vec<Document> = db
        .collection::<Document>(USAGE_METER)
        .find(due())
        .sort(doc! {"updated_at":1,"_id":1})
        .limit(recovery_batch::limit(DEAD_LETTER_BATCH))
        .await?
        .try_collect()
        .await?;
    let mut stats = SettlementRecoveryStats::default();
    let started = std::time::Instant::now();
    for raw in rows {
        if !super::super::reconcile::continue_reconcile()
            || started.elapsed() >= std::time::Duration::from_secs(5)
        {
            break;
        }
        let result: AppResult<()> = recovery_batch::work(async {
            let row: UsageMeterRow = recovery_batch::decode(&raw)?;
            let mut filter = due();
            filter.insert("_id", &row.id);
            filter.insert("quantity", row.quantity);
            let claimed = db
                .collection::<UsageMeterRow>(USAGE_METER)
                .find_one_and_update(
                    filter,
                    vec![doc! {"$set": {
                        "status": "failed", "updated_at": "$$NOW",
                        "settlement_next_retry_at": {"$add": ["$$NOW", DEAD_LETTER_RETRY_MILLIS]},
                    }}],
                )
                .return_document(ReturnDocument::After)
                .await?;
            let Some(claimed) = claimed else {
                return Ok(());
            };
            match claim_released_and_settle(db, &claimed).await {
                Ok(true) => stats.recovered += 1,
                Ok(false) => {}
                Err(error) => {
                    record_settlement_failure(db, &claimed, Utc::now()).await?;
                    stats.retried += 1;
                    return Err(error);
                }
            }
            Ok(())
        })
        .await;
        if let Err(error) = result {
            failures.note(error);
        }
    }
    Ok(stats)
}
