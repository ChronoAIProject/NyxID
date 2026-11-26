use super::{ORG_MEMBERSHIPS, USERS, get_org_user_with_session, is_duplicate_key_error};
use crate::{
    errors::{AppError, AppResult},
    models::{
        org_membership::{MemberScopeSource, OrgMembership, OrgRole},
        user::User,
    },
    services::{
        audit_service::AuditActor, execution_permission_enrollment, node_service,
        org_channel_service, org_membership_authority,
    },
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};
use uuid::Uuid;

pub(crate) struct Creation<'a> {
    org_user_id: &'a str,
    member_user_id: &'a str,
    role: OrgRole,
    scope_source: MemberScopeSource,
    allowed_service_ids: Option<Vec<String>>,
}

pub(crate) async fn prepare_creation<'a>(
    db: &Database,
    org_user_id: &'a str,
    member_user_id: &'a str,
    role: OrgRole,
    scope_source: MemberScopeSource,
    allowed_service_ids: Option<Vec<String>>,
    mut session: Option<&mut ClientSession>,
) -> AppResult<Creation<'a>> {
    let allowed_service_ids = if scope_source == MemberScopeSource::Inherit {
        None
    } else {
        allowed_service_ids
    };

    // Validate the org actually exists and is an org.
    let _ = get_org_user_with_session(db, org_user_id, session.as_deref_mut()).await?;
    // Validate the member exists and is a person.
    let users = db.collection::<User>(USERS);
    let member = users.find_one(doc! { "_id": member_user_id });
    let member = match session {
        Some(session) => member.session(session).await?,
        None => member.await?,
    }
    .ok_or_else(|| AppError::NotFound(format!("user {member_user_id}")))?;
    if !member.user_type.is_person() {
        return Err(AppError::ValidationError(
            "Members must be person accounts, not orgs".to_string(),
        ));
    }

    Ok(Creation {
        org_user_id,
        member_user_id,
        role,
        scope_source,
        allowed_service_ids,
    })
}

pub(crate) async fn create_in_session(
    db: &Database,
    creation: Creation<'_>,
    transaction: &mut ClientSession,
) -> AppResult<OrgMembership> {
    let Creation {
        org_user_id,
        member_user_id,
        role,
        scope_source,
        allowed_service_ids,
    } = creation;

    org_membership_authority::fence_live_member(db, member_user_id, &mut *transaction).await?;
    org_membership_authority::fence_org(db, org_user_id, &mut *transaction).await?;
    let collection = db.collection::<OrgMembership>(ORG_MEMBERSHIPS);

    // Look up any existing row (active or revoked) for this pair.
    let existing = collection
        .find_one(doc! {
            "org_user_id": org_user_id,
            "member_user_id": member_user_id,
        })
        .session(&mut *transaction)
        .await?;

    if let Some(row) = existing {
        if row.is_active() {
            return Err(AppError::Conflict(
                "User is already a member of this org".to_string(),
            ));
        }

        // BSON stores milliseconds; every rejoin must have a distinct authority epoch.
        let now = Utc::now().max(row.created_at + chrono::Duration::milliseconds(1));
        let now_bson = bson::DateTime::from_chrono(now);
        let allowed = match &allowed_service_ids {
            None => bson::Bson::Null,
            Some(ids) => bson::to_bson(ids).map_err(|e| AppError::Internal(e.to_string()))?,
        };
        let changed = collection
            .update_one(
                doc! { "_id": &row.id, "revoked_at": row.revoked_at.map(bson::DateTime::from_chrono),
                    "created_at": bson::DateTime::from_chrono(row.created_at) },
                doc! { "$set": {
                    "role": role.as_str(),
                    "scope_source": scope_source.as_str(),
                    "allowed_service_ids": allowed,
                    "revoked_at": bson::Bson::Null,
                    "created_at": now_bson,
                }},
            )
            .session(&mut *transaction)
            .await?;
        if changed.matched_count != 1 {
            return Err(AppError::Conflict(
                "Membership changed; retry the invitation".into(),
            ));
        }
        execution_permission_enrollment::lifecycle::apply(
            db,
            member_user_id,
            execution_permission_enrollment::lifecycle::Change::Membership {
                org: org_user_id,
                role: Some(role),
            },
            &mut *transaction,
        )
        .await?;
        return Ok(OrgMembership {
            id: row.id,
            org_user_id: org_user_id.to_string(),
            member_user_id: member_user_id.to_string(),
            role,
            scope_source,
            allowed_service_ids,
            created_at: now,
            revoked_at: None,
        });
    }

    let now = Utc::now();
    let membership = OrgMembership {
        id: Uuid::new_v4().to_string(),
        org_user_id: org_user_id.to_string(),
        member_user_id: member_user_id.to_string(),
        role,
        scope_source,
        allowed_service_ids,
        created_at: now,
        revoked_at: None,
    };

    match collection
        .insert_one(&membership)
        .session(&mut *transaction)
        .await
    {
        Ok(_) => {
            execution_permission_enrollment::lifecycle::apply(
                db,
                member_user_id,
                execution_permission_enrollment::lifecycle::Change::Membership {
                    org: org_user_id,
                    role: Some(role),
                },
                &mut *transaction,
            )
            .await?;
            Ok(membership)
        }
        Err(e) if is_duplicate_key_error(&e) => Err(AppError::Conflict(
            "User is already a member of this org".to_string(),
        )),
        Err(e) => Err(AppError::DatabaseError(e)),
    }
}

