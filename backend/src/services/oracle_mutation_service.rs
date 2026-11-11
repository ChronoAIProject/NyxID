//! Database serialization shared by every Oracle writer and pool lifecycle change.
//!
//! A transaction writes the existing enrollment fence before touching related
//! records. MongoDB write conflicts retry the whole operation with a fresh
//! snapshot. Never catch a database error inside an operation and continue.
use crate::{
    errors::{AppError, AppResult},
    models::{
        oracle_pool::{COLLECTION_NAME as POOLS, OraclePool},
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database, options::ReturnDocument};

pub async fn lock_pool(
    db: &Database,
    session: &mut ClientSession,
    id: &str,
) -> AppResult<OraclePool> {
    db.collection::<OraclePool>(POOLS)
        .find_one_and_update(
            doc! {"_id": id, "deleted_at": null},
            doc! {"$inc": {"worker_enrollment_epoch": 1_i64}},
        )
        .return_document(ReturnDocument::After)
        .session(session)
        .await?
        .ok_or_else(|| AppError::OraclePoolNotFound(id.into()))
}

pub async fn lock_expected_pool(
    db: &Database,
    session: &mut ClientSession,
    expected: &OraclePool,
    require_active: bool,
) -> AppResult<OraclePool> {
    let live = lock_pool(db, session, &expected.id).await?;
    if live.lifecycle_version != expected.lifecycle_version
        || live.user_id != expected.user_id
        || live.worker_token_hash != expected.worker_token_hash
    {
        return Err(AppError::Conflict(
            "oracle pool changed; refresh before retrying".into(),
        ));
    }
    if require_active && !live.is_active {
        return Err(AppError::OraclePoolInactive(live.slug));
    }
    Ok(live)
}

/// Write the same user/membership documents changed by account disable and
/// membership revocation. Read checks alone do not fence concurrent revocation.
pub async fn lock_manager(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    owner: &str,
) -> AppResult<User> {
    let users = db.collection::<User>(USERS);
    let actor_live = users
        .update_one(
            doc! {"_id": actor, "is_active": true},
            doc! {"$inc": {"oracle_enrollment_epoch": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if actor_live.matched_count != 1 {
        return Err(AppError::Forbidden("active account required".into()));
    }
    let owner = users
        .find_one_and_update(
            doc! {"_id": owner, "is_active": true},
            doc! {"$inc": {"oracle_enrollment_epoch": 1_i64}},
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Forbidden("active pool owner required".into()))?;
    match owner.user_type {
        UserType::Person if owner.id == actor => {}
        UserType::Org if owner.id == actor => {}
        UserType::Org => {
            let membership = db
                .collection::<OrgMembership>(MEMBERSHIPS)
                .find_one_and_update(
                    doc! {"org_user_id": &owner.id, "member_user_id": actor, "revoked_at": null},
                    doc! {"$inc": {"oracle_enrollment_epoch": 1_i64}},
                )
                .return_document(ReturnDocument::After)
                .session(&mut *session)
                .await?;
            if !membership.is_some_and(|m| m.is_active() && m.role.can_admin()) {
                return Err(AppError::Forbidden("org admin access required".into()));
            }
        }
        _ => {
            return Err(AppError::Forbidden(
                "personal destination must be your own active person account".into(),
            ));
        }
    }
    Ok(owner)
}

pub async fn lock_inventory(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
) -> AppResult<()> {
    // Production owners exist. Keeping this a non-upserting update avoids
    // manufacturing users if an internal caller passes an unknown owner.
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id": owner},
            doc! {"$inc": {"oracle_enrollment_epoch": 1_i64}},
        )
        .session(session)
        .await?;
    Ok(())
}

pub async fn lookup_pool(
    db: &Database,
    session: &mut ClientSession,
    id: &str,
    include_deleted: bool,
) -> AppResult<OraclePool> {
    let mut filter = doc! {"$or": [{"_id": id}, {"slug": id}]};
    if !include_deleted {
        filter.insert("deleted_at", bson::Bson::Null);
    }
    db.collection::<OraclePool>(POOLS)
        .find_one(filter)
        .session(session)
        .await?
        .ok_or_else(|| AppError::OraclePoolNotFound(id.into()))
}

/// Visibility authorization inside the mutation snapshot, including active
/// accounts and live org membership. Managers keep the shared owner semantics.
pub async fn lock_submitter(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    pool: &OraclePool,
) -> AppResult<()> {
    use crate::models::oracle_pool::OraclePoolVisibility;
    if pool.visibility == OraclePoolVisibility::Private || actor == pool.user_id {
        lock_manager(db, session, actor, &pool.user_id).await?;
        return Ok(());
    }
    let actor_update = db
        .collection::<User>(USERS)
        .update_one(
            doc! {"_id":actor,"is_active":true},
            doc! {"$inc":{"oracle_enrollment_epoch":1_i64}},
        )
        .session(&mut *session)
        .await?;
    if actor_update.matched_count != 1 {
        return Err(AppError::Forbidden("active account required".into()));
    }
    if pool.visibility == OraclePoolVisibility::Org {
        let owner = db
            .collection::<User>(USERS)
            .update_one(
                doc! {"_id":&pool.user_id,"user_type":"org","is_active":true},
                doc! {"$inc":{"oracle_enrollment_epoch":1_i64}},
            )
            .session(&mut *session)
            .await?;
        let membership = db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .find_one_and_update(
                doc! {"org_user_id":&pool.user_id,"member_user_id":actor,"revoked_at":null},
                doc! {"$inc":{"oracle_enrollment_epoch":1_i64}},
            )
            .return_document(ReturnDocument::After)
            .session(&mut *session)
            .await?;
        if owner.matched_count != 1 || !membership.is_some_and(|m| m.is_active()) {
            return Err(AppError::Forbidden("active org membership required".into()));
        }
    }
    Ok(())
}
