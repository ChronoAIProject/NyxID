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
}
