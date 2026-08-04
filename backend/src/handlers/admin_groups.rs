use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::models::role::Role;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, group_service};

use mongodb::bson::doc;

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    #[serde(default)]
    pub role_ids: Vec<String>,
    pub parent_group_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub role_ids: Option<Vec<String>>,
    pub parent_group_id: Option<String>,
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
pub struct GroupResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub roles: Vec<RoleResponse>,
    pub parent_group_id: Option<String>,
    pub member_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct GroupListResponse {
    pub groups: Vec<GroupResponse>,
}

#[derive(Debug, Serialize)]
pub struct GroupMemberItem {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GroupMembersResponse {
    pub members: Vec<GroupMemberItem>,
    pub total: u64,
}

#[derive(Debug, Serialize)]
pub struct GroupMembershipResponse {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct UserGroupsResponse {
    pub groups: Vec<GroupResponse>,
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

/// Build a GroupResponse by fetching group roles and member count.
async fn build_group_response(
    db: &mongodb::Database,
    group: crate::models::group::Group,
) -> AppResult<GroupResponse> {
    // Fetch roles for this group
    let roles: Vec<Role> = if group.role_ids.is_empty() {
        vec![]
    } else {
        use futures::TryStreamExt;
        db.collection::<Role>(crate::models::role::COLLECTION_NAME)
            .find(doc! { "_id": { "$in": &group.role_ids } })
            .await?
            .try_collect()
            .await?
    };

    // Count members
    let member_count = db
        .collection::<User>(USERS)
        .count_documents(doc! { "group_ids": &group.id })
        .await?;

    Ok(GroupResponse {
        id: group.id,
        name: group.name,
        slug: group.slug,
        description: group.description,
        roles: roles.into_iter().map(role_to_response).collect(),
        parent_group_id: group.parent_group_id,
        member_count,
        created_at: group.created_at.to_rfc3339(),
        updated_at: group.updated_at.to_rfc3339(),
    })
}

// --- Handlers ---

/// GET /api/v1/admin/groups
pub async fn list_groups(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<GroupListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.groups.list").await?;

    let groups = group_service::list_groups(&state.db).await?;
    let mut items = Vec::with_capacity(groups.len());
    for group in groups {
        items.push(build_group_response(&state.db, group).await?);
    }

    Ok(Json(GroupListResponse { groups: items }))
}

/// POST /api/v1/admin/groups
pub async fn create_group(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Json(body): Json<CreateGroupRequest>,
) -> AppResult<Json<GroupResponse>> {
    require_admin(&state, &auth_user).await?;

    if body.name.is_empty() || body.slug.is_empty() {
        return Err(crate::errors::AppError::ValidationError(
            "Name and slug are required".to_string(),
        ));
    }

    let group = group_service::create_group(
        &state.db,
        &body.name,
        &body.slug,
        body.description.as_deref(),
        &body.role_ids,
        body.parent_group_id.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.group.created",
        Some(serde_json::json!({
            "group_id": &group.id,
            "group_slug": &group.slug,
        })),
    );

    let response = build_group_response(&state.db, group).await?;
    Ok(Json(response))
}

/// GET /api/v1/admin/groups/:group_id
pub async fn get_group(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(group_id): Path<String>,
) -> AppResult<Json<GroupResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.groups.get").await?;

    let group = group_service::get_group(&state.db, &group_id).await?;
    let response = build_group_response(&state.db, group).await?;
    Ok(Json(response))
}

/// PUT /api/v1/admin/groups/:group_id
pub async fn update_group(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(group_id): Path<String>,
    Json(body): Json<UpdateGroupRequest>,
) -> AppResult<Json<GroupResponse>> {
    require_admin(&state, &auth_user).await?;

    let parent = body
        .parent_group_id
        .as_ref()
        .map(|p| if p.is_empty() { None } else { Some(p.as_str()) });

    let group = group_service::update_group(
        &state.db,
        &group_id,
        body.name.as_deref(),
        body.slug.as_deref(),
        body.description.as_deref(),
        body.role_ids.as_deref(),
        parent,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.group.updated",
        Some(serde_json::json!({
            "group_id": &group_id,
            "group_slug": &group.slug,
        })),
    );

    let response = build_group_response(&state.db, group).await?;
    Ok(Json(response))
}

/// DELETE /api/v1/admin/groups/:group_id
pub async fn delete_group(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(group_id): Path<String>,
) -> AppResult<Json<GroupMembershipResponse>> {
    require_admin(&state, &auth_user).await?;

    group_service::delete_group(&state.db, &group_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.group.deleted",
        Some(serde_json::json!({ "group_id": &group_id })),
    );

    Ok(Json(GroupMembershipResponse {
        message: "Group deleted".to_string(),
    }))
}

/// GET /api/v1/admin/groups/:group_id/members
pub async fn get_members(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(group_id): Path<String>,
) -> AppResult<Json<GroupMembersResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.groups.members.list").await?;

