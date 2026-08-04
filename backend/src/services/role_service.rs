use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::role::{
    COLLECTION_NAME as ROLES, PLATFORM_ADMIN_ROLE_SLUG, PLATFORM_OPERATOR_ROLE_SLUG,
    PLATFORM_USER_ROLE_SLUG, Role,
};
use crate::models::user::{COLLECTION_NAME as USERS, PlatformRole, User};

const PLATFORM_ADMIN_ROLE_NAME: &str = "Admin";
const PLATFORM_OPERATOR_ROLE_NAME: &str = "Operator";
const PLATFORM_USER_ROLE_NAME: &str = "User";
const PLATFORM_ADMIN_ROLE_DESCRIPTION: &str = "System administrator with full access";
const PLATFORM_OPERATOR_ROLE_DESCRIPTION: &str = "Read-only platform admin operator";
const PLATFORM_USER_ROLE_DESCRIPTION: &str = "Default user role";
const PLATFORM_ADMIN_PERMISSIONS: &[&str] = &["*"];
const PLATFORM_OPERATOR_PERMISSIONS: &[&str] = &["nyxid:admin:read"];
const PLATFORM_USER_PERMISSIONS: &[&str] = &[];

#[derive(Clone, Debug)]
pub struct PlatformRoleIds {
    pub admin: String,
    pub operator: String,
}

struct SystemRoleSpec {
    name: &'static str,
    slug: &'static str,
    description: &'static str,
    permissions: &'static [&'static str],
    is_default: bool,
}

