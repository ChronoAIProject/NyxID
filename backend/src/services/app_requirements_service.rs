//! Local readiness only: no decryption, OAuth refresh, or provider transport.
//! Readiness is independent of the app token's grant, which is reported separately.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use super::assistant_readiness_service::{ConnectionState, connection_state_for_key};
use super::user_service_service::{self, CredentialSource};
use super::validator_profiles::ValidationOutcome;
use super::{
    execution_authority, node_routing_service, proxy_service, service_validation_service,
    unified_key_service,
};
use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::app_requirement_manifest::{
    AppRequirementManifest, OwnerPolicy, ServiceRequirement, ValidatorSelection,
};
use crate::models::app_requirement_result::{
    AppRequirementResult, COLLECTION_NAME as RESULTS, RequirementSelection,
};
use crate::models::downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService};
use crate::models::service_validation_record::{
    COLLECTION_NAME as VALIDATIONS, ServiceValidationRecord,
};
use crate::models::user_api_key::{COLLECTION_NAME as KEYS, UserApiKey};
use crate::models::user_service::UserService;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequirementState {
    Unmet,
    Met,
    Included,
    Unknown,
    Broken,
    NeedsReauth,
    Unsatisfiable,
    Disabled,
}

impl RequirementState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unmet => "unmet",
            Self::Met => "met",
            Self::Included => "included",
            Self::Unknown => "unknown",
            Self::Broken => "broken",
            Self::NeedsReauth => "needs_reauth",
            Self::Unsatisfiable => "unsatisfiable",
            Self::Disabled => "disabled",
        }
    }
}

pub struct EvaluationCaller<'a> {
    pub client_id: &'a str,
    pub allow_all_services: bool,
    pub allowed_service_ids: &'a [String],
    /// Only selections made by a person belong here; automatic choices are not preferences.
    pub explicit_selections: BTreeMap<String, String>,
}

pub struct RequirementChoice {
    pub user_service_id: String,
    pub slug: String,
    pub catalog_slug: String,
    pub owner_id: String,
}

pub struct RequirementStatus {
    pub candidates: Vec<RequirementChoice>,
    pub requirement_id: String,
    pub state: RequirementState,
    pub reason_code: Option<&'static str>,
    pub user_service_id: Option<String>,
    pub slug: Option<String>,
    pub resource_uri: Option<String>,
    pub owner_id: Option<String>,
    pub validated_at: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub credential_health: Option<String>,
    pub granted_to_caller: bool,
}

pub struct RequirementsReport {
    #[cfg(test)]
    pub authority_snapshots: usize,
    pub requirements_version: u32,
    pub result_id: String,
    pub requirements: Vec<RequirementStatus>,
}

struct Candidate {
    status: RequirementStatus,
    last_used_at: Option<DateTime<Utc>>,
    authenticated_at: Option<DateTime<Utc>>,
}

impl RequirementStatus {
    fn empty(id: &str, state: RequirementState) -> Self {
        Self {
            candidates: vec![],
            requirement_id: id.into(),
            state,
            reason_code: None,
            user_service_id: None,
            slug: None,
            resource_uri: None,
            owner_id: None,
            validated_at: None,
            valid_until: None,
            credential_health: None,
            granted_to_caller: false,
        }
    }
}

pub async fn evaluate_local(
    state: &AppState,
    manifest: &AppRequirementManifest,
    user_id: &str,
    caller: &EvaluationCaller<'_>,
) -> AppResult<RequirementsReport> {
    evaluate_local_inner(state, manifest, user_id, caller, true).await
}

/// Evaluate a proposed selection without persisting a partial-manifest result.
pub(crate) async fn evaluate_selection(
    state: &AppState,
    manifest: &AppRequirementManifest,
    user_id: &str,
    caller: &EvaluationCaller<'_>,
) -> AppResult<RequirementsReport> {
    evaluate_local_inner(state, manifest, user_id, caller, false).await
}

