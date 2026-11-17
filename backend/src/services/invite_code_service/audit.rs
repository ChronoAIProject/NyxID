use bson::{Document, doc};
use mongodb::{ClientSession, Database};

use super::{COLLECTION_NAME, InviteCode, admin_projection};
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

pub(super) fn intent(
    actor: &AuditActor,
    event: &str,
    id: &str,
    max_uses: Option<i32>,
) -> AppResult<SecurityEffectIntent> {
    let mut data = serde_json::json!({"invite_code_id":required::text(TextField::Identifier,id)});
    match event {
        "admin_invite_code_create" => {
            // A redeemable invite is credential material, not audit metadata.
            data["code"] = "[REDACTED]".into();
            data["max_uses"] = max_uses.into();
        }
        "admin_invite_code_update" => data["fields_changed"] = serde_json::json!(["note"]),
        _ => {}
    }
    required::intent_preserving_actor(actor, event, Some(data))
}
struct Create<'a> {
    db: Database,
    invite: &'a InviteCode,
    intent: SecurityEffectIntent,
}
pub(super) async fn create(
    db: &Database,
    invite: &InviteCode,
    actor: &AuditActor,
) -> AppResult<()> {
    let context = Create {
        db: db.clone(),
        invite,
        intent: intent(
            actor,
            "admin_invite_code_create",
            &invite.id,
            Some(invite.max_uses),
        )?,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(create_in_session(session, context))
    })
    .await
}
async fn create_in_session(session: &mut ClientSession, context: &Create<'_>) -> AppResult<()> {
    context
        .db
        .collection::<InviteCode>(COLLECTION_NAME)
        .insert_one(context.invite)
        .session(&mut *session)
        .await?;
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}
struct Update<'a> {
    db: Database,
    id: &'a str,
    update: Document,
    intent: SecurityEffectIntent,
}
pub(super) async fn update(
    db: &Database,
    id: &str,
    update: Document,
    actor: &AuditActor,
) -> AppResult<InviteCode> {
    let context = Update {
        db: db.clone(),
        id,
        update,
        intent: intent(actor, "admin_invite_code_update", id, None)?,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(update_in_session(session, context))
    })
    .await
}
async fn update_in_session(
    session: &mut ClientSession,
    context: &Update<'_>,
) -> AppResult<InviteCode> {
    let invite = context
        .db
        .collection::<InviteCode>(COLLECTION_NAME)
        .find_one_and_update(doc! {"_id":context.id}, context.update.clone())
        .return_document(mongodb::options::ReturnDocument::After)
        .projection(admin_projection())
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("Invite code not found".into()))?;
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await?;
    Ok(invite)
}
pub(super) async fn deactivate(
    db: &Database,
    id: &str,
    update: Document,
    actor: &AuditActor,
) -> AppResult<()> {
    let context = Update {
        db: db.clone(),
        id,
        update,
        intent: intent(actor, "admin_invite_code_deactivate", id, None)?,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(deactivate_in_session(session, context))
    })
    .await
}
async fn deactivate_in_session(session: &mut ClientSession, context: &Update<'_>) -> AppResult<()> {
    let result = context
        .db
        .collection::<InviteCode>(COLLECTION_NAME)
        .update_one(doc! {"_id":context.id}, context.update.clone())
        .session(&mut *session)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::NotFound("Invite code not found".into()));
    }
    if result.modified_count == 0 {
        return Ok(());
    }
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}
