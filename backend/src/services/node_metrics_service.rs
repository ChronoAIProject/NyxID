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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::test_utils::connect_test_database;

    fn make_test_node(id: &str) -> Node {
        let now = Utc::now();
        Node {
            id: id.to_string(),
            user_id: "test-user".to_string(),
            name: "test-node".to_string(),
            status: NodeStatus::Online,
            auth_token_hash: "hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: String::new(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn record_success_increments_counters() {
        let Some(db) = connect_test_database("nmetrics_success").await else {
            return;
        };

        let node = make_test_node("node-success-1");
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .unwrap();

        record_success(db.clone(), "node-success-1".to_string(), 150)
            .await
            .unwrap();

        let updated = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": "node-success-1" })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.metrics.total_requests, 1);
        assert_eq!(updated.metrics.success_count, 1);
        assert!(updated.metrics.avg_latency_ms > 0.0);
        assert!(updated.metrics.last_success_at.is_some());
    }

    #[tokio::test]
    async fn record_error_increments_error_count() {
        let Some(db) = connect_test_database("nmetrics_error").await else {
            return;
        };

        let node = make_test_node("node-error-1");
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .unwrap();

        record_error(
            db.clone(),
            "node-error-1".to_string(),
            "connection refused".to_string(),
        )
        .await
        .unwrap();

        let updated = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": "node-error-1" })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.metrics.total_requests, 1);
        assert_eq!(updated.metrics.error_count, 1);
        assert_eq!(
            updated.metrics.last_error.as_deref(),
            Some("connection refused")
        );
        assert!(updated.metrics.last_error_at.is_some());
    }

    #[tokio::test]
    async fn record_error_truncates_long_error_message() {
        let Some(db) = connect_test_database("nmetrics_error_trunc").await else {
            return;
        };

        let node = make_test_node("node-trunc-1");
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .unwrap();

        let long_error = "x".repeat(500);
        record_error(db.clone(), "node-trunc-1".to_string(), long_error)
            .await
            .unwrap();

        let updated = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": "node-trunc-1" })
            .await
            .unwrap()
            .unwrap();

        let err = updated.metrics.last_error.unwrap();
        // Should be truncated to 256 chars + "..."
        assert!(err.len() <= 260);
        assert!(err.ends_with("..."));
    }

    #[tokio::test]
    async fn record_success_on_nonexistent_node_is_noop() {
        let Some(db) = connect_test_database("nmetrics_noop").await else {
            return;
        };
        // Should not error -- update_one on a missing document is a no-op
        record_success(db.clone(), "nonexistent-node".to_string(), 100)
            .await
            .unwrap();
    }
}
