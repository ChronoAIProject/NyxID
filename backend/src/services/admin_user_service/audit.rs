use bson::doc;
use mongodb::{ClientSession, Database, options::UpdateModifications};

use super::{AuditActor, USERS, User};
use crate::{
    errors::{AppError, AppResult},
    models::security_effect_intent::SecurityEffectIntent,
    services::{
        audit_service::required::{self, TextField},
        security_effect_service, session_authority, transaction_service,
    },
};

/// Preserve the submitted origin/path without retaining query/fragment tokens
/// or URL userinfo. Work lexically: the existing profile validator also accepts
/// HTTPS-prefixed strings that a URL parser would reject.
pub(super) fn avatar_for_audit(value: &str) -> String {
    let value = value.split(['?', '#']).next().unwrap_or_default();
    let Some(start) = value.find("://").map(|offset| offset + 3) else {
        return value.to_owned();
    };
    let authority_end = value[start..]
        .find(['/', '\\'])
        .map_or(value.len(), |offset| start + offset);
    match value[start..authority_end].rfind('@') {
        Some(offset) => format!("{}{}", &value[..start], &value[start + offset + 1..]),
        None => value.to_owned(),
    }
}

pub(super) fn intent(
    actor: Option<&AuditActor>,
    event: &str,
    metadata: serde_json::Value,
) -> AppResult<Option<SecurityEffectIntent>> {
    actor
        .map(|actor| required::intent_preserving_actor(actor, event, Some(metadata)))
        .transpose()
}

pub(super) async fn record_noop(
    db: &Database,
    intent: Option<SecurityEffectIntent>,
) -> AppResult<()> {
    if let Some(intent) = intent {
        // No mutation occurred on this branch: the receipt is awaited before
        // returning the existing successful no-op response.
        db.collection::<SecurityEffectIntent>(
            crate::models::security_effect_intent::COLLECTION_NAME,
        )
        .insert_one(intent)
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    }
    Ok(())
}

struct UserInsert<'a> {
    db: Database,
    user: &'a User,
    intent: SecurityEffectIntent,
}

pub(super) async fn insert_user(
    db: &Database,
    user: &User,
    intent: Option<SecurityEffectIntent>,
) -> AppResult<()> {
    let Some(intent) = intent else {
        db.collection::<User>(USERS).insert_one(user).await?;
        return Ok(());
    };
    let context = UserInsert {
        db: db.clone(),
        user,
        intent,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(insert_in_session(session, context))
    })
    .await
}

async fn insert_in_session(session: &mut ClientSession, context: &UserInsert<'_>) -> AppResult<()> {
    context
        .db
        .collection::<User>(USERS)
        .insert_one(context.user)
        .session(&mut *session)
        .await?;
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}

struct UserUpdate<'a> {
    db: Database,
    user_id: &'a str,
    update: UpdateModifications,
    missing_target: fn() -> AppError,
    intent: SecurityEffectIntent,
}

pub(super) async fn update_user(
    db: &Database,
    user_id: &str,
    update: impl Into<UpdateModifications>,
    missing_target: fn() -> AppError,
    intent: Option<SecurityEffectIntent>,
) -> AppResult<()> {
    let update = update.into();
    let Some(intent) = intent else {
        let result = db
            .collection::<User>(USERS)
            .update_one(doc! {"_id":user_id}, update)
            .await?;
        if result.matched_count != 1 {
            return Err(missing_target());
        }
        return Ok(());
    };
    let context = UserUpdate {
        db: db.clone(),
        user_id,
        update,
        missing_target,
        intent,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(update_in_session(session, context))
    })
    .await
}

async fn update_in_session(session: &mut ClientSession, context: &UserUpdate<'_>) -> AppResult<()> {
    let result = context
        .db
        .collection::<User>(USERS)
        .update_one(doc! {"_id":context.user_id}, context.update.clone())
        .session(&mut *session)
        .await?;
    if result.matched_count != 1 {
        return Err((context.missing_target)());
    }
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}

pub(super) async fn revoke_sessions_in_session(
    db: &Database,
    user_id: &str,
    session: &mut ClientSession,
    actor: Option<&AuditActor>,
) -> AppResult<u64> {
    let count = session_authority::revoke_all_in_transaction(db, user_id, session).await?;
    if let Some(intent) = intent(
        actor,
        "admin.user.sessions_revoked",
        serde_json::json!({
            "target_user_id": required::text(TextField::Identifier, user_id), "revoked_count": count,
        }),
    )? {
        security_effect_service::enqueue_in_session(db, session, &[intent]).await?;
    }
    Ok(count)
}

struct PasswordReset<'a> {
    db: Database,
    user_id: &'a str,
    reset: Option<super::super::auth_service::PreparedPasswordReset>,
    intent: SecurityEffectIntent,
}

pub(super) async fn force_password_reset(
    db: &Database,
    user_id: &str,
    actor: &AuditActor,
) -> AppResult<Option<String>> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":user_id})
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    if user.password_hash.is_none() {
        return Err(AppError::BadRequest(
            "User has no password (social login only)".into(),
        ));
    }
    let reset = super::super::auth_service::prepare_password_reset(db, &user.email).await?;
    if reset.is_none() && !user.user_type.is_org() {
        return Err(AppError::NotFound("User not found".into()));
    }
    let context = PasswordReset {
        db: db.clone(),
        user_id,
        reset,
        intent: required::intent_preserving_actor(
            actor,
            "admin.user.password_reset",
            Some(
                serde_json::json!({"target_user_id": required::text(TextField::Identifier, user_id)}),
            ),
        )?,
    };
    transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(reset_in_session(session, context))
    })
    .await?;
    Ok(context.reset.map(|reset| reset.token))
}

async fn reset_in_session(
    session: &mut ClientSession,
    context: &PasswordReset<'_>,
) -> AppResult<()> {
    if let Some(reset) = &context.reset {
        let result = context
            .db
            .collection::<User>(USERS)
            .update_one(doc! {"_id":&reset.user_id}, reset.update.clone())
            .session(&mut *session)
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::NotFound("User not found".into()));
        }
    }
    session_authority::revoke_all_in_transaction(&context.db, context.user_id, session).await?;
    security_effect_service::enqueue_in_session(
        &context.db,
        session,
        std::slice::from_ref(&context.intent),
    )
    .await
}
