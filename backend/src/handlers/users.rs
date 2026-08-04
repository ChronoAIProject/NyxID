use axum::{Json, extract::State, http::HeaderMap};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{admin_user_service, audit_service, role_service, telemetry_erasure_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Request / Response types ---

/// AI-services (and future) onboarding flags exposed to the frontend.
/// Timestamps are rfc3339 strings; `None` means the flow is not done.
#[derive(Debug, Serialize)]
pub struct OnboardingStateResponse {
    pub ai_services_completed_at: Option<String>,
}

/// User-scoped config / preferences surfaced on `GET /users/me`.
#[derive(Debug, Serialize)]
pub struct ProfileConfigResponse {
    pub onboarding: OnboardingStateResponse,
}

#[derive(Debug, Serialize)]
pub struct UserProfileResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified: bool,
    pub mfa_enabled: bool,
    pub is_admin: bool,
    pub is_operator: bool,
    /// Resolved platform role: `"admin"`, `"operator"`, or `"user"`.
    pub role: String,
    pub is_active: bool,
    pub social_provider: Option<String>,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub profile_config: ProfileConfigResponse,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpdateProfileResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteAccountResponse {
    pub status: String,
    pub deleted_at: String,
}

// --- Handlers ---

/// GET /api/v1/users/me
///
/// Returns the profile of the currently authenticated user.
pub async fn get_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UserProfileResponse>> {
    let user_id = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let platform_role = role_service::resolve_platform_role(&state.db, &user_model).await?;
    let role = platform_role.as_str().to_string();
    let (is_admin, is_operator) = platform_role.legacy_flags();
    Ok(Json(UserProfileResponse {
        id: user_model.id,
        email: user_model.email,
        display_name: user_model.display_name,
        avatar_url: user_model.avatar_url,
        email_verified: user_model.email_verified,
        mfa_enabled: user_model.mfa_enabled,
        is_admin,
        is_operator,
        role,
        is_active: user_model.is_active,
        social_provider: user_model.social_provider,
        created_at: user_model.created_at.to_rfc3339(),
        last_login_at: user_model.last_login_at.map(|t| t.to_rfc3339()),
        profile_config: ProfileConfigResponse {
            onboarding: OnboardingStateResponse {
                ai_services_completed_at: user_model
                    .profile_config
                    .onboarding
                    .ai_services_completed_at
                    .map(|t| t.to_rfc3339()),
            },
        },
    }))
}

#[derive(Debug, Deserialize)]
pub struct CompleteOnboardingRequest {
    /// Onboarding flow identifier. Currently only `"ai_services"`.
    pub key: String,
}

/// POST /api/v1/users/me/onboarding/complete
///
/// Marks a first-run onboarding flow as completed (or skipped) for the
/// caller, so the post-login wizard redirect stops firing. Idempotent:
/// re-stamping an already-completed flow just refreshes the timestamp.
pub async fn complete_onboarding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CompleteOnboardingRequest>,
) -> AppResult<Json<OnboardingStateResponse>> {
    let user_id = auth_user.user_id.to_string();

    let field = match req.key.as_str() {
        "ai_services" => "profile_config.onboarding.ai_services_completed_at",
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown onboarding key: {other}"
            )));
        }
    };

    let now = Utc::now();
    // Build the `$set` doc explicitly: `field` is a dynamic dotted path, not
    // a literal, so insert it rather than relying on `doc!` key parsing.
    let mut set_doc = bson::Document::new();
    set_doc.insert(field, bson::DateTime::from_chrono(now));
    let result = state
        .db
        .collection::<User>(USERS)
        .update_one(doc! { "_id": &user_id }, doc! { "$set": set_doc })
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("User not found".to_string()));
    }

    Ok(Json(OnboardingStateResponse {
        ai_services_completed_at: Some(now.to_rfc3339()),
    }))
}

