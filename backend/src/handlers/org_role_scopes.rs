//! HTTP handlers for org role-level service scopes.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::orgs::{OrgRoleWire, require_org_admin};
use crate::models::org_membership::OrgRole;
use crate::models::org_role_scope::OrgRoleScope;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, org_role_scope_service, user_service_service};

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleScopeResponse {
    pub role: OrgRoleWire,
    pub allowed_service_ids: Option<Vec<String>>,
    pub is_default: bool,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleScopeListResponse {
    pub role_scopes: Vec<RoleScopeResponse>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRoleScopeRequest {
    #[serde(default)]
    pub allowed_service_ids: Option<Vec<String>>,
}

fn parse_role(role: &str) -> AppResult<OrgRole> {
    match role {
        "admin" => Ok(OrgRole::Admin),
        "member" => Ok(OrgRole::Member),
        "viewer" => Ok(OrgRole::Viewer),
        _ => Err(AppError::BadRequest(format!(
            "invalid org role '{role}'; expected admin, member, or viewer"
        ))),
    }
}

fn stored_scope_to_response(scope: OrgRoleScope) -> RoleScopeResponse {
    RoleScopeResponse {
        role: scope.role.into(),
        allowed_service_ids: scope.allowed_service_ids,
        is_default: false,
        updated_at: Some(scope.updated_at.to_rfc3339()),
        updated_by: Some(scope.updated_by),
    }
}

/// GET /api/v1/orgs/{org_id}/role-scopes
pub async fn list_role_scopes(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
) -> AppResult<Json<RoleScopeListResponse>> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    let scopes = org_role_scope_service::list_scopes(&state.db, &org_id).await?;
    Ok(Json(RoleScopeListResponse {
        role_scopes: scopes
            .into_iter()
            .map(|scope| RoleScopeResponse {
                role: scope.role.into(),
                allowed_service_ids: scope.allowed_service_ids,
                is_default: scope.is_default,
                updated_at: scope.updated_at.map(|ts| ts.to_rfc3339()),
                updated_by: scope.updated_by,
            })
            .collect(),
    }))
}

/// PUT /api/v1/orgs/{org_id}/role-scopes/{role}
pub async fn set_role_scope(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((org_id, role)): Path<(String, String)>,
    Json(body): Json<UpdateRoleScopeRequest>,
) -> AppResult<Json<RoleScopeResponse>> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;
    let role = parse_role(&role)?;

    // Reject phantom service IDs up front. Without this, admins can store
    // UUIDs that never match anything at proxy time — safe, but confusing
    // and hard to diagnose later. The `remove_service_from_all_scopes`
    // cascade cleans up IDs when a service is deleted; this check keeps
    // the other side of the door closed.
    if let Some(ids) = body.allowed_service_ids.as_ref()
        && !ids.is_empty()
    {
        let existing = user_service_service::list_user_services(&state.db, &org_id).await?;
        let valid: std::collections::HashSet<&str> =
            existing.iter().map(|s| s.id.as_str()).collect();
        let unknown: Vec<&str> = ids
            .iter()
            .map(|s| s.as_str())
            .filter(|id| !valid.contains(id))
            .collect();
        if !unknown.is_empty() {
            return Err(AppError::BadRequest(format!(
                "unknown service id(s) for this org: {}",
                unknown.join(", ")
            )));
        }
    }

    let scope = org_role_scope_service::set_scope(
        &state.db,
        &org_id,
        role,
        body.allowed_service_ids,
        &actor,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "org_role_scope_set",
        Some(serde_json::json!({
            "org_user_id": org_id,
            "role": role,
            "allowed_service_ids": scope.allowed_service_ids.clone(),
        })),
    );

    Ok(Json(stored_scope_to_response(scope)))
}

