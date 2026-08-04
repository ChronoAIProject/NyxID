use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::models::role::Role;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, role_service};

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_default: Option<bool>,
    pub client_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub permissions: Option<Vec<String>>,
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RoleListQuery {
    pub client_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RoleResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_default: bool,
    pub is_system: bool,
    pub client_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct RoleListResponse {
    pub roles: Vec<RoleResponse>,
}

#[derive(Debug, Serialize)]
pub struct UserRolesResponse {
    pub direct_roles: Vec<RoleResponse>,
    pub inherited_roles: Vec<RoleResponse>,
    pub effective_permissions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RoleAssignmentResponse {
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct BulkAssignRequest {
    /// If `true`, assign the role to all users. Mutually exclusive with `user_ids`.
    #[serde(default)]
    pub all: bool,
    /// Specific user IDs to assign the role to. Mutually exclusive with `all`.
    #[serde(default)]
    pub user_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BulkAssignResponse {
    pub assigned_count: u64,
    pub already_assigned_count: u64,
    pub message: String,
}

// --- Helpers ---

fn role_to_response(r: Role) -> RoleResponse {
    RoleResponse {
        id: r.id,
        name: r.name,
        slug: r.slug,
        description: r.description,
        permissions: r.permissions,
        is_default: r.is_default,
        is_system: r.is_system,
        client_id: r.client_id,
        created_at: r.created_at.to_rfc3339(),
        updated_at: r.updated_at.to_rfc3339(),
    }
}

// --- Handlers ---

/// GET /api/v1/admin/roles
pub async fn list_roles(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<RoleListQuery>,
) -> AppResult<Json<RoleListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.roles.list").await?;

    let roles = role_service::list_roles(&state.db, query.client_id.as_deref()).await?;
    let items: Vec<RoleResponse> = roles.into_iter().map(role_to_response).collect();

    Ok(Json(RoleListResponse { roles: items }))
}

/// POST /api/v1/admin/roles
pub async fn create_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Json(body): Json<CreateRoleRequest>,
) -> AppResult<Json<RoleResponse>> {
    require_admin(&state, &auth_user).await?;

    if body.name.is_empty() || body.slug.is_empty() {
        return Err(crate::errors::AppError::ValidationError(
            "Name and slug are required".to_string(),
        ));
    }

    let role = role_service::create_role(
        &state.db,
        &body.name,
        &body.slug,
        body.description.as_deref(),
        &body.permissions,
        body.is_default.unwrap_or(false),
        body.client_id.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.role.created",
        Some(serde_json::json!({
            "role_id": &role.id,
            "role_slug": &role.slug,
        })),
    );

    Ok(Json(role_to_response(role)))
}

/// GET /api/v1/admin/roles/:role_id
pub async fn get_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(role_id): Path<String>,
) -> AppResult<Json<RoleResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.roles.get").await?;

    let role = role_service::get_role(&state.db, &role_id).await?;
    Ok(Json(role_to_response(role)))
}

/// PUT /api/v1/admin/roles/:role_id
pub async fn update_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(role_id): Path<String>,
    Json(body): Json<UpdateRoleRequest>,
) -> AppResult<Json<RoleResponse>> {
    require_admin(&state, &auth_user).await?;

    let role = role_service::update_role(
        &state.db,
        &role_id,
        body.name.as_deref(),
        body.slug.as_deref(),
        body.description.as_deref(),
        body.permissions.as_deref(),
        body.is_default,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.role.updated",
        Some(serde_json::json!({
            "role_id": &role_id,
            "role_slug": &role.slug,
        })),
    );

    Ok(Json(role_to_response(role)))
}

/// DELETE /api/v1/admin/roles/:role_id
pub async fn delete_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(role_id): Path<String>,
) -> AppResult<Json<RoleAssignmentResponse>> {
    require_admin(&state, &auth_user).await?;

    role_service::delete_role(&state.db, &role_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.role.deleted",
        Some(serde_json::json!({ "role_id": &role_id })),
    );

    Ok(Json(RoleAssignmentResponse {
        message: "Role deleted".to_string(),
    }))
}

