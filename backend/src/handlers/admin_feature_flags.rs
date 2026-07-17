//! Platform-admin feature-flag management handlers.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::models::feature_flag_override::{FeatureFlagOverride, FlagTargetKind};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, feature_flag_service};
use feature_flag_service::FlagTarget;

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserFeatureFlagOverride {
    pub user_id: String,
    pub user_email: Option<String>,
    pub user_display_name: Option<String>,
    pub enabled: bool,
    pub updated_at: String,
    pub updated_by: String,
}

impl AdminUserFeatureFlagOverride {
    fn from_row(
        row: FeatureFlagOverride,
        users: &std::collections::HashMap<
            String,
            feature_flag_service::PlatformOverrideUserDisplay,
        >,
    ) -> AppResult<Self> {
        let user_id = row.target_key.ok_or_else(|| {
            AppError::Internal("platform user override is missing its target key".to_string())
        })?;
        let user = users.get(&user_id);
        Ok(Self {
            user_id,
            user_email: user.map(|item| item.email.clone()),
            user_display_name: user.and_then(|item| item.display_name.clone()),
            enabled: row.enabled,
            updated_at: row.updated_at.to_rfc3339(),
            updated_by: row.updated_by,
        })
    }
}

/// One org-wide override row in the platform-admin listing. Role/user rows
/// inside an org stay on the org's own feature-flag page.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminOrgFeatureFlagOverride {
    pub org_user_id: String,
    pub org_display_name: Option<String>,
    pub org_slug: Option<String>,
    pub enabled: bool,
    pub updated_at: String,
    pub updated_by: String,
}