fn permissions_vec(permissions: &[&str]) -> Vec<String> {
    permissions
        .iter()
        .map(|permission| (*permission).to_string())
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

async fn ensure_system_role(db: &mongodb::Database, spec: &SystemRoleSpec) -> AppResult<Role> {
    let roles = db.collection::<Role>(ROLES);
    let permissions = permissions_vec(spec.permissions);
    let now = Utc::now();

    if let Some(existing) = roles.find_one(doc! { "slug": spec.slug }).await? {
        roles
            .update_one(
                doc! { "_id": &existing.id },
                doc! { "$set": {
                    "name": spec.name,
                    "description": spec.description,
                    "permissions": &permissions,
                    "is_default": spec.is_default,
                    "is_system": true,
                    "client_id": null,
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;

        return roles
            .find_one(doc! { "_id": &existing.id })
            .await?
            .ok_or_else(|| AppError::Internal("System role disappeared after update".to_string()));
    }

    let role = Role {
        id: Uuid::new_v4().to_string(),
        name: spec.name.to_string(),
        slug: spec.slug.to_string(),
        description: Some(spec.description.to_string()),
        permissions,
        is_default: spec.is_default,
        is_system: true,
        client_id: None,
        created_at: now,
        updated_at: now,
    };

    roles.insert_one(&role).await?;
    tracing::info!(slug = spec.slug, "Seeded system role");
    Ok(role)
}

async fn get_platform_role_by_slug(db: &mongodb::Database, slug: &str) -> AppResult<Role> {
    db.collection::<Role>(ROLES)
        .find_one(doc! {
            "slug": slug,
            "client_id": null,
            "is_system": true,
        })
        .await?
        .ok_or_else(|| {
            AppError::Internal(format!(
                "Platform system role '{slug}' is missing; startup seed did not complete"
            ))
        })
}

pub async fn get_platform_role_ids(db: &mongodb::Database) -> AppResult<PlatformRoleIds> {
    let admin = get_platform_role_by_slug(db, PLATFORM_ADMIN_ROLE_SLUG).await?;
    let operator = get_platform_role_by_slug(db, PLATFORM_OPERATOR_ROLE_SLUG).await?;
    Ok(PlatformRoleIds {
        admin: admin.id,
        operator: operator.id,
    })
}

pub fn resolve_platform_role_from_ids(user: &User, role_ids: &PlatformRoleIds) -> PlatformRole {
    if user
        .role_ids
        .iter()
        .any(|role_id| role_id == &role_ids.admin)
    {
        PlatformRole::Admin
    } else if user
        .role_ids
        .iter()
        .any(|role_id| role_id == &role_ids.operator)
    {
        PlatformRole::Operator
    } else {
        PlatformRole::User
    }
}

pub async fn resolve_platform_role(db: &mongodb::Database, user: &User) -> AppResult<PlatformRole> {
    let role_ids = get_platform_role_ids(db).await?;
    Ok(resolve_platform_role_from_ids(user, &role_ids))
}

pub fn add_platform_role_id(
    role_ids: &mut Vec<String>,
    platform_role: PlatformRole,
    platform_role_ids: &PlatformRoleIds,
) {
    match platform_role {
        PlatformRole::Admin => push_unique(role_ids, platform_role_ids.admin.clone()),
        PlatformRole::Operator => push_unique(role_ids, platform_role_ids.operator.clone()),
        PlatformRole::User => {}
    }
}

/// Build an aggregation-pipeline `update_one` doc that atomically swaps a
/// user's platform RBAC membership to `target` and mirrors the legacy
/// `is_admin` / `is_operator` flags in a single round-trip. The pipeline:
///   1. Strips any existing admin/operator role IDs from `role_ids`.
///   2. Appends the role ID for `target` (none when `target = User`).
///   3. Sets the legacy flag mirror.
///   4. Stamps `updated_at`.
pub fn set_platform_role_update(
    target: PlatformRole,
    platform_role_ids: &PlatformRoleIds,
    now: bson::DateTime,
) -> Vec<bson::Document> {
    let (is_admin, is_operator) = target.legacy_flags();
    let to_add: Vec<String> = match target {
        PlatformRole::Admin => vec![platform_role_ids.admin.clone()],
        PlatformRole::Operator => vec![platform_role_ids.operator.clone()],
        PlatformRole::User => vec![],
    };

    vec![doc! {
        "$set": {
            "is_admin": is_admin,
            "is_operator": is_operator,
            "updated_at": now,
            "role_ids": {
                "$concatArrays": [
                    {
                        "$filter": {
                            "input": { "$ifNull": ["$role_ids", []] },
                            "cond": {
                                "$not": {
                                    "$in": [
                                        "$$this",
                                        [&platform_role_ids.admin, &platform_role_ids.operator],
                                    ]
                                }
                            }
                        }
                    },
                    to_add,
                ]
            }
        }
    }]
}

pub struct PlatformRoleBackfillResult {
    pub admin_role_id: String,
    pub operator_role_id: String,
    pub admin_users_modified: u64,
    pub operator_users_modified: u64,
}

/// Create a new role.
pub async fn create_role(
    db: &mongodb::Database,
    name: &str,
    slug: &str,
    description: Option<&str>,
    permissions: &[String],
    is_default: bool,
    client_id: Option<&str>,
) -> AppResult<Role> {
    crate::handlers::admin_helpers::validate_slug(slug)?;

    // Check for duplicate slug
    let existing = db
        .collection::<Role>(ROLES)
        .find_one(doc! { "slug": slug })
        .await?;
    if existing.is_some() {
        return Err(AppError::DuplicateSlug(slug.to_string()));
    }

    let now = Utc::now();
    let role = Role {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        slug: slug.to_string(),
        description: description.map(String::from),
        permissions: permissions.to_vec(),
        is_default,
        is_system: false,
        client_id: client_id.map(String::from),
        created_at: now,
        updated_at: now,
    };

    db.collection::<Role>(ROLES).insert_one(&role).await?;
    Ok(role)
}

/// Get a role by ID.
pub async fn get_role(db: &mongodb::Database, role_id: &str) -> AppResult<Role> {
    db.collection::<Role>(ROLES)
        .find_one(doc! { "_id": role_id })
        .await?
        .ok_or_else(|| AppError::RoleNotFound(role_id.to_string()))
}

/// List all roles (with optional client_id filter).
pub async fn list_roles(db: &mongodb::Database, client_id: Option<&str>) -> AppResult<Vec<Role>> {
    let filter = match client_id {
        Some(cid) => doc! { "client_id": cid },
        None => doc! {},
    };

    let roles: Vec<Role> = db
        .collection::<Role>(ROLES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .limit(200)
        .await?
        .try_collect()
        .await?;

    Ok(roles)
}

/// Update a role. System roles only allow description and permissions updates.
pub async fn update_role(
    db: &mongodb::Database,
    role_id: &str,
    name: Option<&str>,
    slug: Option<&str>,
    description: Option<&str>,
    permissions: Option<&[String]>,
    is_default: Option<bool>,
) -> AppResult<Role> {
    let existing = get_role(db, role_id).await?;

    // System roles: only description and is_default can be changed
    if existing.is_system && (name.is_some() || slug.is_some() || permissions.is_some()) {
        return Err(AppError::SystemRoleProtected(existing.slug));
    }

    // Validate slug format if changing
    if let Some(new_slug) = slug {
        crate::handlers::admin_helpers::validate_slug(new_slug)?;
    }

    // Check slug uniqueness if changing
    if let Some(new_slug) = slug
        && new_slug != existing.slug
    {
        let dup = db
            .collection::<Role>(ROLES)
            .find_one(doc! { "slug": new_slug })
            .await?;
        if dup.is_some() {
            return Err(AppError::DuplicateSlug(new_slug.to_string()));
        }
    }

    let now = bson::DateTime::from_chrono(Utc::now());
    let mut update = doc! { "updated_at": now };

    if let Some(n) = name {
        update.insert("name", n);
    }
    if let Some(s) = slug {
        update.insert("slug", s);
    }
    if let Some(d) = description {
        update.insert("description", d);
    }
    if let Some(p) = permissions {
        update.insert("permissions", p);
    }
    if let Some(d) = is_default {
        update.insert("is_default", d);
    }

    db.collection::<Role>(ROLES)
        .update_one(doc! { "_id": role_id }, doc! { "$set": update })
        .await?;

    get_role(db, role_id).await
}

/// Delete a role (non-system roles only). Removes from all users and groups.
pub async fn delete_role(db: &mongodb::Database, role_id: &str) -> AppResult<()> {
    let role = get_role(db, role_id).await?;

    if role.is_system {
        return Err(AppError::SystemRoleProtected(role.slug));
    }

    // Remove role from all users
    db.collection::<User>(USERS)
        .update_many(
            doc! { "role_ids": role_id },
            doc! { "$pull": { "role_ids": role_id } },
        )
        .await?;

    // Remove role from all groups
    db.collection::<Group>(GROUPS)
        .update_many(
            doc! { "role_ids": role_id },
            doc! { "$pull": { "role_ids": role_id } },
        )
        .await?;

    db.collection::<Role>(ROLES)
        .delete_one(doc! { "_id": role_id })
        .await?;

    Ok(())
}

/// Assign a role to a user.
pub async fn assign_role_to_user(
    db: &mongodb::Database,
    user_id: &str,
    role_id: &str,
) -> AppResult<()> {
    // Verify role exists
    let _role = get_role(db, role_id).await?;

    // Verify user exists and check if role is already assigned
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if user.role_ids.contains(&role_id.to_string()) {
        return Err(AppError::RoleAlreadyAssigned);
    }

    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": user_id },
            doc! { "$addToSet": { "role_ids": role_id } },
        )
        .await?;

    Ok(())
}

/// Revoke a role from a user.
pub async fn revoke_role_from_user(
    db: &mongodb::Database,
    user_id: &str,
    role_id: &str,
) -> AppResult<()> {
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": user_id },
            doc! { "$pull": { "role_ids": role_id } },
        )
        .await?;

    Ok(())
}

/// Get all directly-assigned roles for a user.
pub async fn get_user_roles(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<Role>> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if user.role_ids.is_empty() {
        return Ok(vec![]);
    }

    let roles: Vec<Role> = db
        .collection::<Role>(ROLES)
        .find(doc! { "_id": { "$in": &user.role_ids } })
        .await?
        .try_collect()
        .await?;

    Ok(roles)
}

/// Result of a bulk role assignment operation.
pub struct BulkAssignResult {
    /// Number of users who had the role added (did not already have it).
    pub assigned_count: u64,
    /// Number of users who already had the role (skipped).
    pub already_assigned_count: u64,
}

/// Assign a role to multiple users in bulk.
///
/// If `user_ids` is `Some`, assigns only to those users. If `None`, assigns to
/// all users who don't already have the role.
pub async fn bulk_assign_role(
    db: &mongodb::Database,
    role_id: &str,
    user_ids: Option<&[String]>,
) -> AppResult<BulkAssignResult> {
    // Verify role exists
    let _role = get_role(db, role_id).await?;

    let users_coll = db.collection::<User>(USERS);

    // Count users who already have this role (for reporting)
    let already_filter = match user_ids {
        Some(ids) => doc! { "_id": { "$in": ids }, "role_ids": role_id },
        None => doc! { "role_ids": role_id },
    };
    let already_assigned_count = users_coll.count_documents(already_filter).await?;

    // Add role to users who don't have it yet
    let update_filter = match user_ids {
        Some(ids) => doc! { "_id": { "$in": ids }, "role_ids": { "$ne": role_id } },
        None => doc! { "role_ids": { "$ne": role_id } },
    };

    let now = bson::DateTime::from_chrono(Utc::now());
    let result = users_coll
        .update_many(
            update_filter,
            doc! {
                "$addToSet": { "role_ids": role_id },
                "$set": { "updated_at": now },
            },
        )
        .await?;

    Ok(BulkAssignResult {
        assigned_count: result.modified_count,
        already_assigned_count,
    })
}

/// Get IDs of all roles marked as `is_default: true`.
///
/// Used during user registration to auto-assign default roles.
pub async fn get_default_role_ids(db: &mongodb::Database) -> AppResult<Vec<String>> {
    let roles: Vec<Role> = db
        .collection::<Role>(ROLES)
        .find(doc! { "is_default": true })
        .await?
        .try_collect()
        .await?;

    Ok(roles.into_iter().map(|r| r.id).collect())
}

/// Seed system roles if they don't exist, and normalize their immutable
/// metadata. Existing role IDs are preserved so deployments that already have
/// a seeded `admin` role can safely backfill users onto that role.
pub async fn seed_system_roles(db: &mongodb::Database) -> AppResult<()> {
    let admin = SystemRoleSpec {
        name: PLATFORM_ADMIN_ROLE_NAME,
        slug: PLATFORM_ADMIN_ROLE_SLUG,
        description: PLATFORM_ADMIN_ROLE_DESCRIPTION,
        permissions: PLATFORM_ADMIN_PERMISSIONS,
        is_default: false,
    };
    ensure_system_role(db, &admin).await?;

    let operator = SystemRoleSpec {
        name: PLATFORM_OPERATOR_ROLE_NAME,
        slug: PLATFORM_OPERATOR_ROLE_SLUG,
        description: PLATFORM_OPERATOR_ROLE_DESCRIPTION,
        permissions: PLATFORM_OPERATOR_PERMISSIONS,
        is_default: false,
    };
    ensure_system_role(db, &operator).await?;

    let user = SystemRoleSpec {
        name: PLATFORM_USER_ROLE_NAME,
        slug: PLATFORM_USER_ROLE_SLUG,
        description: PLATFORM_USER_ROLE_DESCRIPTION,
        permissions: PLATFORM_USER_PERMISSIONS,
        is_default: true,
    };
    ensure_system_role(db, &user).await?;

    Ok(())
}

pub async fn backfill_platform_role_memberships(
    db: &mongodb::Database,
) -> AppResult<PlatformRoleBackfillResult> {
    let role_ids = get_platform_role_ids(db).await?;
    let users = db.collection::<User>(USERS);
    let now = bson::DateTime::from_chrono(Utc::now());

    let admin_result = users
        .update_many(
            doc! {
                "is_admin": true,
                "role_ids": { "$ne": &role_ids.admin },
            },
            doc! {
                "$addToSet": { "role_ids": &role_ids.admin },
                "$set": { "updated_at": now },
            },
        )
        .await?;

    let operator_result = users
        .update_many(
            doc! {
                "is_operator": true,
                "role_ids": { "$ne": &role_ids.operator },
            },
            doc! {
                "$addToSet": { "role_ids": &role_ids.operator },
                "$set": { "updated_at": now },
            },
        )
        .await?;

    Ok(PlatformRoleBackfillResult {
        admin_role_id: role_ids.admin,
        operator_role_id: role_ids.operator,
        admin_users_modified: admin_result.modified_count,
        operator_users_modified: operator_result.modified_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use crate::test_utils::{connect_test_database, test_user};

    async fn insert_flagged_user(
        db: &mongodb::Database,
        is_admin: bool,
        is_operator: bool,
    ) -> String {
        let user_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, UserType::Person);
        user.is_admin = is_admin;
        user.is_operator = is_operator;
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert flagged user");
        user_id
    }

    fn make_user_with_role_ids(role_ids: Vec<String>) -> User {
        let mut user = test_user("00000000-0000-0000-0000-000000000000", UserType::Person);
        user.role_ids = role_ids;
        user
    }

    #[test]
    fn resolve_platform_role_from_ids_picks_admin_over_operator() {
        let role_ids = PlatformRoleIds {
            admin: "admin-id".to_string(),
            operator: "operator-id".to_string(),
        };

        let none = make_user_with_role_ids(vec![]);
        assert_eq!(
            resolve_platform_role_from_ids(&none, &role_ids),
            PlatformRole::User
        );

        let only_operator = make_user_with_role_ids(vec!["operator-id".to_string()]);
        assert_eq!(
            resolve_platform_role_from_ids(&only_operator, &role_ids),
            PlatformRole::Operator
        );

        let both = make_user_with_role_ids(vec!["admin-id".to_string(), "operator-id".to_string()]);
        assert_eq!(
            resolve_platform_role_from_ids(&both, &role_ids),
            PlatformRole::Admin
        );

        let only_admin = make_user_with_role_ids(vec!["admin-id".to_string()]);
        assert_eq!(
            resolve_platform_role_from_ids(&only_admin, &role_ids),
            PlatformRole::Admin
        );
        assert!(resolve_platform_role_from_ids(&only_admin, &role_ids).has_admin_read());
    }

    #[tokio::test]
    async fn seed_creates_platform_roles_exactly_once_when_run_twice() {
        let Some(db) = connect_test_database("role_seed_platform").await else {
            eprintln!("skipping role seed test: no local MongoDB available");
            return;
        };

        seed_system_roles(&db).await.expect("seed roles");
        seed_system_roles(&db).await.expect("seed roles again");

        let roles: Vec<Role> = db
            .collection::<Role>(ROLES)
            .find(doc! {
                "slug": {
                    "$in": [
                        PLATFORM_ADMIN_ROLE_SLUG,
                        PLATFORM_OPERATOR_ROLE_SLUG,
                    ],
                },
                "client_id": null,
                "is_system": true,
            })
            .await
            .expect("query roles")
            .try_collect()
            .await
            .expect("collect roles");

        assert_eq!(roles.len(), 2, "admin/operator roles should be unique");
        let admin = roles
            .iter()
            .find(|role| role.slug == PLATFORM_ADMIN_ROLE_SLUG)
            .expect("admin role exists");
        assert_eq!(
            admin.permissions,
            permissions_vec(PLATFORM_ADMIN_PERMISSIONS)
        );
        let operator = roles
            .iter()
            .find(|role| role.slug == PLATFORM_OPERATOR_ROLE_SLUG)
            .expect("operator role exists");
        assert_eq!(
            operator.permissions,
            permissions_vec(PLATFORM_OPERATOR_PERMISSIONS)
        );
    }

    #[tokio::test]
    async fn backfill_assigns_admin_role_to_legacy_admin_flag() {
        let Some(db) = connect_test_database("role_backfill_admin").await else {
            eprintln!("skipping admin backfill test: no local MongoDB available");
            return;
        };
        seed_system_roles(&db).await.expect("seed roles");
        let user_id = insert_flagged_user(&db, true, false).await;

        let result = backfill_platform_role_memberships(&db)
            .await
            .expect("backfill memberships");
        assert_eq!(result.admin_users_modified, 1);
        let second = backfill_platform_role_memberships(&db)
            .await
            .expect("backfill memberships again");
        assert_eq!(second.admin_users_modified, 0);

        let user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .expect("query user")
            .expect("user exists");
        let role_ids = get_platform_role_ids(&db).await.expect("platform role ids");
        assert!(user.role_ids.contains(&role_ids.admin));
        assert!(!user.role_ids.contains(&role_ids.operator));
    }

    #[tokio::test]
    async fn backfill_assigns_operator_role_to_legacy_operator_flag() {
        let Some(db) = connect_test_database("role_backfill_operator").await else {
            eprintln!("skipping operator backfill test: no local MongoDB available");
            return;
        };
        seed_system_roles(&db).await.expect("seed roles");
        let user_id = insert_flagged_user(&db, false, true).await;

        let result = backfill_platform_role_memberships(&db)
            .await
            .expect("backfill memberships");
        assert_eq!(result.operator_users_modified, 1);
        let second = backfill_platform_role_memberships(&db)
            .await
            .expect("backfill memberships again");
        assert_eq!(second.operator_users_modified, 0);

        let user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .expect("query user")
            .expect("user exists");
        let role_ids = get_platform_role_ids(&db).await.expect("platform role ids");
        assert!(!user.role_ids.contains(&role_ids.admin));
        assert!(user.role_ids.contains(&role_ids.operator));
    }

    #[test]
    fn permissions_vec_converts_str_slices() {
        let perms = permissions_vec(&["read", "write", "admin"]);
        assert_eq!(
            perms,
            vec!["read".to_string(), "write".to_string(), "admin".to_string()]
        );
    }

    #[test]
    fn permissions_vec_empty_input() {
        let perms = permissions_vec(&[]);
        assert!(perms.is_empty());
    }

    #[test]
    fn push_unique_adds_new_value() {
        let mut values = vec!["a".to_string()];
        push_unique(&mut values, "b".to_string());
        assert_eq!(values, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn push_unique_skips_duplicate() {
        let mut values = vec!["a".to_string(), "b".to_string()];
        push_unique(&mut values, "a".to_string());
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn add_platform_role_id_admin_adds_admin_id() {
        let platform_ids = PlatformRoleIds {
            admin: "admin-role-id".to_string(),
            operator: "operator-role-id".to_string(),
        };
        let mut ids = vec![];
        add_platform_role_id(&mut ids, PlatformRole::Admin, &platform_ids);
        assert_eq!(ids, vec!["admin-role-id".to_string()]);
    }

    #[test]
    fn add_platform_role_id_operator_adds_operator_id() {
        let platform_ids = PlatformRoleIds {
            admin: "admin-role-id".to_string(),
            operator: "operator-role-id".to_string(),
        };
        let mut ids = vec![];
        add_platform_role_id(&mut ids, PlatformRole::Operator, &platform_ids);
        assert_eq!(ids, vec!["operator-role-id".to_string()]);
    }

    #[test]
    fn add_platform_role_id_user_adds_nothing() {
        let platform_ids = PlatformRoleIds {
            admin: "admin-role-id".to_string(),
            operator: "operator-role-id".to_string(),
        };
        let mut ids = vec![];
        add_platform_role_id(&mut ids, PlatformRole::User, &platform_ids);
        assert!(ids.is_empty());
    }

    #[test]
    fn add_platform_role_id_does_not_duplicate() {
        let platform_ids = PlatformRoleIds {
            admin: "admin-role-id".to_string(),
            operator: "operator-role-id".to_string(),
        };
        let mut ids = vec!["admin-role-id".to_string()];
        add_platform_role_id(&mut ids, PlatformRole::Admin, &platform_ids);
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn set_platform_role_update_admin_sets_flags_correctly() {
        let platform_ids = PlatformRoleIds {
            admin: "aid".to_string(),
            operator: "oid".to_string(),
        };
        let now = bson::DateTime::from_chrono(Utc::now());
        let pipeline = set_platform_role_update(PlatformRole::Admin, &platform_ids, now);
        assert_eq!(pipeline.len(), 1);
        let set_stage = pipeline[0].get_document("$set").expect("$set key");
        assert!(set_stage.get_bool("is_admin").unwrap());
        assert!(!set_stage.get_bool("is_operator").unwrap());
    }

    #[test]
    fn set_platform_role_update_operator_sets_flags_correctly() {
        let platform_ids = PlatformRoleIds {
            admin: "aid".to_string(),
            operator: "oid".to_string(),
        };
        let now = bson::DateTime::from_chrono(Utc::now());
        let pipeline = set_platform_role_update(PlatformRole::Operator, &platform_ids, now);
        let set_stage = pipeline[0].get_document("$set").expect("$set key");
        assert!(!set_stage.get_bool("is_admin").unwrap());
        assert!(set_stage.get_bool("is_operator").unwrap());
    }

    #[test]
    fn set_platform_role_update_user_clears_both_flags() {
        let platform_ids = PlatformRoleIds {
            admin: "aid".to_string(),
            operator: "oid".to_string(),
        };
        let now = bson::DateTime::from_chrono(Utc::now());
        let pipeline = set_platform_role_update(PlatformRole::User, &platform_ids, now);
        let set_stage = pipeline[0].get_document("$set").expect("$set key");
        assert!(!set_stage.get_bool("is_admin").unwrap());
        assert!(!set_stage.get_bool("is_operator").unwrap());
    }

    #[tokio::test]
    async fn create_role_happy_path() {
        let Some(db) = connect_test_database("role_create_ok").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(
            &db,
            "Tester",
            "tester",
            Some("Test role"),
            &["read".to_string()],
            false,
            None,
        )
        .await
        .expect("create role");

        assert_eq!(role.name, "Tester");
        assert_eq!(role.slug, "tester");
        assert_eq!(role.description.as_deref(), Some("Test role"));
        assert_eq!(role.permissions, vec!["read".to_string()]);
        assert!(!role.is_default);
        assert!(!role.is_system);
        assert!(role.client_id.is_none());
    }

    #[tokio::test]
    async fn create_role_duplicate_slug_error() {
        let Some(db) = connect_test_database("role_create_dup").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        create_role(&db, "First", "dup-slug", None, &[], false, None)
            .await
            .expect("first create");

        let err = create_role(&db, "Second", "dup-slug", None, &[], false, None)
            .await
            .expect_err("duplicate slug must fail");
        assert!(matches!(err, AppError::DuplicateSlug(_)));
    }

    #[tokio::test]
    async fn create_role_invalid_slug_error() {
        let Some(db) = connect_test_database("role_create_slug").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let err = create_role(&db, "Bad", "UPPER-CASE!", None, &[], false, None)
            .await
            .expect_err("invalid slug must fail");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn get_role_not_found() {
        let Some(db) = connect_test_database("role_get_nf").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let err = get_role(&db, "nonexistent-id")
            .await
            .expect_err("must fail");
        assert!(matches!(err, AppError::RoleNotFound(_)));
    }

    #[tokio::test]
    async fn get_role_happy_path() {
        let Some(db) = connect_test_database("role_get_ok").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let created = create_role(&db, "Fetcher", "fetcher", None, &[], false, None)
            .await
            .expect("create");
        let fetched = get_role(&db, &created.id).await.expect("get");
        assert_eq!(fetched.slug, "fetcher");
    }

    #[tokio::test]
    async fn update_role_system_role_protection() {
        let Some(db) = connect_test_database("role_upd_sys").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        seed_system_roles(&db).await.expect("seed");

        let admin = db
            .collection::<Role>(ROLES)
            .find_one(doc! { "slug": PLATFORM_ADMIN_ROLE_SLUG })
            .await
            .expect("query")
            .expect("admin role");

        let err = update_role(&db, &admin.id, Some("New Name"), None, None, None, None)
            .await
            .expect_err("system role update must fail");
        assert!(matches!(err, AppError::SystemRoleProtected(_)));
    }

    #[tokio::test]
    async fn update_role_slug_uniqueness() {
        let Some(db) = connect_test_database("role_upd_slug").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        create_role(&db, "A", "slug-a", None, &[], false, None)
            .await
            .expect("create A");
        let b = create_role(&db, "B", "slug-b", None, &[], false, None)
            .await
            .expect("create B");

        let err = update_role(&db, &b.id, None, Some("slug-a"), None, None, None)
            .await
            .expect_err("duplicate slug");
        assert!(matches!(err, AppError::DuplicateSlug(_)));
    }

    #[tokio::test]
    async fn update_role_name_and_permissions() {
        let Some(db) = connect_test_database("role_upd_np").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(&db, "Old", "updatable", None, &[], false, None)
            .await
            .expect("create");

        let updated = update_role(
            &db,
            &role.id,
            Some("New"),
            None,
            Some("desc"),
            Some(&["write".to_string()]),
            Some(true),
        )
        .await
        .expect("update");

        assert_eq!(updated.name, "New");
        assert_eq!(updated.description.as_deref(), Some("desc"));
        assert_eq!(updated.permissions, vec!["write".to_string()]);
        assert!(updated.is_default);
    }

    #[tokio::test]
    async fn delete_role_cannot_delete_system_role() {
        let Some(db) = connect_test_database("role_del_sys").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        seed_system_roles(&db).await.expect("seed");

        let admin = db
            .collection::<Role>(ROLES)
            .find_one(doc! { "slug": PLATFORM_ADMIN_ROLE_SLUG })
            .await
            .expect("query")
            .expect("admin role");

        let err = delete_role(&db, &admin.id)
            .await
            .expect_err("delete system role must fail");
        assert!(matches!(err, AppError::SystemRoleProtected(_)));
    }

    #[tokio::test]
    async fn delete_role_removes_from_users_and_groups() {
        let Some(db) = connect_test_database("role_del_clean").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(&db, "Temp", "temp-role", None, &[], false, None)
            .await
            .expect("create role");

        let user_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, UserType::Person);
        user.role_ids = vec![role.id.clone()];
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert user");

        let group = Group {
            id: Uuid::new_v4().to_string(),
            name: "Test Group".to_string(),
            slug: "test-group".to_string(),
            description: None,
            role_ids: vec![role.id.clone()],
            parent_group_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.collection::<Group>(GROUPS)
            .insert_one(&group)
            .await
            .expect("insert group");

        delete_role(&db, &role.id).await.expect("delete");

        let updated_user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .expect("query user")
            .expect("user exists");
        assert!(!updated_user.role_ids.contains(&role.id));

        let updated_group = db
            .collection::<Group>(GROUPS)
            .find_one(doc! { "_id": &group.id })
            .await
            .expect("query group")
            .expect("group exists");
        assert!(!updated_group.role_ids.contains(&role.id));

        let gone = get_role(&db, &role.id).await;
        assert!(matches!(gone, Err(AppError::RoleNotFound(_))));
    }

    #[tokio::test]
    async fn assign_role_to_user_happy_path() {
        let Some(db) = connect_test_database("role_assign_ok").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(&db, "Assignable", "assignable", None, &[], false, None)
            .await
            .expect("create");

        let user_id = Uuid::new_v4().to_string();
        let user = test_user(&user_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert user");

        assign_role_to_user(&db, &user_id, &role.id)
            .await
            .expect("assign");

        let updated = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .expect("query")
            .expect("user");
        assert!(updated.role_ids.contains(&role.id));
    }

    #[tokio::test]
    async fn assign_role_to_user_role_not_found() {
        let Some(db) = connect_test_database("role_assign_rnf").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let user_id = Uuid::new_v4().to_string();
        let user = test_user(&user_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert user");

        let err = assign_role_to_user(&db, &user_id, "nonexistent-role")
            .await
            .expect_err("must fail");
        assert!(matches!(err, AppError::RoleNotFound(_)));
    }

    #[tokio::test]
    async fn assign_role_to_user_user_not_found() {
        let Some(db) = connect_test_database("role_assign_unf").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(&db, "Role", "role-unf", None, &[], false, None)
            .await
            .expect("create");

        let err = assign_role_to_user(&db, "nonexistent-user", &role.id)
            .await
            .expect_err("must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn assign_role_to_user_already_assigned() {
        let Some(db) = connect_test_database("role_assign_dup").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(&db, "DupRole", "dup-role", None, &[], false, None)
            .await
            .expect("create");

        let user_id = Uuid::new_v4().to_string();
        let user = test_user(&user_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert user");

        assign_role_to_user(&db, &user_id, &role.id)
            .await
            .expect("first assign");

        let err = assign_role_to_user(&db, &user_id, &role.id)
            .await
            .expect_err("second assign must fail");
        assert!(matches!(err, AppError::RoleAlreadyAssigned));
    }

    #[tokio::test]
    async fn revoke_role_from_user_happy_path() {
        let Some(db) = connect_test_database("role_revoke_ok").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(&db, "Revokable", "revokable", None, &[], false, None)
            .await
            .expect("create");

        let user_id = Uuid::new_v4().to_string();
        let user = test_user(&user_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert");

        assign_role_to_user(&db, &user_id, &role.id)
            .await
            .expect("assign");
        revoke_role_from_user(&db, &user_id, &role.id)
            .await
            .expect("revoke");

        let updated = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .expect("query")
            .expect("user");
        assert!(!updated.role_ids.contains(&role.id));
    }

    #[tokio::test]
    async fn get_user_roles_returns_roles() {
        let Some(db) = connect_test_database("role_get_user").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(
            &db,
            "UserRole",
            "user-role",
            None,
            &["r".to_string()],
            false,
            None,
        )
        .await
        .expect("create");

        let user_id = Uuid::new_v4().to_string();
        let user = test_user(&user_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert");

        assign_role_to_user(&db, &user_id, &role.id)
            .await
            .expect("assign");

        let roles = get_user_roles(&db, &user_id).await.expect("get roles");
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].slug, "user-role");
    }

    #[tokio::test]
    async fn get_user_roles_empty_when_none() {
        let Some(db) = connect_test_database("role_get_empty").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let user_id = Uuid::new_v4().to_string();
        let user = test_user(&user_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert");

        let roles = get_user_roles(&db, &user_id).await.expect("get roles");
        assert!(roles.is_empty());
    }

    #[tokio::test]
    async fn bulk_assign_role_assigns_to_multiple_users() {
        let Some(db) = connect_test_database("role_bulk_ok").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };

        let role = create_role(&db, "Bulk", "bulk-role", None, &[], false, None)
            .await
            .expect("create");

        let mut user_ids = Vec::new();
        for _ in 0..3 {
            let uid = Uuid::new_v4().to_string();
            let user = test_user(&uid, UserType::Person);
            db.collection::<User>(USERS)
                .insert_one(&user)
                .await
                .expect("insert");
            user_ids.push(uid);
        }

        let result = bulk_assign_role(&db, &role.id, Some(&user_ids))
            .await
            .expect("bulk assign");
        assert_eq!(result.assigned_count, 3);
        assert_eq!(result.already_assigned_count, 0);

        let result2 = bulk_assign_role(&db, &role.id, Some(&user_ids))
            .await
            .expect("bulk assign again");
        assert_eq!(result2.assigned_count, 0);
        assert_eq!(result2.already_assigned_count, 3);
    }

    #[tokio::test]
    async fn get_default_role_ids_returns_defaults() {
        let Some(db) = connect_test_database("role_defaults").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        seed_system_roles(&db).await.expect("seed");

        let default_ids = get_default_role_ids(&db).await.expect("get defaults");
        assert!(!default_ids.is_empty());

        let user_role = db
            .collection::<Role>(ROLES)
            .find_one(doc! { "slug": PLATFORM_USER_ROLE_SLUG })
            .await
            .expect("query")
            .expect("user role");
        assert!(default_ids.contains(&user_role.id));
    }
}
