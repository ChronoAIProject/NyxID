use super::{USERS, get_org_user_with_session, membership, prepare_org_user, synthetic_org_email};
use crate::{
    errors::{AppError, AppResult},
    models::{
        org_membership::{MemberScopeSource, OrgMembership, OrgRole},
        user::User,
    },
    services::{
        audit_service::{
            AuditActor,
            required::{self, TextField},
        },
        org_membership_authority, security_effect_service,
    },
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};

pub(crate) struct Update<'a> {
    pub display_name: Option<&'a str>,
    pub slug: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub contact_email: Option<&'a str>,
    pub remote_credential_integrity_verification_opt_out: Option<bool>,
}
fn prepare_update(org_user_id: &str, patch: &Update<'_>) -> AppResult<Document> {
    let Update {
        display_name,
        slug,
        avatar_url,
        contact_email,
        remote_credential_integrity_verification_opt_out,
    } = *patch;
    let mut update = doc! {};
    if let Some(name) = display_name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(AppError::ValidationError(
                "Org display name cannot be empty".to_string(),
            ));
        }
        update.insert("display_name", trimmed);
    }
    if let Some(slug) = slug {
        update.insert("slug", slug);
    }
    if let Some(avatar) = avatar_url {
        let trimmed = avatar.trim();
        if trimmed.is_empty() {
            update.insert("avatar_url", bson::Bson::Null);
        } else {
            update.insert("avatar_url", trimmed);
        }
    }
    if let Some(email) = contact_email {
        let trimmed = email.trim();
        if trimmed.is_empty() {
            // Restore the synthetic placeholder so admin/audit surfaces
            // still show a stable, unique-looking identifier.
            update.insert("email", synthetic_org_email(org_user_id));
        } else {
            // Defensive validation — handler-level validator already runs,
            // but this keeps the service safe for direct callers.
            if !trimmed.contains('@') {
                return Err(AppError::ValidationError(
                    "contact_email must be a valid email".to_string(),
                ));
            }
            update.insert("email", trimmed.to_lowercase());
        }
    }
    if let Some(opt_out) = remote_credential_integrity_verification_opt_out {
        update.insert(
            "profile_config.release_integrity.remote_credential_integrity_verification_opt_out",
            opt_out,
        );
    }
    update.insert("updated_at", bson::DateTime::from_chrono(Utc::now()));

    Ok(update)
}

pub(crate) fn created_intent(
    actor: &AuditActor,
    id: &str,
    display_name: &str,
) -> AppResult<crate::models::security_effect_intent::SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "org_created",
        Some(
            serde_json::json!({"org_user_id":required::text(TextField::Identifier,id),"display_name":required::text(TextField::DisplayName,display_name)}),
        ),
    )
}
pub(crate) fn updated_intent(
    actor: &AuditActor,
    id: &str,
    patch: &Update<'_>,
) -> AppResult<crate::models::security_effect_intent::SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "org_updated",
        Some(
            serde_json::json!({"org_user_id":required::text(TextField::Identifier,id),"contact_email_changed":patch.contact_email.is_some(),"remote_credential_integrity_verification_opt_out_changed":patch.remote_credential_integrity_verification_opt_out.is_some()}),
        ),
    )
}

pub async fn create(
    db: &Database,
    id: &str,
    display_name: &str,
    contact_email: Option<&str>,
    avatar_url: Option<&str>,
    actor: &AuditActor,
) -> AppResult<(User, OrgMembership)> {
    let org = prepare_org_user(db, id, display_name, contact_email, avatar_url).await?;
    let intent = created_intent(actor, id, display_name)?;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = create_in_session(db, &org, actor, &intent, &mut tx).await;
    let membership = org_membership_authority::finish(tx, result).await?;
    crate::services::billing::schedules::eligibility::after_org_joined(
        db,
        &membership.org_user_id,
        &membership.member_user_id,
        membership.created_at,
    )
    .await;
    tracing::info!(org_user_id=%id,"Org user created");
    Ok((org, membership))
}
async fn create_in_session(
    db: &Database,
    org: &User,
    actor: &AuditActor,
    intent: &crate::models::security_effect_intent::SecurityEffectIntent,
    tx: &mut ClientSession,
) -> AppResult<OrgMembership> {
    db.collection::<User>(USERS)
        .insert_one(org)
        .session(&mut *tx)
        .await?;
    let creation = membership::prepare_creation(
        db,
        &org.id,
        &actor.user_id,
        OrgRole::Admin,
        MemberScopeSource::Inherit,
        None,
        Some(&mut *tx),
    )
    .await?;
    let membership = membership::create_in_session(db, creation, tx).await?;
    security_effect_service::enqueue_in_session(db, tx, std::slice::from_ref(intent)).await?;
    Ok(membership)
}

pub async fn update(
    db: &Database,
    id: &str,
    patch: Update<'_>,
    actor: &AuditActor,
) -> AppResult<User> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = update_in_session(db, id, &patch, actor, &mut tx).await;
    org_membership_authority::finish(tx, result).await
}
async fn update_in_session(
    db: &Database,
    id: &str,
    patch: &Update<'_>,
    actor: &AuditActor,
    tx: &mut ClientSession,
) -> AppResult<User> {
    get_org_user_with_session(db, id, Some(&mut *tx)).await?;
    let update = prepare_update(id, patch)?;
    let changed = db
        .collection::<User>(USERS)
        .update_one(doc! {"_id":id,"user_type":"org"}, doc! {"$set":update})
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(AppError::OrgNotFound(id.to_owned()));
    }
    let org = get_org_user_with_session(db, id, Some(&mut *tx)).await?;
    let intent = updated_intent(actor, id, patch)?;
    security_effect_service::enqueue_in_session(db, tx, &[intent]).await?;
    Ok(org)
}