/// PUT /api/v1/users/me
///
/// Update the profile of the currently authenticated user.
pub async fn update_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateProfileRequest>,
) -> AppResult<Json<UpdateProfileResponse>> {
    let user_id = auth_user.user_id.to_string();

    // Verify user exists
    let _existing = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let mut set_doc = doc! {};

    if let Some(ref name) = body.display_name {
        if name.len() > 200 {
            return Err(AppError::ValidationError(
                "Display name must be 200 characters or less".to_string(),
            ));
        }
        set_doc.insert("display_name", name);
    }

    if let Some(ref url) = body.avatar_url {
        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Avatar URL must be 2048 characters or less".to_string(),
            ));
        }
        // Validate URL scheme to prevent javascript: and data: URI injection
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(AppError::ValidationError(
                "Avatar URL must use https:// or http:// scheme".to_string(),
            ));
        }
        set_doc.insert("avatar_url", url);
    }

    let now = chrono::Utc::now();
    set_doc.insert("updated_at", bson::DateTime::from_chrono(now));

    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! { "_id": &user_id }, doc! { "$set": set_doc })
        .await?;

    // Re-fetch the updated user
    let updated = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::Internal("User disappeared after update".to_string()))?;

    Ok(Json(UpdateProfileResponse {
        id: updated.id,
        email: updated.email,
        display_name: updated.display_name,
        avatar_url: updated.avatar_url,
        message: "Profile updated successfully".to_string(),
    }))
}

