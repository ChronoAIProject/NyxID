//! Repair sessions bind a human subject, an app, and an immutable manifest.
//! Reads reconcile durable child completions and local evidence only. Provider I/O
//! requires an explicit item validation. Revision CAS and item attempt identities
//! prevent late work from reviving cancelled sessions or replacing a newer choice.

use std::collections::BTreeMap;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::ReturnDocument;
use uuid::Uuid;

use super::app_requirements_service::{EvaluationCaller, RequirementState, RequirementsReport};
use super::{
    app_connect_rollout, app_requirement_manifest_service, app_requirements_service,
    connect_link_service, oauth_service, org_service, service_validation_service,
    validator_profiles,
};
use crate::AppState;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::app_connect_link::{
    AppConnectItem, AppConnectLink, AppConnectOrigin, AppConnectStatus, COLLECTION_NAME, ItemState,
};
use crate::models::app_requirement_manifest::{
    AppRequirementManifest, COLLECTION_NAME as MANIFESTS, ServiceRequirement, ValidatorSelection,
};
use crate::models::connect_link::{COLLECTION_NAME as CHILDREN, ConnectLink, ConnectLinkStatus};
use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
use crate::models::oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient};
use crate::models::service_validation_record::CallerContext;
use crate::models::user_api_key::{COLLECTION_NAME as KEYS, UserApiKey};
use crate::models::user_service::{COLLECTION_NAME as SERVICES, UserService};

pub struct CreatedSession {
    pub link: AppConnectLink,
    pub connect_url: String,
}

pub async fn enabled_client(state: &AppState, id: &str) -> AppResult<OauthClient> {
    let client = state
        .db
        .collection::<OauthClient>(CLIENTS)
        .find_one(doc! { "_id": id })
        .await?
        .ok_or(AppError::AppConnectLinkNotFound)?;
    if !app_connect_rollout::is_enabled_for(state, &client).await? {
        return Err(AppError::AppConnectLinkNotFound);
    }
    Ok(client)
}

pub async fn start_from_app(
    state: &AppState,
    client_id: &str,
    user_id: &str,
    callback_url: &str,
    correlation: &str,
) -> AppResult<CreatedSession> {
    let client = enabled_client(state, client_id).await?;
    if correlation.is_empty() || correlation.len() > 1024 || callback_url.len() > 2048 {
        return Err(AppError::ValidationError(
            "state is required (at most 1024 bytes); callback_url is limited to 2048 bytes".into(),
        ));
    }
    let callback = url::Url::parse(callback_url)
        .map_err(|_| AppError::ValidationError("callback_url must be absolute".into()))?;
    if !callback.username().is_empty()
        || callback.password().is_some()
        || callback.fragment().is_some()
        || matches!(callback.scheme(), "javascript" | "data" | "file")
    {
        return Err(AppError::ValidationError(
            "callback_url must not contain userinfo or a fragment".into(),
        ));
    }
    oauth_service::validate_client(&state.db, client_id, callback_url).await?;
    let manifest = app_requirement_manifest_service::current(&state.db, &client).await?;
    let prior =
        app_requirements_service::prior_explicit_selections(&state.db, user_id, client_id).await?;
    let report = app_requirements_service::evaluate_local(
        state,
        &manifest,
        user_id,
        &EvaluationCaller {
            client_id,
            allow_all_services: true,
            allowed_service_ids: &[],
            explicit_selections: prior.clone(),
        },
    )
    .await?;
    let now = Utc::now();
    let capability = generate_random_token();
    let link = AppConnectLink {
        id: Uuid::new_v4().to_string(),
        oauth_client_id: client_id.into(),
        user_id: user_id.into(),
        manifest_id: manifest.id.clone(),
        manifest_version: manifest.version,
        origin: AppConnectOrigin::App {
            callback_url: callback_url.into(),
            state: correlation.into(),
        },
        items: report
            .requirements
            .iter()
            .map(|r| AppConnectItem {
                requirement_id: r.requirement_id.clone(),
                state: item_state(r.state),
                connect_link_id: None,
                user_service_id: r.user_service_id.clone(),
                explicit_selection: prior.get(&r.requirement_id) == r.user_service_id.as_ref()
                    && r.user_service_id.is_some(),
                validation_record_id: None,
                attempt_id: None,
                attempt_started_at: None,
                reason_code: Some(r.state.as_str().into()),
                extended_ttl: false,
            })
            .collect(),
        status: AppConnectStatus::InProgress,
        capability_hash: hash_token(&capability),
        redeemed_at: None,
        revision: 0,
        result_id: Some(report.result_id),
        grant_update_required: false,
        created_at: now,
        expires_at: now + Duration::minutes(30),
        completed_at: None,
        failure_reason: None,
    };
    state
        .db
        .collection::<AppConnectLink>(COLLECTION_NAME)
        .insert_one(&link)
        .await?;
    let connect_url = format!("{}#t={capability}", return_url(state, &link.id));
    Ok(CreatedSession { link, connect_url })
}

