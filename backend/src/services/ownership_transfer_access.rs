use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};

use crate::{
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS},
        org_membership::{
            COLLECTION_NAME as MEMBERSHIPS, MemberScopeSource, OrgMembership, OrgRole,
        },
        org_role_scope::{COLLECTION_NAME as ROLE_SCOPES, OrgRoleScope},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
};

use super::ownership_transfer_service::ResourceKind;

fn forbidden() -> AppError {
    AppError::Forbidden(
        "Asset ownership or organization admin access is required to transfer ownership".into(),
    )
}

async fn resource_document(
    db: &Database,
    session: &mut ClientSession,
    kind: ResourceKind,
    id: &str,
) -> AppResult<Document> {
    db.collection::<Document>(kind.collection())
        .find_one(doc! { "_id": id, "is_active": true })
        .projection(doc! { "user_id": 1, "owner_user_id": 1, "created_by": 1, "name": 1, "label": 1, "slug": 1, "platform": 1 })
        .session(session).await?
        .ok_or_else(|| AppError::NotFound("Active resource not found".into()))
}

fn owner_from_document(resource: &Document, kind: ResourceKind) -> AppResult<String> {
    let owner = match kind {
        ResourceKind::ChannelBot => resource.get_str("user_id"),
        ResourceKind::Service => resource
            .get_str("owner_user_id")
            .or_else(|_| resource.get_str("created_by")),
    };
    owner
        .map(str::to_owned)
        .map_err(|_| AppError::Internal("Resource owner missing".into()))
}

pub async fn resource_owner(
    db: &Database,
    session: &mut ClientSession,
    kind: ResourceKind,
    id: &str,
) -> AppResult<String> {
    owner_from_document(&resource_document(db, session, kind, id).await?, kind)
}

async fn fence(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    id: &str,
) -> AppResult<()> {
    let result = db
        .collection::<Document>(collection)
        .update_one(
            doc! { "_id": id },
            doc! { "$inc": { "ownership_transfer_revision": 1_i64 } },
        )
        .session(session)
        .await?;
    if result.matched_count != 1 {
        return Err(forbidden());
    }
    Ok(())
}

/// Returns whether the caller has platform-wide authority. Agent keys always
/// act through their owner's asset authority, even when that person is an admin.
pub async fn authorize(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    api_key_id: Option<&str>,
    owner: &str,
    for_commit: bool,
) -> AppResult<bool> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": actor, "is_active": true })
        .session(&mut *session)
        .await?
        .ok_or_else(forbidden)?;
    if let Some(key_id) = api_key_id {
        let key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": key_id, "user_id": actor, "is_active": true })
            .session(&mut *session)
            .await?
            .ok_or_else(forbidden)?;
        if key.purpose != ApiKeyPurpose::General
            || key.expires_at.is_some_and(|expiry| expiry <= Utc::now())
            || !key
                .scopes
                .split_whitespace()
                .any(|scope| matches!(scope, "write" | "admin"))
            || !key.allow_all_services
        {
            return Err(AppError::Forbidden("Ownership transfer requires a general Agent Key with write or admin scope and unrestricted service management".into()));
        }
        if for_commit {
            fence(db, session, API_KEYS, key_id).await?;
        }
    } else if user.user_type != UserType::Person {
        return Err(forbidden());
    }
    let platform_admin = api_key_id.is_none()
        && super::role_service::resolve_platform_role(db, &user)
            .await?
            .is_admin();
    if !platform_admin && actor != owner {
        let org = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": owner, "user_type": "org", "is_active": true })
            .session(&mut *session)
            .await?
            .ok_or_else(forbidden)?;
        let membership = db.collection::<OrgMembership>(MEMBERSHIPS)
            .find_one(doc! { "org_user_id": &org.id, "member_user_id": actor, "revoked_at": null, "role": "admin" })
            .session(&mut *session).await?.ok_or_else(forbidden)?;
        let scope = if membership.scope_source == MemberScopeSource::Inherit {
            let role_scope = db
                .collection::<OrgRoleScope>(ROLE_SCOPES)
                .find_one(doc! { "org_user_id": owner, "role": OrgRole::Admin.as_str() })
                .session(&mut *session)
                .await?;
            if let Some(ref row) = role_scope {
                if for_commit {
                    fence(db, session, ROLE_SCOPES, &row.id).await?;
                }
            }
            role_scope.and_then(|row| row.allowed_service_ids)
        } else {
            membership.allowed_service_ids
        };
        // These scopes identify connected UserServices, not catalog definitions
        // or bots. They cannot confer ownership rights over unrelated assets.
        if scope.is_some() {
            return Err(forbidden());
        }
        if for_commit {
            fence(db, session, MEMBERSHIPS, &membership.id).await?;
            fence(db, session, USERS, owner).await?;
        }
    }
    if for_commit {
        fence(db, session, USERS, actor).await?;
    }
    Ok(platform_admin)
}

pub async fn require_resource_access(
    db: &Database,
    actor: &str,
    api_key_id: Option<&str>,
    kind: ResourceKind,
    id: &str,
) -> AppResult<(String, bool, Document)> {
    let mut session = db.client().start_session().await?;
    let resource = resource_document(db, &mut session, kind, id).await?;
    let owner = owner_from_document(&resource, kind)?;
    let admin = authorize(db, &mut session, actor, api_key_id, &owner, false).await?;
    Ok((owner, admin, resource))
}

pub async fn destination_candidates(
    db: &Database,
    actor: &str,
    owner: &str,
    admin: bool,
    user_type: &str,
    search: &str,
    offset: u64,
) -> AppResult<(Vec<Document>, u64)> {
    if !matches!(user_type, "person" | "org") || search.len() > 200 || offset > 100_000 {
        return Err(AppError::ValidationError(
            "Invalid destination search".into(),
        ));
    }
    let search = search.trim();
    let mut filter = doc! { "user_type": user_type, "is_active": true };
    if !admin {
        let mut known = vec![actor.to_owned()];
        let memberships: Vec<OrgMembership> = db.collection::<OrgMembership>(MEMBERSHIPS)
            .find(doc! { "revoked_at": null, "$or": [ { "member_user_id": actor }, { "org_user_id": owner } ] })
            .await?.try_collect().await?;
        for member in memberships {
            if member.member_user_id == actor {
                known.push(member.org_user_id);
            } else {
                known.push(member.member_user_id);
            }
        }
        filter.insert(
            "$or",
            bson::to_bson(&vec![
                doc! { "_id": { "$in": known } },
                doc! { "_id": search },
                doc! { "email": search.to_lowercase() },
            ])
            .map_err(|e| AppError::Internal(e.to_string()))?,
        );
    }
    if !search.is_empty() {
        let pattern = regex::escape(search);
        filter = doc! { "$and": [filter, { "$or": [
            { "_id": search },
            { "display_name": { "$regex": &pattern, "$options": "i" } },
            { "email": { "$regex": &pattern, "$options": "i" } },
        ] }] };
    }
    let users = db.collection::<Document>(USERS);
    let total = users.count_documents(filter.clone()).await?;
    let rows = users
        .find(filter)
        .projection(doc! { "_id": 1, "display_name": 1, "email": 1 })
        .sort(doc! { "display_name": 1, "_id": 1 })
        .skip(offset)
        .limit(20)
        .await?
        .try_collect()
        .await?;
    Ok((rows, total))
}
