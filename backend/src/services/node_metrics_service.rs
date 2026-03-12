use chrono::Utc;
use mongodb::bson::doc;

use crate::errors::AppResult;
use crate::models::node::{COLLECTION_NAME as NODES, Node};

/// Record a successful proxy request with latency.
/// Uses an aggregation pipeline update to compute an exponential moving average.
pub async fn record_success(
    db: mongodb::Database,
    node_id: String,
    latency_ms: u64,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let latency = latency_ms as f64;

    db.collection::<Node>(NODES)
        .update_one(
            doc! { "_id": &node_id },
            vec![doc! {
                "$set": {
                    "metrics.total_requests": { "$add": [{ "$ifNull": ["$metrics.total_requests", 0_i64] }, 1_i64] },
                    "metrics.success_count": { "$add": [{ "$ifNull": ["$metrics.success_count", 0_i64] }, 1_i64] },
                    "metrics.avg_latency_ms": {
                        "$add": [
                            { "$multiply": [0.9, { "$ifNull": ["$metrics.avg_latency_ms", latency] }] },
                            { "$multiply": [0.1, latency] },
                        ]
                    },
                    "metrics.last_success_at": now,
                    "updated_at": now,
                }
            }],
        )
        .await?;
    Ok(())
}

/// Record a failed proxy request.
pub async fn record_error(db: mongodb::Database, node_id: String, error: String) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    // Truncate error message to prevent unbounded storage
    // Use floor_char_boundary to avoid panic on multi-byte UTF-8 characters
    let error_truncated = if error.len() > 256 {
        let boundary = error.floor_char_boundary(256);
        format!("{}...", &error[..boundary])
    } else {
        error
    };

    db.collection::<Node>(NODES)
        .update_one(
            doc! { "_id": &node_id },
            doc! {
                "$inc": {
                    "metrics.total_requests": 1_i64,
                    "metrics.error_count": 1_i64,
                },
                "$set": {
                    "metrics.last_error": &error_truncated,
                    "metrics.last_error_at": &now,
                    "updated_at": &now,
                }
            },
        )
        .await?;
    Ok(())
}