async fn evaluate_local_inner(
    state: &AppState,
    manifest: &AppRequirementManifest,
    user_id: &str,
    caller: &EvaluationCaller<'_>,
    persist_result: bool,
) -> AppResult<RequirementsReport> {
    if caller.client_id != manifest.oauth_client_id {
        return Err(AppError::AppConnectResultMismatch);
    }
    if manifest
        .requirements
        .iter()
        .any(|r| r.allow_no_credential || r.allow_master_credential)
    {
        unified_key_service::auto_provision_no_auth_services(&state.db, user_id).await?;
    }
    let visible = user_service_service::list_user_services_with_sources(&state.db, user_id).await?;
    // Disabled rows are disclosure-only evidence; they never enter credential resolution.
    let disabled = user_service_service::list_user_services_with_sources_including_disabled(
        &state.db, user_id,
    )
    .await?
    .into_iter()
    .filter(|item| !item.service.is_active)
    .collect::<Vec<_>>();
    let catalog_ids = manifest
        .compiled
        .catalog_service_ids
        .values()
        .collect::<Vec<_>>();
    let catalog: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(CATALOG)
        .find(doc! { "_id": { "$in": &catalog_ids }, "is_active": true,
        "service_category": { "$ne": "provider" } })
        .await?
        .try_collect()
        .await?;
    let catalog: HashMap<_, _> = catalog
        .into_iter()
        .map(|service| (service.id.clone(), service))
        .collect();
    // Snapshot each relevant service once, even when requirements overlap. These
    // facts are scoped to this evaluation; every new read rechecks live authority.
    let mut facts = HashMap::new();
    #[cfg(test)]
    let mut authority_snapshots = 0;
    for item in &visible {
        if !manifest.requirements.iter().any(|r| {
            owner_allowed(r.owner_policy, &item.source)
                && r.any_of_catalog_slugs.iter().any(|slug| {
                    manifest.compiled.catalog_service_ids.get(slug)
                        == item.service.catalog_service_id.as_ref()
                })
        }) || !item
            .service
            .catalog_service_id
            .as_ref()
            .is_some_and(|id| catalog.contains_key(id))
        {
            continue;
        }
        #[cfg(test)]
        {
            authority_snapshots += 1;
        }
        facts.insert(
            item.service.id.clone(),
            candidate_facts(state, user_id, &item.service).await?,
        );
    }
    let mut requirements = Vec::with_capacity(manifest.requirements.len());
    for requirement in &manifest.requirements {
        let catalog_ids: HashSet<_> = requirement
            .any_of_catalog_slugs
            .iter()
            .filter_map(|slug| manifest.compiled.catalog_service_ids.get(slug))
            .filter(|id| catalog.contains_key(*id))
            .collect();
        if catalog_ids.is_empty() {
            requirements.push(RequirementStatus::empty(
                &requirement.id,
                RequirementState::Unsatisfiable,
            ));
            continue;
        }
        let mut candidates = Vec::new();
        for item in &visible {
            let Some(catalog_id) = item.service.catalog_service_id.as_ref() else {
                continue;
            };
            if !catalog_ids.contains(catalog_id)
                || !owner_allowed(requirement.owner_policy, &item.source)
            {
                continue;
            }
            let Some(Some(facts)) = facts.get(&item.service.id) else {
                continue;
            };
            if let Some(candidate) = evaluate_candidate(
                state,
                facts,
                requirement,
                manifest,
                &item.service,
                &catalog[catalog_id],
                caller,
            ) {
                candidates.push(candidate);
            }
        }
        let preferred = caller.explicit_selections.get(&requirement.id);
        candidates.sort_by(|left, right| {
            let rank = |candidate: &Candidate| {
                (
                    candidate.status.user_service_id.as_ref() == preferred && preferred.is_some(),
                    match candidate.status.state {
                        RequirementState::Met | RequirementState::Included => 4,
                        RequirementState::Unknown => 3,
                        RequirementState::NeedsReauth => 2,
                        RequirementState::Broken => 1,
                        _ => 0,
                    },
                    candidate.authenticated_at,
                    candidate.last_used_at,
                )
            };
            rank(right).cmp(&rank(left)).then_with(|| {
                left.status
                    .user_service_id
                    .cmp(&right.status.user_service_id)
            })
        });
        let choices = candidates
            .iter()
            .map(|c| {
                let service_id = c
                    .status
                    .user_service_id
                    .as_ref()
                    .expect("candidate has a service");
                let service = visible
                    .iter()
                    .find(|i| &i.service.id == service_id)
                    .expect("visible candidate");
                RequirementChoice {
                    user_service_id: service_id.clone(),
                    slug: service.service.slug.clone(),
                    catalog_slug: catalog[service
                        .service
                        .catalog_service_id
                        .as_ref()
                        .expect("catalog candidate")]
                    .slug
                    .clone(),
                    owner_id: service.service.user_id.clone(),
                }
            })
            .collect();
        if let Some(mut candidate) = candidates.into_iter().next() {
            candidate.status.candidates = choices;
            requirements.push(candidate.status);
        } else {
            let paused = disabled.iter().find(|item| {
                item.service
                    .catalog_service_id
                    .as_ref()
                    .is_some_and(|id| catalog_ids.contains(id))
                    && owner_allowed(requirement.owner_policy, &item.source)
            });
            let mut status = RequirementStatus::empty(
                &requirement.id,
                if paused.is_some() {
                    RequirementState::Disabled
                } else {
                    RequirementState::Unmet
                },
            );
            if let Some(item) = paused {
                set_selection(&mut status, &item.service, state, caller);
            }
            requirements.push(status);
        }
    }
    if !persist_result {
        return Ok(RequirementsReport {
            #[cfg(test)]
            authority_snapshots,
            requirements_version: manifest.version,
            result_id: String::new(),
            requirements,
        });
    }
    let now = Utc::now();
    let result = AppRequirementResult {
        id: Uuid::new_v4().to_string(),
        oauth_client_id: caller.client_id.into(),
        user_id: user_id.into(),
        manifest_id: manifest.id.clone(),
        manifest_version: manifest.version,
        selections: requirements
            .iter()
            .map(|item| RequirementSelection {
                requirement_id: item.requirement_id.clone(),
                user_service_id: item.user_service_id.clone(),
                explicit: caller.explicit_selections.get(&item.requirement_id)
                    == item.user_service_id.as_ref()
                    && item.user_service_id.is_some(),
            })
            .collect(),
        created_at: now,
        expires_at: now + chrono::Duration::hours(1),
    };
    let results = state.db.collection::<AppRequirementResult>(RESULTS);
    let previous = results
        .find_one(doc! {
            "user_id": user_id, "oauth_client_id": caller.client_id,
            "manifest_id": &manifest.id, "manifest_version": i64::from(manifest.version),
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
        })
        .sort(doc! { "created_at": -1, "_id": 1 })
        .await?;
    // Re-evaluate readiness on every read, but keep a stable result identity while
    // the selections are unchanged. Reuse never extends the original expiry.
    let result_id = if let Some(previous) = previous.filter(|r| r.selections == result.selections) {
        previous.id
    } else {
        results.insert_one(&result).await?;
        result.id
    };
    Ok(RequirementsReport {
        #[cfg(test)]
        authority_snapshots,
        requirements_version: manifest.version,
        result_id,
        requirements,
    })
}

