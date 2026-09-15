//! Authorize gates use local evidence only. Consent binds the immutable result,
//! subject, client and session revision; completion and code publication commit together.

use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use super::app_requirements_service::{RequirementState, RequirementsReport};
use super::{
    app_connect_link_service as links, app_connect_rollout, app_requirement_manifest_service,
    oauth_resource_service,
};
use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::app_connect_link::{
    AppConnectLink, AppConnectOrigin, AppConnectStatus, COLLECTION_NAME as LINKS,
};
use crate::models::app_requirement_manifest::{
    AppRequirementManifest, Enforcement, ValidatorSelection,
};
use crate::models::app_requirement_result::{AppRequirementResult, COLLECTION_NAME as RESULTS};
use crate::models::authorization_code::{AuthorizationCode, COLLECTION_NAME as CODES};
use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
use crate::models::oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient};

pub const GATE_WINDOW_SECS: i64 = 60;

#[derive(Clone, Serialize, Deserialize)]
pub struct ConsentBinding {
    pub result_id: String,
    pub session_id: String,
    pub nonce: String,
}

impl std::fmt::Debug for ConsentBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsentBinding").finish_non_exhaustive()
    }
}

pub async fn gate_manifest(
    state: &AppState,
    client: &OauthClient,
) -> AppResult<Option<AppRequirementManifest>> {
    if client.current_manifest_version.is_none()
        || !app_connect_rollout::is_enabled_for(state, client).await?
    {
        return Ok(None);
    }
    let manifest = app_requirement_manifest_service::current(&state.db, client).await?;
    Ok((manifest.enforcement == Enforcement::Gate).then_some(manifest))
}

pub fn required_fresh(manifest: &AppRequirementManifest, report: &RequirementsReport) -> bool {
    let now = Utc::now();
    let mut times = Vec::new();
    for requirement in manifest.requirements.iter().filter(|r| !r.optional) {
        let Some(status) = report
            .requirements
            .iter()
            .find(|r| r.requirement_id == requirement.id)
        else {
            return false;
        };
        if !matches!(
            status.state,
            RequirementState::Met | RequirementState::Included
        ) || status.user_service_id.is_none()
        {
            return false;
        }
        // Included and StoredOnly have local authority, not fabricated probe timestamps.
        if status.state != RequirementState::Included
            && matches!(requirement.validator, ValidatorSelection::Profile { .. })
        {
            let Some(checked) = status.validated_at else {
                return false;
            };
            if checked > now
                || now - checked > Duration::seconds(GATE_WINDOW_SECS)
                || status.valid_until.is_none_or(|until| until <= now)
            {
                return false;
            }
            times.push(checked);
        }
    }
    times
        .iter()
        .min()
        .zip(times.iter().max())
        .is_none_or(|(old, new)| *new - *old <= Duration::seconds(GATE_WINDOW_SECS))
}

