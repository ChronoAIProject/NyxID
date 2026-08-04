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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::group::COLLECTION_NAME as GROUPS;
    use crate::models::role::COLLECTION_NAME as ROLES;
    use crate::models::user::COLLECTION_NAME as USERS;
    use crate::models::user::UserType;
    use crate::test_utils::{connect_test_database, test_user};

    fn make_role(id: &str, slug: &str, permissions: Vec<String>) -> Role {
        Role {
            id: id.to_string(),
            name: slug.to_string(),
            slug: slug.to_string(),
            description: None,
            permissions,
            is_default: false,
            is_system: false,
            client_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn make_group(id: &str, slug: &str, role_ids: Vec<String>) -> Group {
        Group {
            id: id.to_string(),
            name: slug.to_string(),
            slug: slug.to_string(),
            description: None,
            role_ids,
            parent_group_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // ---- resolve_user_rbac ----

    #[tokio::test]
    async fn resolve_user_rbac_missing_user_returns_empty() {
        let Some(db) = connect_test_database("rbac_missing_user").await else {
            return; // skip if no MongoDB
        };
        let result = resolve_user_rbac(&db, "nonexistent-user").await.unwrap();
        assert!(result.role_slugs.is_empty());
        assert!(result.group_slugs.is_empty());
        assert!(result.permissions.is_empty());
    }

    #[tokio::test]
    async fn resolve_user_rbac_user_no_roles_or_groups() {
        let Some(db) = connect_test_database("rbac_no_roles").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440001", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = resolve_user_rbac(&db, &user.id).await.unwrap();
        assert!(result.role_slugs.is_empty());
        assert!(result.group_slugs.is_empty());
        assert!(result.permissions.is_empty());
    }

    #[tokio::test]
    async fn resolve_user_rbac_with_direct_roles() {
        let Some(db) = connect_test_database("rbac_direct_roles").await else {
            return;
        };
        let role = make_role(
            "role-1",
            "editor",
            vec!["write".to_string(), "read".to_string()],
        );
        db.collection::<Role>(ROLES)
            .insert_one(&role)
            .await
            .unwrap();

        let mut user = test_user("550e8400-e29b-41d4-a716-446655440002", UserType::Person);
        user.role_ids = vec!["role-1".to_string()];
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = resolve_user_rbac(&db, &user.id).await.unwrap();
        assert_eq!(result.role_slugs, vec!["editor"]);
        assert!(result.group_slugs.is_empty());
        assert!(result.permissions.contains(&"write".to_string()));
        assert!(result.permissions.contains(&"read".to_string()));
    }

    #[tokio::test]
    async fn resolve_user_rbac_with_group_inherited_roles() {
        let Some(db) = connect_test_database("rbac_group_roles").await else {
            return;
        };
        let role = make_role("role-g1", "admin", vec!["*".to_string()]);
        db.collection::<Role>(ROLES)
            .insert_one(&role)
            .await
            .unwrap();

        let group = make_group("group-1", "engineering", vec!["role-g1".to_string()]);
        db.collection::<Group>(GROUPS)
            .insert_one(&group)
            .await
            .unwrap();

        let mut user = test_user("550e8400-e29b-41d4-a716-446655440003", UserType::Person);
        user.group_ids = vec!["group-1".to_string()];
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = resolve_user_rbac(&db, &user.id).await.unwrap();
        assert_eq!(result.role_slugs, vec!["admin"]);
        assert_eq!(result.group_slugs, vec!["engineering"]);
        assert!(result.permissions.contains(&"*".to_string()));
    }

    // ---- build_rbac_claim_data ----

    #[tokio::test]
    async fn build_rbac_claim_data_empty_scope() {
        let Some(db) = connect_test_database("rbac_claim_empty_scope").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440010", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_rbac_claim_data(&db, &user.id, "").await.unwrap();
        assert!(result.roles.is_none());
        assert!(result.groups.is_none());
        assert!(result.permissions.is_none());
        assert!(result.sid.is_none());
    }

    #[tokio::test]
    async fn build_rbac_claim_data_roles_scope_only() {
        let Some(db) = connect_test_database("rbac_claim_roles_scope").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440011", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_rbac_claim_data(&db, &user.id, "roles").await.unwrap();
        assert!(result.roles.is_some());
        assert!(result.groups.is_none());
        assert!(result.permissions.is_some());
    }

    #[tokio::test]
    async fn build_rbac_claim_data_groups_scope_only() {
        let Some(db) = connect_test_database("rbac_claim_groups_scope").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440012", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_rbac_claim_data(&db, &user.id, "groups")
            .await
            .unwrap();
        assert!(result.roles.is_none());
        assert!(result.groups.is_some());
        assert!(result.permissions.is_none());
    }

    #[tokio::test]
    async fn build_rbac_claim_data_both_scopes() {
        let Some(db) = connect_test_database("rbac_claim_both_scopes").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440013", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_rbac_claim_data(&db, &user.id, "roles groups")
            .await
            .unwrap();
        assert!(result.roles.is_some());
        assert!(result.groups.is_some());
        assert!(result.permissions.is_some());
    }

    #[tokio::test]
    async fn build_rbac_claim_data_unknown_scope_ignored() {
        let Some(db) = connect_test_database("rbac_claim_unknown_scope").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440014", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_rbac_claim_data(&db, &user.id, "something_else")
            .await
            .unwrap();
        assert!(result.roles.is_none());
        assert!(result.groups.is_none());
        assert!(result.permissions.is_none());
    }

    // ---- build_id_token_auth_context ----

    #[tokio::test]
    async fn build_id_token_auth_context_empty_scope() {
        let Some(db) = connect_test_database("rbac_idtoken_empty").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440020", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_id_token_auth_context(&db, &user.id, "")
            .await
            .unwrap();
        assert!(result.roles.is_none());
        assert!(result.groups.is_none());
        assert!(result.acr.is_none());
        assert!(result.amr.is_none());
        assert!(result.auth_time.is_none());
        assert!(result.sid.is_none());
    }

    #[tokio::test]
    async fn build_id_token_auth_context_roles_scope() {
        let Some(db) = connect_test_database("rbac_idtoken_roles").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440021", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_id_token_auth_context(&db, &user.id, "roles")
            .await
            .unwrap();
        assert!(result.roles.is_some());
        assert!(result.groups.is_none());
    }

    #[tokio::test]
    async fn build_id_token_auth_context_groups_scope() {
        let Some(db) = connect_test_database("rbac_idtoken_groups").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440022", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_id_token_auth_context(&db, &user.id, "groups")
            .await
            .unwrap();
        assert!(result.roles.is_none());
        assert!(result.groups.is_some());
    }

    #[tokio::test]
    async fn build_id_token_auth_context_both_scopes() {
        let Some(db) = connect_test_database("rbac_idtoken_both").await else {
            return;
        };
        let user = test_user("550e8400-e29b-41d4-a716-446655440023", UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = build_id_token_auth_context(&db, &user.id, "roles groups")
            .await
            .unwrap();
        assert!(result.roles.is_some());
        assert!(result.groups.is_some());
    }
}