pub fn return_url(state: &AppState, id: &str) -> String {
    format!(
        "{}/connect/app/{id}",
        state.config.frontend_url.trim_end_matches('/')
    )
}

pub async fn load(state: &AppState, id: &str, subject: &str) -> AppResult<AppConnectLink> {
    let mut link = state
        .db
        .collection::<AppConnectLink>(COLLECTION_NAME)
        .find_one(doc! { "_id": id, "user_id": subject })
        .await?
        .ok_or(AppError::AppConnectLinkNotFound)?;
    enabled_client(state, &link.oauth_client_id).await?;
    if is_open(link.status) && link.expires_at <= Utc::now() {
        expire(&state.db, id).await?;
        link = state
            .db
            .collection::<AppConnectLink>(COLLECTION_NAME)
            .find_one(doc! { "_id": id, "user_id": subject })
            .await?
            .ok_or(AppError::AppConnectLinkNotFound)?;
    }
    Ok(link)
}

pub async fn redeem(
    state: &AppState,
    id: &str,
    subject: &str,
    capability: &str,
) -> AppResult<AppConnectLink> {
    let link = load(state, id, subject).await?;
    ensure_open(&link)?;
    if capability.len() != 64 {
        return Err(AppError::AppConnectLinkNotFound);
    }
    state
        .db
        .collection::<AppConnectLink>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": id, "user_id": subject, "redeemed_at": null,
            "capability_hash": hash_token(capability), "status": "in_progress",
            "expires_at": { "$gt": bson::DateTime::now() } },
            doc! { "$set": { "redeemed_at": bson::DateTime::now(), "capability_hash": "" },
            "$inc": { "revision": 1 } },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::AppConnectLinkNotFound)
}

pub fn ensure_redeemed(link: &AppConnectLink) -> AppResult<()> {
    if link.redeemed_at.is_none() {
        return Err(AppError::AppConnectLinkNotFound);
    }
    Ok(())
}

fn is_open(status: AppConnectStatus) -> bool {
    matches!(
        status,
        AppConnectStatus::InProgress | AppConnectStatus::ReadyForConsent
    )
}

pub fn ensure_open(link: &AppConnectLink) -> AppResult<()> {
    match link.status {
        AppConnectStatus::Cancelled => Err(AppError::AppConnectLinkCancelled),
        AppConnectStatus::Expired => Err(AppError::AppConnectLinkExpired),
        AppConnectStatus::Completed | AppConnectStatus::Failed => {
            Err(AppError::AppConnectLinkCompleted)
        }
        _ if link.expires_at <= Utc::now() => Err(AppError::AppConnectLinkExpired),
        _ => Ok(()),
    }
}

pub async fn manifest(
    state: &AppState,
    link: &AppConnectLink,
) -> AppResult<AppRequirementManifest> {
    state
        .db
        .collection::<AppRequirementManifest>(MANIFESTS)
        .find_one(doc! {
            "_id": &link.manifest_id, "oauth_client_id": &link.oauth_client_id,
            "version": i64::from(link.manifest_version),
        })
        .await?
        .ok_or(AppError::AppConnectResultMismatch)
}

pub fn requirement<'a>(
    manifest: &'a AppRequirementManifest,
    id: &str,
) -> AppResult<&'a ServiceRequirement> {
    manifest
        .requirements
        .iter()
        .find(|r| r.id == id)
        .ok_or(AppError::AppConnectLinkNotFound)
}