pub async fn prior_explicit_selections(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
) -> AppResult<BTreeMap<String, String>> {
    let mut cursor = db
        .collection::<AppRequirementResult>(RESULTS)
        .find(doc! {
            "user_id": user_id, "oauth_client_id": client_id,
            "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
            "selections.explicit": true,
        })
        .sort(doc! { "created_at": -1 })
        .await?;
    let mut selected = BTreeMap::new();
    while let Some(result) = cursor.try_next().await? {
        for item in result.selections {
            if item.explicit
                && let Some(id) = item.user_service_id
            {
                selected.entry(item.requirement_id).or_insert(id);
            }
        }
    }
    Ok(selected)
}

fn owner_allowed(policy: OwnerPolicy, source: &CredentialSource) -> bool {
    matches!(source, CredentialSource::Personal)
        || (policy == OwnerPolicy::PersonalOrOrgAllowed
            && matches!(source, CredentialSource::Org { allowed: true, .. }))
}

fn set_selection(
    status: &mut RequirementStatus,
    service: &UserService,
    state: &AppState,
    caller: &EvaluationCaller<'_>,
) {
    status.user_service_id = Some(service.id.clone());
    status.slug = Some(service.slug.clone());
    status.owner_id = Some(service.user_id.clone());
    status.resource_uri = Some(super::oauth_resource_service::user_service_resource_uri(
        &state.config,
        &service.slug,
    ));
    status.granted_to_caller =
        caller.allow_all_services || caller.allowed_service_ids.contains(&service.id);
}

