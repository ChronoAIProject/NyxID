//! Add-only OAuth service grants. Signed snapshots and CAS writes prevent stale
//! decisions from overwriting later grants or resurrecting a revoked consent.
use crate::errors::{AppError, AppResult};
use crate::models::authorization_code::ExternalSubjectRef;
use crate::models::consent::{Consent, IncrementalConsent};
use crate::services::{consent_service, oauth_broker_service, oauth_resource_service};

pub fn union(left: &[String], right: &[String]) -> Vec<String> {
    let mut result = left.to_vec();
    result.extend_from_slice(right);
    result.sort();
    result.dedup();
    result
}

pub fn union_scopes(left: &str, right: &str) -> String {
    let scopes = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
    union(&scopes(left), &scopes(right)).join(" ")
}

pub fn validate_requested_ids(ids: &[String]) -> AppResult<()> {
    if ids.len() > 100 || ids.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
        return Err(AppError::BadRequest(
            "requested_service_ids must contain at most 100 UserService UUIDs".to_string(),
        ));
    }
    Ok(())
}

async fn validate_services(db: &mongodb::Database, user_id: &str, ids: &[String]) -> AppResult<()> {
    if !oauth_resource_service::validate_grantable_service_ids(db, user_id, ids).await? {
        return Err(AppError::InvalidTarget(
            "A service is unavailable or you no longer have access. Review application access and restart authorization.".to_string(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn prepare(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    client_name: &str,
    requested_scopes: Option<&str>,
    required_service_ids: &[String],
    access_resources: &[String],
    binding: Option<&oauth_broker_service::BindingGrantSnapshot>,
) -> AppResult<IncrementalConsent> {
    let consent = consent_service::check_consent(db, user_id, client_id, "")
        .await?
        .ok_or_else(consent_service::grant_changed)?;
    if !consent.allow_all_services && consent.allowed_service_ids.is_none() {
        return Err(consent_service::grant_changed());
    }
    if let Some(binding) = binding {
        let consent_scopes = consent
            .scopes
            .split_whitespace()
            .collect::<std::collections::HashSet<_>>();
        let services_covered = consent.allow_all_services
            || (!binding.allow_all_services
                && binding.allowed_service_ids.iter().all(|id| {
                    consent
                        .allowed_service_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(id))
                }));
        if !services_covered
            || !binding
                .scopes
                .split_whitespace()
                .all(|scope| consent_scopes.contains(scope))
        {
            return Err(consent_service::grant_changed());
        }
    }
    let (current_service_ids, allow_all_services, current_scopes) = match binding {
        Some(binding) => (
            binding.allowed_service_ids.clone(),
            binding.allow_all_services,
            binding.scopes.as_str(),
        ),
        None => (
            consent.allowed_service_ids.clone().unwrap_or_default(),
            consent.allow_all_services,
            consent.scopes.as_str(),
        ),
    };
    let current_service_ids = if allow_all_services {
        Vec::new()
    } else {
        current_service_ids
    };
    let additions = required_service_ids
        .iter()
        .filter(|id| !current_service_ids.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    validate_services(db, user_id, &additions).await?;
    let consent_fingerprint = consent_service::fingerprint(&consent)?;
    Ok(IncrementalConsent {
        consent_fingerprint,
        consent_id: consent.id,
        consent_revision: consent.revision,
        binding_grant_version: binding.map(|b| b.grant_version),
        client_name: client_name.to_string(),
        current_scopes: current_scopes.to_string(),
        scopes: union_scopes(current_scopes, requested_scopes.unwrap_or(current_scopes)),
        current_service_ids,
        allow_all_services,
        required_service_ids: union(&[], required_service_ids),
        access_resources: access_resources.to_vec(),
    })
}

/// Live checks on decision and token exchange. Use `validate_issued` after
/// insertion to serialize with transactional revocation before returning tokens.
pub async fn validate_current(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    snapshot: &IncrementalConsent,
    binding_hash: Option<&str>,
    external_subject: Option<&ExternalSubjectRef>,
) -> AppResult<Consent> {
    let consent = consent_service::check_consent(db, user_id, client_id, "")
        .await?
        .ok_or_else(consent_service::grant_changed)?;
    if consent.id != snapshot.consent_id
        || consent.revision != snapshot.consent_revision
        || consent_service::fingerprint(&consent)? != snapshot.consent_fingerprint
    {
        return Err(consent_service::grant_changed());
    }
    if let Some(hash) = binding_hash {
        let binding = oauth_broker_service::resolve_binding_grant_for_subject(
            db,
            client_id,
            user_id,
            external_subject,
            hash,
        )
        .await?;
        if Some(binding.grant_version) != snapshot.binding_grant_version {
            return Err(consent_service::grant_changed());
        }
    } else if snapshot.binding_grant_version.is_some() {
        return Err(consent_service::grant_changed());
    }
    let additions = snapshot
        .required_service_ids
        .iter()
        .filter(|id| !snapshot.current_service_ids.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    validate_services(db, user_id, &additions).await?;
    Ok(consent)
}

/// A real write after token insertion conflicts with transactional consent
/// revocation. A read-only last check could observe the pre-delete snapshot
/// while revocation's refresh scan misses a concurrently inserted token.
pub async fn validate_issued(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    snapshot: &IncrementalConsent,
    binding_hash: Option<&str>,
    external_subject: Option<&ExternalSubjectRef>,
) -> AppResult<()> {
    let consent = validate_current(
        db,
        user_id,
        client_id,
        snapshot,
        binding_hash,
        external_subject,
    )
    .await?;
    let result = db
        .collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .update_one(
            consent_service::version_filter(&consent),
            mongodb::bson::doc! { "$set": { "issuance_fence": uuid::Uuid::new_v4().to_string() } },
        )
        .await?;
    if result.matched_count != 1 {
        return Err(consent_service::grant_changed());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn approve(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    snapshot: &IncrementalConsent,
    selected_ids: &[String],
    allow_all_services: bool,
    binding_hash: Option<&str>,
    external_subject: Option<&ExternalSubjectRef>,
) -> AppResult<IncrementalConsent> {
    // Add-only mode cannot introduce a wildcard grant. Existing wildcard
    // authority is retained even if a modified form submits false.
    if allow_all_services && !snapshot.allow_all_services {
        return Err(AppError::BadRequest(
            "Incremental consent cannot grant all services".to_string(),
        ));
    }
    let existing = validate_current(
        db,
        user_id,
        client_id,
        snapshot,
        binding_hash,
        external_subject,
    )
    .await?;
    let additions = union(selected_ids, &snapshot.required_service_ids)
        .into_iter()
        .filter(|id| !snapshot.current_service_ids.contains(id))
        .collect::<Vec<_>>();
    validate_services(db, user_id, &additions).await?;
    let grant_ids = if snapshot.allow_all_services {
        Vec::new()
    } else {
        union(&snapshot.current_service_ids, &additions)
    };
    // The client-wide consent may also cover another binding. Preserve that
    // consent, but never copy another binding's authority into this one.
    let consent_ids = union(
        existing.allowed_service_ids.as_deref().unwrap_or_default(),
        &grant_ids,
    );
    let updated = consent_service::replace_incremental_consent(
        db,
        &existing,
        &union_scopes(&existing.scopes, &snapshot.scopes),
        consent_ids,
    )
    .await?;
    Ok(IncrementalConsent {
        consent_fingerprint: consent_service::fingerprint(&updated)?,
        consent_revision: updated.revision,
        current_service_ids: grant_ids,
        ..snapshot.clone()
    })
}
