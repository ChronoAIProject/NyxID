use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};

pub(crate) const REVOCATION_FENCES: &str = "consent_revocation_fences";

fn revocation_version(row: Option<&bson::Document>) -> AppResult<String> {
    match row {
        None => Ok("initial".into()),
        Some(row) => match row.get_str("version") {
            Ok(version) => Ok(version.to_owned()),
            Err(_) => row
                .get_datetime("revoked_at")
                .map(|date| format!("legacy:{}", date.timestamp_millis()))
                .map_err(|_| grant_changed()),
        },
    }
}

pub async fn consent_request_fence(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
) -> AppResult<String> {
    let row = db
        .collection::<bson::Document>(REVOCATION_FENCES)
        .find_one(doc! {"_id":format!("{user_id}:{client_id}")})
        .await?;
    revocation_version(row.as_ref())
}

/// Grant consent for a user to a client with specific scopes.
/// Upserts: if consent exists for (user_id, client_id), replaces scopes.
///
/// Test-only convenience wrapper: production code uses
/// [`grant_consent_union_with_machine`]; the plain `grant_consent` is retained only
/// for existing tests, so it is scoped to test builds to avoid a bin dead-code error.
#[cfg(test)]
pub async fn grant_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
) -> AppResult<Consent> {
    grant_consent_internal(db, user_id, client_id, scopes, false, Some(Vec::new())).await
}

/// Grant consent for a user to a client with optional service restriction.
///
/// `Some(ids)`, including an empty list, is an explicit per-service consent
/// grant. `None` is an explicit unrestricted service grant.
#[cfg(test)]
pub async fn grant_consent_with_services(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
    allowed_service_ids: Option<Vec<String>>,
) -> AppResult<Consent> {
    let allow_all_services = allowed_service_ids.is_none();
    let explicit_allowed_service_ids = allowed_service_ids.or_else(|| Some(Vec::new()));
    grant_consent_internal(
        db,
        user_id,
        client_id,
        scopes,
        allow_all_services,
        explicit_allowed_service_ids,
    )
    .await
}

