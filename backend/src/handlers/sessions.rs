use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, token_service};

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct SessionItem {
    pub id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    /// True for exactly one row — the session whose access token authorized
    /// this request. Lets the UI mark the "you are here" row so users know
    /// which session they're about to revoke. Always false for API-key /
    /// service-account / delegated callers (they don't carry a session_id).
    pub is_current: bool,
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
    // Only first-party JWT callers carry a session_id; API-key /
    // service-account / delegated callers will have None, so every row
    // ends up is_current=false for them (correct — they have no
    // "current session" to mark).
    let current_session_id = auth_user.session_id.map(|u| u.to_string());

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
        .map(|s| {
            let is_current = current_session_id.as_deref().is_some_and(|cur| cur == s.id);
            SessionItem {
                id: s.id,
                ip_address: s.ip_address,
                user_agent: s.user_agent,
                created_at: s.created_at.to_rfc3339(),
                expires_at: s.expires_at.to_rfc3339(),
                is_current,
            }
        })
        .collect();

    Ok(Json(items))
}

/// DELETE /api/v1/sessions/{id}
///
/// Revoke one of the authenticated user's own sessions by id.
///
/// Self-revocation of the *current* session is intentionally allowed (trust
/// parity with GitHub): the next request from that session will fail auth and
/// funnel the client back to login, so no special-casing is required here.
///
/// - Other user's session -> 403 Forbidden
/// - Non-existent session  -> 404 Not Found
/// - Own session           -> 204 No Content
pub async fn revoke_own_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(session_id): Path<String>,
) -> AppResult<StatusCode> {
    let user_id = auth_user.user_id.to_string();

    let session = state
        .db
        .collection::<Session>(SESSIONS)
        .find_one(doc! { "_id": &session_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Session not found".to_string()))?;

    if session.user_id != user_id {
        return Err(AppError::Forbidden(
            "Cannot revoke another user's session".to_string(),
        ));
    }

    // Shared revoke path: marks the session revoked, nukes its refresh tokens,
    // and cascades to in-memory MCP sessions for this user. Reused so that the
    // DELETE endpoint stays in lock-step with `logout` and admin revocation.
    token_service::revoke_session(&state.db, &session_id, Some(&state.mcp_sessions)).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "user.session.revoked",
        Some(serde_json::json!({ "session_id": &session_id })),
    );

    Ok(StatusCode::NO_CONTENT)
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
            is_current: true,
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "sess-1");
        assert_eq!(json["ip_address"], "192.168.1.1");
        assert_eq!(json["user_agent"], "Mozilla/5.0");
        assert_eq!(json["created_at"], "2026-01-01T00:00:00+00:00");
        assert_eq!(json["expires_at"], "2026-01-08T00:00:00+00:00");
        assert_eq!(json["is_current"], true);
    }

    #[test]
    fn session_item_with_none_optional_fields() {
        let item = SessionItem {
            id: "sess-2".to_string(),
            ip_address: None,
            user_agent: None,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            expires_at: "2026-01-08T00:00:00+00:00".to_string(),
            is_current: false,
        };
        let json = serde_json::to_value(&item).unwrap();
        assert!(json["ip_address"].is_null());
        assert!(json["user_agent"].is_null());
        // Required fields still present
        assert_eq!(json["id"], "sess-2");
        assert_eq!(json["is_current"], false);
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
                is_current: false,
            },
            SessionItem {
                id: "sess-b".to_string(),
                ip_address: Some("10.0.0.1".to_string()),
                user_agent: Some("curl/8.0".to_string()),
                created_at: "2026-01-02T00:00:00+00:00".to_string(),
                expires_at: "2026-01-09T00:00:00+00:00".to_string(),
                is_current: true,
            },
        ];
        let json = serde_json::to_value(&items).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "sess-a");
        assert_eq!(arr[1]["id"], "sess-b");
    }

    // ── revoke_own_session handler tests ───────────────────────────────────
    //
    // These tests need MongoDB and skip cleanly when no local mongod is
    // available (see `test_utils::connect_test_database`). They exercise the
    // three acceptance paths the PM called out:
    //   1. own session   -> 204
    //   2. other user    -> 403
    //   3. not found     -> 404

    use crate::services::token_service::create_session;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user};
    use mongodb::bson::doc;
    use uuid::Uuid;

    async fn seed_session(db: &mongodb::Database, user_id: &str) -> String {
        let issued = create_session(db, user_id, None, None)
            .await
            .expect("create_session in revoke test");
        issued.session_id
    }

    #[tokio::test]
    async fn revoke_own_session_returns_204() {
        let Some(db) = connect_test_database("sessions_revoke_own").await else {
            eprintln!("skipping revoke_own_session_returns_204: no local MongoDB available");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let session_id = seed_session(&db, &user_id).await;
        let state = test_app_state(db.clone());

        let result = revoke_own_session(
            State(state),
            test_auth_user(&user_id),
            Path(session_id.clone()),
        )
        .await
        .expect("revoking own session should succeed");

        assert_eq!(result, StatusCode::NO_CONTENT);

        // Verify the session is actually marked revoked in the DB.
        let stored = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &session_id })
            .await
            .unwrap()
            .expect("session still present after revoke");
        assert!(stored.revoked);
    }

    #[tokio::test]
    async fn revoke_other_user_session_returns_403() {
        let Some(db) = connect_test_database("sessions_revoke_other").await else {
            eprintln!("skipping revoke_other_user_session_returns_403: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let intruder_id = Uuid::new_v4().to_string();
        let session_id = seed_session(&db, &owner_id).await;
        let state = test_app_state(db.clone());

        let err = revoke_own_session(
            State(state),
            test_auth_user(&intruder_id),
            Path(session_id.clone()),
        )
        .await
        .expect_err("revoking another user's session must be forbidden");
        assert!(
            matches!(err, AppError::Forbidden(_)),
            "expected 403 Forbidden, got {err:?}"
        );

        // The session must remain unrevoked: the authorization check must run
        // before any mutation.
        let stored = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &session_id })
            .await
            .unwrap()
            .expect("session still present");
        assert!(!stored.revoked);
    }

    #[tokio::test]
    async fn revoke_nonexistent_session_returns_404() {
        let Some(db) = connect_test_database("sessions_revoke_404").await else {
            eprintln!(
                "skipping revoke_nonexistent_session_returns_404: no local MongoDB available"
            );
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let state = test_app_state(db);

        let err = revoke_own_session(
            State(state),
            test_auth_user(&user_id),
            Path(Uuid::nil().to_string()),
        )
        .await
        .expect_err("revoking a non-existent session must be not found");
        assert!(
            matches!(err, AppError::NotFound(_)),
            "expected 404 Not Found, got {err:?}"
        );
    }
}