pub async fn evaluate(
    state: &AppState,
    link: &AppConnectLink,
    manifest: &AppRequirementManifest,
) -> AppResult<RequirementsReport> {
    app_requirements_service::evaluate_local(
        state,
        manifest,
        &link.user_id,
        &EvaluationCaller {
            client_id: &link.oauth_client_id,
            allow_all_services: true,
            allowed_service_ids: &[],
            explicit_selections: link
                .items
                .iter()
                .filter(|i| i.explicit_selection)
                .filter_map(|i| {
                    i.user_service_id
                        .as_ref()
                        .map(|id| (i.requirement_id.clone(), id.clone()))
                })
                .collect(),
        },
    )
    .await
}

fn item_state(state: RequirementState) -> ItemState {
    match state {
        RequirementState::Met | RequirementState::Included => ItemState::Met,
        RequirementState::Unmet | RequirementState::Disabled => ItemState::Unmet,
        RequirementState::Unknown => ItemState::Unknown,
        RequirementState::Broken
        | RequirementState::NeedsReauth
        | RequirementState::Unsatisfiable => ItemState::Failed,
    }
}

/// Returns false on a losing CAS. Callers reload before doing any further work.
async fn persist(state: &AppState, link: &AppConnectLink) -> AppResult<bool> {
    let result = state.db.collection::<AppConnectLink>(COLLECTION_NAME).update_one(
        doc! { "_id": &link.id, "revision": link.revision, "status": { "$in": ["in_progress", "ready_for_consent"] },
            "expires_at": { "$gt": bson::DateTime::now() } },
        doc! { "$set": { "items": bson::to_bson(&link.items).map_err(|e| AppError::Internal(e.to_string()))?, "status": bson::to_bson(&link.status).map_err(|e| AppError::Internal(e.to_string()))?,
            "result_id": &link.result_id, "grant_update_required": link.grant_update_required,
            "expires_at": bson::DateTime::from_chrono(link.expires_at),
            "completed_at": link.completed_at.map(bson::DateTime::from_chrono), "failure_reason": &link.failure_reason },
            "$inc": { "revision": 1 } },
    ).await?;
    Ok(result.modified_count == 1)
}

/// Reconciles child outcomes even when the completing process died before touching the parent.
pub async fn refresh(
    state: &AppState,
    id: &str,
    subject: &str,
) -> AppResult<(AppConnectLink, RequirementsReport)> {
    for _ in 0..5 {
        let mut link = load(state, id, subject).await?;
        let manifest = manifest(state, &link).await?;
        let original_items = link.items.clone();
        let original_result = link.result_id.clone();
        if is_open(link.status) {
            for item in &mut link.items {
                if let Some(child_id) = &item.connect_link_id {
                    let child = state.db.collection::<ConnectLink>(CHILDREN).find_one(doc! {
                        "_id": child_id, "parent_session_id": id, "requirement_id": &item.requirement_id,
                    }).await?;
                    if let Some(child) = child {
                        let child = connect_link_service::expire_if_due(&state.db, child).await?;
                        match child.status {
                            ConnectLinkStatus::Completed => {
                                if matches!(
                                    item.state,
                                    ItemState::Connecting | ItemState::Reauthorizing
                                ) {
                                    item.user_service_id = child.completed_user_service_id;
                                    item.explicit_selection = true;
                                    item.state = ItemState::Unknown;
                                    item.attempt_id = None;
                                    item.attempt_started_at = None;
                                }
                            }
                            ConnectLinkStatus::Cancelled | ConnectLinkStatus::Expired => {
                                item.state = ItemState::Failed;
                                item.reason_code = Some("child_cancelled".into());
                                item.attempt_id = None;
                                item.attempt_started_at = None;
                                item.connect_link_id = None;
                            }
                            ConnectLinkStatus::Pending if child.last_error.is_some() => {
                                item.state = ItemState::Failed;
                                item.reason_code = Some("provider_authorization_failed".into());
                                item.attempt_id = None;
                                item.attempt_started_at = None;
                            }
                            _ => {}
                        }
                    }
                }
                if item.state == ItemState::Validating
                    && item
                        .attempt_started_at
                        .is_none_or(|at| at + Duration::seconds(30) <= Utc::now())
                {
                    item.state = ItemState::Unknown;
                    item.attempt_id = None;
                    item.reason_code = Some("attempt_interrupted".into());
                }
            }
        }
        let report = evaluate(state, &link, &manifest).await?;
        if !is_open(link.status) {
            return Ok((link, report));
        }
        for status in &report.requirements {
            let item = link
                .items
                .iter_mut()
                .find(|i| i.requirement_id == status.requirement_id)
                .ok_or(AppError::AppConnectResultMismatch)?;
            if matches!(
                item.state,
                ItemState::Connecting
                    | ItemState::Reauthorizing
                    | ItemState::Validating
                    | ItemState::Skipped
            ) {
                continue;
            }
            if matches!(
                item.reason_code.as_deref(),
                Some(
                    "child_cancelled"
                        | "provider_authorization_failed"
                        | "attempt_interrupted"
                        | "validation_unavailable"
                        | "credential_unavailable"
                        | "attempt_superseded"
                        | "lease_lost"
                        | "internal_error"
                        | "node_agent_upgrade_required"
                )
            ) {
                continue;
            }
            item.state = item_state(status.state);
            item.user_service_id = status.user_service_id.clone();
            item.reason_code = Some(status.state.as_str().into());
            if item.state == ItemState::Met && !item.extended_ttl {
                item.extended_ttl = true;
                link.expires_at = (link.expires_at + Duration::minutes(15))
                    .min(link.created_at + Duration::hours(2));
            }
        }
        if original_items == link.items && original_result.as_ref() == Some(&report.result_id) {
            return Ok((link, report));
        }
        link.result_id = Some(report.result_id.clone());
        if persist(state, &link).await? {
            link.revision += 1;
            return Ok((link, report));
        }
    }
    Err(AppError::AppConnectResultMismatch)
}

