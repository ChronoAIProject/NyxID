use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::platform_service_preference::{
    PlatformOperationPreferenceOverride, PlatformServicePreference,
};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{audit_service, platform_preference_service};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformPreferenceOwnerQuery {
    owner_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlatformPreferenceRequest {
    pub platform_enabled: bool,
    pub max_credits_per_call: String,
    pub max_credits_per_day: String,
    #[serde(default)]
    pub operation_overrides: Vec<PlatformOperationPreferenceOverride>,
}

#[derive(Debug, Serialize)]
pub struct PlatformPreferenceListResponse {
    preferences: Vec<PlatformPreferenceResponse>,
}

#[derive(Debug, Serialize)]
pub struct PlatformPreferenceResponse {
    id: String,
    owner_id: String,
    catalog_service_id: String,
    platform_enabled: bool,
    max_credits_per_call: String,
    max_credits_per_day: String,
    operation_overrides: Vec<PlatformOperationPreferenceOverride>,
    created_by: String,
    updated_by: String,
    created_at: String,
    updated_at: String,
}

/// GET /api/v1/platform-ops/preferences
pub async fn list_preferences(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<PlatformPreferenceOwnerQuery>,
) -> AppResult<Json<PlatformPreferenceListResponse>> {
    ensure_human_owner(&auth_user)?;
    let actor = auth_user.user_id.to_string();
    let owner = query.owner_id.as_deref().unwrap_or(&actor);
    let preferences = platform_preference_service::list_preferences(&state.db, &actor, owner)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(PlatformPreferenceListResponse { preferences }))
}

/// PUT /api/v1/platform-ops/preferences/{catalog_service_id}
pub async fn update_preference(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(catalog_service_id): Path<String>,
    Query(query): Query<PlatformPreferenceOwnerQuery>,
    Json(body): Json<UpdatePlatformPreferenceRequest>,
) -> AppResult<Json<PlatformPreferenceResponse>> {
    ensure_human_owner(&auth_user)?;
    let actor = auth_user.user_id.to_string();
    let owner = query.owner_id.as_deref().unwrap_or(&actor);
    let override_count = body.operation_overrides.len();
    let preference = platform_preference_service::upsert_preference(
        &state.db,
        &actor,
        owner,
        &catalog_service_id,
        platform_preference_service::PreferenceWrite {
            platform_enabled: body.platform_enabled,
            max_credits_per_call: body.max_credits_per_call,
            max_credits_per_day: body.max_credits_per_day,
            operation_overrides: body.operation_overrides,
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "platform_service_preference_updated",
        Some(serde_json::json!({
            "owner_id": owner,
            "catalog_service_id": catalog_service_id,
            "platform_enabled": preference.platform_enabled,
            "operation_override_count": override_count,
        })),
    );
    Ok(Json(preference.into()))
}

/// DELETE /api/v1/platform-ops/preferences/{catalog_service_id}
pub async fn delete_preference(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(catalog_service_id): Path<String>,
    Query(query): Query<PlatformPreferenceOwnerQuery>,
) -> AppResult<StatusCode> {
    ensure_human_owner(&auth_user)?;
    let actor = auth_user.user_id.to_string();
    let owner = query.owner_id.as_deref().unwrap_or(&actor);
    platform_preference_service::delete_preference(&state.db, &actor, owner, &catalog_service_id)
        .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "platform_service_preference_deleted",
        Some(serde_json::json!({
            "owner_id": owner,
            "catalog_service_id": catalog_service_id,
        })),
    );
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_human_owner(auth_user: &AuthUser) -> AppResult<()> {
    if auth_user.auth_method != AuthMethod::Session {
        return Err(AppError::Forbidden(
            "Platform spending preferences require a human session".to_string(),
        ));
    }
    Ok(())
}

impl From<PlatformServicePreference> for PlatformPreferenceResponse {
    fn from(preference: PlatformServicePreference) -> Self {
        Self {
            id: preference.id,
            owner_id: preference.owner_id,
            catalog_service_id: preference.catalog_service_id,
            platform_enabled: preference.platform_enabled,
            max_credits_per_call: preference.max_credits_per_call,
            max_credits_per_day: preference.max_credits_per_day,
            operation_overrides: preference.operation_overrides,
            created_by: preference.created_by,
            updated_by: preference.updated_by,
            created_at: preference.created_at.to_rfc3339(),
            updated_at: preference.updated_at.to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_request_rejects_unknown_fields() {
        let parsed = serde_json::from_value::<UpdatePlatformPreferenceRequest>(serde_json::json!({
            "platform_enabled": true,
            "max_credits_per_call": "5",
            "max_credits_per_day": "50",
            "operation_overrides": [],
            "unexpected": true,
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn preference_management_requires_a_human_session() {
        let user_id = uuid::Uuid::new_v4().to_string();
        let mut auth_user = crate::test_utils::test_auth_user(&user_id);
        assert!(ensure_human_owner(&auth_user).is_ok());

        for method in [AuthMethod::ApiKey, AuthMethod::AccessToken] {
            auth_user.auth_method = method;
            assert!(matches!(
                ensure_human_owner(&auth_user),
                Err(AppError::Forbidden(_))
            ));
        }
    }
}