/// GET /api/v1/admin/users/:user_id/roles
pub async fn get_user_roles(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(user_id): Path<String>,
) -> AppResult<Json<UserRolesResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.users.roles.list").await?;

    let rbac = crate::services::rbac_helpers::resolve_user_rbac(&state.db, &user_id).await?;
    let direct_roles = role_service::get_user_roles(&state.db, &user_id).await?;

    // Inherited roles = effective roles minus direct roles.
    // Use a targeted $in query on inherited slugs instead of fetching all roles.
    let direct_slugs: std::collections::HashSet<&str> =
        direct_roles.iter().map(|r| r.slug.as_str()).collect();
    let inherited_slugs: Vec<&str> = rbac
        .role_slugs
        .iter()
        .filter(|s| !direct_slugs.contains(s.as_str()))
        .map(|s| s.as_str())
        .collect();

    let inherited_roles = if inherited_slugs.is_empty() {
        vec![]
    } else {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        state
            .db
            .collection::<Role>(crate::models::role::COLLECTION_NAME)
            .find(doc! { "slug": { "$in": &inherited_slugs } })
            .await?
            .try_collect()
            .await?
    };

    Ok(Json(UserRolesResponse {
        direct_roles: direct_roles.into_iter().map(role_to_response).collect(),
        inherited_roles: inherited_roles.into_iter().map(role_to_response).collect(),
        effective_permissions: rbac.permissions,
    }))
}

/// POST /api/v1/admin/users/:user_id/roles/:role_id
pub async fn assign_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((user_id, role_id)): Path<(String, String)>,
) -> AppResult<Json<RoleAssignmentResponse>> {
    require_admin(&state, &auth_user).await?;

    role_service::assign_role_to_user(&state.db, &user_id, &role_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.role.assigned",
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "role_id": &role_id,
        })),
    );

    Ok(Json(RoleAssignmentResponse {
        message: "Role assigned".to_string(),
    }))
}

/// DELETE /api/v1/admin/users/:user_id/roles/:role_id
pub async fn revoke_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((user_id, role_id)): Path<(String, String)>,
) -> AppResult<Json<RoleAssignmentResponse>> {
    require_admin(&state, &auth_user).await?;

    role_service::revoke_role_from_user(&state.db, &user_id, &role_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.role.revoked",
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "role_id": &role_id,
        })),
    );

    Ok(Json(RoleAssignmentResponse {
        message: "Role revoked".to_string(),
    }))
}

/// POST /api/v1/admin/roles/:role_id/assign-bulk
pub async fn bulk_assign_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(role_id): Path<String>,
    Json(body): Json<BulkAssignRequest>,
) -> AppResult<Json<BulkAssignResponse>> {
    require_admin(&state, &auth_user).await?;

    if !body.all && body.user_ids.is_empty() {
        return Err(crate::errors::AppError::ValidationError(
            "Either set 'all' to true or provide 'user_ids'".to_string(),
        ));
    }
    if body.all && !body.user_ids.is_empty() {
        return Err(crate::errors::AppError::ValidationError(
            "'all' and 'user_ids' are mutually exclusive".to_string(),
        ));
    }

    let user_ids_opt = if body.all {
        None
    } else {
        Some(&body.user_ids[..])
    };

    let result = role_service::bulk_assign_role(&state.db, &role_id, user_ids_opt).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.role.bulk_assigned",
        Some(serde_json::json!({
            "role_id": &role_id,
            "all": body.all,
            "user_ids_count": body.user_ids.len(),
            "assigned_count": result.assigned_count,
            "already_assigned_count": result.already_assigned_count,
        })),
    );

    Ok(Json(BulkAssignResponse {
        assigned_count: result.assigned_count,
        already_assigned_count: result.already_assigned_count,
        message: format!(
            "Role assigned to {} users ({} already had it)",
            result.assigned_count, result.already_assigned_count
        ),
    }))
}

#[cfg(test)]
mod pure_tests {
    use super::*;
    use crate::models::role::Role;
    use chrono::Utc;

