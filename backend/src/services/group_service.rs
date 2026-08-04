use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::role::{COLLECTION_NAME as ROLES, Role};
use crate::models::user::{COLLECTION_NAME as USERS, User};

/// Create a new group.
pub async fn create_group(
    db: &mongodb::Database,
    name: &str,
    slug: &str,
    description: Option<&str>,
    role_ids: &[String],
    parent_group_id: Option<&str>,
) -> AppResult<Group> {
    crate::handlers::admin_helpers::validate_slug(slug)?;

    // Validate that all referenced role_ids exist
    if !role_ids.is_empty() {
        let role_id_strs: Vec<&str> = role_ids.iter().map(|s| s.as_str()).collect();
        let found_count = db
            .collection::<Role>(ROLES)
            .count_documents(doc! { "_id": { "$in": &role_id_strs } })
            .await?;
        if found_count != role_ids.len() as u64 {
            return Err(AppError::BadRequest(
                "One or more role_ids do not exist".to_string(),
            ));
        }
    }

    // Check for duplicate slug
    let existing = db
        .collection::<Group>(GROUPS)
        .find_one(doc! { "slug": slug })
        .await?;
    if existing.is_some() {
        return Err(AppError::DuplicateSlug(slug.to_string()));
    }

    let now = Utc::now();
    let group = Group {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        slug: slug.to_string(),
        description: description.map(String::from),
        role_ids: role_ids.to_vec(),
        parent_group_id: parent_group_id.map(String::from),
        created_at: now,
        updated_at: now,
    };

    db.collection::<Group>(GROUPS).insert_one(&group).await?;
    Ok(group)
}

/// Get a group by ID.
pub async fn get_group(db: &mongodb::Database, group_id: &str) -> AppResult<Group> {
    db.collection::<Group>(GROUPS)
        .find_one(doc! { "_id": group_id })
        .await?
        .ok_or_else(|| AppError::GroupNotFound(group_id.to_string()))
}

/// List groups with a default limit for safety.
pub async fn list_groups(db: &mongodb::Database) -> AppResult<Vec<Group>> {
    let groups: Vec<Group> = db
        .collection::<Group>(GROUPS)
        .find(doc! {})
        .sort(doc! { "name": 1 })
        .limit(200)
        .await?
        .try_collect()
        .await?;

    Ok(groups)
}

/// Update a group.
pub async fn update_group(
    db: &mongodb::Database,
    group_id: &str,
    name: Option<&str>,
    slug: Option<&str>,
    description: Option<&str>,
    role_ids: Option<&[String]>,
    parent_group_id: Option<Option<&str>>,
) -> AppResult<Group> {
    let existing = get_group(db, group_id).await?;

    // Validate slug format if changing
    if let Some(new_slug) = slug {
        crate::handlers::admin_helpers::validate_slug(new_slug)?;
    }

    // Check slug uniqueness if changing
    if let Some(new_slug) = slug
        && new_slug != existing.slug
    {
        let dup = db
            .collection::<Group>(GROUPS)
            .find_one(doc! { "slug": new_slug })
            .await?;
        if dup.is_some() {
            return Err(AppError::DuplicateSlug(new_slug.to_string()));
        }
    }

    // Validate that all referenced role_ids exist
    if let Some(r) = role_ids
        && !r.is_empty()
    {
        let role_id_strs: Vec<&str> = r.iter().map(|s| s.as_str()).collect();
        let found_count = db
            .collection::<Role>(ROLES)
            .count_documents(doc! { "_id": { "$in": &role_id_strs } })
            .await?;
        if found_count != r.len() as u64 {
            return Err(AppError::BadRequest(
                "One or more role_ids do not exist".to_string(),
            ));
        }
    }

    // Check for circular hierarchy if parent_group_id is being set
    if let Some(Some(new_parent_id)) = parent_group_id {
        // Validate that the parent group exists
        let _parent = get_group(db, new_parent_id).await?;
        check_circular_hierarchy(db, group_id, new_parent_id).await?;
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
    if let Some(r) = role_ids {
        update.insert("role_ids", r);
    }
    if let Some(p) = parent_group_id {
        match p {
            Some(pid) => {
                update.insert("parent_group_id", pid);
            }
            None => {
                update.insert("parent_group_id", bson::Bson::Null);
            }
        }
    }

    db.collection::<Group>(GROUPS)
        .update_one(doc! { "_id": group_id }, doc! { "$set": update })
        .await?;

    get_group(db, group_id).await
}

/// Delete a group. Blocks deletion if the group has child groups.
/// Removes group_id from all member users.
pub async fn delete_group(db: &mongodb::Database, group_id: &str) -> AppResult<()> {
    let _group = get_group(db, group_id).await?;

    // Block deletion if the group has child groups
    let children_count = db
        .collection::<Group>(GROUPS)
        .count_documents(doc! { "parent_group_id": group_id })
        .await?;
    if children_count > 0 {
        return Err(AppError::BadRequest(format!(
            "Cannot delete group with {children_count} child group(s). Move or delete children first."
        )));
    }

    // Remove group from all users
    db.collection::<User>(USERS)
        .update_many(
            doc! { "group_ids": group_id },
            doc! { "$pull": { "group_ids": group_id } },
        )
        .await?;

    db.collection::<Group>(GROUPS)
        .delete_one(doc! { "_id": group_id })
        .await?;

    Ok(())
}

/// Add a user to a group.
pub async fn add_member(db: &mongodb::Database, group_id: &str, user_id: &str) -> AppResult<()> {
    // Verify group exists
    let _group = get_group(db, group_id).await?;

    // Verify user exists and check membership
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if user.group_ids.contains(&group_id.to_string()) {
        return Err(AppError::GroupMembershipExists);
    }

    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": user_id },
            doc! { "$addToSet": { "group_ids": group_id } },
        )
        .await?;

    Ok(())
}

