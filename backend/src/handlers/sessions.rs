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