impl AdminOrgFeatureFlagOverride {
    fn from_row(
        row: FeatureFlagOverride,
        orgs: &std::collections::HashMap<String, feature_flag_service::OverrideOrgDisplay>,
    ) -> AppResult<Self> {
        let org_user_id = row.org_user_id.ok_or_else(|| {
            AppError::Internal("org-wide override is missing its org id".to_string())
        })?;
        let org = orgs.get(&org_user_id);
        Ok(Self {
            org_user_id,
            org_display_name: org.and_then(|item| item.display_name.clone()),
            org_slug: org.and_then(|item| item.slug.clone()),
            enabled: row.enabled,
            updated_at: row.updated_at.to_rfc3339(),
            updated_by: row.updated_by,
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminFeatureFlagItem {
    pub key: String,
    pub description: String,
    pub default_enabled: bool,
    pub org_manageable: bool,
    pub global_override: Option<bool>,
    pub org_overrides: Vec<AdminOrgFeatureFlagOverride>,
    pub user_overrides: Vec<AdminUserFeatureFlagOverride>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminFeatureFlagListResponse {
    pub flags: Vec<AdminFeatureFlagItem>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdminFlagTargetKind {
    Global,
    Org,
    Role,
    User,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetAdminFeatureFlagRequest {
    pub target_kind: AdminFlagTargetKind,
    #[serde(default)]
    pub target_key: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClearAdminFeatureFlagQuery {
    pub target_kind: AdminFlagTargetKind,
    #[serde(default)]
    pub target_key: Option<String>,
}

/// Parse a **platform-scope** target (global / personal user). `Org` is
/// handled by the caller before this (it maps to an org-scoped row, not a
/// platform row); `Role` stays org self-serve only.
fn parse_platform_target(kind: AdminFlagTargetKind, key: Option<&str>) -> AppResult<FlagTarget> {
    match kind {
        AdminFlagTargetKind::Global if key.is_none() => Ok(FlagTarget::Global),
        AdminFlagTargetKind::Global => Err(AppError::BadRequest(
            "global target requires target_key to be null".to_string(),
        )),
        AdminFlagTargetKind::User => FlagTarget::from_parts(FlagTargetKind::User, key),
        AdminFlagTargetKind::Org => Err(AppError::BadRequest(
            "org target requires target_key to be the org id".to_string(),
        )),
        AdminFlagTargetKind::Role => Err(AppError::BadRequest(
            "role targets require an org; use the org feature-flag API".to_string(),
        )),
    }
}

/// The org id for an `Org` target, when present and non-empty.
fn org_target_key(kind: AdminFlagTargetKind, key: Option<&str>) -> Option<&str> {
    match kind {
        AdminFlagTargetKind::Org => key.map(str::trim).filter(|k| !k.is_empty()),
        _ => None,
    }
}

/// GET /api/v1/admin/feature-flags
pub async fn list_feature_flags(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminFeatureFlagListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.feature_flags.list").await?;
    let overrides = feature_flag_service::list_platform_overrides(&state.db).await?;
    let users = feature_flag_service::fetch_platform_override_users(&state.db, &overrides).await?;
    let org_rows = feature_flag_service::list_org_wide_overrides(&state.db).await?;
    let orgs = feature_flag_service::fetch_override_orgs(&state.db, &org_rows).await?;
    let mut flags = Vec::with_capacity(feature_flag_service::FEATURE_FLAGS.len());

    for def in feature_flag_service::FEATURE_FLAGS {
        let global_override = overrides
            .iter()
            .find(|row| row.flag_key == def.key && row.target_kind == FlagTargetKind::Global)
            .map(|row| row.enabled);
        let org_overrides = org_rows
            .iter()
            .filter(|row| row.flag_key == def.key)
            .cloned()
            .map(|row| AdminOrgFeatureFlagOverride::from_row(row, &orgs))
            .collect::<AppResult<Vec<_>>>()?;
        let user_overrides = overrides
            .iter()
            .filter(|row| row.flag_key == def.key && row.target_kind == FlagTargetKind::User)
            .cloned()
            .map(|row| AdminUserFeatureFlagOverride::from_row(row, &users))
            .collect::<AppResult<Vec<_>>>()?;
        flags.push(AdminFeatureFlagItem {
            key: def.key.to_string(),
            description: def.description.to_string(),
            default_enabled: def.default_enabled,
            org_manageable: def.org_manageable,
            global_override,
            org_overrides,
            user_overrides,
        });
    }

    Ok(Json(AdminFeatureFlagListResponse { flags }))
}

/// PUT /api/v1/admin/feature-flags/{flag_key}
pub async fn set_feature_flag(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(flag_key): Path<String>,
    Json(body): Json<SetAdminFeatureFlagRequest>,
) -> AppResult<Json<AdminUserFeatureFlagOverrideOrGlobal>> {
    require_admin(&state, &auth_user).await?;
    let actor = auth_user.user_id.to_string();

    let row = if let Some(org_id) = org_target_key(body.target_kind, body.target_key.as_deref()) {
        feature_flag_service::admin_set_org_override(
            &state.db,
            org_id,
            &flag_key,
            body.enabled,
            &actor,
        )
        .await?
    } else {
        let target = parse_platform_target(body.target_kind, body.target_key.as_deref())?;
        feature_flag_service::set_platform_override(
            &state.db,
            &flag_key,
            &target,
            body.enabled,
            &actor,
        )
        .await?
    };

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_feature_flag_override_set",
        Some(serde_json::json!({
            "flag_key": flag_key,
            "target_kind": row.target_kind.as_str(),
            "target_key": row.target_key,
            "org_user_id": row.org_user_id,
            "enabled": row.enabled,
        })),
    );

    Ok(Json(row.into()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserFeatureFlagOverrideOrGlobal {
    pub target_kind: String,
    pub target_key: Option<String>,
    /// Set when this row is an org-scoped override (org target).
    pub org_user_id: Option<String>,
    pub enabled: bool,
    pub updated_at: String,
    pub updated_by: String,
}

impl From<FeatureFlagOverride> for AdminUserFeatureFlagOverrideOrGlobal {
    fn from(row: FeatureFlagOverride) -> Self {
        Self {
            target_kind: row.target_kind.as_str().to_string(),
            target_key: row.target_key,
            org_user_id: row.org_user_id,
            enabled: row.enabled,
            updated_at: row.updated_at.to_rfc3339(),
            updated_by: row.updated_by,
        }
    }
}

/// DELETE /api/v1/admin/feature-flags/{flag_key}
pub async fn clear_feature_flag(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(flag_key): Path<String>,
    Query(query): Query<ClearAdminFeatureFlagQuery>,
) -> AppResult<impl IntoResponse> {
    require_admin(&state, &auth_user).await?;
    feature_flag_service::find_flag(&flag_key)
        .ok_or_else(|| AppError::BadRequest(format!("unknown feature flag '{flag_key}'")))?;
    let removed =
        if let Some(org_id) = org_target_key(query.target_kind, query.target_key.as_deref()) {
            feature_flag_service::admin_clear_org_override(&state.db, org_id, &flag_key).await?
        } else {
            let target = parse_platform_target(query.target_kind, query.target_key.as_deref())?;
            feature_flag_service::clear_platform_override(&state.db, &flag_key, &target).await?
        };

    if let Some(row) = removed {
        audit_service::log_for_user(
            state.db.clone(),
            &auth_user,
            "admin_feature_flag_override_cleared",
            Some(serde_json::json!({
                "flag_key": flag_key,
                "target_kind": query.target_kind,
                "target_key": query.target_key,
                "org_user_id": row.org_user_id,
                "enabled": row.enabled,
            })),
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{Path, Query, State};
    use uuid::Uuid;

    async fn setup(prefix: &str) -> Option<(AppState, String, String, String)> {
        let db = connect_test_database(prefix).await?;
        role_service::seed_system_roles(&db).await.ok()?;
        let role_ids = role_service::get_platform_role_ids(&db).await.ok()?;
        let admin_id = Uuid::new_v4().to_string();
        let operator_id = Uuid::new_v4().to_string();
        let target_id = Uuid::new_v4().to_string();
        let mut admin = test_user(&admin_id, UserType::Person);
        admin.role_ids.push(role_ids.admin);
        let mut operator = test_user(&operator_id, UserType::Person);
        operator.role_ids.push(role_ids.operator);
        db.collection(USERS)
            .insert_many([admin, operator, test_user(&target_id, UserType::Person)])
            .await
            .ok()?;
        Some((test_app_state(db), admin_id, operator_id, target_id))
    }

    #[test]
    fn target_validation_rejects_invalid_platform_scopes() {
        assert!(parse_platform_target(AdminFlagTargetKind::Global, Some("x")).is_err());
        assert!(parse_platform_target(AdminFlagTargetKind::User, None).is_err());
        // Org without an org id falls through to platform parsing and is
        // rejected there; with an id it routes to the org write path.
        assert!(org_target_key(AdminFlagTargetKind::Org, None).is_none());
        assert!(org_target_key(AdminFlagTargetKind::Org, Some("  ")).is_none());
        assert!(parse_platform_target(AdminFlagTargetKind::Org, None).is_err());
        assert_eq!(
            org_target_key(AdminFlagTargetKind::Org, Some("org-1")),
            Some("org-1")
        );
        assert!(org_target_key(AdminFlagTargetKind::Global, Some("org-1")).is_none());
        assert!(parse_platform_target(AdminFlagTargetKind::Role, Some("admin")).is_err());
    }

    #[tokio::test]
    async fn org_target_round_trip_and_acl() {
        use crate::models::org_membership::{
            COLLECTION_NAME as MEMBERSHIPS, MemberScopeSource, OrgMembership, OrgRole,
        };
        use crate::services::feature_flag_service;

        let Some((state, admin_id, operator_id, _target_id)) =
            setup("admin_feature_flags_org_target").await
        else {
            eprintln!("skipping admin feature flag handler test: no local MongoDB available");
            return;
        };
        let flag_key = "example_ui".to_string();
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let mut org_user = test_user(&org_id, UserType::Org);
        org_user.display_name = Some("ChronoAI".to_string());
        state
            .db
            .collection(USERS)
            .insert_one(org_user)
            .await
            .expect("insert org user");
        state
            .db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(OrgMembership {
                id: Uuid::new_v4().to_string(),
                org_user_id: org_id.clone(),
                member_user_id: member_id.clone(),
                role: OrgRole::Member,
                scope_source: MemberScopeSource::Inherit,
                allowed_service_ids: None,
                created_at: chrono::Utc::now(),
                revoked_at: None,
            })
            .await
            .expect("insert membership");

        // Operator may not write org targets.
        let operator_err = set_feature_flag(
            State(state.clone()),
            test_auth_user(&operator_id),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::Org,
                target_key: Some(org_id.clone()),
                enabled: true,
            }),
        )
        .await
        .expect_err("operator org PUT forbidden");
        assert!(matches!(operator_err, AppError::Forbidden(_)));

        // Admin enables org-wide; the response carries the org id.
        let admin = test_auth_user(&admin_id);
        let Json(row) = set_feature_flag(
            State(state.clone()),
            admin.clone(),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::Org,
                target_key: Some(org_id.clone()),
                enabled: true,
            }),
        )
        .await
        .expect("admin org PUT");
        assert_eq!(row.target_kind, "org");
        assert_eq!(row.org_user_id.as_deref(), Some(org_id.as_str()));
        assert!(row.enabled);

        // The member's personal resolution now carries the flag — the exact
        // prod regression ("enable for org → assistant shows up").
        assert!(
            feature_flag_service::resolve_personal_features(&state.db, &member_id)
                .await
                .expect("resolve member")
                .contains(&flag_key)
        );

        // Listing surfaces the org override with display data.
        let Json(list) = list_feature_flags(State(state.clone()), admin.clone())
            .await
            .expect("list flags");
        let item = list.flags.iter().find(|flag| flag.key == flag_key).unwrap();
        assert_eq!(item.org_overrides.len(), 1);
        assert_eq!(item.org_overrides[0].org_user_id, org_id);
        assert_eq!(
            item.org_overrides[0].org_display_name.as_deref(),
            Some("ChronoAI")
        );

        // Unknown org rejected.
        let missing_org = set_feature_flag(
            State(state.clone()),
            admin.clone(),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::Org,
                target_key: Some(Uuid::new_v4().to_string()),
                enabled: true,
            }),
        )
        .await
        .expect_err("unknown org rejected");
        assert!(matches!(missing_org, AppError::OrgNotFound(_)));

        // Clear removes the grant.
        clear_feature_flag(
            State(state.clone()),
            admin,
            Path(flag_key.clone()),
            Query(ClearAdminFeatureFlagQuery {
                target_kind: AdminFlagTargetKind::Org,
                target_key: Some(org_id),
            }),
        )
        .await
        .expect("clear org override");
        assert!(
            !feature_flag_service::resolve_personal_features(&state.db, &member_id)
                .await
                .expect("resolve after clear")
                .contains(&flag_key)
        );
    }

    #[tokio::test]
    async fn set_list_clear_round_trip_and_unknown_rejection() {
        let Some((state, admin_id, _operator_id, target_id)) =
            setup("admin_feature_flags_flow").await
        else {
            eprintln!("skipping admin feature flag handler test: no local MongoDB available");
            return;
        };
        let auth = test_auth_user(&admin_id);
        let flag_key = "example_ui".to_string();

        let _ = set_feature_flag(
            State(state.clone()),
            auth.clone(),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::User,
                target_key: Some(target_id.clone()),
                enabled: true,
            }),
        )
        .await
        .expect("set user override");

        let Json(list) = list_feature_flags(State(state.clone()), auth.clone())
            .await
            .expect("list flags");
        let item = list.flags.iter().find(|flag| flag.key == flag_key).unwrap();
        assert_eq!(item.user_overrides.len(), 1);
        assert_eq!(item.user_overrides[0].user_id, target_id);
        let expected_email = format!("{target_id}@example.com");
        assert_eq!(
            item.user_overrides[0].user_email.as_deref(),
            Some(expected_email.as_str())
        );
        assert_eq!(
            item.user_overrides[0].user_display_name.as_deref(),
            Some("Test User")
        );

        state
            .db
            .collection::<crate::models::user::User>(USERS)
            .delete_one(mongodb::bson::doc! { "_id": &target_id })
            .await
            .expect("delete target user");
        let Json(list) = list_feature_flags(State(state.clone()), auth.clone())
            .await
            .expect("list flags after target deletion");
        let item = list.flags.iter().find(|flag| flag.key == flag_key).unwrap();
        assert_eq!(item.user_overrides.len(), 1);
        assert_eq!(item.user_overrides[0].user_id, target_id);
        assert!(item.user_overrides[0].user_email.is_none());
        assert!(item.user_overrides[0].user_display_name.is_none());

        let missing_user_error = set_feature_flag(
            State(state.clone()),
            auth.clone(),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::User,
                target_key: Some(Uuid::new_v4().to_string()),
                enabled: true,
            }),
        )
        .await
        .expect_err("missing target user rejected");
        assert!(matches!(missing_user_error, AppError::NotFound(_)));

        clear_feature_flag(
            State(state.clone()),
            auth.clone(),
            Path(flag_key),
            Query(ClearAdminFeatureFlagQuery {
                target_kind: AdminFlagTargetKind::User,
                target_key: Some(target_id),
            }),
        )
        .await
        .expect("clear user override");

        let err = set_feature_flag(
            State(state),
            auth,
            Path("unknown".to_string()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::Global,
                target_key: None,
                enabled: true,
            }),
        )
        .await
        .expect_err("unknown flag rejected");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn admin_operator_read_write_split_is_enforced() {
        let Some((state, admin_id, operator_id, _target_id)) =
            setup("admin_feature_flags_access_split").await
        else {
            eprintln!("skipping admin feature flag access test: no local MongoDB available");
            return;
        };
        let flag_key = "example_ui".to_string();
        let operator = test_auth_user(&operator_id);
        let admin = test_auth_user(&admin_id);

        let _ = list_feature_flags(State(state.clone()), operator.clone())
            .await
            .expect("operator GET should succeed");

        let operator_put = set_feature_flag(
            State(state.clone()),
            operator.clone(),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::Global,
                target_key: None,
                enabled: true,
            }),
        )
        .await
        .expect_err("operator PUT should be forbidden");
        assert!(matches!(operator_put, AppError::Forbidden(_)));

        let operator_delete = clear_feature_flag(
            State(state.clone()),
            operator,
            Path(flag_key.clone()),
            Query(ClearAdminFeatureFlagQuery {
                target_kind: AdminFlagTargetKind::Global,
                target_key: None,
            }),
        )
        .await
        .err()
        .expect("operator DELETE should be forbidden");
        assert!(matches!(operator_delete, AppError::Forbidden(_)));

        let _ = list_feature_flags(State(state.clone()), admin.clone())
            .await
            .expect("admin GET should succeed");
        let _ = set_feature_flag(
            State(state.clone()),
            admin.clone(),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::Global,
                target_key: None,
                enabled: true,
            }),
        )
        .await
        .expect("admin PUT should succeed");
        clear_feature_flag(
            State(state),
            admin,
            Path(flag_key),
            Query(ClearAdminFeatureFlagQuery {
                target_kind: AdminFlagTargetKind::Global,
                target_key: None,
            }),
        )
        .await
        .expect("admin DELETE should succeed");
    }
}
