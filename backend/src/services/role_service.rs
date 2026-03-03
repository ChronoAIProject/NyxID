use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::role::{COLLECTION_NAME as ROLES, Role};
use crate::models::user::{COLLECTION_NAME as USERS, User};

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
    if let Some(new_slug) = slug {
        if new_slug != existing.slug {
            let dup = db
                .collection::<Role>(ROLES)
                .find_one(doc! { "slug": new_slug })
                .await?;
            if dup.is_some() {
                return Err(AppError::DuplicateSlug(new_slug.to_string()));
            }
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

/// Seed system roles ("admin", "user") if they don't exist.
pub async fn seed_system_roles(db: &mongodb::Database) -> AppResult<()> {
    let now = Utc::now();

    // Seed "admin" role
    let admin_exists = db
        .collection::<Role>(ROLES)
        .find_one(doc! { "slug": "admin" })
        .await?;

    if admin_exists.is_none() {
        let admin_role = Role {
            id: Uuid::new_v4().to_string(),
            name: "Admin".to_string(),
            slug: "admin".to_string(),
            description: Some("System administrator with full access".to_string()),
            permissions: vec!["*".to_string()],
            is_default: false,
            is_system: true,
            client_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<Role>(ROLES).insert_one(&admin_role).await?;
        tracing::info!("Seeded system role: admin");
    }

    // Seed "user" role
    let user_exists = db
        .collection::<Role>(ROLES)
        .find_one(doc! { "slug": "user" })
        .await?;

    if user_exists.is_none() {
        let user_role = Role {
            id: Uuid::new_v4().to_string(),
            name: "User".to_string(),
            slug: "user".to_string(),
            description: Some("Default user role".to_string()),
            permissions: vec![],
            is_default: true,
            is_system: true,
            client_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<Role>(ROLES).insert_one(&user_role).await?;
        tracing::info!("Seeded system role: user");
    }

    Ok(())
}
