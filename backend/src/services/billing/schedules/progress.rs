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