/// DELETE /api/v1/orgs/{org_id}/role-scopes/{role}
pub async fn clear_role_scope(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((org_id, role)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;
    let parsed_role = parse_role(&role)?;

    org_role_scope_service::clear_scope(&state.db, &org_id, parsed_role).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "org_role_scope_cleared",
        Some(serde_json::json!({
            "org_user_id": org_id,
            "role": parsed_role,
        })),
    );

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, MemberScopeSource};
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
    };
    use axum::extract::{Path, State};
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

        let mut admin_membership = test_membership(&org_id, &admin_id, OrgRole::Admin, None);
        admin_membership.scope_source = MemberScopeSource::Inherit;
        let member_membership = test_membership(&org_id, &member_id, OrgRole::Member, None);
        db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([admin_membership, member_membership])
            .await
            .unwrap();

        Some((db, org_id, admin_id, member_id))
    }

    #[tokio::test]
    async fn role_scope_endpoints_create_update_list_and_clear() {
        use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
        use crate::test_utils::test_user_service;

        let Some((db, org_id, admin_id, _member_id)) =
            setup_org_admin("org_role_scope_handler_flow").await
        else {
            eprintln!("skipping org role scope handler test: no local MongoDB available");
            return;
        };
        let scoped_svc_id = Uuid::new_v4().to_string();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &scoped_svc_id,
                &org_id,
                "scoped-svc",
                &Uuid::new_v4().to_string(),
                None,
                None,
            ))
            .await
            .unwrap();
        let state = test_app_state(db);

        let Json(listed) = list_role_scopes(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(org_id.clone()),
        )
        .await
        .expect("list defaults");
        assert_eq!(listed.role_scopes.len(), 3);
        assert!(listed.role_scopes.iter().all(|scope| scope.is_default));

        let Json(updated) = set_role_scope(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((org_id.clone(), "member".to_string())),
            Json(UpdateRoleScopeRequest {
                allowed_service_ids: Some(vec![scoped_svc_id.clone()]),
            }),
        )
        .await
        .expect("set member role scope");
        assert_eq!(updated.role, OrgRoleWire::Member);
        assert_eq!(updated.allowed_service_ids, Some(vec![scoped_svc_id]));
        assert!(!updated.is_default);

        let Json(listed) = list_role_scopes(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(org_id.clone()),
        )
        .await
        .expect("list updated");
        let member = listed
            .role_scopes
            .iter()
            .find(|scope| scope.role == OrgRoleWire::Member)
            .unwrap();
        assert!(!member.is_default);

        clear_role_scope(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((org_id.clone(), "member".to_string())),
        )
        .await
        .expect("clear member role scope");

        let Json(listed) = list_role_scopes(State(state), test_auth_user(&admin_id), Path(org_id))
            .await
            .expect("list after clear");
        let member = listed
            .role_scopes
            .iter()
            .find(|scope| scope.role == OrgRoleWire::Member)
            .unwrap();
        assert!(member.is_default);
        assert!(member.allowed_service_ids.is_none());
    }

    #[tokio::test]
    async fn role_scope_endpoints_require_admin() {
        let Some((db, org_id, _admin_id, member_id)) =
            setup_org_admin("org_role_scope_handler_auth").await
        else {
            eprintln!("skipping org role scope handler test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db);

        let err = list_role_scopes(State(state), test_auth_user(&member_id), Path(org_id))
            .await
            .expect_err("non-admin should not list role scopes");
        assert!(matches!(err, AppError::OrgRoleInsufficient(_)));
    }

    #[tokio::test]
    async fn role_scope_endpoint_rejects_unknown_role() {
        let Some((db, org_id, admin_id, _member_id)) =
            setup_org_admin("org_role_scope_handler_role").await
        else {
            eprintln!("skipping org role scope handler test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db);

        let err = set_role_scope(
            State(state),
            test_auth_user(&admin_id),
            Path((org_id, "owner".to_string())),
            Json(UpdateRoleScopeRequest {
                allowed_service_ids: None,
            }),
        )
        .await
        .expect_err("unknown role should fail");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn role_scope_endpoint_rejects_phantom_service_ids() {
        use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
        use crate::test_utils::test_user_service;

        let Some((db, org_id, admin_id, _member_id)) =
            setup_org_admin("org_role_scope_handler_phantom").await
        else {
            eprintln!("skipping org role scope handler test: no local MongoDB available");
            return;
        };

        // Seed one real org-owned service so the valid id is accepted.
        let real_svc_id = Uuid::new_v4().to_string();
        let endpoint_id = Uuid::new_v4().to_string();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &real_svc_id,
                &org_id,
                "real-svc",
                &endpoint_id,
                None,
                None,
            ))
            .await
            .unwrap();

        let state = test_app_state(db);

        // Pure phantom list: should 400.
        let phantom_id = Uuid::new_v4().to_string();
        let err = set_role_scope(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((org_id.clone(), "member".to_string())),
            Json(UpdateRoleScopeRequest {
                allowed_service_ids: Some(vec![phantom_id.clone()]),
            }),
        )
        .await
        .expect_err("phantom id should be rejected");
        match err {
            AppError::BadRequest(msg) => {
                assert!(
                    msg.contains(&phantom_id),
                    "message should name the id: {msg}"
                );
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }

        // Mixed list: also 400.
        let err = set_role_scope(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((org_id.clone(), "member".to_string())),
            Json(UpdateRoleScopeRequest {
                allowed_service_ids: Some(vec![real_svc_id.clone(), phantom_id]),
            }),
        )
        .await
        .expect_err("mixed real+phantom should reject on the phantom");
        assert!(matches!(err, AppError::BadRequest(_)));

        // Valid list: accepted.
        let Json(resp) = set_role_scope(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((org_id.clone(), "member".to_string())),
            Json(UpdateRoleScopeRequest {
                allowed_service_ids: Some(vec![real_svc_id.clone()]),
            }),
        )
        .await
        .expect("valid id should succeed");
        assert_eq!(resp.allowed_service_ids, Some(vec![real_svc_id]));

        // Empty list is a lockout, not a phantom — still accepted.
        let Json(resp) = set_role_scope(
            State(state),
            test_auth_user(&admin_id),
            Path((org_id, "member".to_string())),
            Json(UpdateRoleScopeRequest {
                allowed_service_ids: Some(vec![]),
            }),
        )
        .await
        .expect("empty list is a legitimate lockout");
        assert_eq!(resp.allowed_service_ids, Some(vec![]));
    }
}