pub async fn select_item(
    state: &AppState,
    id: &str,
    subject: &str,
    requirement_id: &str,
    service_id: Option<&str>,
) -> AppResult<()> {
    let mut link = load(state, id, subject).await?;
    ensure_redeemed(&link)?;
    ensure_open(&link)?;
    let manifest = manifest(state, &link).await?;
    let required = requirement(&manifest, requirement_id)?;
    if let Some(service_id) = service_id {
        eligible_selection(state, &link, &manifest, required, service_id).await?;
    } else if !required.optional {
        return Err(AppError::RequirementNotMet);
    }
    let item = link
        .items
        .iter_mut()
        .find(|i| i.requirement_id == requirement_id)
        .ok_or(AppError::AppConnectLinkNotFound)?;
    item.user_service_id = service_id.map(Into::into);
    item.explicit_selection = service_id.is_some();
    item.state = if service_id.is_some() {
        ItemState::Unknown
    } else {
        ItemState::Skipped
    };
    item.connect_link_id = None;
    item.attempt_id = None;
    item.attempt_started_at = None;
    item.validation_record_id = None;
    item.reason_code = None;
    if !persist(state, &link).await? {
        return Err(AppError::AppConnectResultMismatch);
    }
    Ok(())
}

async fn eligible_selection(
    state: &AppState,
    link: &AppConnectLink,
    manifest: &AppRequirementManifest,
    required: &ServiceRequirement,
    service_id: &str,
) -> AppResult<UserService> {
    let mut selected = manifest.clone();
    selected.requirements = vec![required.clone()];
    let report = app_requirements_service::evaluate_selection(
        state,
        &selected,
        &link.user_id,
        &EvaluationCaller {
            client_id: &link.oauth_client_id,
            allow_all_services: true,
            allowed_service_ids: &[],
            explicit_selections: BTreeMap::from([(required.id.clone(), service_id.into())]),
        },
    )
    .await?;
    if !report.requirements.iter().any(|r| {
        r.candidates
            .iter()
            .any(|candidate| candidate.user_service_id == service_id)
    }) {
        return Err(AppError::RequirementNotSatisfiable);
    }
    state
        .db
        .collection::<UserService>(SERVICES)
        .find_one(doc! { "_id": service_id, "is_active": true })
        .await?
        .ok_or(AppError::RequirementNotSatisfiable)
}