    let members = group_service::get_members(&state.db, &group_id).await?;
    let total = members.len() as u64;

    let items: Vec<GroupMemberItem> = members
        .into_iter()
        .map(|u| GroupMemberItem {
            id: u.id,
            email: u.email,
            display_name: u.display_name,
        })
        .collect();

    Ok(Json(GroupMembersResponse {
        members: items,
        total,
    }))
}

/// POST /api/v1/admin/groups/:group_id/members/:user_id
pub async fn add_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((group_id, user_id)): Path<(String, String)>,
) -> AppResult<Json<GroupMembershipResponse>> {
    require_admin(&state, &auth_user).await?;

    group_service::add_member(&state.db, &group_id, &user_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.group.member_added",
        Some(serde_json::json!({
            "group_id": &group_id,
            "target_user_id": &user_id,
        })),
    );

    Ok(Json(GroupMembershipResponse {
        message: "Member added".to_string(),
    }))
}

/// DELETE /api/v1/admin/groups/:group_id/members/:user_id
pub async fn remove_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((group_id, user_id)): Path<(String, String)>,
) -> AppResult<Json<GroupMembershipResponse>> {
    require_admin(&state, &auth_user).await?;

    group_service::remove_member(&state.db, &group_id, &user_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.group.member_removed",
        Some(serde_json::json!({
            "group_id": &group_id,
            "target_user_id": &user_id,
        })),
    );

    Ok(Json(GroupMembershipResponse {
        message: "Member removed".to_string(),
    }))
}