    fn make_role(id: &str, slug: &str) -> Role {
        let now = Utc::now();
        Role {
            id: id.to_string(),
            name: "Test Role".to_string(),
            slug: slug.to_string(),
            description: Some("A test role".to_string()),
            permissions: vec!["read".to_string(), "write".to_string()],
            is_default: false,
            is_system: false,
            client_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    // --- role_to_response tests ---

    #[test]
    fn role_to_response_maps_all_fields() {
        let role = make_role("role-1", "test-role");
        let resp = role_to_response(role);

        assert_eq!(resp.id, "role-1");
        assert_eq!(resp.name, "Test Role");
        assert_eq!(resp.slug, "test-role");
        assert_eq!(resp.description, Some("A test role".to_string()));
        assert_eq!(resp.permissions, vec!["read", "write"]);
        assert!(!resp.is_default);
        assert!(!resp.is_system);
        assert!(resp.client_id.is_none());
    }

    #[test]
    fn role_to_response_with_system_default_role() {
        let now = Utc::now();
        let role = Role {
            id: "sys-1".to_string(),
            name: "Admin".to_string(),
            slug: "admin".to_string(),
            description: None,
            permissions: vec!["*".to_string()],
            is_default: true,
            is_system: true,
            client_id: Some("client-1".to_string()),
            created_at: now,
            updated_at: now,
        };
        let resp = role_to_response(role);

        assert!(resp.is_default);
        assert!(resp.is_system);
        assert_eq!(resp.client_id, Some("client-1".to_string()));
        assert!(resp.description.is_none());
    }

    #[test]
    fn role_to_response_timestamps_are_rfc3339() {
        let role = make_role("role-2", "timestamped");
        let resp = role_to_response(role);

        chrono::DateTime::parse_from_rfc3339(&resp.created_at)
            .expect("created_at should be valid RFC 3339");
        chrono::DateTime::parse_from_rfc3339(&resp.updated_at)
            .expect("updated_at should be valid RFC 3339");
    }

    #[test]
    fn role_to_response_empty_permissions() {
        let now = Utc::now();
        let role = Role {
            id: "role-3".to_string(),
            name: "Empty".to_string(),
            slug: "empty".to_string(),
            description: None,
            permissions: vec![],
            is_default: false,
            is_system: false,
            client_id: None,
            created_at: now,
            updated_at: now,
        };
        let resp = role_to_response(role);

        assert!(resp.permissions.is_empty());
    }

    // --- Serde tests ---

    #[test]
    fn create_role_request_deserializes() {
        let json = r#"{
            "name": "Editor",
            "slug": "editor",
            "description": "Can edit",
            "permissions": ["edit", "view"],
            "is_default": true,
            "client_id": "client-abc"
        }"#;
        let req: CreateRoleRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.name, "Editor");
        assert_eq!(req.slug, "editor");
        assert_eq!(req.description, Some("Can edit".to_string()));
        assert_eq!(req.permissions, vec!["edit", "view"]);
        assert_eq!(req.is_default, Some(true));
        assert_eq!(req.client_id, Some("client-abc".to_string()));
    }

