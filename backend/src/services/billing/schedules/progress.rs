use chrono::{DateTime, Utc};
use mongodb::bson::{self, doc};

use crate::errors::{AppError, AppResult};
use crate::models::credit_grant::{COLLECTION_NAME as CREDIT_GRANTS, CreditGrant};

pub(super) async fn count_period_grants(
    db: &mongodb::Database,
    schedule_id: &str,
    period_start: DateTime<Utc>,
) -> AppResult<i64> {
    let count = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .count_documents(doc! {
            "schedule_origin.schedule_id": schedule_id,
            "schedule_origin.period_start": bson::DateTime::from_chrono(period_start),
        })
        .await?;
    i64::try_from(count)
        .map_err(|_| AppError::Internal("credit schedule grant count overflowed".to_string()))
}

// Publish only a lower bound derived from durable grant identities. Other
// issuers can count the same grants before this update reaches MongoDB.
pub(super) async fn refresh_count(
    db: &mongodb::Database,
    period: &crate::models::credit_schedule_period::CreditSchedulePeriod,
) -> AppResult<()> {
    use crate::models::credit_schedule_period::{COLLECTION_NAME, CreditSchedulePeriod};

    let count = count_period_grants(db, &period.schedule_id, period.period_start).await?;
    db.collection::<CreditSchedulePeriod>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": &period.id },
            doc! { "$max": { "disbursed_count": count } },
        )
        .await?;
    Ok(())
}