pub(super) async fn create(
    db: &Database,
    org_user_id: &str,
    member_user_id: &str,
    role: OrgRole,
    scope_source: MemberScopeSource,
    allowed_service_ids: Option<Vec<String>>,
    actor: Option<&AuditActor>,
) -> AppResult<OrgMembership> {
    let creation = prepare_creation(
        db,
        org_user_id,
        member_user_id,
        role,
        scope_source,
        allowed_service_ids,
        None,
    )
    .await?;
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = create_and_audit(db, creation, actor, &mut transaction).await;
    let membership = org_membership_authority::finish(transaction, result).await?;
    // The membership has committed; scheduled credits are best effort.
    crate::services::billing::schedules::eligibility::after_org_joined(
        db,
        &membership.org_user_id,
        &membership.member_user_id,
        membership.created_at,
    )
    .await;
    Ok(membership)
}
async fn create_and_audit(
    db: &Database,
    creation: Creation<'_>,
    actor: Option<&AuditActor>,
    transaction: &mut ClientSession,
) -> AppResult<OrgMembership> {
    let membership = create_in_session(db, creation, transaction).await?;
    super::audit::member(db, actor, "org_member_added", &membership, transaction).await?;
    Ok(membership)
}

pub(super) async fn update_membership(
    db: &mongodb::Database,
    membership_id: &str,
    role: Option<OrgRole>,
    scope_source: Option<MemberScopeSource>,
    allowed_service_ids: Option<Option<Vec<String>>>,
    actor: Option<&AuditActor>,
) -> AppResult<OrgMembership> {
    let mut update = doc! {};
    if let Some(role) = role {
        update.insert("role", role.as_str());
    }
    match scope_source {
        Some(MemberScopeSource::Inherit) => {
            update.insert("scope_source", MemberScopeSource::Inherit.as_str());
            update.insert("allowed_service_ids", bson::Bson::Null);
        }
        Some(MemberScopeSource::Override) => {
            update.insert("scope_source", MemberScopeSource::Override.as_str());
            match allowed_service_ids {
                None => {
                    update.insert("allowed_service_ids", bson::Bson::Null);
                }
                Some(None) => {
                    update.insert("allowed_service_ids", bson::Bson::Null);
                }
                Some(Some(ids)) => {
                    update.insert("allowed_service_ids", ids);
                }
            };
        }
        None => {
            if let Some(scope) = allowed_service_ids {
                update.insert("scope_source", MemberScopeSource::Override.as_str());
                match scope {
                    None => update.insert("allowed_service_ids", bson::Bson::Null),
                    Some(ids) => update.insert("allowed_service_ids", ids),
                };
            }
        }
    }
    if update.is_empty() {
        return Err(AppError::BadRequest("No fields to update".to_string()));
    }

    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result =
        update_membership_in_session(db, membership_id, role, update, actor, &mut transaction)
            .await;
    org_membership_authority::finish(transaction, result).await
}
async fn update_membership_in_session(
    db: &Database,
    membership_id: &str,
    role: Option<OrgRole>,
    update: Document,
    actor: Option<&AuditActor>,
    transaction: &mut ClientSession,
) -> AppResult<OrgMembership> {
    let current = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find_one(doc! {"_id":membership_id,"revoked_at":null})
        .session(&mut *transaction)
        .await?
        .ok_or_else(|| AppError::NotFound("Active membership not found".into()))?;
    org_membership_authority::fence_org(db, &current.org_user_id, &mut *transaction).await?;
    if role.is_some_and(|role| role != OrgRole::Admin) && current.role == OrgRole::Admin {
        crate::services::org_service_approvers::prune(
            db,
            &current.org_user_id,
            &current.member_user_id,
            transaction,
        )
        .await?;
        org_membership_authority::ensure_other_admin(
            db,
            &current.org_user_id,
            &current.member_user_id,
            &mut *transaction,
        )
        .await?;
        node_service::retire_org_registrations_in_session(
            db,
            &current.org_user_id,
            &current.member_user_id,
            &mut *transaction,
        )
        .await?;
        crate::services::org_invite_service::retire_issuer_in_session(
            db,
            &current.org_user_id,
            &current.member_user_id,
            actor,
            transaction,
        )
        .await?;
    }
    if role.is_some_and(|r| !r.can_proxy()) {
        crate::services::org_agent_service::prune_member(
            db,
            transaction,
            &current.org_user_id,
            &current.member_user_id,
        )
        .await?;
        org_channel_service::lifecycle::revoke_member_in_session(
            db,
            &mut *transaction,
            &current.org_user_id,
            &current.member_user_id,
        )
        .await?;
    }
    if role == Some(OrgRole::Admin) {
        org_membership_authority::fence_live_member(db, &current.member_user_id, &mut *transaction)
            .await?;
    }
    let updated = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find_one_and_update(
            doc! {"_id": membership_id,"revoked_at":null},
            doc! {"$set":update},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *transaction)
        .await?
        .ok_or_else(|| AppError::Conflict("Membership changed; reload before editing".into()))?;
    execution_permission_enrollment::lifecycle::apply(
        db,
        &current.member_user_id,
        execution_permission_enrollment::lifecycle::Change::Membership {
            org: &current.org_user_id,
            role: Some(updated.role),
        },
        &mut *transaction,
    )
    .await?;
    super::audit::member(db, actor, "org_member_updated", &updated, transaction).await?;
    Ok(updated)
}