    #[test]
    fn create_role_request_minimal() {
        let json = r#"{"name": "Viewer", "slug": "viewer", "permissions": []}"#;
        let req: CreateRoleRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.name, "Viewer");
        assert!(req.description.is_none());
        assert!(req.is_default.is_none());
        assert!(req.client_id.is_none());
    }

    #[test]
    fn update_role_request_all_none() {
        let json = r#"{}"#;
        let req: UpdateRoleRequest = serde_json::from_str(json).expect("deserialize");
        assert!(req.name.is_none());
        assert!(req.slug.is_none());
        assert!(req.description.is_none());
        assert!(req.permissions.is_none());
        assert!(req.is_default.is_none());
    }

    #[test]
    fn role_response_serializes_all_fields() {
        let resp = RoleResponse {
            id: "r-1".to_string(),
            name: "Admin".to_string(),
            slug: "admin".to_string(),
            description: Some("Full access".to_string()),
            permissions: vec!["*".to_string()],
            is_default: false,
            is_system: true,
            client_id: None,
            created_at: "2024-01-01T00:00:00+00:00".to_string(),
            updated_at: "2024-01-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["id"], "r-1");
        assert_eq!(json["slug"], "admin");
        assert!(json["is_system"].as_bool().unwrap());
        assert!(json["client_id"].is_null());
    }

    #[test]
    fn bulk_assign_request_defaults() {
        let json = r#"{}"#;
        let req: BulkAssignRequest = serde_json::from_str(json).expect("deserialize");
        assert!(!req.all);
        assert!(req.user_ids.is_empty());
    }

    #[test]
    fn bulk_assign_request_with_all() {
        let json = r#"{"all": true}"#;
        let req: BulkAssignRequest = serde_json::from_str(json).expect("deserialize");
        assert!(req.all);
        assert!(req.user_ids.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{Path, Query, State};
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

    #[tokio::test]
    async fn test_list_roles_empty() {
        let Some(db) = connect_test_database("h_admin_roles_list").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let result = list_roles(State(state), auth, Query(RoleListQuery { client_id: None }))
            .await
            .expect("list_roles should succeed");

        assert!(result.0.roles.len() >= 3);
    }

    #[tokio::test]
    async fn test_create_role() {
        let Some(db) = connect_test_database("h_admin_roles_create").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let result = create_role(
            State(state),
            auth,
            HeaderMap::new(),
            Json(CreateRoleRequest {
                name: "Tester".to_string(),
                slug: "tester".to_string(),
                description: Some("A test role".to_string()),
                permissions: vec!["read".to_string()],
                is_default: Some(false),
                client_id: None,
            }),
        )
        .await
        .expect("create_role should succeed");

        assert_eq!(result.0.name, "Tester");
        assert_eq!(result.0.slug, "tester");
        assert_eq!(result.0.permissions, vec!["read"]);
        assert!(!result.0.is_system);
    }

    #[tokio::test]
    async fn test_get_role() {
        let Some(db) = connect_test_database("h_admin_roles_get").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_role(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateRoleRequest {
                name: "Viewer".to_string(),
                slug: "viewer".to_string(),
                description: None,
                permissions: vec![],
                is_default: None,
                client_id: None,
            }),
        )
        .await
        .expect("create_role should succeed");

        let role_id = created.0.id.clone();

        let result = get_role(
            State(state),
            test_auth_user(&admin_id),
            Path(role_id.clone()),
        )
        .await
        .expect("get_role should succeed");

        assert_eq!(result.0.id, role_id);
        assert_eq!(result.0.name, "Viewer");
    }

    #[tokio::test]
    async fn test_update_role() {
        let Some(db) = connect_test_database("h_admin_roles_update").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_role(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateRoleRequest {
                name: "Editor".to_string(),
                slug: "editor".to_string(),
                description: None,
                permissions: vec!["edit".to_string()],
                is_default: None,
                client_id: None,
            }),
        )
        .await
        .expect("create_role should succeed");

        let role_id = created.0.id.clone();

        let result = update_role(
            State(state),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(role_id.clone()),
            Json(UpdateRoleRequest {
                name: Some("Senior Editor".to_string()),
                slug: None,
                description: Some("Updated description".to_string()),
                permissions: Some(vec!["edit".to_string(), "publish".to_string()]),
                is_default: None,
            }),
        )
        .await
        .expect("update_role should succeed");

        assert_eq!(result.0.name, "Senior Editor");
        assert_eq!(result.0.permissions, vec!["edit", "publish"]);
    }

    #[tokio::test]
    async fn test_delete_role() {
        let Some(db) = connect_test_database("h_admin_roles_delete").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_role(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateRoleRequest {
                name: "Temp".to_string(),
                slug: "temp".to_string(),
                description: None,
                permissions: vec![],
                is_default: None,
                client_id: None,
            }),
        )
        .await
        .expect("create_role should succeed");

        let role_id = created.0.id.clone();

        let result = delete_role(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(role_id.clone()),
        )
        .await
        .expect("delete_role should succeed");

        assert_eq!(result.0.message, "Role deleted");

        let err = get_role(State(state), test_auth_user(&admin_id), Path(role_id)).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_assign_role() {
        let Some(db) = connect_test_database("h_admin_roles_assign").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;

        let target_id = Uuid::new_v4().to_string();
        let target_user = test_user(&target_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&target_user)
            .await
            .expect("insert target user");

        let state = test_app_state(db);

        let created = create_role(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateRoleRequest {
                name: "Assignable".to_string(),
                slug: "assignable".to_string(),
                description: None,
                permissions: vec!["test.perm".to_string()],
                is_default: None,
                client_id: None,
            }),
        )
        .await
        .expect("create_role should succeed");

        let role_id = created.0.id.clone();

        let result = assign_role(
            State(state),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path((target_id, role_id)),
        )
        .await
        .expect("assign_role should succeed");

        assert_eq!(result.0.message, "Role assigned");
    }
}
