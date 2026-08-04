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
use crate::models::feature_flag_metadata::FeatureFlagMetadata;
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

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminOrgFeatureFlagOverride {
    pub org_id: String,
    pub org_display_name: Option<String>,
    pub org_slug: Option<String>,
    pub enabled: bool,
    pub updated_at: String,
    pub updated_by: String,
}

impl AdminOrgFeatureFlagOverride {
    fn from_row(
        row: FeatureFlagOverride,
        orgs: &std::collections::HashMap<String, feature_flag_service::PlatformOverrideOrgDisplay>,
    ) -> AppResult<Self> {
        let org_id = row.org_user_id.ok_or_else(|| {
            AppError::Internal("org-scope override is missing its org id".to_string())
        })?;
        let org = orgs.get(&org_id);
        Ok(Self {
            org_display_name: org.and_then(|item| item.display_name.clone()),
            org_slug: org.and_then(|item| item.slug.clone()),
            org_id,
            enabled: row.enabled,
            updated_at: row.updated_at.to_rfc3339(),
            updated_by: row.updated_by,
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminFeatureFlagItem {
    pub key: String,
    /// The description to show: the admin-authored one when set, otherwise the
    /// code-declared fallback.
    pub description: String,
    /// The code-declared description, always present. Rendered as the
    /// placeholder / reset target in the admin editor.
    pub code_description: String,
    /// The admin-authored description, when one has been written.
    pub custom_description: Option<String>,
    /// Who owns this flag (person, team, or contact). Admin-authored.
    pub owner: Option<String>,
    pub metadata_updated_at: Option<String>,
    pub metadata_updated_by: Option<String>,
    pub default_enabled: bool,
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

/// A parsed admin-API target: a platform-level row (`org_user_id = null`) or
/// an org-wide row for one org (wire `target_key` = that org's user id).
#[derive(Debug)]
enum AdminTarget {
    Platform(FlagTarget),
    OrgWide { org_user_id: String },
}

fn parse_target(kind: AdminFlagTargetKind, key: Option<&str>) -> AppResult<AdminTarget> {
    match kind {
        AdminFlagTargetKind::Global if key.is_none() => {
            Ok(AdminTarget::Platform(FlagTarget::Global))
        }
        AdminFlagTargetKind::Global => Err(AppError::BadRequest(
            "global target requires target_key to be null".to_string(),
        )),
        AdminFlagTargetKind::User => Ok(AdminTarget::Platform(FlagTarget::from_parts(
            FlagTargetKind::User,
            key,
        )?)),
        AdminFlagTargetKind::Org => {
            let org_id = key
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "org target requires target_key to be the org's user id".to_string(),
                    )
                })?;
            Ok(AdminTarget::OrgWide {
                org_user_id: org_id.to_string(),
            })
        }
        AdminFlagTargetKind::Role => Err(AppError::BadRequest(
            "role targets are org-scoped; use the org feature-flag API".to_string(),
        )),
    }
}

/// GET /api/v1/admin/feature-flags
pub async fn list_feature_flags(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminFeatureFlagListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.feature_flags.list").await?;
    let overrides = feature_flag_service::list_platform_overrides(&state.db).await?;
    let org_rows = feature_flag_service::list_all_org_scope_overrides(&state.db).await?;
    let users = feature_flag_service::fetch_platform_override_users(&state.db, &overrides).await?;
    let orgs = feature_flag_service::fetch_override_org_display(&state.db, &org_rows).await?;
    let metadata = feature_flag_service::list_metadata(&state.db).await?;
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
        let meta = metadata.get(def.key);
        let custom_description = meta.and_then(|row| row.description.clone());
        flags.push(AdminFeatureFlagItem {
            key: def.key.to_string(),
            description: custom_description
                .clone()
                .unwrap_or_else(|| def.description.to_string()),
            code_description: def.description.to_string(),
            custom_description,
            owner: meta.and_then(|row| row.owner.clone()),
            metadata_updated_at: meta.map(|row| row.updated_at.to_rfc3339()),
            metadata_updated_by: meta.map(|row| row.updated_by.clone()),
            default_enabled: def.default_enabled,
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
    let row = match parse_target(body.target_kind, body.target_key.as_deref())? {
        AdminTarget::Platform(target) => {
            feature_flag_service::set_platform_override(
                &state.db,
                &flag_key,
                &target,
                body.enabled,
                &actor,
            )
            .await?
        }
        AdminTarget::OrgWide { org_user_id } => {
            feature_flag_service::set_platform_org_override(
                &state.db,
                &org_user_id,
                &flag_key,
                body.enabled,
                &actor,
            )
            .await?
        }
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
    pub enabled: bool,
    pub updated_at: String,
    pub updated_by: String,
}

impl From<FeatureFlagOverride> for AdminUserFeatureFlagOverrideOrGlobal {
    fn from(row: FeatureFlagOverride) -> Self {
        Self {
            target_kind: row.target_kind.as_str().to_string(),
            // Org-wide rows store the org in `org_user_id` with a null
            // `target_key`; echo the org id back so the response mirrors the
            // request's wire shape.
            target_key: row.target_key.or(row.org_user_id),
            enabled: row.enabled,
            updated_at: row.updated_at.to_rfc3339(),
            updated_by: row.updated_by,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateFeatureFlagMetadataRequest {
    /// Replaces the code-declared description. Blank or absent clears it.
    #[serde(default)]
    pub description: Option<String>,
    /// Who owns the flag. Blank or absent clears it.
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminFeatureFlagMetadataResponse {
    pub key: String,
    /// Effective description after the write: the admin text when set,
    /// otherwise the code-declared fallback.
    pub description: String,
    pub code_description: String,
    pub custom_description: Option<String>,
    pub owner: Option<String>,
    pub metadata_updated_at: Option<String>,
    pub metadata_updated_by: Option<String>,
}

impl AdminFeatureFlagMetadataResponse {
    fn build(def: &feature_flag_service::FeatureFlagDef, row: Option<FeatureFlagMetadata>) -> Self {
        let custom_description = row.as_ref().and_then(|meta| meta.description.clone());
        Self {
            key: def.key.to_string(),
            description: custom_description
                .clone()
                .unwrap_or_else(|| def.description.to_string()),
            code_description: def.description.to_string(),
            custom_description,
            owner: row.as_ref().and_then(|meta| meta.owner.clone()),
            metadata_updated_at: row.as_ref().map(|meta| meta.updated_at.to_rfc3339()),
            metadata_updated_by: row.map(|meta| meta.updated_by),
        }
    }
}

/// PUT /api/v1/admin/feature-flags/{flag_key}/metadata
///
/// Full replace: whichever of `description` / `owner` is blank or absent is
/// cleared, and clearing both drops the row so the flag falls back to its
/// code-declared description.
pub async fn update_feature_flag_metadata(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(flag_key): Path<String>,
    Json(body): Json<UpdateFeatureFlagMetadataRequest>,
) -> AppResult<Json<AdminFeatureFlagMetadataResponse>> {
    require_admin(&state, &auth_user).await?;
    let def = feature_flag_service::find_flag(&flag_key)
        .ok_or_else(|| AppError::BadRequest(format!("unknown feature flag '{flag_key}'")))?;
    let row = feature_flag_service::set_metadata(
        &state.db,
        &flag_key,
        body.description.as_deref(),
        body.owner.as_deref(),
        &auth_user.user_id.to_string(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_feature_flag_metadata_set",
        // Metadata is staff-authored documentation, not user data — the values
        // are safe to record so an owner change is reconstructible.
        Some(serde_json::json!({
            "flag_key": flag_key,
            "description": row.as_ref().and_then(|meta| meta.description.clone()),
            "owner": row.as_ref().and_then(|meta| meta.owner.clone()),
            "cleared": row.is_none(),
        })),
    );

    Ok(Json(AdminFeatureFlagMetadataResponse::build(def, row)))
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
    let removed = match parse_target(query.target_kind, query.target_key.as_deref())? {
        AdminTarget::Platform(target) => {
            feature_flag_service::clear_platform_override(&state.db, &flag_key, &target).await?
        }
        AdminTarget::OrgWide { org_user_id } => {
            feature_flag_service::clear_platform_org_override(&state.db, &org_user_id, &flag_key)
                .await?
        }
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
        assert!(parse_target(AdminFlagTargetKind::Global, Some("x")).is_err());
        assert!(parse_target(AdminFlagTargetKind::User, None).is_err());
        assert!(parse_target(AdminFlagTargetKind::Org, None).is_err());
        assert!(parse_target(AdminFlagTargetKind::Org, Some("  ")).is_err());
        assert!(parse_target(AdminFlagTargetKind::Role, Some("admin")).is_err());
        assert!(matches!(
            parse_target(AdminFlagTargetKind::Org, Some("org-1")),
            Ok(AdminTarget::OrgWide { org_user_id }) if org_user_id == "org-1"
        ));
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
    async fn org_override_round_trip_and_validation() {
        let Some((state, admin_id, _operator_id, target_id)) =
            setup("admin_feature_flags_org_flow").await
        else {
            eprintln!("skipping admin feature flag org test: no local MongoDB available");
            return;
        };
        let auth = test_auth_user(&admin_id);
        let flag_key = "example_ui".to_string();
        let org_id = Uuid::new_v4().to_string();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .expect("insert org user");

        // Set an org-wide override. The response echoes the org id.
        let Json(row) = set_feature_flag(
            State(state.clone()),
            auth.clone(),
            Path(flag_key.clone()),
            Json(SetAdminFeatureFlagRequest {
                target_kind: AdminFlagTargetKind::Org,
                target_key: Some(org_id.clone()),
                enabled: true,
            }),
        )
        .await
        .expect("set org override");
        assert_eq!(row.target_kind, "org");
        assert_eq!(row.target_key.as_deref(), Some(org_id.as_str()));
        assert!(row.enabled);

        // The admin list shows the enriched org override; the org's own
        // override set picks up the identical row.
        let Json(list) = list_feature_flags(State(state.clone()), auth.clone())
            .await
            .expect("list flags");
        let item = list.flags.iter().find(|flag| flag.key == flag_key).unwrap();
        assert_eq!(item.org_overrides.len(), 1);
        assert_eq!(item.org_overrides[0].org_id, org_id);
        assert_eq!(
            item.org_overrides[0].org_display_name.as_deref(),
            Some("Test Org")
        );
        let org_rows = crate::services::feature_flag_service::list_overrides(&state.db, &org_id)
            .await
            .expect("org rows");
        assert_eq!(org_rows.len(), 1);

        // Nonexistent org and person accounts are rejected as not-an-org.
        for bad_org in [Uuid::new_v4().to_string(), target_id] {
            let err = set_feature_flag(
                State(state.clone()),
                auth.clone(),
                Path(flag_key.clone()),
                Json(SetAdminFeatureFlagRequest {
                    target_kind: AdminFlagTargetKind::Org,
                    target_key: Some(bad_org),
                    enabled: true,
                }),
            )
            .await
            .expect_err("non-org target rejected");
            assert!(matches!(err, AppError::NotFound(_)));
        }

        // Clear removes the row from both views.
        clear_feature_flag(
            State(state.clone()),
            auth.clone(),
            Path(flag_key.clone()),
            Query(ClearAdminFeatureFlagQuery {
                target_kind: AdminFlagTargetKind::Org,
                target_key: Some(org_id.clone()),
            }),
        )
        .await
        .expect("clear org override");
        let Json(list) = list_feature_flags(State(state), auth)
            .await
            .expect("list after clear");
        let item = list.flags.iter().find(|flag| flag.key == flag_key).unwrap();
        assert!(item.org_overrides.is_empty());
    }

    #[tokio::test]
    async fn metadata_edit_round_trip_and_access_control() {
        let Some((state, admin_id, operator_id, _target_id)) =
            setup("admin_feature_flags_metadata").await
        else {
            eprintln!("skipping admin feature flag metadata test: no local MongoDB available");
            return;
        };
        let admin = test_auth_user(&admin_id);
        let operator = test_auth_user(&operator_id);
        let flag_key = "example_ui".to_string();

        // Before any edit, the list serves the code-declared description with
        // no owner.
        let Json(list) = list_feature_flags(State(state.clone()), admin.clone())
            .await
            .expect("list flags");
        let item = list.flags.iter().find(|flag| flag.key == flag_key).unwrap();
        assert_eq!(item.description, "Test-only placeholder flag.");
        assert_eq!(item.code_description, "Test-only placeholder flag.");
        assert!(item.custom_description.is_none());
        assert!(item.owner.is_none());
        assert!(item.metadata_updated_at.is_none());

        let Json(saved) = update_feature_flag_metadata(
            State(state.clone()),
            admin.clone(),
            Path(flag_key.clone()),
            Json(UpdateFeatureFlagMetadataRequest {
                description: Some("Gates the redesigned panel.".to_string()),
                owner: Some("Platform team".to_string()),
            }),
        )
        .await
        .expect("write metadata");
        assert_eq!(saved.description, "Gates the redesigned panel.");
        assert_eq!(saved.code_description, "Test-only placeholder flag.");
        assert_eq!(saved.owner.as_deref(), Some("Platform team"));
        assert_eq!(
            saved.metadata_updated_by.as_deref(),
            Some(admin_id.as_str())
        );

        // The list now serves the admin description while still exposing the
        // code fallback, so the editor can offer a reset.
        let Json(list) = list_feature_flags(State(state.clone()), operator.clone())
            .await
            .expect("operator list");
        let item = list.flags.iter().find(|flag| flag.key == flag_key).unwrap();
        assert_eq!(item.description, "Gates the redesigned panel.");
        assert_eq!(item.code_description, "Test-only placeholder flag.");
        assert_eq!(
            item.custom_description.as_deref(),
            Some("Gates the redesigned panel.")
        );
        assert_eq!(item.owner.as_deref(), Some("Platform team"));

        // Operators may read but not write.
        let forbidden = update_feature_flag_metadata(
            State(state.clone()),
            operator,
            Path(flag_key.clone()),
            Json(UpdateFeatureFlagMetadataRequest {
                description: Some("nope".to_string()),
                owner: None,
            }),
        )
        .await
        .expect_err("operator metadata write is forbidden");
        assert!(matches!(forbidden, AppError::Forbidden(_)));

        // Clearing both fields falls back to the code description.
        let Json(cleared) = update_feature_flag_metadata(
            State(state.clone()),
            admin.clone(),
            Path(flag_key.clone()),
            Json(UpdateFeatureFlagMetadataRequest {
                description: None,
                owner: Some("   ".to_string()),
            }),
        )
        .await
        .expect("clear metadata");
        assert_eq!(cleared.description, "Test-only placeholder flag.");
        assert!(cleared.custom_description.is_none());
        assert!(cleared.owner.is_none());
        assert!(cleared.metadata_updated_at.is_none());

        // Unknown keys are rejected rather than creating orphan rows.
        let unknown = update_feature_flag_metadata(
            State(state),
            admin,
            Path("does-not-exist".to_string()),
            Json(UpdateFeatureFlagMetadataRequest {
                description: Some("x".to_string()),
                owner: None,
            }),
        )
        .await
        .expect_err("unknown flag rejected");
        assert!(matches!(unknown, AppError::BadRequest(_)));
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
