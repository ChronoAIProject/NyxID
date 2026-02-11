use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::errors::AppResult;
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent, require_admin};
use crate::models::role::Role;
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, group_service};
use crate::AppState;

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
    require_admin(&state, &auth_user).await?;

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
    headers: HeaderMap,
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

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.group.created".to_string(),
        Some(serde_json::json!({
            "group_id": &group.id,
            "group_slug": &group.slug,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
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
    require_admin(&state, &auth_user).await?;

    let group = group_service::get_group(&state.db, &group_id).await?;
    let response = build_group_response(&state.db, group).await?;
    Ok(Json(response))
}

/// PUT /api/v1/admin/groups/:group_id
pub async fn update_group(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(group_id): Path<String>,
    Json(body): Json<UpdateGroupRequest>,
) -> AppResult<Json<GroupResponse>> {
    require_admin(&state, &auth_user).await?;

    let parent = body.parent_group_id.as_ref().map(|p| {
        if p.is_empty() {
            None
        } else {
            Some(p.as_str())
        }
    });

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

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.group.updated".to_string(),
        Some(serde_json::json!({
            "group_id": &group_id,
            "group_slug": &group.slug,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    let response = build_group_response(&state.db, group).await?;
    Ok(Json(response))
}

/// DELETE /api/v1/admin/groups/:group_id
pub async fn delete_group(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(group_id): Path<String>,
) -> AppResult<Json<GroupMembershipResponse>> {
    require_admin(&state, &auth_user).await?;

    group_service::delete_group(&state.db, &group_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.group.deleted".to_string(),
        Some(serde_json::json!({ "group_id": &group_id })),
        extract_ip(&headers),
        extract_user_agent(&headers),
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
    require_admin(&state, &auth_user).await?;

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
    headers: HeaderMap,
    Path((group_id, user_id)): Path<(String, String)>,
) -> AppResult<Json<GroupMembershipResponse>> {
    require_admin(&state, &auth_user).await?;

    group_service::add_member(&state.db, &group_id, &user_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.group.member_added".to_string(),
        Some(serde_json::json!({
            "group_id": &group_id,
            "target_user_id": &user_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(GroupMembershipResponse {
        message: "Member added".to_string(),
    }))
}

/// DELETE /api/v1/admin/groups/:group_id/members/:user_id
pub async fn remove_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((group_id, user_id)): Path<(String, String)>,
) -> AppResult<Json<GroupMembershipResponse>> {
    require_admin(&state, &auth_user).await?;

    group_service::remove_member(&state.db, &group_id, &user_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.group.member_removed".to_string(),
        Some(serde_json::json!({
            "group_id": &group_id,
            "target_user_id": &user_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
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
    require_admin(&state, &auth_user).await?;

    let groups = group_service::get_user_groups(&state.db, &user_id).await?;
    let mut items = Vec::with_capacity(groups.len());
    for group in groups {
        items.push(build_group_response(&state.db, group).await?);
    }

    Ok(Json(UserGroupsResponse { groups: items }))
}
