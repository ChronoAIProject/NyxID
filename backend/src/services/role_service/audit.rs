use bson::{Document, doc};
use mongodb::{ClientSession, Database};

use super::{BulkAssignResult, GROUPS, ROLES, Role, USERS};
use crate::{
    errors::{AppError, AppResult},
    models::security_effect_intent::SecurityEffectIntent,
    services::{
        audit_service::{
            AuditActor,
            required::{self, TextField},
        },
        security_effect_service, transaction_service,
    },
};

pub(super) fn role_intent(
    actor: &AuditActor,
    event: &str,
    id: &str,
    slug: Option<&str>,
) -> AppResult<SecurityEffectIntent> {
    let mut data = serde_json::json!({"role_id":required::text(TextField::Identifier,id)});
    if let Some(slug) = slug {
        data["role_slug"] = required::text(TextField::Identifier, slug).into();
    }
    required::intent_preserving_actor(actor, event, Some(data))
}

struct Create<'a> {
    db: Database,
    role: &'a Role,
    intent: SecurityEffectIntent,
}
pub(super) async fn create(db: &Database, role: &Role, actor: &AuditActor) -> AppResult<()> {
    let intent = role_intent(actor, "admin.role.created", &role.id, Some(&role.slug))?;
    let context = Create {
        db: db.clone(),
        role,
        intent,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(create_in_session(session, context))
    })
    .await
}
async fn create_in_session(session: &mut ClientSession, context: &Create<'_>) -> AppResult<()> {
    context
        .db
        .collection::<Role>(ROLES)
        .insert_one(context.role)
        .session(&mut *session)
        .await?;
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}

struct UpdateRole<'a> {
    db: Database,
    id: &'a str,
    update: Document,
    actor: &'a AuditActor,
}
pub(super) async fn update_role(
    db: &Database,
    id: &str,
    update: Document,
    actor: &AuditActor,
) -> AppResult<Role> {
    let context = UpdateRole {
        db: db.clone(),
        id,
        update,
        actor,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(update_role_in_session(session, context))
    })
    .await
}
async fn update_role_in_session(
    session: &mut ClientSession,
    context: &UpdateRole<'_>,
) -> AppResult<Role> {
    let result = context
        .db
        .collection::<Role>(ROLES)
        .update_one(doc! {"_id":context.id}, doc! {"$set":&context.update})
        .session(&mut *session)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::RoleNotFound(context.id.to_owned()));
    }
    // The original response read also supplies the actual persisted slug for
    // the audit. It shares the transaction without adding a read.
    let role = context
        .db
        .collection::<Role>(ROLES)
        .find_one(doc! {"_id":context.id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::RoleNotFound(context.id.to_owned()))?;
    let intent = role_intent(
        context.actor,
        "admin.role.updated",
        context.id,
        Some(&role.slug),
    )?;
    security_effect_service::enqueue_in_session(&context.db, session, &[intent]).await?;
    Ok(role)
}

struct Delete<'a> {
    db: Database,
    id: &'a str,
    intent: SecurityEffectIntent,
}
pub(super) async fn delete(db: &Database, id: &str, actor: &AuditActor) -> AppResult<()> {
    let intent = role_intent(actor, "admin.role.deleted", id, None)?;
    let context = Delete {
        db: db.clone(),
        id,
        intent,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(delete_in_session(session, context))
    })
    .await
}
async fn delete_in_session(session: &mut ClientSession, context: &Delete<'_>) -> AppResult<()> {
    for collection in [USERS, GROUPS] {
        context
            .db
            .collection::<Document>(collection)
            .update_many(
                doc! {"role_ids":context.id},
                doc! {"$pull":{"role_ids":context.id}},
            )
            .session(&mut *session)
            .await?;
    }
    let result = context
        .db
        .collection::<Role>(ROLES)
        .delete_one(doc! {"_id":context.id})
        .session(&mut *session)
        .await?;
    if result.deleted_count != 1 {
        return Err(AppError::RoleNotFound(context.id.to_owned()));
    }
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}

