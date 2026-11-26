use bson::{Document, doc};
use mongodb::{ClientSession, Database};

use super::{GROUPS, Group, USERS};
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

pub(super) fn group_intent(
    actor: &AuditActor,
    event: &str,
    id: &str,
    slug: Option<&str>,
) -> AppResult<SecurityEffectIntent> {
    let mut data = serde_json::json!({"group_id":required::text(TextField::Identifier,id)});
    if let Some(slug) = slug {
        data["group_slug"] = required::text(TextField::Identifier, slug).into();
    }
    required::intent_preserving_actor(actor, event, Some(data))
}

struct Create<'a> {
    db: Database,
    group: &'a Group,
    intent: SecurityEffectIntent,
}
pub(super) async fn create(db: &Database, group: &Group, actor: &AuditActor) -> AppResult<()> {
    let intent = group_intent(actor, "admin.group.created", &group.id, Some(&group.slug))?;
    let context = Create {
        db: db.clone(),
        group,
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
        .collection::<Group>(GROUPS)
        .insert_one(context.group)
        .session(&mut *session)
        .await?;
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}

struct UpdateGroup<'a> {
    db: Database,
    id: &'a str,
    update: Document,
    actor: &'a AuditActor,
}
pub(super) async fn update_group(
    db: &Database,
    id: &str,
    update: Document,
    actor: &AuditActor,
) -> AppResult<Group> {
    let context = UpdateGroup {
        db: db.clone(),
        id,
        update,
        actor,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(update_group_in_session(session, context))
    })
    .await
}
async fn update_group_in_session(
    session: &mut ClientSession,
    context: &UpdateGroup<'_>,
) -> AppResult<Group> {
    let result = context
        .db
        .collection::<Group>(GROUPS)
        .update_one(doc! {"_id":context.id}, doc! {"$set":&context.update})
        .session(&mut *session)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::GroupNotFound(context.id.to_owned()));
    }
    // The original response read also supplies the actual persisted slug for
    // the audit. It shares the transaction without adding a read.
    let group = context
        .db
        .collection::<Group>(GROUPS)
        .find_one(doc! {"_id":context.id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::GroupNotFound(context.id.to_owned()))?;
    let intent = group_intent(
        context.actor,
        "admin.group.updated",
        context.id,
        Some(&group.slug),
    )?;
    security_effect_service::enqueue_in_session(&context.db, session, &[intent]).await?;
    Ok(group)
}

struct Delete<'a> {
    db: Database,
    id: &'a str,
    intent: SecurityEffectIntent,
}
pub(super) async fn delete(db: &Database, id: &str, actor: &AuditActor) -> AppResult<()> {
    let intent = group_intent(actor, "admin.group.deleted", id, None)?;
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
    context
        .db
        .collection::<Document>(USERS)
        .update_many(
            doc! {"group_ids":context.id},
            doc! {"$pull":{"group_ids":context.id}},
        )
        .session(&mut *session)
        .await?;
    let result = context
        .db
        .collection::<Group>(GROUPS)
        .delete_one(doc! {"_id":context.id})
        .session(&mut *session)
        .await?;
    if result.deleted_count != 1 {
        return Err(AppError::GroupNotFound(context.id.to_owned()));
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
    Add,
    Remove,
}
impl Membership {
    pub(super) fn intent(
        self,
        actor: &AuditActor,
        user_id: &str,
        group_id: &str,
    ) -> AppResult<SecurityEffectIntent> {
        required::intent_preserving_actor(
            actor,
            match self {
                Self::Add => "admin.group.member_added",
                Self::Remove => "admin.group.member_removed",
            },
            Some(serde_json::json!({
                "target_user_id":required::text(TextField::Identifier,user_id),
                "group_id":required::text(TextField::Identifier,group_id),
            })),
        )
    }
}
struct UserGroup<'a> {
    db: Database,
    user_id: &'a str,
    update: Document,
    intent: SecurityEffectIntent,
    change: Membership,
}
pub(super) async fn user_group(
    db: &Database,
    user_id: &str,
    group_id: &str,
    change: Membership,
    actor: &AuditActor,
) -> AppResult<()> {
    let update = match change {
        Membership::Add => doc! {"$addToSet":{"group_ids":group_id}},
        Membership::Remove => doc! {"$pull":{"group_ids":group_id}},
    };
    let context = UserGroup {
        db: db.clone(),
        user_id,
        update,
        intent: change.intent(actor, user_id, group_id)?,
        change,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(user_group_in_session(session, context))
    })
    .await
}
async fn user_group_in_session(
    session: &mut ClientSession,
    context: &UserGroup<'_>,
) -> AppResult<()> {
    let result = context
        .db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id":context.user_id}, context.update.clone())
        .session(&mut *session)
        .await?;
    if matches!(context.change, Membership::Add) {
        if result.matched_count != 1 {
            return Err(AppError::NotFound("User not found".into()));
        }
        if result.modified_count != 1 {
            return Err(AppError::GroupMembershipExists);
        }
    } else if result.modified_count == 0 {
        // Removing an absent membership remains a successful no-op.
        return Ok(());
    }
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}