pub(super) async fn revoke_membership(
    db: &mongodb::Database,
    org_user_id: &str,
    member_user_id: &str,
    actor: Option<&AuditActor>,
) -> AppResult<()> {
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result =
        revoke_membership_in_session(db, org_user_id, member_user_id, actor, &mut transaction)
            .await;
    org_membership_authority::finish(transaction, result).await?;
    crate::services::billing::schedules::eligibility::after_org_exited(
        db,
        org_user_id,
        member_user_id,
    );
    Ok(())
}
async fn revoke_membership_in_session(
    db: &Database,
    org_user_id: &str,
    member_user_id: &str,
    actor: Option<&AuditActor>,
    transaction: &mut ClientSession,
) -> AppResult<()> {
    org_membership_authority::fence_org(db, org_user_id, &mut *transaction).await?;
    let current = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find_one(doc! {
            "org_user_id":org_user_id,"member_user_id":member_user_id,"revoked_at":null
        })
        .session(&mut *transaction)
        .await?
        .ok_or_else(|| AppError::NotFound("Active membership not found".into()))?;
    if current.role == OrgRole::Admin {
        org_membership_authority::ensure_other_admin(
            db,
            org_user_id,
            member_user_id,
            &mut *transaction,
        )
        .await?;
        node_service::retire_org_registrations_in_session(
            db,
            org_user_id,
            member_user_id,
            &mut *transaction,
        )
        .await?;
        crate::services::org_invite_service::retire_issuer_in_session(
            db,
            org_user_id,
            member_user_id,
            actor,
            transaction,
        )
        .await?;
    }
    let result = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .update_one(
            doc! {"_id":&current.id,"revoked_at":null},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .session(&mut *transaction)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Membership changed; reload before removal".into(),
        ));
    }
    crate::services::org_service_approvers::prune(db, org_user_id, member_user_id, transaction)
        .await?;
    crate::services::org_agent_service::prune_member(db, transaction, org_user_id, member_user_id)
        .await?;
    org_channel_service::lifecycle::revoke_member_in_session(
        db,
        &mut *transaction,
        org_user_id,
        member_user_id,
    )
    .await?;
    execution_permission_enrollment::lifecycle::apply(
        db,
        member_user_id,
        execution_permission_enrollment::lifecycle::Change::Membership {
            org: org_user_id,
            role: None,
        },
        &mut *transaction,
    )
    .await?;
    super::audit::member(db, actor, "org_member_revoked", &current, transaction).await?;
    Ok(())
}
