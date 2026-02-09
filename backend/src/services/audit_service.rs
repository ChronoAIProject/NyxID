use chrono::Utc;
use uuid::Uuid;

use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};

/// Fire-and-forget audit log entry.
///
/// Spawns a background task to write the audit record so that the calling
/// handler is not blocked by the database write. Errors are logged but
/// do not propagate.
pub fn log_async(
    db: mongodb::Database,
    user_id: Option<String>,
    event_type: String,
    event_data: Option<serde_json::Value>,
    ip_address: Option<String>,
    user_agent: Option<String>,
) {
    tokio::spawn(async move {
        let entry = AuditLog {
            id: Uuid::new_v4().to_string(),
            user_id,
            event_type: event_type.clone(),
            event_data,
            ip_address,
            user_agent,
            created_at: Utc::now(),
        };

        if let Err(e) = db
            .collection::<AuditLog>(AUDIT_LOG)
            .insert_one(&entry)
            .await
        {
            tracing::error!(event_type = %event_type, error = %e, "Failed to write audit log");
        }
    });
}
