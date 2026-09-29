use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::models::invite_code::{InviteCode, InviteCodeUsage};
use crate::mw::auth::AuthUser;
use crate::services::invite_code_service::InviteCodeUsageUser;
use crate::services::{audit_service, invite_code_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Request / Response types ---
//
// Note: the 1..=1000 bound on `max_uses` is enforced by the `validator` crate
// attributes on `CreateInviteCodeRequest::max_uses` below. Keeping the limit
// at the request-type level means the error message is returned through the
// normal validation-error path instead of a bespoke handler check.

#[derive(Debug, Deserialize, Validate)]
pub struct CreateInviteCodeRequest {
    #[validate(range(min = 1, max = 1000, message = "max_uses must be between 1 and 1000"))]
    pub max_uses: Option<i32>,
    #[validate(length(max = 512, message = "Note must be at most 512 characters"))]
    pub note: Option<String>,
}

/// Body for `PATCH /api/v1/admin/invite-codes/{id}`.
///
/// The `note` field is authoritative: whatever value is sent (or absent)
/// becomes the new note. Specifically:
/// - `{"note": "text"}` → sets the note to "text"
/// - `{"note": ""}` → clears the note (stored as null)
/// - `{"note": null}` → clears the note
/// - `{}` (field omitted) → clears the note
///
/// Today only the note is mutable; other fields (code, max_uses, is_active)
/// stay immutable after creation.
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateInviteCodeRequest {
    #[validate(length(max = 512, message = "Note must be at most 512 characters"))]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InviteCodeUsageResponse {
    pub user_id: String,
    pub used_at: String,
    /// Email of the user who redeemed the code, or `null` if the user has
    /// been deleted since the redemption was recorded.
    pub user_email: Option<String>,
    /// Display name of the user who redeemed the code, or `null` if the user
    /// has no display name set or has been deleted.
    pub user_display_name: Option<String>,
}

/// Nested sidecar describing the admin who created this invite code. Resolved
/// via the same batch user lookup used for redemption enrichment, so exposing
/// this adds zero extra DB round-trips. `None` when the creator has been
/// deleted since the code was minted — callers should fall back to rendering
/// the raw `created_by` UUID in that case.
#[derive(Debug, Serialize)]
pub struct InviteCodeCreator {
    /// Email of the admin. Always present whenever `creator` itself is non-null
    /// (the user projection requires it).
    pub email: String,
    /// Display name of the admin, or `null` if they have no display name set.
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InviteCodeResponse {
    pub id: String,
    pub code: String,
    pub max_uses: i32,
    pub used_count: i32,
    pub created_by: String,
    /// Resolved creator info (email + display name). `null` when the admin
    /// has been deleted since the code was minted. See [`InviteCodeCreator`].
    pub creator: Option<InviteCodeCreator>,
    pub note: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
    pub usages: Vec<InviteCodeUsageResponse>,
}

#[derive(Debug, Serialize)]
pub struct InviteCodeListResponse {
    pub invite_codes: Vec<InviteCodeResponse>,
}

#[derive(Debug, Serialize)]
pub struct DeactivateInviteCodeResponse {
    pub message: String,
}

fn usage_to_response(
    usage: InviteCodeUsage,
    users: &HashMap<String, InviteCodeUsageUser>,
) -> InviteCodeUsageResponse {
    let lookup = users.get(&usage.user_id);
    InviteCodeUsageResponse {
        user_email: lookup.map(|u| u.email.clone()),
        user_display_name: lookup.and_then(|u| u.display_name.clone()),
        user_id: usage.user_id,
        used_at: usage.used_at.to_rfc3339(),
    }
}

fn to_response(ic: InviteCode, users: &HashMap<String, InviteCodeUsageUser>) -> InviteCodeResponse {
    let creator = users.get(&ic.created_by).map(|u| InviteCodeCreator {
        email: u.email.clone(),
        display_name: u.display_name.clone(),
    });
    InviteCodeResponse {
        id: ic.id,
        code: ic.code,
        max_uses: ic.max_uses,
        used_count: ic.used_count,
        creator,
        created_by: ic.created_by,
        note: ic.note,
        is_active: ic.is_active,
        created_at: ic.created_at.to_rfc3339(),
        updated_at: ic.updated_at.to_rfc3339(),
        usages: ic
            .usages
            .into_iter()
            .map(|u| usage_to_response(u, users))
            .collect(),
    }
}

// --- Handlers ---

/// POST /api/v1/admin/invite-codes
///
/// Create a new invite code (admin only).
pub async fn create_invite_code(
    State(state): State<AppState>,
    ConnectInfo(_peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    _headers: HeaderMap,
    Json(body): Json<CreateInviteCodeRequest>,
) -> AppResult<Json<InviteCodeResponse>> {
    require_admin(&state, &auth_user).await?;

    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let max_uses = body.max_uses.unwrap_or(10);
    let admin_id = auth_user.user_id.to_string();

    let invite = invite_code_service::create_invite_code(
        &state.db,
        &admin_id,
        max_uses,
        body.note.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_invite_code_create",
        Some(serde_json::json!({
            "invite_code_id": invite.id,
            "code": invite.code,
            "max_uses": invite.max_uses,
        })),
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::InviteCodeGenerated {
            generated_by_role: "admin".to_string(),
        },
    );

    // A freshly-created code has no usages, so the empty user map is fine.
    Ok(Json(to_response(invite, &HashMap::new())))
}

/// GET /api/v1/admin/invite-codes
///
/// List all invite codes (admin only). Each usage entry is enriched with the
/// redeeming user's email and display name via a single batch lookup against
/// the `users` collection.
pub async fn list_invite_codes(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<InviteCodeListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.invite_codes.list").await?;

    let result = invite_code_service::list_invite_codes(&state.db).await?;

    Ok(Json(InviteCodeListResponse {
        invite_codes: result
            .codes
            .into_iter()
            .map(|ic| to_response(ic, &result.users))
            .collect(),
    }))
}

/// PATCH /api/v1/admin/invite-codes/{id}
///
/// Update mutable fields on an invite code (admin only). Currently only the
/// freeform `note` is mutable; the code value, max_uses, and is_active stay
/// immutable after creation. The audit log entry intentionally records *that*
/// the note changed rather than the new value, since notes can contain
/// freeform admin-supplied text we shouldn't persist twice.
pub async fn update_invite_code(
    State(state): State<AppState>,
    ConnectInfo(_peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateInviteCodeRequest>,
) -> AppResult<Json<InviteCodeResponse>> {
    require_admin(&state, &auth_user).await?;

    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let updated = invite_code_service::update_invite_code_note(&state.db, &id, body.note).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_invite_code_update",
        Some(serde_json::json!({
            "invite_code_id": id,
            "fields_changed": ["note"],
        })),
    );

    // Resolve usage users for the single updated code so the response carries
    // the same enrichment shape as the list endpoint. The drawer reuses this
    // payload and expects email/display_name on each usage entry.
    let users =
        invite_code_service::fetch_usage_users(&state.db, std::slice::from_ref(&updated)).await?;
    Ok(Json(to_response(updated, &users)))
}

/// DELETE /api/v1/admin/invite-codes/{id}
///
/// Deactivate an invite code (admin only).
pub async fn deactivate_invite_code(
    State(state): State<AppState>,
    ConnectInfo(_peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<DeactivateInviteCodeResponse>> {
    require_admin(&state, &auth_user).await?;

    invite_code_service::deactivate_invite_code(&state.db, &id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_invite_code_deactivate",
        Some(serde_json::json!({ "invite_code_id": id })),
    );

    Ok(Json(DeactivateInviteCodeResponse {
        message: "Invite code deactivated".to_string(),
    }))
}

#[cfg(test)]
mod pure_tests {
    use super::*;
    use crate::models::invite_code::{InviteCode, InviteCodeUsage};
    use crate::services::invite_code_service::InviteCodeUsageUser;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_invite_code(id: &str, created_by: &str) -> InviteCode {
        let now = Utc::now();
        InviteCode {
            id: id.to_string(),
            code: "NYXID-TEST123".to_string(),
            max_uses: 10,
            used_count: 1,
            created_by: created_by.to_string(),
            note: Some("Test note".to_string()),
            is_active: true,
            created_at: now,
            updated_at: now,
            usages: vec![InviteCodeUsage {
                user_id: "user-1".to_string(),
                used_at: now,
                email: Some("user1@example.com".to_string()),
            }],
        }
    }

    fn make_users_map() -> HashMap<String, InviteCodeUsageUser> {
        let mut map = HashMap::new();
        map.insert(
            "user-1".to_string(),
            InviteCodeUsageUser {
                email: "user1@example.com".to_string(),
                display_name: Some("User One".to_string()),
            },
        );
        map.insert(
            "admin-1".to_string(),
            InviteCodeUsageUser {
                email: "admin@example.com".to_string(),
                display_name: Some("Admin User".to_string()),
            },
        );
        map
    }

    // --- usage_to_response tests ---

    #[test]
    fn usage_to_response_with_known_user() {
        let now = Utc::now();
        let usage = InviteCodeUsage {
            user_id: "user-1".to_string(),
            used_at: now,
            email: Some("user1@example.com".to_string()),
        };
        let users = make_users_map();
        let resp = usage_to_response(usage, &users);

        assert_eq!(resp.user_id, "user-1");
        assert_eq!(resp.user_email, Some("user1@example.com".to_string()));
        assert_eq!(resp.user_display_name, Some("User One".to_string()));
        chrono::DateTime::parse_from_rfc3339(&resp.used_at)
            .expect("used_at should be valid RFC 3339");
    }

    #[test]
    fn usage_to_response_with_unknown_user() {
        let now = Utc::now();
        let usage = InviteCodeUsage {
            user_id: "deleted-user".to_string(),
            used_at: now,
            email: None,
        };
        let users = make_users_map();
        let resp = usage_to_response(usage, &users);

        assert_eq!(resp.user_id, "deleted-user");
        assert!(resp.user_email.is_none());
        assert!(resp.user_display_name.is_none());
    }

    #[test]
    fn usage_to_response_with_empty_users_map() {
        let now = Utc::now();
        let usage = InviteCodeUsage {
            user_id: "user-1".to_string(),
            used_at: now,
            email: None,
        };
        let resp = usage_to_response(usage, &HashMap::new());

        assert_eq!(resp.user_id, "user-1");
        assert!(resp.user_email.is_none());
        assert!(resp.user_display_name.is_none());
    }

    // --- to_response tests ---

    #[test]
    fn to_response_with_known_creator() {
        let ic = make_invite_code("ic-1", "admin-1");
        let users = make_users_map();
        let resp = to_response(ic, &users);

        assert_eq!(resp.id, "ic-1");
        assert_eq!(resp.code, "NYXID-TEST123");
        assert_eq!(resp.max_uses, 10);
        assert_eq!(resp.used_count, 1);
        assert_eq!(resp.created_by, "admin-1");
        assert!(resp.is_active);
        assert_eq!(resp.note, Some("Test note".to_string()));
        assert_eq!(resp.usages.len(), 1);

        let creator = resp.creator.expect("creator should be resolved");
        assert_eq!(creator.email, "admin@example.com");
        assert_eq!(creator.display_name, Some("Admin User".to_string()));
    }

    #[test]
    fn to_response_with_deleted_creator() {
        let ic = make_invite_code("ic-2", "deleted-admin");
        let users = make_users_map();
        let resp = to_response(ic, &users);

        assert!(resp.creator.is_none());
        assert_eq!(resp.created_by, "deleted-admin");
    }

    #[test]
    fn to_response_with_no_usages() {
        let now = Utc::now();
        let ic = InviteCode {
            id: "ic-3".to_string(),
            code: "NYXID-EMPTY".to_string(),
            max_uses: 5,
            used_count: 0,
            created_by: "admin-1".to_string(),
            note: None,
            is_active: true,
            created_at: now,
            updated_at: now,
            usages: vec![],
        };
        let users = make_users_map();
        let resp = to_response(ic, &users);

        assert!(resp.usages.is_empty());
        assert!(resp.note.is_none());
    }

    #[test]
    fn to_response_timestamps_are_rfc3339() {
        let ic = make_invite_code("ic-4", "admin-1");
        let users = make_users_map();
        let resp = to_response(ic, &users);

        chrono::DateTime::parse_from_rfc3339(&resp.created_at)
            .expect("created_at should be valid RFC 3339");
        chrono::DateTime::parse_from_rfc3339(&resp.updated_at)
            .expect("updated_at should be valid RFC 3339");
    }

    #[test]
    fn to_response_enriches_multiple_usages() {
        let now = Utc::now();
        let ic = InviteCode {
            id: "ic-5".to_string(),
            code: "NYXID-MULTI".to_string(),
            max_uses: 100,
            used_count: 2,
            created_by: "admin-1".to_string(),
            note: None,
            is_active: true,
            created_at: now,
            updated_at: now,
            usages: vec![
                InviteCodeUsage {
                    user_id: "user-1".to_string(),
                    used_at: now,
                    email: Some("user1@example.com".to_string()),
                },
                InviteCodeUsage {
                    user_id: "unknown-user".to_string(),
                    used_at: now,
                    email: None,
                },
            ],
        };
        let users = make_users_map();
        let resp = to_response(ic, &users);

        assert_eq!(resp.usages.len(), 2);
        assert_eq!(
            resp.usages[0].user_email,
            Some("user1@example.com".to_string())
        );
        assert!(resp.usages[1].user_email.is_none());
    }

    // --- Serde tests ---

    #[test]
    fn create_invite_code_request_deserializes_defaults() {
        let json = r#"{}"#;
        let req: CreateInviteCodeRequest = serde_json::from_str(json).expect("deserialize");
        assert!(req.max_uses.is_none());
        assert!(req.note.is_none());
    }

    #[test]
    fn create_invite_code_request_deserializes_with_values() {
        let json = r#"{"max_uses": 5, "note": "Beta"}"#;
        let req: CreateInviteCodeRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.max_uses, Some(5));
        assert_eq!(req.note, Some("Beta".to_string()));
    }

    #[test]
    fn invite_code_response_serializes_all_fields() {
        let resp = InviteCodeResponse {
            id: "id-1".to_string(),
            code: "NYXID-ABC".to_string(),
            max_uses: 10,
            used_count: 0,
            created_by: "admin".to_string(),
            creator: Some(InviteCodeCreator {
                email: "admin@example.com".to_string(),
                display_name: None,
            }),
            note: None,
            is_active: true,
            created_at: "2024-01-01T00:00:00+00:00".to_string(),
            updated_at: "2024-01-01T00:00:00+00:00".to_string(),
            usages: vec![],
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["code"], "NYXID-ABC");
        assert!(json["is_active"].as_bool().unwrap());
        assert!(json["note"].is_null());
        assert!(json["creator"]["email"].as_str().is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{ConnectInfo, Path, State};
    use uuid::Uuid;

    async fn insert_admin(db: &mongodb::Database) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed platform roles");
        let platform_role_ids = role_service::get_platform_role_ids(db)
            .await
            .expect("platform role ids");
        let id = Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.role_ids.push(platform_role_ids.admin);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert admin user");
        id
    }

    fn test_peer() -> ConnectInfo<SocketAddr> {
        ConnectInfo("127.0.0.1:12345".parse().unwrap())
    }

    #[tokio::test]
    async fn test_create_invite_code() {
        let Some(db) = connect_test_database("h_invite_codes_create").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let result = create_invite_code(
            State(state),
            test_peer(),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            HeaderMap::new(),
            Json(CreateInviteCodeRequest {
                max_uses: Some(5),
                note: Some("Test invite".to_string()),
            }),
        )
        .await
        .expect("create_invite_code should succeed");

        assert_eq!(result.0.max_uses, 5);
        assert_eq!(result.0.used_count, 0);
        assert!(result.0.is_active);
        assert!(!result.0.code.is_empty());
        assert_eq!(result.0.note, Some("Test invite".to_string()));
    }

    #[tokio::test]
    async fn test_list_invite_codes() {
        let Some(db) = connect_test_database("h_invite_codes_list").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let _ = create_invite_code(
            State(state.clone()),
            test_peer(),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            HeaderMap::new(),
            Json(CreateInviteCodeRequest {
                max_uses: None,
                note: None,
            }),
        )
        .await
        .expect("create_invite_code should succeed");

        let result = list_invite_codes(State(state), test_auth_user(&admin_id))
            .await
            .expect("list_invite_codes should succeed");

        assert_eq!(result.0.invite_codes.len(), 1);
        assert!(result.0.invite_codes[0].is_active);
    }

    #[tokio::test]
    async fn test_create_invite_code_default_max_uses() {
        let Some(db) = connect_test_database("h_invite_codes_defaults").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let result = create_invite_code(
            State(state),
            test_peer(),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            HeaderMap::new(),
            Json(CreateInviteCodeRequest {
                max_uses: None,
                note: None,
            }),
        )
        .await
        .expect("create_invite_code should succeed");

        assert_eq!(result.0.max_uses, 10);
        assert!(result.0.note.is_none());
    }

    #[tokio::test]
    async fn test_update_invite_code() {
        let Some(db) = connect_test_database("h_invite_codes_update").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_invite_code(
            State(state.clone()),
            test_peer(),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            HeaderMap::new(),
            Json(CreateInviteCodeRequest {
                max_uses: Some(3),
                note: None,
            }),
        )
        .await
        .expect("create_invite_code should succeed");

        let code_id = created.0.id.clone();

        let result = update_invite_code(
            State(state),
            test_peer(),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(code_id.clone()),
            Json(UpdateInviteCodeRequest {
                note: Some("Updated note".to_string()),
            }),
        )
        .await
        .expect("update_invite_code should succeed");

        assert_eq!(result.0.id, code_id);
        assert_eq!(result.0.note, Some("Updated note".to_string()));
    }

    #[tokio::test]
    async fn test_deactivate_invite_code() {
        let Some(db) = connect_test_database("h_invite_codes_deactivate").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_invite_code(
            State(state.clone()),
            test_peer(),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            HeaderMap::new(),
            Json(CreateInviteCodeRequest {
                max_uses: Some(10),
                note: None,
            }),
        )
        .await
        .expect("create_invite_code should succeed");

        let code_id = created.0.id.clone();

        let result = deactivate_invite_code(
            State(state.clone()),
            test_peer(),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(code_id.clone()),
        )
        .await
        .expect("deactivate_invite_code should succeed");

        assert_eq!(result.0.message, "Invite code deactivated");

        let list = list_invite_codes(State(state), test_auth_user(&admin_id))
            .await
            .expect("list_invite_codes should succeed");

        let deactivated = list
            .0
            .invite_codes
            .iter()
            .find(|ic| ic.id == code_id)
            .expect("code should still be in list");
        assert!(!deactivated.is_active);
    }
}