pub async fn connect_item(
    state: &AppState,
    id: &str,
    subject: &str,
    requirement_id: &str,
    slug: &str,
    reauthorize: bool,
) -> AppResult<connect_link_service::CreatedLink> {
    let mut link = load(state, id, subject).await?;
    ensure_redeemed(&link)?;
    ensure_open(&link)?;
    let manifest = manifest(state, &link).await?;
    let required = requirement(&manifest, requirement_id)?;
    if !required.any_of_catalog_slugs.iter().any(|s| s == slug) {
        return Err(AppError::RequirementNotSatisfiable);
    }
    let index = link
        .items
        .iter()
        .position(|i| i.requirement_id == requirement_id)
        .ok_or(AppError::AppConnectLinkNotFound)?;
    let selected = if reauthorize {
        let service_id = link.items[index]
            .user_service_id
            .as_deref()
            .ok_or(AppError::RequirementNotSatisfiable)?;
        let service = eligible_selection(state, &link, &manifest, required, service_id).await?;
        if service.catalog_service_id.as_ref() != manifest.compiled.catalog_service_ids.get(slug) {
            return Err(AppError::RequirementNotSatisfiable);
        }
        let access =
            org_service::resolve_owner_access(&state.db, subject, &service.user_id).await?;
        if !access.can_write() {
            return Err(AppError::Forbidden(
                "Write access is required to reauthorize this connection".into(),
            ));
        }
        let key = state
            .db
            .collection::<UserApiKey>(KEYS)
            .find_one(doc! { "_id": &service.api_key_id, "user_id": &service.user_id })
            .await?
            .ok_or(AppError::RequirementNotSatisfiable)?;
        if key.credential_type != "oauth2" || key.connection_id.is_none() {
            return Err(AppError::RequirementNotSatisfiable);
        }
        Some(service)
    } else {
        None
    };
    let client = enabled_client(state, &link.oauth_client_id).await?;
    let created = connect_link_service::create_child(
        &state.db,
        &link,
        required,
        &client,
        slug,
        selected.as_ref(),
        &return_url(state, id),
    )
    .await?;
    let item = &mut link.items[index];
    item.connect_link_id = Some(created.link.id.clone());
    item.attempt_id = Some(Uuid::new_v4().to_string());
    item.attempt_started_at = Some(Utc::now());
    item.state = if reauthorize {
        ItemState::Reauthorizing
    } else {
        ItemState::Connecting
    };
    item.reason_code = None;
    item.validation_record_id = None;
    if !persist(state, &link).await? {
        state
            .db
            .collection::<ConnectLink>(CHILDREN)
            .delete_one(doc! { "_id": &created.link.id })
            .await?;
        return Err(AppError::AppConnectResultMismatch);
    }
    Ok(created)
}

pub async fn validate_item(
    state: &AppState,
    id: &str,
    subject: &str,
    requirement_id: &str,
) -> AppResult<()> {
    let (mut link, _) = refresh(state, id, subject).await?;
    ensure_redeemed(&link)?;
    ensure_open(&link)?;
    let manifest = manifest(state, &link).await?;
    let required = requirement(&manifest, requirement_id)?;
    let index = link
        .items
        .iter()
        .position(|i| i.requirement_id == requirement_id)
        .ok_or(AppError::AppConnectLinkNotFound)?;
    if matches!(
        link.items[index].state,
        ItemState::Connecting | ItemState::Reauthorizing
    ) {
        return Err(AppError::RequirementNotMet);
    }
    let service_id = link.items[index]
        .user_service_id
        .clone()
        .ok_or(AppError::RequirementNotMet)?;
    eligible_selection(state, &link, &manifest, required, &service_id).await?;
    let ValidatorSelection::Profile { id: profile_id } = &required.validator else {
        link.items[index].reason_code = None;
        link.items[index].connect_link_id = None;
        link.items[index].state = ItemState::Unknown;
        if !persist(state, &link).await? {
            return Err(AppError::AppConnectResultMismatch);
        }
        return Ok(());
    };
    let profile = validator_profiles::PROFILES
        .iter()
        .find(|p| {
            p.id == profile_id
                && manifest.compiled.validator_versions.get(profile_id) == Some(&p.version)
        })
        .ok_or(AppError::ServiceValidationRejected)?;
    let service = state
        .db
        .collection::<UserService>(SERVICES)
        .find_one(doc! { "_id": &service_id })
        .await?
        .ok_or(AppError::RequirementNotMet)?;
    let catalog_slug = manifest
        .compiled
        .catalog_service_ids
        .iter()
        .find(|(_, id)| Some(*id) == service.catalog_service_id.as_ref())
        .map(|(slug, _)| slug.as_str());
    if catalog_slug
        .and_then(validator_profiles::for_slug)
        .is_none_or(|p| p.id != profile.id)
    {
        return Err(AppError::ServiceValidationRejected);
    }
    if link.items[index].state == ItemState::Validating {
        return Ok(());
    }
    let previous_item = link.items[index].clone();
    let attempt = Uuid::new_v4().to_string();
    link.items[index].reason_code = None;
    if previous_item.reason_code.as_deref() == Some("provider_authorization_failed") {
        link.items[index].connect_link_id = None;
    }
    link.items[index].attempt_id = Some(attempt.clone());
    link.items[index].attempt_started_at = Some(Utc::now());
    link.items[index].state = ItemState::Validating;
    if !persist(state, &link).await? {
        return Err(AppError::AppConnectResultMismatch);
    }
    let result = service_validation_service::validate(
        state,
        service_validation_service::ValidationCaller {
            user_id: subject.into(),
            context: CallerContext::App {
                client_id: link.oauth_client_id.clone(),
            },
            // The human is authorizing the check. The app's stored grant is checked
            // separately on completion; it cannot authorize a resource the human lacks.
            allow_all_services: true,
            allowed_service_ids: vec![],
            allow_all_nodes: true,
            allowed_node_ids: vec![],
        },
        &service_id,
        true,
    )
    .await;
    for _ in 0..5 {
        let mut live = load(state, id, subject).await?;
        if !is_open(live.status) {
            return Ok(());
        }
        let item = live
            .items
            .iter_mut()
            .find(|i| i.requirement_id == requirement_id)
            .ok_or(AppError::AppConnectLinkNotFound)?;
        if item.attempt_id.as_deref() != Some(&attempt) {
            return Ok(());
        }
        if matches!(result, Err(AppError::ServiceValidationRateLimited)) {
            *item = previous_item.clone();
        } else {
            item.state = ItemState::Unknown;
            item.attempt_id = None;
            item.attempt_started_at = None;
            item.validation_record_id = result.as_ref().ok().map(|r| r.id.clone());
            item.reason_code = Some(
                result
                    .as_ref()
                    .map_or("validation_unavailable", |r| r.reason_code.as_str())
                    .into(),
            );
        }
        if persist(state, &live).await? {
            return result.map(|_| ());
        }
    }
    Err(AppError::AppConnectResultMismatch)
}