/// DELETE /api/v1/users/me
///
/// Permanently delete the currently authenticated user and related credentials/sessions.
pub async fn delete_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    _headers: HeaderMap,
) -> AppResult<Json<DeleteAccountResponse>> {
    let user_id = auth_user.user_id.to_string();

    // GDPR erasure: when telemetry is on, enqueue the PostHog
    // person-delete job BEFORE the user row is removed. Once the user
    // row is gone we cannot re-enqueue on failure, and emitting a
    // `user.deleted` event without a corresponding delete job would
    // leave dangling telemetry we could never reconcile. So: if
    // telemetry is enabled AND enqueue fails, we abort the whole
    // delete with an internal error rather than create that dangling
    // state. User retries the operation once the transient issue
    // clears.
    //
    // The enqueue step is skipped entirely when telemetry is hard-off
    // (no DSN configured), keeping the default-off path identical to
    // the pre-telemetry delete flow.
    let telemetry_on = state.telemetry.is_some();
    if telemetry_on {
        telemetry_erasure_service::enqueue(&state.db, &user_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    user_id = %user_id,
                    error = %e,
                    "telemetry erasure enqueue failed; aborting delete to avoid dangling events"
                );
                e
            })?;
    }

    admin_user_service::delete_current_user_cascade(&state.db, &user_id).await?;

    let deleted_at = chrono::Utc::now().to_rfc3339();
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "user.account.deleted",
        Some(serde_json::json!({ "self_service": true })),
    );

    // Telemetry: emit user.deleted AFTER the DB cascade so the event's
    // distinct_id is still resolvable server-side when PostHog processes
    // it. The erasure worker will cascade the delete on PostHog's side.
    // No-op when `state.telemetry` is None.
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::UserDeleted {
            reason: Some("self_service".to_string()),
        },
    );

    Ok(Json(DeleteAccountResponse {
        status: "DELETED".to_string(),
        deleted_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use uuid::Uuid;

    #[tokio::test]
    async fn get_me_derives_platform_role_fields_from_rbac_membership() {
        let Some(db) = connect_test_database("users_me_platform_role").await else {
            eprintln!("skipping users/me role test: no local MongoDB available");
            return;
        };
        role_service::seed_system_roles(&db)
            .await
            .expect("seed platform roles");
        let platform_role_ids = role_service::get_platform_role_ids(&db)
            .await
            .expect("platform role ids");

        let user_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, UserType::Person);
        user.role_ids.push(platform_role_ids.operator);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert user");
        let state = test_app_state(db);

        let response = get_me(State(state), test_auth_user(&user_id))
            .await
            .expect("get profile");

        assert_eq!(response.0.role, "operator");
        assert!(!response.0.is_admin);
        assert!(response.0.is_operator);
    }

    // --- Serialization tests: UserProfileResponse ---

    #[test]
    fn user_profile_response_serialization_full() {
        let resp = UserProfileResponse {
            id: "user-1".to_string(),
            email: "test@example.com".to_string(),
            display_name: Some("Alice".to_string()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            email_verified: true,
            mfa_enabled: false,
            is_admin: true,
            is_operator: false,
            role: "admin".to_string(),
            is_active: true,
            social_provider: Some("google".to_string()),
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            last_login_at: Some("2025-06-01T12:00:00+00:00".to_string()),
            profile_config: ProfileConfigResponse {
                onboarding: OnboardingStateResponse {
                    ai_services_completed_at: Some("2025-03-15T10:00:00+00:00".to_string()),
                },
            },
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["id"], "user-1");
        assert_eq!(json["email"], "test@example.com");
        assert_eq!(json["display_name"], "Alice");
        assert_eq!(json["avatar_url"], "https://example.com/avatar.png");
        assert_eq!(json["email_verified"], true);
        assert_eq!(json["mfa_enabled"], false);
        assert_eq!(json["is_admin"], true);
        assert_eq!(json["is_operator"], false);
        assert_eq!(json["role"], "admin");
        assert_eq!(json["is_active"], true);
        assert_eq!(json["social_provider"], "google");
        assert_eq!(json["created_at"], "2025-01-01T00:00:00+00:00");
        assert_eq!(json["last_login_at"], "2025-06-01T12:00:00+00:00");
        assert_eq!(
            json["profile_config"]["onboarding"]["ai_services_completed_at"],
            "2025-03-15T10:00:00+00:00"
        );
    }

    #[test]
    fn user_profile_response_serialization_minimal() {
        let resp = UserProfileResponse {
            id: "user-2".to_string(),
            email: "minimal@example.com".to_string(),
            display_name: None,
            avatar_url: None,
            email_verified: false,
            mfa_enabled: false,
            is_admin: false,
            is_operator: false,
            role: "user".to_string(),
            is_active: true,
            social_provider: None,
            created_at: "2025-06-01T00:00:00+00:00".to_string(),
            last_login_at: None,
            profile_config: ProfileConfigResponse {
                onboarding: OnboardingStateResponse {
                    ai_services_completed_at: None,
                },
            },
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert!(json["display_name"].is_null());
        assert!(json["avatar_url"].is_null());
        assert!(json["social_provider"].is_null());
        assert!(json["last_login_at"].is_null());
        assert!(json["profile_config"]["onboarding"]["ai_services_completed_at"].is_null());
        assert_eq!(json["role"], "user");
    }

    // --- Serialization tests: OnboardingStateResponse ---

    #[test]
    fn onboarding_state_response_serialization_with_timestamp() {
        let resp = OnboardingStateResponse {
            ai_services_completed_at: Some("2025-05-20T08:30:00+00:00".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json["ai_services_completed_at"],
            "2025-05-20T08:30:00+00:00"
        );
    }

    #[test]
    fn onboarding_state_response_serialization_without_timestamp() {
        let resp = OnboardingStateResponse {
            ai_services_completed_at: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["ai_services_completed_at"].is_null());
    }

    // --- Serialization tests: ProfileConfigResponse ---

    #[test]
    fn profile_config_response_serialization() {
        let resp = ProfileConfigResponse {
            onboarding: OnboardingStateResponse {
                ai_services_completed_at: Some("2025-01-01T00:00:00+00:00".to_string()),
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["onboarding"].is_object());
        assert_eq!(
            json["onboarding"]["ai_services_completed_at"],
            "2025-01-01T00:00:00+00:00"
        );
    }

    // --- Serialization tests: UpdateProfileResponse ---

    #[test]
    fn update_profile_response_serialization_full() {
        let resp = UpdateProfileResponse {
            id: "user-1".to_string(),
            email: "updated@example.com".to_string(),
            display_name: Some("Bob".to_string()),
            avatar_url: Some("https://cdn.example.com/bob.jpg".to_string()),
            message: "Profile updated successfully".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["id"], "user-1");
        assert_eq!(json["email"], "updated@example.com");
        assert_eq!(json["display_name"], "Bob");
        assert_eq!(json["avatar_url"], "https://cdn.example.com/bob.jpg");
        assert_eq!(json["message"], "Profile updated successfully");
    }

    #[test]
    fn update_profile_response_serialization_with_nulls() {
        let resp = UpdateProfileResponse {
            id: "user-2".to_string(),
            email: "test@example.com".to_string(),
            display_name: None,
            avatar_url: None,
            message: "Profile updated successfully".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert!(json["display_name"].is_null());
        assert!(json["avatar_url"].is_null());
    }

    // --- Serialization tests: DeleteAccountResponse ---

    #[test]
    fn delete_account_response_serialization() {
        let resp = DeleteAccountResponse {
            status: "DELETED".to_string(),
            deleted_at: "2025-06-15T14:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["status"], "DELETED");
        assert_eq!(json["deleted_at"], "2025-06-15T14:00:00+00:00");
    }

    // --- Deserialization tests: UpdateProfileRequest ---

    #[test]
    fn update_profile_request_deserialization_all_fields() {
        let json = serde_json::json!({
            "display_name": "New Name",
            "avatar_url": "https://example.com/new-avatar.png"
        });
        let req: UpdateProfileRequest = serde_json::from_value(json).unwrap();

        assert_eq!(req.display_name.as_deref(), Some("New Name"));
        assert_eq!(
            req.avatar_url.as_deref(),
            Some("https://example.com/new-avatar.png")
        );
    }

    #[test]
    fn update_profile_request_deserialization_empty_body() {
        let json = serde_json::json!({});
        let req: UpdateProfileRequest = serde_json::from_value(json).unwrap();

        assert!(req.display_name.is_none());
        assert!(req.avatar_url.is_none());
    }

    #[test]
    fn update_profile_request_deserialization_partial() {
        let json = serde_json::json!({
            "display_name": "Only Name"
        });
        let req: UpdateProfileRequest = serde_json::from_value(json).unwrap();

        assert_eq!(req.display_name.as_deref(), Some("Only Name"));
        assert!(req.avatar_url.is_none());
    }

    // --- Deserialization tests: CompleteOnboardingRequest ---

    #[test]
    fn complete_onboarding_request_deserialization() {
        let json = serde_json::json!({ "key": "ai_services" });
        let req: CompleteOnboardingRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.key, "ai_services");
    }

    #[test]
    fn complete_onboarding_request_deserialization_unknown_key_accepted() {
        // Deserialization accepts any string; validation happens in handler
        let json = serde_json::json!({ "key": "unknown_flow" });
        let req: CompleteOnboardingRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.key, "unknown_flow");
    }

    // --- Structural tests: response field coverage ---

    #[test]
    fn user_profile_response_has_all_expected_json_keys() {
        let resp = UserProfileResponse {
            id: "u1".to_string(),
            email: "e@e.com".to_string(),
            display_name: None,
            avatar_url: None,
            email_verified: false,
            mfa_enabled: false,
            is_admin: false,
            is_operator: false,
            role: "user".to_string(),
            is_active: true,
            social_provider: None,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            last_login_at: None,
            profile_config: ProfileConfigResponse {
                onboarding: OnboardingStateResponse {
                    ai_services_completed_at: None,
                },
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        let expected_keys = vec![
            "id",
            "email",
            "display_name",
            "avatar_url",
            "email_verified",
            "mfa_enabled",
            "is_admin",
            "is_operator",
            "role",
            "is_active",
            "social_provider",
            "created_at",
            "last_login_at",
            "profile_config",
        ];
        for key in &expected_keys {
            assert!(obj.contains_key(*key), "Missing expected key: {}", key);
        }
        assert_eq!(obj.len(), expected_keys.len());
    }
}