pub fn selected_ids(report: &RequirementsReport) -> Vec<String> {
    let mut ids: Vec<_> = report
        .requirements
        .iter()
        .filter(|r| matches!(r.state, RequirementState::Met | RequirementState::Included))
        .filter_map(|r| r.user_service_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

pub fn consent_covers(consent: Option<&Consent>, ids: &[String]) -> bool {
    consent.is_some_and(|c| {
        c.allow_all_services
            || c.allowed_service_ids
                .as_ref()
                .is_some_and(|allowed| ids.iter().all(|id| allowed.contains(id)))
    })
}

/// Load the signed association, never any form-supplied result/session identifier.
pub async fn bound_session(
    state: &AppState,
    subject: &str,
    client: &OauthClient,
    binding: &ConsentBinding,
) -> AppResult<AppConnectLink> {
    if !app_connect_rollout::is_enabled_for(state, client).await? {
        return Err(AppError::AppConnectResultMismatch);
    }
    let link = links::load(state, &binding.session_id, subject).await?;
    let nonce_matches = matches!(&link.origin, AppConnectOrigin::Authorize { consent_nonce, .. } if consent_nonce == &binding.nonce);
    if link.oauth_client_id != client.id
        || link.status != AppConnectStatus::ReadyForConsent
        || link.result_id.as_deref() != Some(&binding.result_id)
        || !nonce_matches
        || link.expires_at <= Utc::now()
        || link.redeemed_at.is_none()
    {
        return Err(AppError::AppConnectResultMismatch);
    }
    let result = state
        .db
        .collection::<AppRequirementResult>(RESULTS)
        .find_one(doc! {
            "_id": &binding.result_id, "user_id": subject, "oauth_client_id": &client.id,
            "manifest_id": &link.manifest_id, "manifest_version": i64::from(link.manifest_version),
            "expires_at": { "$gt": bson::DateTime::now() },
        })
        .await?
        .ok_or(AppError::AppConnectResultMismatch)?;
    let mut ids: Vec<_> = result
        .selections
        .iter()
        .filter_map(|r| r.user_service_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    if ids != link.selected_service_ids {
        return Err(AppError::AppConnectResultMismatch);
    }
    Ok(link)
}

pub async fn recheck_authority(state: &AppState, link: &AppConnectLink) -> AppResult<()> {
    let manifest = links::manifest(state, link).await?;
    let report = links::evaluate(state, link, &manifest).await?;
    // A consent screen may remain open for fifteen minutes. Recheck current local
    // eligibility/digest (the normal evidence window), not the 60-second entry window.
    for requirement in manifest.requirements.iter().filter(|r| !r.optional) {
        let selected = link
            .items
            .iter()
            .find(|i| i.requirement_id == requirement.id)
            .and_then(|i| i.user_service_id.as_ref());
        if !report.requirements.iter().any(|r| {
            r.requirement_id == requirement.id
                && r.user_service_id.as_ref() == selected
                && matches!(r.state, RequirementState::Met | RequirementState::Included)
        }) {
            return Err(AppError::AppConnectResultMismatch);
        }
    }
    if !oauth_resource_service::validate_grantable_service_ids(
        &state.db,
        &link.user_id,
        &link.selected_service_ids,
    )
    .await?
    {
        return Err(AppError::AppConnectResultMismatch);
    }
    Ok(())
}

pub async fn complete_with_code(
    db: &mongodb::Database,
    link: &AppConnectLink,
    consent: &Consent,
    code: &AuthorizationCode,
) -> AppResult<()> {
    let tx_db = db.clone();
    let link = link.clone();
    let consent = consent.clone();
    let code = code.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<()> = async {
            let client = tx_db.collection::<OauthClient>(CLIENTS).find_one(doc! {
                "_id": &link.oauth_client_id, "is_active": true, "app_connect_capability_enabled": true,
            }).session(&mut *session).await?;
            if client.is_none() { return Err(AppError::AppConnectResultMismatch); }
            let completed = tx_db.collection::<AppConnectLink>(LINKS).update_one(doc! {
                "_id": &link.id, "revision": link.revision, "status": "ready_for_consent",
                "result_id": &link.result_id, "expires_at": { "$gt": bson::DateTime::now() },
            }, doc! { "$set": { "status": "completed", "completed_at": bson::DateTime::now() }, "$inc": { "revision": 1 } })
                .session(&mut *session).await?;
            if completed.modified_count != 1 { return Err(AppError::AppConnectResultMismatch); }
            tx_db.collection::<Consent>(CONSENTS).update_one(
                doc! { "user_id": &consent.user_id, "client_id": &consent.client_id },
                doc! { "$set": { "scopes": &consent.scopes, "allow_all_services": consent.allow_all_services,
                    "allowed_service_ids": &consent.allowed_service_ids, "granted_at": bson::DateTime::from_chrono(consent.granted_at), "expires_at": null },
                    "$setOnInsert": { "_id": &consent.id, "user_id": &consent.user_id, "client_id": &consent.client_id } },
            ).upsert(true).session(&mut *session).await?;
            tx_db.collection::<AuthorizationCode>(CODES).insert_one(&code).session(&mut *session).await?;
            Ok(())
        }.await;
        super::api_key_mutation_service::transaction_result(operation)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)
}