/// Remove a user from a group.
pub async fn remove_member(db: &mongodb::Database, group_id: &str, user_id: &str) -> AppResult<()> {
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": user_id },
            doc! { "$pull": { "group_ids": group_id } },
        )
        .await?;

    Ok(())
}

/// Get all members of a group.
pub async fn get_members(db: &mongodb::Database, group_id: &str) -> AppResult<Vec<User>> {
    // Verify group exists
    let _group = get_group(db, group_id).await?;

    let users: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "group_ids": group_id })
        .limit(200)
        .await?
        .try_collect()
        .await?;

    Ok(users)
}

/// Get all groups a user belongs to.
pub async fn get_user_groups(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<Group>> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if user.group_ids.is_empty() {
        return Ok(vec![]);
    }

    let groups: Vec<Group> = db
        .collection::<Group>(GROUPS)
        .find(doc! { "_id": { "$in": &user.group_ids } })
        .await?
        .try_collect()
        .await?;

    Ok(groups)
}

/// Maximum allowed depth for group hierarchy. If the parent chain exceeds this
/// depth, the hierarchy is rejected to prevent unbounded traversal.
const MAX_GROUP_HIERARCHY_DEPTH: usize = 10;

/// Check that setting parent_group_id does not create a circular hierarchy.
/// Walks up the parent chain (max `MAX_GROUP_HIERARCHY_DEPTH` levels) to detect cycles.
async fn check_circular_hierarchy(
    db: &mongodb::Database,
    group_id: &str,
    new_parent_id: &str,
) -> AppResult<()> {
    if group_id == new_parent_id {
        return Err(AppError::CircularGroupHierarchy);
    }

    let mut current_id = new_parent_id.to_string();
    for _ in 0..MAX_GROUP_HIERARCHY_DEPTH {
        let parent = db
            .collection::<Group>(GROUPS)
            .find_one(doc! { "_id": &current_id })
            .await?;

        match parent {
            Some(g) => {
                if let Some(pid) = g.parent_group_id {
                    if pid == group_id {
                        return Err(AppError::CircularGroupHierarchy);
                    }
                    current_id = pid;
                } else {
                    return Ok(());
                }
            }
            None => return Ok(()),
        }
    }

    // Max depth reached -- treat as circular to be safe
    Err(AppError::CircularGroupHierarchy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::role::Role;
    use crate::models::user::COLLECTION_NAME as USERS;
    use crate::test_utils::*;

    async fn insert_test_role(db: &mongodb::Database, role_id: &str) {
        let role = Role {
            id: role_id.to_string(),
            name: "Test Role".to_string(),
            slug: format!("test-role-{}", &role_id[..8]),
            description: None,
            permissions: vec!["read".to_string()],
            is_default: false,
            is_system: false,
            client_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.collection::<Role>(ROLES)
            .insert_one(&role)
            .await
            .expect("insert test role");
    }

    async fn insert_test_user(db: &mongodb::Database, user_id: &str) {
        let user = test_user(user_id, crate::models::user::UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert test user");
    }

    #[tokio::test]
    async fn test_create_and_get_group() {
        let Some(db) = connect_test_database("group_svc_create").await else {
            return;
        };
        let role_id = Uuid::new_v4().to_string();
        insert_test_role(&db, &role_id).await;

        let group = create_group(
            &db,
            "Engineering",
            "engineering",
            Some("Eng team"),
            std::slice::from_ref(&role_id),
            None,
        )
        .await
        .unwrap();

        assert_eq!(group.name, "Engineering");
        assert_eq!(group.slug, "engineering");
        assert_eq!(group.description.as_deref(), Some("Eng team"));
        assert_eq!(group.role_ids, vec![role_id]);
        assert!(group.parent_group_id.is_none());

        let fetched = get_group(&db, &group.id).await.unwrap();
        assert_eq!(fetched.id, group.id);
        assert_eq!(fetched.name, "Engineering");
    }

    #[tokio::test]
    async fn test_create_group_duplicate_slug() {
        let Some(db) = connect_test_database("group_svc_dup_slug").await else {
            return;
        };

        create_group(&db, "Team A", "team-a", None, &[], None)
            .await
            .unwrap();
        let err = create_group(&db, "Team B", "team-a", None, &[], None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::DuplicateSlug(_)));
    }

    #[tokio::test]
    async fn test_create_group_invalid_role_ids() {
        let Some(db) = connect_test_database("group_svc_bad_role").await else {
            return;
        };
        let fake_role = Uuid::new_v4().to_string();

        let err = create_group(&db, "Bad", "bad-group", None, &[fake_role], None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_update_group() {
        let Some(db) = connect_test_database("group_svc_update").await else {
            return;
        };

        let group = create_group(&db, "Original", "original", None, &[], None)
            .await
            .unwrap();

        let updated = update_group(
            &db,
            &group.id,
            Some("Renamed"),
            Some("renamed"),
            Some("New desc"),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.slug, "renamed");
        assert_eq!(updated.description.as_deref(), Some("New desc"));
    }

    #[tokio::test]
    async fn test_delete_group_removes_from_members() {
        let Some(db) = connect_test_database("group_svc_delete").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_test_user(&db, &user_id).await;

        let group = create_group(&db, "Deletable", "deletable", None, &[], None)
            .await
            .unwrap();
        add_member(&db, &group.id, &user_id).await.unwrap();

        let members_before = get_members(&db, &group.id).await.unwrap();
        assert_eq!(members_before.len(), 1);

        delete_group(&db, &group.id).await.unwrap();

        let user = db
            .collection::<User>(USERS)
            .find_one(mongodb::bson::doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert!(!user.group_ids.contains(&group.id));

        let err = get_group(&db, &group.id).await.unwrap_err();
        assert!(matches!(err, AppError::GroupNotFound(_)));
    }

    #[tokio::test]
    async fn test_delete_group_with_children_blocked() {
        let Some(db) = connect_test_database("group_svc_del_child").await else {
            return;
        };

        let parent = create_group(&db, "Parent", "parent-grp", None, &[], None)
            .await
            .unwrap();
        create_group(&db, "Child", "child-grp", None, &[], Some(&parent.id))
            .await
            .unwrap();

        let err = delete_group(&db, &parent.id).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_add_and_remove_member() {
        let Some(db) = connect_test_database("group_svc_member").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_test_user(&db, &user_id).await;

        let group = create_group(&db, "Members", "members-grp", None, &[], None)
            .await
            .unwrap();

        add_member(&db, &group.id, &user_id).await.unwrap();

        let members = get_members(&db, &group.id).await.unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id, user_id);

        let dup_err = add_member(&db, &group.id, &user_id).await.unwrap_err();
        assert!(matches!(dup_err, AppError::GroupMembershipExists));

        remove_member(&db, &group.id, &user_id).await.unwrap();

        let members_after = get_members(&db, &group.id).await.unwrap();
        assert!(members_after.is_empty());
    }

    #[tokio::test]
    async fn test_circular_hierarchy_detection() {
        let Some(db) = connect_test_database("group_svc_circ").await else {
            return;
        };

        let a = create_group(&db, "A", "group-a", None, &[], None)
            .await
            .unwrap();
        let b = create_group(&db, "B", "group-b", None, &[], Some(&a.id))
            .await
            .unwrap();

        let err = update_group(&db, &a.id, None, None, None, None, Some(Some(&b.id)))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::CircularGroupHierarchy));

        let self_err = update_group(&db, &a.id, None, None, None, None, Some(Some(&a.id)))
            .await
            .unwrap_err();
        assert!(matches!(self_err, AppError::CircularGroupHierarchy));
    }
}
