//! HTTP handlers for per-org feature flags.
//!
//! Org admins toggle code-declared flags for their org, scoped to the whole
//! org, a role, or a single member. Reads/writes are admin-gated; resolution
//! for regular members rides on `OrgResponse.enabled_features` (see
//! `handlers::orgs`). Flag definitions live in `feature_flag_service`.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::orgs::require_org_admin;
use crate::models::feature_flag_override::{FeatureFlagOverride, FlagTargetKind};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, feature_flag_service};
use feature_flag_service::FlagTarget;

// ─────────────────────────────────────────────────────────────────────────────
// Wire types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureFlagOverrideWire {
    /// "org" | "role" | "user"
    pub target_kind: String,
    /// null for org scope; role string for role scope; member_user_id for user scope.
    pub target_value: Option<String>,
    pub enabled: bool,
    pub updated_at: String,
    pub updated_by: String,
}

impl From<FeatureFlagOverride> for FeatureFlagOverrideWire {
    fn from(o: FeatureFlagOverride) -> Self {
        Self {
            target_kind: o.target_kind.as_str().to_string(),
            target_value: o.target_key,
            enabled: o.enabled,
            updated_at: o.updated_at.to_rfc3339(),
            updated_by: o.updated_by,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureFlagWire {
    pub key: String,
    pub description: String,
    pub default_enabled: bool,
    /// Whether org admins may toggle this flag (staff-only flags are `false`).
    pub org_manageable: bool,
    /// Current overrides for this flag in this org, most-specific scopes mixed in.
    pub overrides: Vec<FeatureFlagOverrideWire>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureFlagListResponse {
    pub flags: Vec<FeatureFlagWire>,
}

/// Wire mirror of [`FlagTargetKind`] so request/query structs can derive
/// `ToSchema` without pulling utoipa into the model layer (mirrors the
/// `OrgRoleWire` pattern in `handlers::orgs`).
#[derive(Debug, Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlagTargetKindWire {
    Org,
    Role,
    User,
}

impl From<FlagTargetKindWire> for FlagTargetKind {
    fn from(kind: FlagTargetKindWire) -> Self {
        match kind {
            FlagTargetKindWire::Org => Self::Org,
            FlagTargetKindWire::Role => Self::Role,
            FlagTargetKindWire::User => Self::User,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetFeatureFlagRequest {
    pub target_kind: FlagTargetKindWire,
    /// Required for role scope (role string) and user scope (member_user_id);
    /// omit for org scope.
    #[serde(default)]
    pub target_value: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClearFeatureFlagQuery {
    pub target_kind: FlagTargetKindWire,
    #[serde(default)]
    pub target_value: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/orgs/{org_id}/feature-flags
///
/// Admin view: every code-declared flag plus this org's overrides.
pub async fn list_feature_flags(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
) -> AppResult<Json<FeatureFlagListResponse>> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    let items = feature_flag_service::list_for_admin(&state.db, &org_id).await?;
    Ok(Json(FeatureFlagListResponse {
        flags: items
            .into_iter()
            .map(|item| FeatureFlagWire {
                key: item.def.key.to_string(),
                description: item.def.description.to_string(),
                default_enabled: item.def.default_enabled,
                org_manageable: item.def.org_manageable,
                overrides: item.overrides.into_iter().map(Into::into).collect(),
            })
            .collect(),
    }))
}

/// PUT /api/v1/orgs/{org_id}/feature-flags/{flag_key}
///
/// Upsert one override (org / role / user scope). Unknown flag keys → 400;
/// non-org-manageable flags → 403.
pub async fn set_feature_flag(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((org_id, flag_key)): Path<(String, String)>,
    Json(body): Json<SetFeatureFlagRequest>,
) -> AppResult<Json<FeatureFlagOverrideWire>> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    let target = FlagTarget::from_parts(body.target_kind.into(), body.target_value.as_deref())?;
    let row = feature_flag_service::set_override(
        &state.db,
        &org_id,
        &flag_key,
        &target,
        body.enabled,
        &actor,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "org_feature_flag_set",
        Some(serde_json::json!({
            "org_user_id": org_id,
            "flag_key": flag_key,
            "target_kind": row.target_kind.as_str(),
            "target_value": row.target_key,
            "enabled": row.enabled,
        })),
    );

    Ok(Json(row.into()))
}

/// DELETE /api/v1/orgs/{org_id}/feature-flags/{flag_key}
///
/// Clear one override, reverting that scope to the next-broader resolution
/// layer. Target is passed via query params so the request carries no body.
pub async fn clear_feature_flag(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((org_id, flag_key)): Path<(String, String)>,
    Query(q): Query<ClearFeatureFlagQuery>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    let target = FlagTarget::from_parts(q.target_kind.into(), q.target_value.as_deref())?;
    let removed =
        feature_flag_service::clear_override(&state.db, &org_id, &flag_key, &target).await?;

    if removed {
        audit_service::log_for_user(
            state.db.clone(),
            &auth_user,
            "org_feature_flag_cleared",
            Some(serde_json::json!({
                "org_user_id": org_id,
                "flag_key": flag_key,
                "target_kind": FlagTargetKind::from(q.target_kind).as_str(),
                "target_value": q.target_value,
            })),
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::AppError;
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
    };
    use axum::extract::{Path, Query, State};
    use uuid::Uuid;

    async fn setup_org_admin(prefix: &str) -> Option<(mongodb::Database, String, String, String)> {
        let db = connect_test_database(prefix).await?;
        let org_id = Uuid::new_v4().to_string();
        let admin_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(&org_id, UserType::Org),
                test_user(&admin_id, UserType::Person),
                test_user(&member_id, UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_id, &member_id, OrgRole::Member, None),
            ])
            .await
            .unwrap();
        Some((db, org_id, admin_id, member_id))
    }

    #[tokio::test]
    async fn set_list_and_clear_flow() {
        let Some((db, org_id, admin_id, _member_id)) =
            setup_org_admin("org_feature_flag_handler_flow").await
        else {
            eprintln!("skipping feature flag handler test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db);

        // Defaults: example_ui present but no overrides, default off.
        let Json(listed) = list_feature_flags(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(org_id.clone()),
        )
        .await
        .expect("list defaults");
        let flag = listed.flags.iter().find(|f| f.key == "example_ui").unwrap();
        assert!(!flag.default_enabled);
        assert!(flag.overrides.is_empty());

        // Enable org-wide.
        let Json(row) = set_feature_flag(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((org_id.clone(), "example_ui".to_string())),
            Json(SetFeatureFlagRequest {
                target_kind: FlagTargetKindWire::Org,
                target_value: None,
                enabled: true,
            }),
        )
        .await
        .expect("set org override");
        assert_eq!(row.target_kind, "org");
        assert!(row.enabled);

        let Json(listed) = list_feature_flags(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(org_id.clone()),
        )
        .await
        .expect("list after set");
        let flag = listed.flags.iter().find(|f| f.key == "example_ui").unwrap();
        assert_eq!(flag.overrides.len(), 1);

        // Clear it.
        clear_feature_flag(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((org_id.clone(), "example_ui".to_string())),
            Query(ClearFeatureFlagQuery {
                target_kind: FlagTargetKindWire::Org,
                target_value: None,
            }),
        )
        .await
        .expect("clear override");

        let Json(listed) =
            list_feature_flags(State(state), test_auth_user(&admin_id), Path(org_id))
                .await
                .expect("list after clear");
        let flag = listed.flags.iter().find(|f| f.key == "example_ui").unwrap();
        assert!(flag.overrides.is_empty());
    }

    #[tokio::test]
    async fn non_admin_is_rejected() {
        let Some((db, org_id, _admin_id, member_id)) =
            setup_org_admin("org_feature_flag_handler_auth").await
        else {
            eprintln!("skipping feature flag handler test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db);

        let err = list_feature_flags(State(state), test_auth_user(&member_id), Path(org_id))
            .await
            .expect_err("non-admin should not list flags");
        assert!(matches!(err, AppError::OrgRoleInsufficient(_)));
    }

    #[tokio::test]
    async fn unknown_flag_key_is_rejected() {
        let Some((db, org_id, admin_id, _member_id)) =
            setup_org_admin("org_feature_flag_handler_unknown").await
        else {
            eprintln!("skipping feature flag handler test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db);

        let err = set_feature_flag(
            State(state),
            test_auth_user(&admin_id),
            Path((org_id, "does-not-exist".to_string())),
            Json(SetFeatureFlagRequest {
                target_kind: FlagTargetKindWire::Org,
                target_value: None,
                enabled: true,
            }),
        )
        .await
        .expect_err("unknown flag should fail");
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