pub async fn ready(state: &AppState, id: &str, subject: &str) -> AppResult<AppConnectLink> {
    let (mut link, report) = refresh(state, id, subject).await?;
    ensure_redeemed(&link)?;
    ensure_open(&link)?;
    let manifest = manifest(state, &link).await?;
    for required in &manifest.requirements {
        if required.optional {
            continue;
        }
        let status = report
            .requirements
            .iter()
            .find(|r| r.requirement_id == required.id)
            .ok_or(AppError::RequirementNotMet)?;
        if !matches!(
            status.state,
            RequirementState::Met | RequirementState::Included
        ) || !link
            .items
            .iter()
            .any(|i| i.requirement_id == required.id && i.state == ItemState::Met)
        {
            return Err(AppError::RequirementNotMet);
        }
        // Repair uses the phase-0 five-minute window; there is no authorize gate here.
        if status.valid_until.is_some_and(|until| until <= Utc::now()) {
            return Err(AppError::RequirementNotMet);
        }
    }
    let oldest = report
        .requirements
        .iter()
        .filter_map(|r| r.validated_at)
        .min();
    let newest = report
        .requirements
        .iter()
        .filter_map(|r| r.validated_at)
        .max();
    if oldest
        .zip(newest)
        .is_some_and(|(old, new)| new - old > Duration::minutes(5))
    {
        return Err(AppError::RequirementNotMet);
    }
    let consent = state
        .db
        .collection::<Consent>(CONSENTS)
        .find_one(doc! {
            "user_id": subject, "client_id": &link.oauth_client_id,
            "$or": [{ "expires_at": null }, { "expires_at": { "$gt": bson::DateTime::now() } }],
        })
        .await?;
    link.grant_update_required = report
        .requirements
        .iter()
        .filter(|r| matches!(r.state, RequirementState::Met | RequirementState::Included))
        .filter(|r| {
            link.items
                .iter()
                .any(|item| item.requirement_id == r.requirement_id && item.state == ItemState::Met)
        })
        .filter_map(|r| r.user_service_id.as_ref())
        .any(|id| {
            consent.as_ref().is_none_or(|c| {
                !c.allow_all_services
                    && !c
                        .allowed_service_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(id))
            })
        });
    link.status = AppConnectStatus::Completed;
    link.completed_at = Some(Utc::now());
    if !persist(state, &link).await? {
        return Err(AppError::AppConnectResultMismatch);
    }
    link.revision += 1;
    Ok(link)
}

