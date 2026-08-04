use axum::{Json, extract::State};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Serialize;

use crate::AppState;
use crate::errors::AppResult;
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::mw::auth::AuthUser;

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct SessionItem {
    pub id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

// --- Handlers ---

/// GET /api/v1/sessions
///
/// List all active (non-revoked, non-expired) sessions for the authenticated user.
pub async fn list_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<Vec<SessionItem>>> {
    let user_id = auth_user.user_id.to_string();
    let now = bson::DateTime::from_chrono(Utc::now());

    let sessions: Vec<Session> = state
        .db
        .collection::<Session>(SESSIONS)
        .find(doc! {
            "user_id": &user_id,
            "revoked": false,
            "expires_at": { "$gt": now },
        })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    let items: Vec<SessionItem> = sessions
        .into_iter()
        .map(|s| SessionItem {
            id: s.id,
            ip_address: s.ip_address,
            user_agent: s.user_agent,
            created_at: s.created_at.to_rfc3339(),
            expires_at: s.expires_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_item_serializes_all_fields() {
        let item = SessionItem {
            id: "sess-1".to_string(),
            ip_address: Some("192.168.1.1".to_string()),
            user_agent: Some("Mozilla/5.0".to_string()),
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            expires_at: "2026-01-08T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "sess-1");
        assert_eq!(json["ip_address"], "192.168.1.1");
        assert_eq!(json["user_agent"], "Mozilla/5.0");
        assert_eq!(json["created_at"], "2026-01-01T00:00:00+00:00");
        assert_eq!(json["expires_at"], "2026-01-08T00:00:00+00:00");
    }

    #[test]
    fn session_item_with_none_optional_fields() {
        let item = SessionItem {
            id: "sess-2".to_string(),
            ip_address: None,
            user_agent: None,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            expires_at: "2026-01-08T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&item).unwrap();
        assert!(json["ip_address"].is_null());
        assert!(json["user_agent"].is_null());
        // Required fields still present
        assert_eq!(json["id"], "sess-2");
    }

    #[test]
    fn session_item_vec_serializes_as_array() {
        let items = vec![
            SessionItem {
                id: "sess-a".to_string(),
                ip_address: None,
                user_agent: None,
                created_at: "2026-01-01T00:00:00+00:00".to_string(),
                expires_at: "2026-01-08T00:00:00+00:00".to_string(),
            },
            SessionItem {
                id: "sess-b".to_string(),
                ip_address: Some("10.0.0.1".to_string()),
                user_agent: Some("curl/8.0".to_string()),
                created_at: "2026-01-02T00:00:00+00:00".to_string(),
                expires_at: "2026-01-09T00:00:00+00:00".to_string(),
            },
        ];
        let json = serde_json::to_value(&items).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "sess-a");
        assert_eq!(arr[1]["id"], "sess-b");
    }
}