#[derive(Clone, Copy)]
pub(super) enum Membership {
    Assign,
    Revoke,
}
impl Membership {
    pub(super) fn intent(
        self,
        actor: &AuditActor,
        user_id: &str,
        role_id: &str,
    ) -> AppResult<SecurityEffectIntent> {
        required::intent_preserving_actor(
            actor,
            match self {
                Self::Assign => "admin.role.assigned",
                Self::Revoke => "admin.role.revoked",
            },
            Some(serde_json::json!({
                "target_user_id":required::text(TextField::Identifier,user_id),
                "role_id":required::text(TextField::Identifier,role_id),
            })),
        )
    }
}
struct UserRole<'a> {
    db: Database,
    user_id: &'a str,
    update: Document,
    intent: SecurityEffectIntent,
    change: Membership,
}
pub(super) async fn user_role(
    db: &Database,
    user_id: &str,
    role_id: &str,
    change: Membership,
    actor: &AuditActor,
) -> AppResult<()> {
    let update = match change {
        Membership::Assign => doc! {"$addToSet":{"role_ids":role_id}},
        Membership::Revoke => doc! {"$pull":{"role_ids":role_id}},
    };
    let context = UserRole {
        db: db.clone(),
        user_id,
        update,
        intent: change.intent(actor, user_id, role_id)?,
        change,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(user_role_in_session(session, context))
    })
    .await
}
async fn user_role_in_session(
    session: &mut ClientSession,
    context: &UserRole<'_>,
) -> AppResult<()> {
    let result = context
        .db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id":context.user_id}, context.update.clone())
        .session(&mut *session)
        .await?;
    if matches!(context.change, Membership::Assign) {
        if result.matched_count != 1 {
            return Err(AppError::NotFound("User not found".into()));
        }
        if result.modified_count != 1 {
            return Err(AppError::RoleAlreadyAssigned);
        }
    } else if result.modified_count == 0 {
        // Revoking a missing assignment has always been a successful no-op.
        return Ok(());
    }
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}

pub(super) struct Bulk<'a> {
    pub db: Database,
    pub role_id: &'a str,
    pub user_ids: Option<&'a [String]>,
    pub already_assigned_count: u64,
    pub filter: Document,
    pub update: Document,
    pub actor: &'a AuditActor,
}
pub(super) fn bulk_intent(
    actor: &AuditActor,
    role_id: &str,
    all: bool,
    user_ids_count: usize,
    assigned_count: u64,
    already_assigned_count: u64,
) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "admin.role.bulk_assigned",
        Some(serde_json::json!({
            "role_id":required::text(TextField::Identifier,role_id),"all":all,"user_ids_count":user_ids_count,
            "assigned_count":assigned_count,"already_assigned_count":already_assigned_count,
        })),
    )
}
pub(super) async fn bulk(context: Bulk<'_>) -> AppResult<BulkAssignResult> {
    transaction_service::run_db_only(&context.db, &context, |session, context| {
        Box::pin(bulk_in_session(session, context))
    })
    .await
}
async fn bulk_in_session(
    session: &mut ClientSession,
    context: &Bulk<'_>,
) -> AppResult<BulkAssignResult> {
    let result = context
        .db
        .collection::<Document>(USERS)
        .update_many(context.filter.clone(), context.update.clone())
        .session(&mut *session)
        .await?;
    if result.modified_count > 0 {
        let intent = bulk_intent(
            context.actor,
            context.role_id,
            context.user_ids.is_none(),
            context.user_ids.map_or(0, <[String]>::len),
            result.modified_count,
            context.already_assigned_count,
        )?;
        security_effect_service::enqueue_in_session(&context.db, session, &[intent]).await?;
    }
    Ok(BulkAssignResult {
        assigned_count: result.modified_count,
        already_assigned_count: context.already_assigned_count,
    })
}