/// GET /api/v1/admin/users/:user_id/groups
pub async fn get_user_groups(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(user_id): Path<String>,
) -> AppResult<Json<UserGroupsResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.users.groups.list").await?;

    let groups = group_service::get_user_groups(&state.db, &user_id).await?;
    let mut items = Vec::with_capacity(groups.len());
    for group in groups {
        items.push(build_group_response(&state.db, group).await?);
    }

    Ok(Json(UserGroupsResponse { groups: items }))
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
    fn role_to_response_system_role_with_client_id() {
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
        let role = make_role("role-2", "ts-role");
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
    fn create_group_request_deserializes() {
        let json = r#"{
            "name": "Engineering",
            "slug": "engineering",
            "description": "Eng team",
            "role_ids": ["role-1"],
            "parent_group_id": "parent-1"
        }"#;
        let req: CreateGroupRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.name, "Engineering");
        assert_eq!(req.slug, "engineering");
        assert_eq!(req.description, Some("Eng team".to_string()));
        assert_eq!(req.role_ids, vec!["role-1"]);
        assert_eq!(req.parent_group_id, Some("parent-1".to_string()));
    }

    #[test]
    fn create_group_request_minimal() {
        let json = r#"{"name": "Team", "slug": "team"}"#;
        let req: CreateGroupRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.name, "Team");
        assert!(req.description.is_none());
        assert!(req.role_ids.is_empty());
        assert!(req.parent_group_id.is_none());
    }

    #[test]
    fn update_group_request_all_none() {
        let json = r#"{}"#;
        let req: UpdateGroupRequest = serde_json::from_str(json).expect("deserialize");
        assert!(req.name.is_none());
        assert!(req.slug.is_none());
        assert!(req.description.is_none());
        assert!(req.role_ids.is_none());
        assert!(req.parent_group_id.is_none());
    }

    #[test]
    fn group_response_serializes_all_fields() {
        let resp = GroupResponse {
            id: "g-1".to_string(),
            name: "Eng".to_string(),
            slug: "eng".to_string(),
            description: Some("Engineering".to_string()),
            roles: vec![],
            parent_group_id: None,
            member_count: 5,
            created_at: "2024-01-01T00:00:00+00:00".to_string(),
            updated_at: "2024-01-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["id"], "g-1");
        assert_eq!(json["member_count"], 5);
        assert!(json["parent_group_id"].is_null());
        assert!(json["roles"].as_array().unwrap().is_empty());
    }

    #[test]
    fn group_member_item_serializes() {
        let item = GroupMemberItem {
            id: "u-1".to_string(),
            email: "member@example.com".to_string(),
            display_name: Some("Member".to_string()),
        };
        let json = serde_json::to_value(&item).expect("serialize");
        assert_eq!(json["id"], "u-1");
        assert_eq!(json["email"], "member@example.com");
    }

    #[test]
    fn group_membership_response_serializes() {
        let resp = GroupMembershipResponse {
            message: "Member added".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["message"], "Member added");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{Path, State};
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
    async fn test_list_groups_empty() {
        let Some(db) = connect_test_database("h_admin_groups_list").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let result = list_groups(State(state), auth)
            .await
            .expect("list_groups should succeed");

        assert!(result.0.groups.is_empty());
    }

    #[tokio::test]
    async fn test_create_group() {
        let Some(db) = connect_test_database("h_admin_groups_create").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let result = create_group(
            State(state),
            auth,
            HeaderMap::new(),
            Json(CreateGroupRequest {
                name: "Engineering".to_string(),
                slug: "engineering".to_string(),
                description: Some("Engineering team".to_string()),
                role_ids: vec![],
                parent_group_id: None,
            }),
        )
        .await
        .expect("create_group should succeed");

        assert_eq!(result.0.name, "Engineering");
        assert_eq!(result.0.slug, "engineering");
        assert_eq!(result.0.member_count, 0);
    }

    #[tokio::test]
    async fn test_get_group() {
        let Some(db) = connect_test_database("h_admin_groups_get").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_group(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateGroupRequest {
                name: "Design".to_string(),
                slug: "design".to_string(),
                description: None,
                role_ids: vec![],
                parent_group_id: None,
            }),
        )
        .await
        .expect("create_group should succeed");

        let group_id = created.0.id.clone();

        let result = get_group(
            State(state),
            test_auth_user(&admin_id),
            Path(group_id.clone()),
        )
        .await
        .expect("get_group should succeed");

        assert_eq!(result.0.id, group_id);
        assert_eq!(result.0.name, "Design");
    }

    #[tokio::test]
    async fn test_update_group() {
        let Some(db) = connect_test_database("h_admin_groups_update").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_group(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateGroupRequest {
                name: "Ops".to_string(),
                slug: "ops".to_string(),
                description: None,
                role_ids: vec![],
                parent_group_id: None,
            }),
        )
        .await
        .expect("create_group should succeed");

        let group_id = created.0.id.clone();

        let result = update_group(
            State(state),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(group_id.clone()),
            Json(UpdateGroupRequest {
                name: Some("Operations".to_string()),
                slug: None,
                description: Some("Operations team".to_string()),
                role_ids: None,
                parent_group_id: None,
            }),
        )
        .await
        .expect("update_group should succeed");

        assert_eq!(result.0.name, "Operations");
        assert_eq!(result.0.description, Some("Operations team".to_string()));
    }

    #[tokio::test]
    async fn test_delete_group() {
        let Some(db) = connect_test_database("h_admin_groups_delete").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let created = create_group(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateGroupRequest {
                name: "Temporary".to_string(),
                slug: "temporary".to_string(),
                description: None,
                role_ids: vec![],
                parent_group_id: None,
            }),
        )
        .await
        .expect("create_group should succeed");

        let group_id = created.0.id.clone();

        let result = delete_group(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(group_id.clone()),
        )
        .await
        .expect("delete_group should succeed");

        assert_eq!(result.0.message, "Group deleted");

        let err = get_group(State(state), test_auth_user(&admin_id), Path(group_id)).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_add_member() {
        let Some(db) = connect_test_database("h_admin_groups_add_member").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;

        let member_id = Uuid::new_v4().to_string();
        let member = test_user(&member_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&member)
            .await
            .expect("insert member user");

        let state = test_app_state(db);

        let created = create_group(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Json(CreateGroupRequest {
                name: "Team".to_string(),
                slug: "team".to_string(),
                description: None,
                role_ids: vec![],
                parent_group_id: None,
            }),
        )
        .await
        .expect("create_group should succeed");

        let group_id = created.0.id.clone();

        let result = add_member(
            State(state.clone()),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path((group_id.clone(), member_id)),
        )
        .await
        .expect("add_member should succeed");

        assert_eq!(result.0.message, "Member added");

        let group = get_group(State(state), test_auth_user(&admin_id), Path(group_id))
            .await
            .expect("get_group should succeed");

        assert_eq!(group.0.member_count, 1);
    }
}
