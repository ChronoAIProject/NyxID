//! One-time policy selection bound to the existing signed OAuth consent request.
use chrono::{Duration, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};

use super::{
    api_key_mutation_service as transactions, consent_service,
    machine_external_grant_service::{self, GrantSelection},
};
use crate::{
    crypto::token::hash_token,
    errors::{AppError, AppResult},
    models::{
        consent::Consent,
        machine_oauth_consent_request::{COLLECTION_NAME, MachineOauthConsentRequest},
    },
};

#[derive(Clone)]
pub struct MachineConsentApproval {
    pub request_id: String,
    pub signed_request: String,
}

/// Called only after the handler has verified a first-party human and decoded
/// the existing signed request; user/client are never taken from a JSON body.
pub async fn prepare(
    db: &Database,
    user_id: &str,
    client_id: &str,
    request_id: &str,
    signed_request: &str,
    selection: &GrantSelection,
) -> AppResult<()> {
    let db = db.clone();
    let user_id = user_id.to_owned();
    let client_id = client_id.to_owned();
    let request_id = request_id.to_owned();
    let signed_request_digest = hash_token(signed_request);
    let selection = selection.clone();
    // Capture before the first shared write can conflict. The driver may retry
    // ordinary execution-fence contention, but this browser selection must never
    // adopt a newer consent or source security baseline during that retry.
    let mut baseline: Option<PreparationBaseline> = None;
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let current = preparation_baseline(&db, &user_id, &client_id, session).await?;
                if baseline
                    .as_ref()
                    .is_some_and(|original| original != &current)
                {
                    return Err(consent_service::grant_changed());
                }
                let original = baseline.get_or_insert(current).clone();
                machine_external_grant_service::validate_selection_in_session(
                    &db, &user_id, &selection, session,
                )
                .await?;
                let (user_security_revision, client_security_revision) =
                    machine_external_grant_service::prepare_oauth_source_in_session(
                        &db, &user_id, &client_id, session,
                    )
                    .await?;
                if user_security_revision != original.user_security_revision
                    || client_security_revision != original.client_security_revision
                {
                    return Err(consent_service::grant_changed());
                }
                let now = Utc::now();
                let request = MachineOauthConsentRequest {
                    id: request_id.clone(),
                    user_id: user_id.clone(),
                    client_id: client_id.clone(),
                    signed_request_digest: signed_request_digest.clone(),
                    previous_consent_fingerprint: original.consent_fingerprint,
                    user_security_revision,
                    client_security_revision,
                    selection: bson::to_document(&selection)
                        .map_err(|e| AppError::Internal(e.to_string()))?,
                    expires_at: now + Duration::minutes(15),
                    consumed_at: None,
                    created_at: now,
                };
                db.collection::<MachineOauthConsentRequest>(COLLECTION_NAME)
                    .insert_one(request)
                    .session(&mut *session)
                    .await?;
                Ok(())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

#[derive(Clone, PartialEq, Eq)]
struct PreparationBaseline {
    consent_fingerprint: Option<String>,
    user_security_revision: i64,
    client_security_revision: i64,
    consent_revocation_revision: i64,
}

fn revision(row: &Document, field: &str) -> AppResult<i64> {
    match row.get(field) {
        None => Ok(0),
        Some(bson::Bson::Int64(value)) if *value >= 0 => Ok(*value),
        Some(bson::Bson::Int32(value)) if *value >= 0 => Ok(i64::from(*value)),
        _ => Err(consent_service::grant_changed()),
    }
}

async fn preparation_baseline(
    db: &Database,
    user_id: &str,
    client_id: &str,
    session: &mut ClientSession,
) -> AppResult<PreparationBaseline> {
    let owner = db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(consent_service::grant_changed)?;
    let client = db
        .collection::<Document>(crate::models::oauth_client::COLLECTION_NAME)
        .find_one(doc! {"_id":client_id})
        .session(&mut *session)
        .await?
        .ok_or_else(consent_service::grant_changed)?;
    let previous = db
        .collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .find_one(doc! {"user_id":user_id,"client_id":client_id})
        .session(&mut *session)
        .await?;
    Ok(PreparationBaseline {
        consent_fingerprint: previous
            .as_ref()
            .map(consent_service::fingerprint)
            .transpose()?,
        user_security_revision: revision(&owner, "machine_external_security_revision")?,
        client_security_revision: revision(&client, "machine_external_security_revision")?,
        // A deleted consent cannot retain its own revision. This separate owner
        // generation preserves absent -> approved -> revoked -> absent staleness
        // without treating admission/renewal fence increments as new consent.
        consent_revocation_revision: revision(&owner, "machine_consent_revocation_revision")?,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn approve_in_session(
    db: &Database,
    user_id: &str,
    client_id: &str,
    previous: Option<&Consent>,
    approved: &Consent,
    approval: &MachineConsentApproval,
    session: &mut ClientSession,
) -> AppResult<()> {
    let request = db
        .collection::<MachineOauthConsentRequest>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "_id": &approval.request_id, "user_id": user_id, "client_id": client_id,
                "signed_request_digest": hash_token(&approval.signed_request),
                "expires_at": { "$gt": bson::DateTime::now() }, "consumed_at": bson::Bson::Null,
            },
            doc! { "$set": { "consumed_at": bson::DateTime::now() } },
        )
        .session(&mut *session)
        .await?
        .ok_or_else(consent_service::grant_changed)?;
    let fingerprint = previous.map(consent_service::fingerprint).transpose()?;
    if fingerprint != request.previous_consent_fingerprint {
        return Err(consent_service::grant_changed());
    }
    let (user_epoch, client_epoch) =
        machine_external_grant_service::prepare_oauth_source_in_session(
            db, user_id, client_id, session,
        )
        .await?;
    if request.user_security_revision != user_epoch
        || request.client_security_revision != client_epoch
    {
        return Err(consent_service::grant_changed());
    }
    let selection: GrantSelection = bson::from_document(request.selection)
        .map_err(|_| AppError::Internal("Invalid saved Machine consent selection".into()))?;
    machine_external_grant_service::approve_oauth_in_session(
        db, user_id, client_id, approved, &selection, session,
    )
    .await?;
    Ok(())
}

pub fn indexes() -> Vec<mongodb::IndexModel> {
    vec![
        mongodb::IndexModel::builder()
            .keys(doc! {"expires_at": 1})
            .options(
                mongodb::options::IndexOptions::builder()
                    .expire_after(std::time::Duration::ZERO)
                    .name("machine_consent_request_expiry".to_string())
                    .build(),
            )
            .build(),
    ]
}

pub async fn needs_review(
    db: &mongodb::Database,
    user: &str,
    client: &str,
    scope: &str,
    consent: Option<&Consent>,
) -> AppResult<bool> {
    if !scope.split_whitespace().any(|s| s == "machine") {
        return Ok(false);
    }
    let Some(consent) = consent else {
        return Ok(true);
    };
    let Ok(reference) = consent_service::protected_reference(consent) else {
        return Ok(true);
    };
    // A bounded source revocation scan can leave stale active rows behind.
    // Reuse the authoritative evaluator so a stale source epoch prompts explicit
    // reapproval instead of producing another unusable token.
    let grant = db.collection::<crate::models::machine_external_grant::MachineExternalGrant>(
        crate::models::machine_external_grant::COLLECTION_NAME).find_one(doc! {
        "user_id": user, "binding.kind": "oauth", "binding.oauth_client_id": client,
        "binding.consent_id": reference.consent_id, "binding.consent_revision": reference.consent_revision,
        "status": "active", "expires_at": { "$gt": bson::DateTime::now() },
    }).await?;
    match grant {
        Some(grant) => {
            Ok(!machine_external_grant_service::source_and_grant_valid(db, &grant).await?)
        }
        None => Ok(true),
    }
}
