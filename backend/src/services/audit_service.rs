use chrono::Utc;
use uuid::Uuid;

use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::mw::auth::AuthUser;

/// Fire-and-forget audit log entry.
///
/// Spawns a background task to write the audit record so that the calling
/// handler is not blocked by the database write. Errors are logged but
/// do not propagate.
#[allow(clippy::too_many_arguments)]
pub fn log_async(
    db: mongodb::Database,
    user_id: Option<String>,
    event_type: String,
    event_data: Option<serde_json::Value>,
    ip_address: Option<String>,
    user_agent: Option<String>,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
) {
    tokio::spawn(async move {
        let entry = AuditLog {
            id: Uuid::new_v4().to_string(),
            user_id,
            event_type: event_type.clone(),
            event_data,
            ip_address,
            user_agent,
            api_key_id,
            api_key_name,
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

/// Fire-and-forget audit log entry attributed to an authenticated request.
///
/// Pulls user_id, IP, User-Agent, and API key identity from `AuthUser` so
/// every audit record carries consistent forensic context. Prefer this over
/// `log_async` when the call site already has an `AuthUser` extractor.
pub fn log_for_user(
    db: mongodb::Database,
    auth_user: &AuthUser,
    event_type: impl Into<String>,
    event_data: Option<serde_json::Value>,
) {
    log_async(
        db,
        Some(auth_user.user_id.to_string()),
        event_type.into(),
        event_data,
        auth_user.ip_address.clone(),
        auth_user.user_agent.clone(),
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );
}