pub async fn cancel(state: &AppState, id: &str, subject: &str) -> AppResult<AppConnectLink> {
    let mut link = load(state, id, subject).await?;
    ensure_redeemed(&link)?;
    if link.status == AppConnectStatus::Cancelled {
        cancel_pending_children(&state.db, id).await;
        return Ok(link);
    }
    ensure_open(&link)?;
    link.status = AppConnectStatus::Cancelled;
    link.completed_at = Some(Utc::now());
    for item in &mut link.items {
        item.attempt_id = None;
        item.attempt_started_at = None;
    }
    if !persist(state, &link).await? {
        return Err(AppError::AppConnectResultMismatch);
    }
    link.revision += 1;
    cancel_pending_children(&state.db, id).await;
    Ok(link)
}

async fn cancel_pending_children(db: &mongodb::Database, parent_id: &str) {
    if let Err(error) = db
        .collection::<ConnectLink>(CHILDREN)
        .update_many(
            doc! { "parent_session_id": parent_id, "status": "pending" },
            doc! { "$set": { "status": "cancelled", "completed_at": bson::DateTime::now(),
            "completion_claim_id": null, "completion_claim_at": null } },
        )
        .await
    {
        tracing::warn!(app_connect_link_id = parent_id, error = %error,
            "Could not cancel pending app connect children");
    }
}

async fn expire(db: &mongodb::Database, id: &str) -> AppResult<()> {
    let result = db.collection::<AppConnectLink>(COLLECTION_NAME).update_one(
        doc! { "_id": id, "status": { "$in": ["in_progress", "ready_for_consent"] }, "expires_at": { "$lte": bson::DateTime::now() } },
        doc! { "$set": { "status": "expired", "completed_at": bson::DateTime::now(), "items.$[].attempt_id": null }, "$inc": { "revision": 1 } },
    ).await?;
    if result.modified_count == 1 {
        cancel_pending_children(db, id).await;
    }
    Ok(())
}

pub async fn expire_sessions(db: &mongodb::Database) -> AppResult<()> {
    let mut due = db
        .collection::<AppConnectLink>(COLLECTION_NAME)
        .find(
            doc! { "status": { "$in": ["in_progress", "ready_for_consent"] },
            "expires_at": { "$lte": bson::DateTime::now() } },
        )
        .await?;
    while let Some(link) = due.try_next().await? {
        expire(db, &link.id).await?;
    }
    Ok(())
}

pub fn terminal_callback_url(link: &AppConnectLink) -> AppResult<Option<String>> {
    let status = match link.status {
        AppConnectStatus::Completed => "completed",
        AppConnectStatus::Cancelled => "cancelled",
        AppConnectStatus::Expired => "expired",
        AppConnectStatus::Failed => "failed",
        _ => return Ok(None),
    };
    let AppConnectOrigin::App {
        callback_url,
        state,
    } = &link.origin
    else {
        return Ok(None);
    };
    let mut url = url::Url::parse(callback_url)
        .map_err(|_| AppError::Internal("Stored app callback is invalid".into()))?;
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| {
            ![
                "status",
                "app_connect_link_id",
                "state",
                "grant_update_required",
            ]
            .contains(&k.as_ref())
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut()
        .extend_pairs(pairs)
        .append_pair("status", status)
        .append_pair("app_connect_link_id", &link.id)
        .append_pair("state", state)
        .append_pair(
            "grant_update_required",
            if link.grant_update_required {
                "true"
            } else {
                "false"
            },
        );
    Ok(Some(url.to_string()))
}

/// Service-level child guard is independent from the child's resource owner.
/// HTTP callers must additionally prove Session auth before invoking child operations.
pub async fn ensure_child_subject(
    db: &mongodb::Database,
    child: &ConnectLink,
    actor: &str,
) -> AppResult<()> {
    let Some(parent) = &child.parent_session_id else {
        return Ok(());
    };
    let link = db.collection::<AppConnectLink>(COLLECTION_NAME).find_one(doc! {
        "_id": parent, "user_id": actor, "redeemed_at": { "$ne": null },
        "status": "in_progress", "expires_at": { "$gt": bson::DateTime::now() },
        "items": { "$elemMatch": { "requirement_id": &child.requirement_id, "connect_link_id": &child.id } },
    }).await?.ok_or(AppError::ConnectLinkNotFound)?;
    let _ = link;
    Ok(())
}