struct CandidateFacts {
    slug_shadowed: bool,
    key: Option<UserApiKey>,
    no_credential: bool,
    master: bool,
    connection: ConnectionState,
    digest: Option<String>,
    node_credential: Option<crate::models::service_validation_record::NodeCredentialBinding>,
    records: Vec<ServiceValidationRecord>,
}

async fn candidate_facts(
    state: &AppState,
    actor: &str,
    service: &UserService,
) -> AppResult<Option<CandidateFacts>> {
    let now = Utc::now();
    let resolution = match proxy_service::read_proxy_authority_snapshot_by_user_service_id(
        &state.db,
        &state.encryption_keys,
        actor,
        &service.id,
        Some(&service.slug),
    )
    .await
    {
        Ok(resolution) => resolution,
        Err(
            AppError::BadRequest(_) | AppError::NotFound(_) | AppError::ServiceValidationRejected,
        ) => None,
        Err(AppError::Forbidden(_) | AppError::OrgRoleInsufficient(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    // The read-only resolver is authoritative about the selected credential and
    // master fallback, just as it is when phase 0 creates the evidence digest.
    let key_id = resolution
        .as_ref()
        .map_or(service.api_key_id.as_ref(), |r| r.api_key_id.as_ref());
    let key = if let Some(id) = key_id {
        state
            .db
            .collection::<UserApiKey>(KEYS)
            .find_one(doc! {
                "_id": id, "user_id": &service.user_id,
            })
            .await?
    } else {
        None
    };
    let no_credential = resolution.as_ref().is_some_and(|r| {
        r.api_key_id.is_none() && !r.master_credential && r.target.auth_method == "none"
    });
    let master = resolution.as_ref().is_some_and(|r| r.master_credential);
    let executable = if let Some(node_id) = resolution.as_ref().and_then(|r| r.node_id.as_deref()) {
        Some(
            node_routing_service::is_node_id_dispatchable(
                &state.db,
                node_id,
                &state.node_ws_manager,
            )
            .await?,
        )
    } else {
        resolution.as_ref().map(|r| r.has_server_credential)
    };
    let connection = if let Some(key) = &key {
        connection_state_for_key(key, service, executable, now)
    } else if (no_credential || master) && resolution.is_some() {
        ConnectionState::Connected
    } else {
        ConnectionState::Unknown
    };
    let mut node_credential = None;
    let digest = if let Some(resolution) = &resolution {
        let fallback_nodes = node_routing_service::list_configured_binding_node_ids(
            &state.db,
            &service.user_id,
            &resolution.target.service.id,
        )
        .await?;
        node_credential = node_routing_service::validation_route(
            &state.db,
            &state.node_ws_manager,
            resolution.node_id.as_deref(),
            &fallback_nodes,
            &resolution.target.service.slug,
            None,
        )
        .await?
        .1;
        Some(execution_authority::digest(
            &execution_authority::build_projection(resolution, None, fallback_nodes),
        ))
    } else {
        None
    };
    let records = state
        .db
        .collection::<ServiceValidationRecord>(VALIDATIONS)
        .find(doc! { "user_service_id": &service.id })
        .await?
        .try_collect()
        .await?;
    let slug_shadowed =
        super::oauth_resource_service::selected_service_is_shadowed(&state.db, actor, service)
            .await?;
    Ok(Some(CandidateFacts {
        slug_shadowed,
        key,
        no_credential,
        master,
        connection,
        digest,
        node_credential,
        records,
    }))
}

fn evaluate_candidate(
    state: &AppState,
    facts: &CandidateFacts,
    requirement: &ServiceRequirement,
    manifest: &AppRequirementManifest,
    service: &UserService,
    catalog: &DownstreamService,
    caller: &EvaluationCaller<'_>,
) -> Option<Candidate> {
    let CandidateFacts {
        slug_shadowed,
        key,
        no_credential,
        master,
        connection,
        digest,
        node_credential,
        records,
    } = facts;
    let (no_credential, master, connection) = (*no_credential, *master, *connection);
    if (no_credential && !requirement.allow_no_credential)
        || (master && !requirement.allow_master_credential)
    {
        return None;
    }
    if !no_credential
        && !master
        && !requirement.accepted_credential_types.is_empty()
        && key.as_ref().is_none_or(|key| {
            !requirement
                .accepted_credential_types
                .contains(&key.credential_type)
        })
    {
        return None;
    }
    let scope_shortfall = !requirement.required_downstream_scopes.is_empty()
        && key.as_ref().is_none_or(|key| {
            key.credential_type != "oauth2"
                || requirement
                    .required_downstream_scopes
                    .iter()
                    .any(|required| {
                        !key.token_scopes
                            .as_deref()
                            .unwrap_or_default()
                            .split_whitespace()
                            .any(|scope| scope == required)
                    })
        });
    let mut status = RequirementStatus::empty(&requirement.id, RequirementState::Unknown);
    set_selection(&mut status, service, state, caller);
    status.credential_health = key.as_ref().map(|key| key.status.clone());
    let mut authenticated_at = None;
    if scope_shortfall {
        status.state = RequirementState::NeedsReauth;
    } else if matches!(
        connection,
        ConnectionState::Expired | ConnectionState::Revoked
    ) {
        status.state = RequirementState::Broken;
    } else if (no_credential || master) && connection == ConnectionState::Connected {
        status.state = RequirementState::Included;
    } else if matches!(requirement.validator, ValidatorSelection::Profile { .. }) {
        if let (Some(digest), Some(profile)) = (
            digest,
            super::app_requirement_manifest_service::compiled_profile(
                manifest,
                &requirement.id,
                &catalog.slug,
            ),
        ) {
            let revision = key
                .as_ref()
                .map(service_validation_service::credential_revision);
            if let Some(record) = records
                .iter()
                .find(|record| record.validator_id == profile.id)
                && service_validation_service::evidence_is_fresh(
                    record,
                    digest,
                    revision.as_deref(),
                    node_credential.as_ref(),
                    profile.version,
                    Utc::now(),
                )
            {
                status.validated_at = Some(record.checked_at);
                status.valid_until = Some(record.valid_until);
                status.state = match record.outcome {
                    ValidationOutcome::Authenticated => {
                        authenticated_at = Some(record.checked_at);
                        RequirementState::Met
                    }
                    ValidationOutcome::CredentialRejected => RequirementState::Broken,
                    _ => RequirementState::Unknown,
                };
            }
        }
    } else if connection == ConnectionState::Connected {
        status.state = if no_credential || master {
            RequirementState::Included
        } else {
            RequirementState::Met
        };
    }
    if *slug_shadowed {
        status.state = RequirementState::Unsatisfiable;
        status.reason_code = Some("slug_shadowed");
        status.validated_at = None;
        status.valid_until = None;
        authenticated_at = None;
    }
    Some(Candidate {
        status,
        last_used_at: key.as_ref().and_then(|key| key.last_used_at),
        authenticated_at,
    })
}