#[cfg(test)]
async fn grant_consent_internal(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
    allow_all_services: bool,
    allowed_service_ids: Option<Vec<String>>,
) -> AppResult<Consent> {
    grant_consent_with_machine(
        db,
        user_id,
        client_id,
        scopes,
        allow_all_services,
        allowed_service_ids,
        None,
    )
    .await
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub async fn grant_consent_with_machine(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
    allow_all_services: bool,
    allowed_service_ids: Option<Vec<String>>,
    machine: Option<&super::machine_oauth_consent_service::MachineConsentApproval>,
) -> AppResult<Consent> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user_id = user_id.to_owned();
    let client_id = client_id.to_owned();
    let scopes = scopes.to_owned();
    let machine = machine.cloned();
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let db = &db;
            let user_id = user_id.as_str();
            let client_id = client_id.as_str();
            let scopes = scopes.as_str();
            let machine = machine.as_ref();
            let result: AppResult<Consent> = async {
                let existing = db
                    .collection::<Consent>(CONSENTS)
                    .find_one(doc! { "user_id": user_id, "client_id": client_id })
                    .session(&mut *session)
                    .await?;
                let regranted = existing.is_none()
                    && db
                        .collection::<bson::Document>(REVOCATION_FENCES)
                        .find_one(doc! {"_id": format!("{user_id}:{client_id}")})
                        .session(&mut *session)
                        .await?
                        .is_some();
                write_consent_in_session(
                    db,
                    user_id,
                    client_id,
                    existing.as_ref(),
                    scopes,
                    allow_all_services,
                    allowed_service_ids.clone(),
                    None,
                    machine,
                    regranted,
                    session,
                )
                .await
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

/// OAuth approval variant that merges against the transaction's CAS snapshot.
/// The read, union, and write share one transaction so a concurrent revoke or
/// broadening cannot be resurrected or silently lost.
#[allow(clippy::too_many_arguments)]
pub async fn grant_consent_union_with_machine(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
    allow_all_services: bool,
    allowed_service_ids: Option<Vec<String>>,
    machine: Option<&super::machine_oauth_consent_service::MachineConsentApproval>,
    consent_request_iat: Option<i64>,
    consent_request_fence: Option<String>,
) -> AppResult<Consent> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user_id = user_id.to_owned();
    let client_id = client_id.to_owned();
    let scopes = scopes.to_owned();
    let requested_ids = allowed_service_ids;
    let machine = machine.cloned();
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<Consent> = async {
                let existing = db
                    .collection::<Consent>(CONSENTS)
                    .find_one(doc! { "user_id": &user_id, "client_id": &client_id })
                    .session(&mut *session)
                    .await?;
                let revocation_fence = db
                    .collection::<bson::Document>(REVOCATION_FENCES)
                    .find_one(doc! {"_id": format!("{user_id}:{client_id}")})
                    .session(&mut *session)
                    .await?;
                if let Some(expected) = consent_request_fence.as_deref() {
                    if revocation_version(revocation_fence.as_ref())? != expected {
                        return Err(grant_changed());
                    }
                } else if let Some(request_iat) = consent_request_iat
                    && let Some(fence) = revocation_fence.as_ref()
                {
                    let revoked_at = fence
                        .get_datetime("revoked_at")
                        .map_err(|_| grant_changed())?
                        .timestamp_millis()
                        / 1_000;
                    if request_iat <= revoked_at {
                        return Err(grant_changed());
                    }
                }
                let live_existing = existing.as_ref().filter(|consent| {
                    consent
                        .expires_at
                        .is_none_or(|expires_at| expires_at > Utc::now())
                });
                let merged_scopes = live_existing
                    .as_ref()
                    .map(|consent| {
                        normalize_scope_string(&format!("{} {}", consent.scopes, scopes))
                    })
                    .unwrap_or_else(|| normalize_scope_string(&scopes));
                let merged_allow_all = allow_all_services
                    || live_existing
                        .as_ref()
                        .is_some_and(|consent| consent.allow_all_services);
                let merged_ids = if merged_allow_all {
                    None
                } else {
                    let mut ids = requested_ids.clone().unwrap_or_default();
                    if let Some(existing_ids) = live_existing
                        .as_ref()
                        .and_then(|consent| consent.allowed_service_ids.as_ref())
                    {
                        ids.extend(existing_ids.iter().cloned());
                    }
                    Some(normalize_service_ids(&ids))
                };
                // Ordinary review has no expiry control. Preserve an active
                // time limit rather than silently extending the grant while
                // merging the newly approved authority.
                let merged_expires_at = live_existing.as_ref().and_then(|c| c.expires_at);
                write_consent_in_session(
                    &db,
                    &user_id,
                    &client_id,
                    existing.as_ref(),
                    &merged_scopes,
                    merged_allow_all,
                    merged_ids,
                    merged_expires_at,
                    machine.as_ref(),
                    existing.is_none() && revocation_fence.is_some(),
                    session,
                )
                .await
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

#[allow(clippy::too_many_arguments)]
async fn write_consent_in_session(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    existing: Option<&Consent>,
    scopes: &str,
    allow_all_services: bool,
    allowed_service_ids: Option<Vec<String>>,
    expires_at: Option<chrono::DateTime<Utc>>,
    machine: Option<&super::machine_oauth_consent_service::MachineConsentApproval>,
    regranted: bool,
    session: &mut mongodb::ClientSession,
) -> AppResult<Consent> {
    let normalized_scopes = normalize_scope_string(scopes);
    let normalized_allowed_service_ids = allowed_service_ids.map(|ids| normalize_service_ids(&ids));
    if let Some(existing) = existing
        && effective_authority_equal(
            existing,
            &normalized_scopes,
            allow_all_services,
            normalized_allowed_service_ids.as_ref(),
            expires_at,
        )
        && machine.is_none()
    {
        if db
            .collection::<Consent>(CONSENTS)
            .find_one(version_filter(existing))
            .session(&mut *session)
            .await?
            .is_none()
        {
            return Err(grant_changed());
        }
        tracing::info!(
            client_id = %client_id,
            outcome = "no_op",
            "OAuth consent authority updated"
        );
        return Ok(existing.clone());
    }
    let pure_broadening = existing.is_some_and(|current| {
        let old_scopes = normalize_scope_set(&current.scopes);
        let new_scopes = normalize_scope_set(&normalized_scopes);
        let old_services = normalize_service_set(current.allowed_service_ids.as_deref());
        let new_services = normalize_service_set(normalized_allowed_service_ids.as_deref());
        let expiry_broadened = match (current.expires_at, expires_at) {
            (None, None) | (Some(_), None) => true,
            (Some(previous), Some(next)) => next >= previous,
            (None, Some(_)) => false,
        };
        expiry_broadened
            && current.expires_at.is_none_or(|expiry| expiry > Utc::now())
            && machine.is_none()
            && new_scopes.is_superset(&old_scopes)
            && (allow_all_services
                || (!current.allow_all_services && new_services.is_superset(&old_services)))
            && !effective_authority_equal(
                current,
                &normalized_scopes,
                allow_all_services,
                normalized_allowed_service_ids.as_ref(),
                expires_at,
            )
    });
    let outcome = if existing.is_none() {
        if regranted { "regranted" } else { "created" }
    } else if machine.is_some()
        || existing.is_some_and(|current| {
            current
                .expires_at
                .is_some_and(|expiry| expiry <= Utc::now())
        })
    {
        "regranted"
    } else if pure_broadening {
        "broadened"
    } else if existing.is_some() {
        "narrowed"
    } else {
        "created"
    };
    let revision = if pure_broadening {
        existing.and_then(|consent| consent.revision.clone())
    } else {
        Some(Uuid::new_v4().to_string())
    };
    let updated = Consent {
        id: existing
            .map(|c| c.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        user_id: user_id.to_owned(),
        client_id: client_id.to_owned(),
        scopes: normalized_scopes,
        revision,
        write_version: Some(Uuid::new_v4().to_string()),
        issuance_fence: pure_broadening
            .then(|| existing.and_then(|consent| consent.issuance_fence.clone()))
            .flatten(),
        allow_all_services,
        allowed_service_ids: normalized_allowed_service_ids,
        // Keep the legacy-refresh upgrade cutoff anchored to the broadening
        // event. Bound tokens remain valid because their revision and
        // issuance fence are preserved, while unbound legacy rows created
        // before this new authority cannot acquire it on a later refresh.
        granted_at: Utc::now(),
        expires_at,
    };
    if let Some(existing) = existing {
        let result = db
            .collection::<Consent>(CONSENTS)
            .replace_one(version_filter(existing), &updated)
            .session(&mut *session)
            .await?;
        if result.matched_count != 1 {
            return Err(grant_changed());
        }
    } else {
        db.collection::<Consent>(CONSENTS)
            .insert_one(&updated)
            .session(&mut *session)
            .await?;
    }
    if !pure_broadening {
        super::machine_external_grant_service::invalidate_oauth_in_session(
            db,
            user_id,
            client_id,
            "consent_changed",
            session,
        )
        .await?;
    }
    if let Some(machine) = machine {
        super::machine_oauth_consent_service::approve_in_session(
            db, user_id, client_id, existing, &updated, machine, session,
        )
        .await?;
    }
    tracing::info!(
        client_id = %client_id,
        outcome,
        "OAuth consent authority updated"
    );
    Ok(updated)
}

pub fn normalize_scope_set(scopes: &str) -> std::collections::HashSet<String> {
    scopes.split_whitespace().map(ToOwned::to_owned).collect()
}

pub fn normalize_scope_string(scopes: &str) -> String {
    let mut values = normalize_scope_set(scopes).into_iter().collect::<Vec<_>>();
    values.sort();
    values.join(" ")
}

pub fn normalize_service_set(ids: Option<&[String]>) -> std::collections::HashSet<String> {
    ids.unwrap_or_default().iter().cloned().collect()
}

pub fn normalize_service_ids(ids: &[String]) -> Vec<String> {
    let mut values = normalize_service_set(Some(ids))
        .into_iter()
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn effective_authority_equal(
    existing: &Consent,
    scopes: &str,
    allow_all_services: bool,
    allowed_service_ids: Option<&Vec<String>>,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> bool {
    normalize_scope_set(&existing.scopes) == normalize_scope_set(scopes)
        && existing.allow_all_services == allow_all_services
        && (allow_all_services
            || normalize_service_set(existing.allowed_service_ids.as_deref())
                == normalize_service_set(allowed_service_ids.map(Vec::as_slice)))
        && existing.expires_at == expires_at
}

/// A token reference is accepted only as the original nonempty server-issued version.
pub fn protected_reference(
    consent: &Consent,
) -> AppResult<crate::models::consent::ProtectedConsentRef> {
    let revision = consent
        .revision
        .as_deref()
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(grant_changed)?;
    if consent.id.trim().is_empty() {
        return Err(grant_changed());
    }
    Ok(crate::models::consent::ProtectedConsentRef {
        consent_id: consent.id.clone(),
        consent_revision: revision.to_owned(),
    })
}

/// Upgrade only an actively authorized issuance snapshot. Existing tokens and
/// queued proofs never acquire a replacement revision from a later grant.
pub async fn ensure_issuance_reference(
    db: &mongodb::Database,
    consent: &Consent,
) -> AppResult<crate::models::consent::ProtectedConsentRef> {
    if consent
        .revision
        .as_deref()
        .is_some_and(|revision| !revision.trim().is_empty())
    {
        return protected_reference(consent);
    }
    if consent
        .expires_at
        .is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(grant_changed());
    }
    let revision = Uuid::new_v4().to_string();
    let updated = db
        .collection::<Consent>(CONSENTS)
        .update_one(
            version_filter(consent),
            doc! {"$set":{"revision":&revision,"issuance_fence":Uuid::new_v4().to_string()}},
        )
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    if updated.matched_count != 1 {
        return Err(grant_changed());
    }
    Ok(crate::models::consent::ProtectedConsentRef {
        consent_id: consent.id.clone(),
        consent_revision: revision,
    })
}

/// Indexed primary/linearizable authority read. A majority-acknowledged revoke
/// preceding this read cannot be hidden by another replica's positive snapshot.
/// First-party session refresh uses the reserved nil client without OAuth consent.
pub async fn validate_protected_reference(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scope: &str,
    reference: Option<&crate::models::consent::ProtectedConsentRef>,
) -> AppResult<Option<Consent>> {
    validate_protected_reference_for_route(db, user_id, client_id, scope, reference, "access").await
}

pub async fn validate_protected_reference_for_route(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scope: &str,
    reference: Option<&crate::models::consent::ProtectedConsentRef>,
    route_class: &'static str,
) -> AppResult<Option<Consent>> {
    validate_protected_reference_with_reads(
        &mut super::oauth_token_authority_service::AuthorityReads::for_route(db, route_class),
        user_id,
        client_id,
        scope,
        reference,
    )
    .await
}

pub(super) async fn validate_protected_reference_with_reads(
    reads: &mut super::oauth_token_authority_service::AuthorityReads<'_>,
    user_id: &str,
    client_id: &str,
    scope: &str,
    reference: Option<&crate::models::consent::ProtectedConsentRef>,
) -> AppResult<Option<Consent>> {
    if (client_id.is_empty() || client_id == Uuid::nil().to_string())
        && !scope.split_whitespace().any(|s| s == "machine")
        && reference.is_none()
    {
        return Ok(None);
    }
    let Some(reference) = reference else {
        reads.log_rejection(client_id, "legacy_unbound");
        return Err(grant_changed());
    };
    if reference.consent_id.trim().is_empty() || reference.consent_revision.trim().is_empty() {
        reads.log_rejection(client_id, "revision_mismatch");
        return Err(grant_changed());
    }
    let consent: Consent = match reads
        .read(
            CONSENTS,
            doc! {"_id":&reference.consent_id,"user_id":user_id,"client_id":client_id,"revision":&reference.consent_revision},
        )
        .await?
    {
        Some(consent) => consent,
        None => {
            let current: Option<Consent> = reads
                .read(
                    CONSENTS,
                    doc! {"_id":&reference.consent_id,"user_id":user_id,"client_id":client_id},
                )
                .await?;
            reads.log_rejection(
                client_id,
                if current.is_some() {
                    "revision_mismatch"
                } else {
                    "consent_missing"
                },
            );
            return Err(grant_changed());
        }
    };
    let granted: std::collections::HashSet<_> = consent.scopes.split_whitespace().collect();
    if consent
        .expires_at
        .is_some_and(|expiry| expiry <= Utc::now())
        || !scope
            .split_whitespace()
            .all(|requested| granted.contains(requested))
        || protected_reference(&consent)? != *reference
    {
        let reason = if consent
            .expires_at
            .is_some_and(|expiry| expiry <= Utc::now())
        {
            "expired"
        } else {
            "scope_not_granted"
        };
        reads.log_rejection(client_id, reason);
        return Err(grant_changed());
    }
    Ok(Some(consent))
}

/// Issuance writes the same source row as revocation, after refresh persistence.
/// A revoke racing before this fence makes issuance fail; one racing after it
/// observes the persisted refresh row. Machine admission additionally rechecks live.
pub async fn fence_protected_reference(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scope: &str,
    reference: Option<&crate::models::consent::ProtectedConsentRef>,
) -> AppResult<()> {
    fence_protected_reference_for_route(db, user_id, client_id, scope, reference, "access").await
}

pub async fn fence_protected_reference_for_route(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scope: &str,
    reference: Option<&crate::models::consent::ProtectedConsentRef>,
    route_class: &'static str,
) -> AppResult<()> {
    let Some(consent) = validate_protected_reference_for_route(
        db,
        user_id,
        client_id,
        scope,
        reference,
        route_class,
    )
    .await?
    else {
        return Ok(());
    };
    let result = db
        .collection::<Consent>(CONSENTS)
        .update_one(
            version_filter(&consent),
            doc! { "$set": { "issuance_fence": Uuid::new_v4().to_string() } },
        )
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    if result.matched_count != 1 {
        return Err(grant_changed());
    }
    Ok(())
}

pub fn grant_changed() -> AppError {
    AppError::Conflict(
        "Service access changed or expired. Restart authorization to review the current grant."
            .to_string(),
    )
}

pub fn version_filter(consent: &Consent) -> bson::Document {
    // Include the legacy fields as well: during a rolling deploy an older
    // replica can replace a pre-versioned row without introducing a revision.
    let mut filter = doc! { "_id": &consent.id, "user_id": &consent.user_id,
        "client_id": &consent.client_id, "revision": &consent.revision,
        "write_version": &consent.write_version,
        "scopes": &consent.scopes, "allowed_service_ids": &consent.allowed_service_ids,
        "granted_at": bson::DateTime::from_chrono(consent.granted_at),
        "expires_at": consent.expires_at.map(bson::DateTime::from_chrono),
    };
    filter.insert(
        "allow_all_services",
        if consent.allow_all_services {
            bson::Bson::Boolean(true)
        } else {
            bson::Bson::Document(doc! { "$ne": true })
        },
    );
    filter
}

pub fn fingerprint(consent: &Consent) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    let mut snapshot = consent.clone();
    snapshot.issuance_fence = None;
    let mut document =
        bson::to_document(&snapshot).map_err(|e| AppError::Internal(e.to_string()))?;
    document.remove("write_version");
    let bytes = bson::to_vec(&document).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// Compare-and-swap only: a deleted/revoked grant must never be upserted by an
/// old browser decision. Normal review writes use the same version fence.
#[cfg(test)]
pub async fn replace_incremental_consent(
    db: &mongodb::Database,
    existing: &Consent,
    scopes: &str,
    allowed_service_ids: Vec<String>,
) -> AppResult<Consent> {
    replace_incremental_consent_with_machine(db, existing, scopes, allowed_service_ids, None).await
}

pub async fn replace_incremental_consent_with_machine(
    db: &mongodb::Database,
    existing: &Consent,
    scopes: &str,
    allowed_service_ids: Vec<String>,
    machine: Option<&super::machine_oauth_consent_service::MachineConsentApproval>,
) -> AppResult<Consent> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let existing = existing.clone();
    let scopes = scopes.to_owned();
    let machine = machine.cloned();
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let db = &db;
            let existing = &existing;
            let scopes = scopes.as_str();
            let machine = machine.as_ref();
            let result: AppResult<Consent> = async {
                if existing.expires_at.is_some_and(|e| e <= Utc::now()) {
                    return Err(grant_changed());
                }
                write_consent_in_session(
                    db,
                    &existing.user_id,
                    &existing.client_id,
                    Some(existing),
                    scopes,
                    existing.allow_all_services,
                    Some(allowed_service_ids.clone()),
                    existing.expires_at,
                    machine,
                    false,
                    session,
                )
                .await
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Check if a user has granted consent for the requested scopes to a client.
/// Returns Some(Consent) if all requested scopes are covered.
pub async fn check_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    requested_scopes: &str,
) -> AppResult<Option<Consent>> {
    let consent = db
        .collection::<Consent>(CONSENTS)
        .find_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?;

    match consent {
        Some(c) => {
            // Check if the consent has expired
            if let Some(expires_at) = c.expires_at
                && expires_at <= Utc::now()
            {
                return Ok(None);
            }

            let granted: std::collections::HashSet<&str> = c.scopes.split_whitespace().collect();
            let requested: Vec<&str> = requested_scopes.split_whitespace().collect();

            let all_covered = requested.iter().all(|s| granted.contains(s));
            if all_covered { Ok(Some(c)) } else { Ok(None) }
        }
        None => Ok(None),
    }
}

/// Revoke consent for a specific client.
pub async fn revoke_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
) -> AppResult<ConsentRevocationResult> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user_id = user_id.to_owned();
    let client_id = client_id.to_owned();
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<ConsentRevocationResult> = async {
                // Conflict with Machine consent preparation on the same live
                // owner source row, then remove every unconsumed request. This
                // prevents absent->approved->deleted consent ABA from making a
                // formerly pending initial request authoritative again.
                db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&user_id},
                        // Keep in-flight Machine selections stale even across
                        // absent -> approved -> revoked -> absent consent ABA.
                        // This is not a grant security epoch: other live grants
                        // retain their authority and ordinary fence writes do
                        // not invalidate the preparation's consent baseline.
                        doc! {"$inc":{"machine_external_fence":1_i64,"machine_consent_revocation_revision":1_i64}},
                    )
                    .session(&mut *session)
                    .await?;
                db.collection::<bson::Document>(
                    crate::models::machine_oauth_consent_request::COLLECTION_NAME,
                )
                .delete_many(
                    doc! {"user_id":&user_id,"client_id":&client_id,"consumed_at":bson::Bson::Null},
                )
                .session(&mut *session)
                .await?;
                // Retain the revocation generation after deleting consent so
                // older browser decisions cannot resurrect the revoked grant.
                // Explicit ownership also allows personal-account erasure.
                db.collection::<bson::Document>(REVOCATION_FENCES)
                    .update_one(
                        doc! {"_id": format!("{user_id}:{client_id}")},
                        doc! {"$set": {"user_id":&user_id,"client_id":&client_id,"revoked_at": bson::DateTime::now(), "version":Uuid::new_v4().to_string()}},
                    )
                    .upsert(true)
                    .session(&mut *session)
                    .await?;
                let deleted = db
                    .collection::<Consent>(CONSENTS)
                    .delete_one(doc! { "user_id": &user_id, "client_id": &client_id })
                    .session(&mut *session)
                    .await?;
                if deleted.deleted_count == 0 {
                    return Err(AppError::ConsentNotFound);
                }
                super::machine_external_grant_service::invalidate_oauth_in_session(
                    &db,
                    &user_id,
                    &client_id,
                    "consent_revoked",
                    session,
                )
                .await?;
                let revoked_refresh_tokens = if client_id == Uuid::nil().to_string() {
                    0
                } else {
                    db.collection::<RefreshToken>(REFRESH_TOKENS).update_many(
                    doc! { "user_id": &user_id, "client_id": &client_id, "revoked": false },
                    doc! { "$set": { "revoked": true, "revoked_at": bson::DateTime::now() } },
                ).session(&mut *session).await?.modified_count
                };
                Ok(ConsentRevocationResult {
                    revoked_refresh_tokens,
                })
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentRevocationResult {
    pub revoked_refresh_tokens: u64,
}

/// List all consents for a user.
pub async fn list_user_consents(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<Consent>> {
    let consents: Vec<Consent> = db
        .collection::<Consent>(CONSENTS)
        .find(doc! { "user_id": user_id })
        .await?
        .try_collect()
        .await?;

    Ok(consents)
}

/// List all consents for a client.
pub async fn list_client_consents(
    db: &mongodb::Database,
    client_id: &str,
) -> AppResult<Vec<Consent>> {
    let consents: Vec<Consent> = db
        .collection::<Consent>(CONSENTS)
        .find(doc! { "client_id": client_id })
        .await?
        .try_collect()
        .await?;

    Ok(consents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
    use crate::test_utils::*;
    use chrono::Duration;

    #[tokio::test]
    async fn test_grant_consent_creates_new() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        let consent = grant_consent(&db, &user_id, &client_id, "openid profile")
            .await
            .unwrap();

        assert_eq!(consent.user_id, user_id);
        assert_eq!(consent.client_id, client_id);
        assert_eq!(consent.scopes, "openid profile");
        assert!(!consent.allow_all_services);
        assert_eq!(consent.allowed_service_ids, Some(vec![]));
        assert!(consent.expires_at.is_none());

        let stored = db
            .collection::<Consent>(CONSENTS)
            .find_one(doc! { "_id": &consent.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.scopes, "openid profile");
    }

    #[tokio::test]
    async fn test_grant_consent_upserts_existing() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        let first = grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        let second = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid profile email",
            Some(vec!["svc-1".to_string()]),
        )
        .await
        .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.scopes, "email openid profile");
        assert!(!second.allow_all_services);
        assert_eq!(second.allowed_service_ids, Some(vec!["svc-1".to_string()]));

        let count = db
            .collection::<Consent>(CONSENTS)
            .count_documents(doc! { "user_id": &user_id, "client_id": &client_id })
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn unchanged_consent_is_a_no_op_even_when_ordered_differently() {
        let Some(db) = connect_test_database("consent_no_op").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let first = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid profile email",
            Some(vec!["svc-b".to_string(), "svc-a".to_string()]),
        )
        .await
        .unwrap();
        let second = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "email openid openid profile",
            Some(vec![
                "svc-a".to_string(),
                "svc-b".to_string(),
                "svc-a".to_string(),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(second.id, first.id);
        assert_eq!(second.revision, first.revision);
        assert_eq!(
            second.granted_at.timestamp_millis(),
            first.granted_at.timestamp_millis()
        );
        assert_eq!(second.issuance_fence, first.issuance_fence);
    }

    #[tokio::test]
    async fn ordinary_union_keeps_revision_and_existing_authority() {
        let Some(db) = connect_test_database("consent_union").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let first = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid",
            Some(vec!["svc-a".to_string()]),
        )
        .await
        .unwrap();
        let broadened = grant_consent_union_with_machine(
            &db,
            &user_id,
            &client_id,
            "profile",
            false,
            Some(vec!["svc-b".to_string()]),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(broadened.revision, first.revision);
        assert!(broadened.granted_at > first.granted_at);
        assert_eq!(
            normalize_scope_set(&broadened.scopes),
            normalize_scope_set("openid profile")
        );
        assert_eq!(
            broadened.allowed_service_ids,
            Some(vec!["svc-a".to_string(), "svc-b".to_string()])
        );
    }

    #[tokio::test]
    async fn revoked_consent_request_cannot_resurrect_authority() {
        let Some(db) = connect_test_database("consent_replay_fence").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        let issued_at = Utc::now().timestamp();
        revoke_consent(&db, &user_id, &client_id).await.unwrap();
        let result = grant_consent_union_with_machine(
            &db,
            &user_id,
            &client_id,
            "openid",
            false,
            Some(Vec::new()),
            None,
            Some(issued_at),
            None,
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
        assert!(
            check_consent(&db, &user_id, &client_id, "openid")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn revocation_generation_fences_requests_independently_of_clock_or_same_second() {
        let db = connect_transaction_test_database("consent_generation").await;
        let user = Uuid::new_v4().to_string();
        let client = Uuid::new_v4().to_string();
        grant_consent(&db, &user, &client, "openid").await.unwrap();
        let previous = consent_request_fence(&db, &user, &client).await.unwrap();
        revoke_consent(&db, &user, &client).await.unwrap();
        assert!(
            grant_consent_union_with_machine(
                &db,
                &user,
                &client,
                "openid",
                false,
                Some(vec![]),
                None,
                Some(Utc::now().timestamp() + 60),
                Some(previous)
            )
            .await
            .is_err()
        );
        let fresh = consent_request_fence(&db, &user, &client).await.unwrap();
        let first = grant_consent_union_with_machine(
            &db,
            &user,
            &client,
            "openid",
            false,
            Some(vec![]),
            None,
            Some(Utc::now().timestamp() - 60),
            Some(fresh.clone()),
        )
        .await
        .unwrap();
        let repeated = grant_consent_union_with_machine(
            &db,
            &user,
            &client,
            "openid",
            false,
            Some(vec![]),
            None,
            Some(Utc::now().timestamp() - 60),
            Some(fresh),
        )
        .await
        .unwrap();
        assert_eq!(first.write_version, repeated.write_version);
        assert_eq!(first.revision, repeated.revision);
    }

    #[tokio::test]
    async fn concurrent_ordinary_broadenings_do_not_drop_either_addition() {
        let Some(db) = connect_test_database("consent_union_concurrent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid",
            Some(vec!["svc-base".to_string()]),
        )
        .await
        .unwrap();
        let (left, right) = tokio::join!(
            grant_consent_union_with_machine(
                &db,
                &user_id,
                &client_id,
                "profile",
                false,
                Some(vec!["svc-left".to_string()]),
                None,
                None,
                None,
            ),
            grant_consent_union_with_machine(
                &db,
                &user_id,
                &client_id,
                "email",
                false,
                Some(vec!["svc-right".to_string()]),
                None,
                None,
                None,
            )
        );
        assert!(left.is_ok(), "left broadening failed: {left:?}");
        assert!(right.is_ok(), "right broadening failed: {right:?}");
        let final_consent = check_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap()
            .expect("base consent remains");
        let scopes = normalize_scope_set(&final_consent.scopes);
        let services = normalize_service_set(final_consent.allowed_service_ids.as_deref());
        assert!(scopes.contains("profile"));
        assert!(scopes.contains("email"));
        assert!(services.contains("svc-left"));
        assert!(services.contains("svc-right"));
    }

    #[tokio::test]
    async fn consent_write_version_fences_broadening_and_legacy_replacement() {
        let db = connect_transaction_test_database("consent_cas").await;
        let user = Uuid::new_v4().to_string();
        let client = Uuid::new_v4().to_string();
        let first = grant_consent(&db, &user, &client, "openid").await.unwrap();
        let second = grant_consent(&db, &user, &client, "openid profile")
            .await
            .unwrap();
        assert_eq!(first.revision, second.revision);
        assert_ne!(first.write_version, second.write_version);
        assert_eq!(
            db.collection::<Consent>(CONSENTS)
                .count_documents(version_filter(&first))
                .await
                .unwrap(),
            0
        );
        // An old writer drops the additive field and rotates the revision.
        db.collection::<bson::Document>(CONSENTS)
            .update_one(
                doc! {"_id":&second.id},
                doc! {"$unset":{"write_version":""},"$set":{"revision":Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
        assert_eq!(
            db.collection::<Consent>(CONSENTS)
                .count_documents(version_filter(&second))
                .await
                .unwrap(),
            0
        );
        let legacy = check_consent(&db, &user, &client, "")
            .await
            .unwrap()
            .unwrap();
        assert!(legacy.write_version.is_none());
        assert!(
            grant_consent(&db, &user, &client, "openid profile email")
                .await
                .unwrap()
                .write_version
                .is_some()
        );
    }

    #[tokio::test]
    async fn allow_all_ignores_service_ids_and_expiry_is_preserved_until_regrant() {
        let db = connect_transaction_test_database("consent_expiry").await;
        let user = Uuid::new_v4().to_string();
        let client = Uuid::new_v4().to_string();
        let mut first = grant_consent_with_services(&db, &user, &client, "openid", None)
            .await
            .unwrap();
        first.allowed_service_ids = Some(vec!["irrelevant".into()]);
        first.expires_at =
            Some(bson::DateTime::from_chrono(Utc::now() + Duration::hours(1)).to_chrono());
        db.collection::<Consent>(CONSENTS)
            .replace_one(doc! {"_id":&first.id}, &first)
            .await
            .unwrap();
        let equal = grant_consent_union_with_machine(
            &db, &user, &client, "openid", true, None, None, None, None,
        )
        .await
        .unwrap();
        assert_eq!(equal.write_version, first.write_version);
        assert_eq!(equal.revision, first.revision);
        assert_eq!(equal.expires_at, first.expires_at);
        let broad = grant_consent_union_with_machine(
            &db,
            &user,
            &client,
            "profile",
            false,
            Some(vec![]),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(broad.revision, first.revision);
        assert_eq!(broad.expires_at, first.expires_at);
        db.collection::<Consent>(CONSENTS).update_one(doc! {"_id":&first.id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        let new = grant_consent_union_with_machine(
            &db,
            &user,
            &client,
            "email",
            false,
            Some(vec![]),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_ne!(new.revision, first.revision);
        assert_eq!(new.scopes, "email");
        assert!(!new.allow_all_services);
        assert!(new.expires_at.is_none());
    }

    #[tokio::test]
    async fn approval_racing_revoke_cannot_resurrect_and_double_submit_is_no_op() {
        let db = connect_transaction_test_database("consent_revoke_race").await;
        let user = Uuid::new_v4().to_string();
        let client = Uuid::new_v4().to_string();
        let first = grant_consent(&db, &user, &client, "openid").await.unwrap();
        let iat = Utc::now().timestamp();
        let (approval, revoke) = tokio::join!(
            grant_consent_union_with_machine(
                &db,
                &user,
                &client,
                "profile",
                false,
                Some(vec![]),
                None,
                Some(iat),
                None,
            ),
            revoke_consent(&db, &user, &client)
        );
        assert!(revoke.is_ok());
        if let Ok(approved) = approval {
            assert_eq!(approved.revision, first.revision);
        }
        assert!(
            check_consent(&db, &user, &client, "")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            grant_consent_union_with_machine(
                &db,
                &user,
                &client,
                "openid",
                false,
                Some(vec![]),
                None,
                Some(iat),
                None,
            )
            .await
            .is_err()
        );
        let fresh = grant_consent_union_with_machine(
            &db,
            &user,
            &client,
            "openid",
            false,
            Some(vec![]),
            None,
            Some(iat + 1),
            None,
        )
        .await
        .unwrap();
        let repeated = grant_consent_union_with_machine(
            &db,
            &user,
            &client,
            "openid openid",
            false,
            Some(vec![]),
            None,
            Some(iat + 1),
            None,
        )
        .await
        .unwrap();
        assert_eq!(fresh.write_version, repeated.write_version);
        assert_eq!(fresh.revision, repeated.revision);
        assert_eq!(
            fresh.granted_at.timestamp_millis(),
            repeated.granted_at.timestamp_millis()
        );
    }

    #[tokio::test]
    async fn authority_rejection_telemetry_distinguishes_reason_and_route_once() {
        use tracing::instrument::WithSubscriber;
        let db = connect_transaction_test_database("consent_logs").await;
        let capture = tempfile::NamedTempFile::new().unwrap();
        let writer = capture.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .json()
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        async {
            for route in ["access", "refresh", "mcp"] {
                let user = Uuid::new_v4().to_string();
                let client = Uuid::new_v4().to_string();
                let grant = grant_consent(&db, &user, &client, "openid").await.unwrap();
                let reference = protected_reference(&grant).unwrap();
                for reason in ["scope_not_granted", "expired", "revision_mismatch", "consent_missing"] {
                    match reason {
                        "expired" => { db.collection::<Consent>(CONSENTS).update_one(doc! {"_id":&grant.id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap(); }
                        "revision_mismatch" => { db.collection::<Consent>(CONSENTS).update_one(doc! {"_id":&grant.id}, doc! {"$set":{"expires_at":bson::Bson::Null,"revision":Uuid::new_v4().to_string()}}).await.unwrap(); }
                        "consent_missing" => { db.collection::<Consent>(CONSENTS).delete_one(doc! {"_id":&grant.id}).await.unwrap(); }
                        _ => {}
                    }
                    let mut reads = super::super::oauth_token_authority_service::AuthorityReads::for_route(&db, route);
                    for _ in 0..2 {
                        assert!(validate_protected_reference_with_reads(&mut reads, &user, &client,
                            if reason == "scope_not_granted" { "email" } else { "openid" }, Some(&reference)).await.is_err());
                    }
                }
            }
        }.with_subscriber(subscriber).await;
        let logs = std::fs::read_to_string(capture.path()).unwrap();
        let events = logs
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|row| row["fields"]["message"] == "Protected OAuth authority rejected")
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 12);
        for route in ["access", "refresh", "mcp"] {
            for reason in [
                "scope_not_granted",
                "expired",
                "revision_mismatch",
                "consent_missing",
            ] {
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e["fields"]["route_class"] == route
                            && e["fields"]["reason"] == reason)
                        .count(),
                    1
                );
            }
        }
        assert!(!logs.contains("openid"));
        assert!(!logs.contains("email"));
    }

    #[tokio::test]
    async fn test_grant_consent_with_services_persists_allowlist() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        let consent = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid",
            Some(vec!["svc-1".to_string(), "svc-2".to_string()]),
        )
        .await
        .unwrap();

        assert_eq!(
            consent.allowed_service_ids,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()])
        );
        assert!(!consent.allow_all_services);

        let updated =
            grant_consent_with_services(&db, &user_id, &client_id, "openid", Some(vec![]))
                .await
                .unwrap();
        assert_eq!(updated.id, consent.id);
        assert!(!updated.allow_all_services);
        assert_eq!(updated.allowed_service_ids, Some(vec![]));

        let unrestricted = grant_consent_with_services(&db, &user_id, &client_id, "openid", None)
            .await
            .unwrap();
        assert_eq!(unrestricted.id, consent.id);
        assert!(unrestricted.allow_all_services);
        assert_eq!(unrestricted.allowed_service_ids, Some(vec![]));
    }

    #[tokio::test]
    async fn test_check_consent_covers_all_scopes() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        grant_consent(&db, &user_id, &client_id, "openid profile email")
            .await
            .unwrap();

        let found = check_consent(&db, &user_id, &client_id, "openid profile")
            .await
            .unwrap();
        assert!(found.is_some());

        let missing = check_consent(&db, &user_id, &client_id, "openid admin")
            .await
            .unwrap();
        assert!(missing.is_none());

        let no_consent = check_consent(&db, &user_id, &Uuid::new_v4().to_string(), "openid")
            .await
            .unwrap();
        assert!(no_consent.is_none());
    }

    #[tokio::test]
    async fn test_revoke_consent_deletes_and_errors_on_missing() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        let result = revoke_consent(&db, &user_id, &client_id).await.unwrap();
        assert_eq!(result.revoked_refresh_tokens, 0);

        let fence = db
            .collection::<bson::Document>(REVOCATION_FENCES)
            .find_one(doc! {"user_id":&user_id,"client_id":&client_id})
            .await
            .unwrap()
            .expect("revocation fence retains explicit ownership for erasure");
        assert_eq!(
            fence.get_str("_id").unwrap(),
            format!("{user_id}:{client_id}")
        );

        let after = check_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        assert!(after.is_none());

        let err = revoke_consent(&db, &user_id, &client_id).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_revoke_consent_revokes_refresh_token_chain_for_client_only() {
        let Some(db) = connect_test_database("consent_revoke_refresh").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let other_client_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.clone(),
                client_name: "Revoked Client".to_string(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["https://app.example/callback".to_string()],
                allowed_scopes: "openid profile".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: Some(user_id.clone()),
                default_service_catalog_slugs: Vec::new(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        let user_uuid = Uuid::parse_str(&user_id).unwrap();
        let (refresh_jwt, refresh_jti) =
            crate::crypto::jwt::generate_refresh_token(&jwt_keys, &config, &user_uuid).unwrap();
        let first_party_jti = Uuid::new_v4().to_string();
        let other_client_jti = Uuid::new_v4().to_string();

        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_many([
                RefreshToken {
                    protected_consent: None,
                    id: Uuid::new_v4().to_string(),
                    jti: refresh_jti.clone(),
                    client_id: client_id.clone(),
                    user_id: user_id.clone(),
                    session_id: None,
                    scope: Some("openid".to_string()),
                    expires_at: now + Duration::days(7),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    created_at: now,
                },
                RefreshToken {
                    protected_consent: None,
                    id: Uuid::new_v4().to_string(),
                    jti: first_party_jti.clone(),
                    client_id: Uuid::nil().to_string(),
                    user_id: user_id.clone(),
                    session_id: None,
                    scope: None,
                    expires_at: now + Duration::days(7),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    created_at: now,
                },
                RefreshToken {
                    protected_consent: None,
                    id: Uuid::new_v4().to_string(),
                    jti: other_client_jti.clone(),
                    client_id: other_client_id,
                    user_id: user_id.clone(),
                    session_id: None,
                    scope: Some("openid".to_string()),
                    expires_at: now + Duration::days(7),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    created_at: now,
                },
            ])
            .await
            .unwrap();

        let result = revoke_consent(&db, &user_id, &client_id).await.unwrap();
        assert_eq!(result.revoked_refresh_tokens, 1);

        let revoked = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &refresh_jti })
            .await
            .unwrap()
            .unwrap();
        assert!(revoked.revoked);
        assert!(revoked.revoked_at.is_some());

        let first_party = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &first_party_jti })
            .await
            .unwrap()
            .unwrap();
        assert!(!first_party.revoked);
        let other_client = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &other_client_jti })
            .await
            .unwrap()
            .unwrap();
        assert!(!other_client.revoked);

        let refresh_result = crate::services::token_service::refresh_tokens(
            &db,
            &config,
            &jwt_keys,
            &refresh_jwt,
            None,
            None,
        )
        .await;
        assert!(matches!(refresh_result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn test_list_user_and_client_consents() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_a = Uuid::new_v4().to_string();
        let client_b = Uuid::new_v4().to_string();

        grant_consent(&db, &user_id, &client_a, "openid")
            .await
            .unwrap();
        grant_consent(&db, &user_id, &client_b, "profile")
            .await
            .unwrap();

        let user_consents = list_user_consents(&db, &user_id).await.unwrap();
        assert_eq!(user_consents.len(), 2);

        let client_consents = list_client_consents(&db, &client_a).await.unwrap();
        assert_eq!(client_consents.len(), 1);
        assert_eq!(client_consents[0].scopes, "openid");

        let empty = list_user_consents(&db, &Uuid::new_v4().to_string())
            .await
            .unwrap();
        assert!(empty.is_empty());
    }
}
