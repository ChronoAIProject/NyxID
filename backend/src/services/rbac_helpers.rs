use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::jwt::{IdTokenAuthContext, RbacClaimData};
use crate::errors::AppResult;
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::role::{COLLECTION_NAME as ROLES, Role};
use crate::models::user::{COLLECTION_NAME as USERS, User};

/// Resolved RBAC data for a user, ready to inject into JWT claims.
pub struct UserRbacData {
    pub role_slugs: Vec<String>,
    pub group_slugs: Vec<String>,
    pub permissions: Vec<String>,
}

/// Fetch and resolve all RBAC data for a user.
///
/// Collects directly-assigned roles, group-inherited roles, and flattened
/// permissions. Performs at most 3 MongoDB queries.
pub async fn resolve_user_rbac(db: &mongodb::Database, user_id: &str) -> AppResult<UserRbacData> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?;

    let user = match user {
        Some(u) => u,
        None => {
            return Ok(UserRbacData {
                role_slugs: vec![],
                group_slugs: vec![],
                permissions: vec![],
            });
        }
    };

    // Collect all role IDs: direct + group-inherited
    let mut all_role_ids: HashSet<String> = user.role_ids.iter().cloned().collect();

    // Get user's groups and their role_ids
    let groups: Vec<Group> = if user.group_ids.is_empty() {
        vec![]
    } else {
        db.collection::<Group>(GROUPS)
            .find(doc! { "_id": { "$in": &user.group_ids } })
            .await?
            .try_collect()
            .await?
    };

    let group_slugs: Vec<String> = groups.iter().map(|g| g.slug.clone()).collect();

    for group in &groups {
        for role_id in &group.role_ids {
            all_role_ids.insert(role_id.clone());
        }
    }

    // Fetch all roles
    let role_id_list: Vec<&str> = all_role_ids.iter().map(|s| s.as_str()).collect();
    let roles: Vec<Role> = if role_id_list.is_empty() {
        vec![]
    } else {
        db.collection::<Role>(ROLES)
            .find(doc! { "_id": { "$in": &role_id_list } })
            .await?
            .try_collect()
            .await?
    };

    let role_slugs: Vec<String> = roles.iter().map(|r| r.slug.clone()).collect();

    // Flatten permissions (deduplicated)
    let mut perm_set: HashSet<String> = HashSet::new();
    for role in &roles {
        for perm in &role.permissions {
            perm_set.insert(perm.clone());
        }
    }
    let permissions: Vec<String> = perm_set.into_iter().collect();

    Ok(UserRbacData {
        role_slugs,
        group_slugs,
        permissions,
    })
}

/// Build `RbacClaimData` for JWT access token injection, filtered by scope.
///
/// Only includes roles/permissions when the "roles" scope is present, and
/// groups when the "groups" scope is present.
pub async fn build_rbac_claim_data(
    db: &mongodb::Database,
    user_id: &str,
    scope: &str,
) -> AppResult<RbacClaimData> {
    let scopes: Vec<&str> = scope.split_whitespace().collect();
    let include_roles = scopes.contains(&"roles");
    let include_groups = scopes.contains(&"groups");

    if !include_roles && !include_groups {
        return Ok(RbacClaimData {
            roles: None,
            groups: None,
            permissions: None,
            sid: None,
        });
    }

    let rbac = resolve_user_rbac(db, user_id).await?;

    Ok(RbacClaimData {
        roles: if include_roles {
            Some(rbac.role_slugs)
        } else {
            None
        },
        groups: if include_groups {
            Some(rbac.group_slugs)
        } else {
            None
        },
        permissions: if include_roles {
            Some(rbac.permissions)
        } else {
            None
        },
        sid: None,
    })
}

/// Build `IdTokenAuthContext` for ID token injection, filtered by scope.
pub async fn build_id_token_auth_context(
    db: &mongodb::Database,
    user_id: &str,
    scope: &str,
) -> AppResult<IdTokenAuthContext> {
    let scopes: Vec<&str> = scope.split_whitespace().collect();
    let include_roles = scopes.contains(&"roles");
    let include_groups = scopes.contains(&"groups");

    if !include_roles && !include_groups {
        return Ok(IdTokenAuthContext {
            roles: None,
            groups: None,
            acr: None,
            amr: None,
            auth_time: None,
            sid: None,
        });
    }

    let rbac = resolve_user_rbac(db, user_id).await?;

    Ok(IdTokenAuthContext {
        roles: if include_roles {
            Some(rbac.role_slugs)
        } else {
            None
        },
        groups: if include_groups {
            Some(rbac.group_slugs)
        } else {
            None
        },
        acr: None,
        amr: None,
        auth_time: None,
        sid: None,
    })
}
