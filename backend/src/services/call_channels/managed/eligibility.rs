//! Explicit approval, merchant consent and revision handoff; no self approval.
use super::{conflict, unavailable};
use crate::{
    errors::{AppError, AppResult},
    models::managed_comms::*,
    services::{
        audit_service::{self, AuditActor},
        transaction_service,
    },
};
use chrono::Utc;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EligibilityInput {
    pub business_legal_name: String,
    pub registration_id: String,
    pub jurisdiction: String,
    pub allowed_use_case: String,
    pub sms_sender_path: String,
    pub sms_registration_status: String,
    pub required_terms: TermsVersions,
    pub premium_allowed: bool,
    #[serde(default = "one")]
    pub max_lines: i64,
    pub monthly_spend_limit: Option<crate::models::credits::Credits>,
    pub expected_revision: Option<i64>,
}
fn one() -> i64 {
    1
}
fn validate(input: &EligibilityInput) -> AppResult<()> {
    for value in [
        &input.business_legal_name,
        &input.registration_id,
        &input.allowed_use_case,
        &input.sms_sender_path,
        &input.sms_registration_status,
        &input.required_terms.managed_line,
    ] {
        if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
            return Err(AppError::ValidationError(
                "Invalid managed eligibility field".into(),
            ));
        }
    }
    if !super::valid_country(&input.jurisdiction)
        || !(1..=100).contains(&input.max_lines)
        || input
            .monthly_spend_limit
            .is_some_and(|v| v <= crate::models::credits::Credits::ZERO)
        || input
            .required_terms
            .elevenlabs_eua
            .as_ref()
            .is_some_and(|v| v.trim().is_empty() || v.len() > 128)
        || (input.premium_allowed && input.required_terms.elevenlabs_eua.is_none())
    {
        return Err(AppError::ValidationError(
            "Invalid managed eligibility policy".into(),
        ));
    }
    Ok(())
}
pub async fn load(db: &Database, owner: &str) -> AppResult<ManagedCommsEligibility> {
    db.collection::<ManagedCommsEligibility>(ManagedCommsEligibility::COLLECTION_NAME)
        .find_one(doc! {"user_id":owner})
        .await?
        .ok_or_else(|| AppError::NotFound("Managed line eligibility has not been approved".into()))
}
pub async fn fence(
    db: &Database,
    tx: &mut ClientSession,
    row: &ManagedCommsEligibility,
) -> AppResult<()> {
    super::require_ready(db).await?;
    let changed = db
        .collection::<ManagedCommsEligibility>(ManagedCommsEligibility::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id,"revision":row.revision,"revoked_at":bson::Bson::Null},
            doc! {"$inc":{"fence":1_i64}},
        )
        .session(tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(unavailable());
    }
    Ok(())
}
pub(super) async fn admin_fence(
    db: &Database,
    tx: &mut ClientSession,
    actor: &AuditActor,
) -> AppResult<()> {
    if !crate::services::tools_manager_service::role_authority(db, tx, &actor.user_id, None, true)
        .await?
    {
        return Err(AppError::Forbidden(
            "Platform Admin authority required".into(),
        ));
    }
    Ok(())
}
pub async fn save(
    db: &Database,
    actor: &AuditActor,
    owner: &str,
    input: EligibilityInput,
    key: &[u8],
) -> AppResult<ManagedCommsEligibility> {
    super::require_ready(db).await?;
    validate(&input)?;
    let context = (db, actor, owner, &input, key);
    transaction_service::run_db_only(db,&context,|tx,c| Box::pin(async move {
        let &(db,actor,owner,input,key)=c;
        admin_fence(db,tx,actor).await?;
        super::super::owner_admission::fence(db,tx,owner).await?;
        let collection=db.collection::<ManagedCommsEligibility>(ManagedCommsEligibility::COLLECTION_NAME);
        let old=collection.find_one(doc!{"user_id":owner}).session(&mut *tx).await?;
        if old.as_ref().map(|r|r.revision)!=input.expected_revision { return Err(conflict("Eligibility revision changed")); }
        let same_terms=old.as_ref().is_some_and(|r|r.required_terms==input.required_terms);
        let now=Utc::now();
        let row=ManagedCommsEligibility {
            id:old.as_ref().map(|r|r.id.clone()).unwrap_or_else(||uuid::Uuid::new_v4().to_string()), user_id:owner.into(),
            business_legal_name:input.business_legal_name.clone(),registration_id:input.registration_id.clone(),jurisdiction:input.jurisdiction.clone(),allowed_use_case:input.allowed_use_case.clone(),sms_sender_path:input.sms_sender_path.clone(),sms_registration_status:input.sms_registration_status.clone(),required_terms:input.required_terms.clone(),
            accepted_terms:old.as_ref().filter(|_|same_terms).and_then(|r|r.accepted_terms.clone()), terms_accepted_at:old.as_ref().filter(|_|same_terms).and_then(|r|r.terms_accepted_at),
            premium_allowed:input.premium_allowed,max_lines:input.max_lines,monthly_spend_limit:input.monthly_spend_limit,approved_by:actor.user_id.clone(),approved_at:now,
            revision:old.as_ref().map_or(1,|r|r.revision+1),fence:old.as_ref().map_or(1,|r|r.fence+1),revoked_at:None,
        };
        if let Some(old)=old { let changed=collection.replace_one(doc!{"_id":&old.id,"revision":old.revision},&row).session(&mut *tx).await?; if changed.matched_count!=1{return Err(unavailable());} }
        else { collection.insert_one(&row).session(&mut *tx).await?; }
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_eligibility_saved",serde_json::json!({"eligibility_id":row.id,"revision":row.revision,"owner_id":owner,"terms_changed":!same_terms})).await?;
        Ok(row)
    })).await
}
pub async fn accept_terms(
    db: &Database,
    actor: &AuditActor,
    revision: i64,
    terms: TermsVersions,
    key: &[u8],
) -> AppResult<()> {
    super::require_ready(db).await?;
    let context = (db, actor, revision, &terms, key);
    transaction_service::run_db_only(db,&context,|tx,c|Box::pin(async move {
        let &(db,actor,revision,terms,key)=c;
        super::super::owner_admission::fence(db,tx,&actor.user_id).await?;
        let changed=db.collection::<ManagedCommsEligibility>(ManagedCommsEligibility::COLLECTION_NAME).update_one(doc!{"user_id":&actor.user_id,"revision":revision,"revoked_at":bson::Bson::Null,"required_terms":bson::to_bson(terms).map_err(|_|unavailable())?},doc!{"$set":{"accepted_terms":bson::to_bson(terms).map_err(|_|unavailable())?,"terms_accepted_at":bson::DateTime::now()},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        if changed.matched_count!=1{return Err(conflict("Accept the current managed line terms"));}
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_terms_accepted",serde_json::json!({"revision":revision})).await?;Ok(())
    })).await
}
pub async fn apply_revision(
    db: &Database,
    actor: &AuditActor,
    owner: &str,
    revision: i64,
    key: &[u8],
) -> AppResult<()> {
    super::require_ready(db).await?;
    let context = (db, actor, owner, revision, key);
    transaction_service::run_db_only(db,&context,|tx,c|Box::pin(async move {
        let &(db,actor,owner,revision,key)=c;
        admin_fence(db,tx,actor).await?;
        super::super::owner_admission::fence(db,tx,owner).await?;
        let row=db.collection::<ManagedCommsEligibility>(ManagedCommsEligibility::COLLECTION_NAME).find_one(doc!{"user_id":owner,"revision":revision}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if row.accepted_terms.as_ref()!=Some(&row.required_terms){return Err(conflict("Merchant acceptance is required before applying this revision"));}
        fence(db,tx,&row).await?;
        if !row.premium_allowed && db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).find_one(doc!{"user_id":owner,"voice_tier":"premium","state":{"$in":["claimed","suspended"]}}).session(&mut *tx).await?.is_some(){return Err(conflict("Release premium claims before removing premium eligibility"));}
        db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).update_many(doc!{"user_id":owner,"state":{"$in":["claimed","suspended"]}},doc!{"$set":{"eligibility_revision":revision,"accepted_terms":bson::to_bson(&row.required_terms).map_err(|_|unavailable())?},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_eligibility_applied",serde_json::json!({"eligibility_id":row.id,"revision":revision})).await?;Ok(())
    })).await
}
pub async fn revoke(
    state: &crate::AppState,
    actor: &AuditActor,
    owner: &str,
    revision: i64,
    key: &[u8],
) -> AppResult<()> {
    let db = &state.db;
    super::require_ready(db).await?;
    let context = (db, actor, owner, revision, key);
    transaction_service::run_db_only(db,&context,|tx,c|Box::pin(async move {
        let &(db,actor,owner,revision,key)=c;
        admin_fence(db,tx,actor).await?;
        super::super::owner_admission::fence(db,tx,owner).await?;
        let now=bson::DateTime::now();
        let changed=db.collection::<ManagedCommsEligibility>(ManagedCommsEligibility::COLLECTION_NAME).update_one(doc!{"user_id":owner,"revision":revision,"revoked_at":bson::Bson::Null},doc!{"$set":{"revoked_at":now},"$inc":{"revision":1_i64,"fence":1_i64}}).session(&mut *tx).await?;
        if changed.matched_count!=1{return Err(conflict("Eligibility revision changed"));}
        let mut claims=db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).find(doc!{"user_id":owner,"state":{"$in":["claimed","suspended"]}}).session(&mut *tx).await?;
        let mut ids=Vec::new();
        while let Some(claim)=claims.next(&mut *tx).await.transpose()? {ids.push(claim.id);}
        db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).update_many(doc!{"current_claim_id":{"$in":ids}},doc!{"$set":{"state":"suspended","suspension":"eligibility_revoked","updated_at":now},"$inc":{"revision":1_i64,"fence":1_i64}}).session(&mut *tx).await?;
        db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).update_many(doc!{"user_id":owner,"state":{"$in":["claimed","suspended"]}},doc!{"$set":{"state":"suspended","suspension":"eligibility_revoked","auto_renew":false,"release_requested_at":now},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        db.collection::<bson::Document>(crate::models::channel_adapter::BINDINGS).update_many(doc!{"user_id":owner,"credential.kind":"managed_line"},doc!{"$set":{"enabled":false},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
        db.collection::<bson::Document>(crate::models::channel_adapter::SESSIONS).update_many(doc!{"user_id":owner,"managed_claim_id":{"$type":"string"},"billing_finalized":false},doc!{"$set":{"lease_expires_at":now},"$inc":{"generation":1_i64}}).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_eligibility_revoked",serde_json::json!({"owner_id":owner,"revision":revision})).await?;Ok(())
    })).await?;
    use futures::TryStreamExt;
    let bindings: Vec<crate::models::channel_adapter::Binding> = db
        .collection(crate::models::channel_adapter::BINDINGS)
        .find(doc! {"user_id":owner,"credential.kind":"managed_line"})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for binding in bindings {
        if super::super::management::close_disabled_binding(state, &binding)
            .await
            .is_err()
        {
            super::super::management::audit(
                state,
                "system",
                "managed_revocation_cleanup_pending",
                serde_json::json!({"binding_id":binding.id}),
            );
        }
    }
    Ok(())
}

/// Check live Admin authority before any secret material or provider I/O is used;
/// mutations check and fence it again at commit.
pub async fn require_admin(db: &Database, actor: &AuditActor) -> AppResult<()> {
    super::require_ready(db).await?;
    require_existing_effect_admin(db, actor).await
}

/// Admitted-effect evidence remains recoverable while fleet readiness is down.
/// Keep the same live Admin fence as configuration writes, without admission.
pub(super) async fn require_existing_effect_admin(
    db: &Database,
    actor: &AuditActor,
) -> AppResult<()> {
    let context = (db, actor);
    transaction_service::run_db_only(db, &context, |tx, c| {
        Box::pin(async move { admin_fence(c.0, tx, c.1).await })
    })
    .await
}

pub async fn list(db: &Database, after: Option<&str>) -> AppResult<Vec<ManagedCommsEligibility>> {
    use futures::TryStreamExt;
    let mut filter = doc! {};
    if let Some(after) = after {
        filter.insert("_id", doc! {"$gt":after});
    }
    Ok(db
        .collection(ManagedCommsEligibility::COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(101)
        .await?
        .try_collect()
        .await?)
}
