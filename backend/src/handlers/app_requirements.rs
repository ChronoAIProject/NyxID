//! App readiness is local observation. These routes do not change OAuth consent.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, State},
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use super::admin_helpers::require_admin;
use super::developer_apps::{resolve_developer_app_read_owner, resolve_developer_app_write_owner};
use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::app_requirement_manifest::{
    AppRequirementManifest, Enforcement, OwnerPolicy, ValidatorSelection,
};
use crate::models::oauth_client::OauthClient;
use crate::models::platform_settings::AppConnectRollout;
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    app_connect_rollout, app_requirement_manifest_service as manifests,
    app_requirements_service as requirements, audit_service, validator_profiles,
};

#[derive(Debug, Serialize)]
pub struct RequirementResponse {
    pub id: String,
    pub label: String,
    pub any_of_catalog_slugs: Vec<String>,
    pub any_of_catalog_prefix: Option<String>,
    pub owner_policy: OwnerPolicy,
    pub accepted_credential_types: Vec<String>,
    pub allow_master_credential: bool,
    pub allow_no_credential: bool,
    pub required_downstream_scopes: Vec<String>,
    pub validator: ValidatorSelection,
    pub optional: bool,
}

#[derive(Debug, Serialize)]
pub struct CompiledManifestResponse {
    pub catalog_service_ids: BTreeMap<String, String>,
    pub validator_versions: BTreeMap<String, u32>,
}

#[derive(Debug, Serialize)]
pub struct ManifestResponse {
    pub id: String,
    pub oauth_client_id: String,
    pub version: u32,
    pub enforcement: Enforcement,
    pub requirements: Vec<RequirementResponse>,
    pub compiled: CompiledManifestResponse,
    pub published_by: String,
    pub published_at: String,
}

impl From<AppRequirementManifest> for ManifestResponse {
    fn from(manifest: AppRequirementManifest) -> Self {
        Self {
            id: manifest.id,
            oauth_client_id: manifest.oauth_client_id,
            version: manifest.version,
            enforcement: manifest.enforcement,
            requirements: manifest
                .requirements
                .into_iter()
                .map(|r| RequirementResponse {
                    id: r.id,
                    label: r.label,
                    any_of_catalog_slugs: r.any_of_catalog_slugs,
                    any_of_catalog_prefix: r.any_of_catalog_prefix,
                    owner_policy: r.owner_policy,
                    accepted_credential_types: r.accepted_credential_types,
                    allow_master_credential: r.allow_master_credential,
                    allow_no_credential: r.allow_no_credential,
                    required_downstream_scopes: r.required_downstream_scopes,
                    validator: r.validator,
                    optional: r.optional,
                })
                .collect(),
            compiled: CompiledManifestResponse {
                catalog_service_ids: manifest.compiled.catalog_service_ids,
                validator_versions: manifest.compiled.validator_versions,
            },
            published_by: manifest.published_by,
            published_at: manifest.published_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub id: &'static str,
    pub version: u32,
    pub claim: &'static str,
    pub catalog_slugs: &'static [&'static str],
}

#[derive(Debug, Serialize)]
pub struct ManifestsResponse {
    pub versions: Vec<ManifestResponse>,
    pub validator_profiles: Vec<ProfileResponse>,
}

pub(crate) async fn enabled_client(state: &AppState, id: &str) -> AppResult<OauthClient> {
    crate::services::app_connect_link_service::enabled_client(state, id).await
}

pub async fn list_manifests(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ManifestsResponse>> {
    enabled_client(&state, &id).await?;
    resolve_developer_app_read_owner(&state, &auth.user_id.to_string(), &id).await?;
    Ok(Json(ManifestsResponse {
        versions: manifests::list(&state.db, &id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
        validator_profiles: validator_profiles::PROFILES
            .iter()
            .map(|p| ProfileResponse {
                id: p.id,
                version: p.version,
                claim: p.claim,
                catalog_slugs: p.catalog_slugs,
            })
            .collect(),
    }))
}

pub async fn publish_manifest(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<manifests::PublishManifest>,
) -> AppResult<Json<ManifestResponse>> {
    let client = enabled_client(&state, &id).await?;
    let actor = auth.user_id.to_string();
    resolve_developer_app_write_owner(&state, &actor, &id).await?;
    Ok(Json(
        manifests::publish(&state, &client, &actor, body)
            .await?
            .into(),
    ))
}

#[derive(Debug, Serialize)]
pub struct RequirementStatusResponse {
    pub requirement_id: String,
    pub state: &'static str,
    pub user_service_id: Option<String>,
    pub slug: Option<String>,
    pub resource_uri: Option<String>,
    pub owner_id: Option<String>,
    pub validated_at: Option<String>,
    pub valid_until: Option<String>,
    pub credential_health: Option<String>,
    pub granted_to_caller: bool,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub requirements_version: u32,
    pub result_id: String,
    pub requirements: Vec<RequirementStatusResponse>,
}

/// Ordinary developer-app user access tokens only. Readiness is independent of
/// the token's service allowlist; only manifest services can appear in the response.
pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<StatusResponse>> {
    let client_id = require_app_user(&state, &auth).await?;
    let client = enabled_client(&state, &client_id).await?;
    let manifest = manifests::current(&state.db, &client).await?;
    let actor = auth.user_id.to_string();
    if !state
        .app_requirements_status_limiter
        .check_shared(&actor)
        .await?
    {
        return Err(AppError::RateLimited);
    }
    let caller = requirements::EvaluationCaller {
        client_id: &client_id,
        allow_all_services: auth.allow_all_services,
        allowed_service_ids: &auth.allowed_service_ids,
        explicit_selections: requirements::prior_explicit_selections(&state.db, &actor, &client_id)
            .await?,
    };
    let report = requirements::evaluate_local(&state, &manifest, &actor, &caller).await?;
    Ok(Json(StatusResponse {
        requirements_version: report.requirements_version,
        result_id: report.result_id,
        requirements: report
            .requirements
            .into_iter()
            .map(|r| RequirementStatusResponse {
                requirement_id: r.requirement_id,
                state: r.state.as_str(),
                user_service_id: r.user_service_id,
                slug: r.slug,
                resource_uri: r.resource_uri,
                owner_id: r.owner_id,
                validated_at: r.validated_at.map(|t| t.to_rfc3339()),
                valid_until: r.valid_until.map(|t| t.to_rfc3339()),
                credential_health: r.credential_health,
                granted_to_caller: r.granted_to_caller,
            })
            .collect(),
    }))
}

pub(crate) async fn require_app_user(state: &AppState, auth: &AuthUser) -> AppResult<String> {
    let denied = || AppError::Forbidden("A developer-app user access token is required".into());
    if auth.auth_method != AuthMethod::AccessToken
        || auth.acting_client_id.is_some()
        || auth.api_key_id.is_some()
    {
        return Err(denied());
    }
    let client_id = auth.oauth_client_id.as_ref().ok_or_else(denied)?;
    let person = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": auth.user_id.to_string(), "is_active": true })
        .await?;
    if !person.is_some_and(|user| user.user_type == UserType::Person) {
        return Err(denied());
    }
    Ok(client_id.clone())
}

#[derive(Debug, Serialize)]
pub struct RolloutResponse {
    pub effective: AppConnectRollout,
    pub env_default: AppConnectRollout,
    pub override_value: Option<AppConnectRollout>,
    pub allowed_org_ids: Vec<String>,
}

fn policy_response(
    state: &AppState,
    policy: app_connect_rollout::AppConnectPolicy,
) -> RolloutResponse {
    RolloutResponse {
        effective: policy.rollout,
        env_default: policy.env_default,
        override_value: policy.override_value,
        allowed_org_ids: state.config.app_connect_allowed_org_ids.clone(),
    }
}

pub async fn get_rollout(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<RolloutResponse>> {
    require_admin(&state, &auth).await?;
    let policy = app_connect_rollout::load_policy(&state.db, &state.config).await?;
    state.set_app_connect_policy_if_fresh(policy);
    Ok(Json(policy_response(&state, state.app_connect_policy())))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRolloutRequest {
    /// Omit to leave unchanged; null restores the deployment default.
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub rollout: Option<Option<AppConnectRollout>>,
}

pub async fn update_rollout(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateRolloutRequest>,
) -> AppResult<Json<RolloutResponse>> {
    require_admin(&state, &auth).await?;
    let Some(rollout) = body.rollout else {
        return get_rollout(State(state), auth).await;
    };
    let policy = app_connect_rollout::update_policy(&state.db, &state.config, rollout).await?;
    state.set_app_connect_policy_if_fresh(policy);
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "app_connect_rollout_changed",
        Some(serde_json::json!({
            "rollout": policy.rollout,
            "override": policy.override_value,
            "revision": policy.revision,
        })),
    );
    Ok(Json(policy_response(&state, state.app_connect_policy())))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCapabilityRequest {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct CapabilityResponse {
    pub app_connect_capability_enabled: bool,
}

pub async fn update_capability(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateCapabilityRequest>,
) -> AppResult<Json<CapabilityResponse>> {
    require_admin(&state, &auth).await?;
    let changed = app_connect_rollout::set_client_capability(&state.db, &id, body.enabled).await?;
    if changed {
        audit_service::log_for_user(
            state.db.clone(),
            &auth,
            if body.enabled {
                "app_connect_capability_granted"
            } else {
                "app_connect_capability_revoked"
            },
            Some(serde_json::json!({ "client_id": id })),
        );
    }
    Ok(Json(CapabilityResponse {
        app_connect_capability_enabled: body.enabled,
    }))
}

#[cfg(test)]
#[path = "app_requirements_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
use crate::models::oauth_client::COLLECTION_NAME as CLIENTS;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffRequest {
    pub handoff_blurb: String,
}
#[derive(Debug, Serialize)]
pub struct HandoffResponse {
    pub handoff_blurb: Option<String>,
}

pub async fn update_handoff(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<HandoffRequest>,
) -> AppResult<Json<HandoffResponse>> {
    enabled_client(&state, &id).await?;
    let owner = resolve_developer_app_write_owner(&state, &auth.user_id.to_string(), &id).await?;
    let blurb = crate::services::oauth_client_service::update_handoff_blurb(
        &state.db,
        &id,
        &owner,
        &body.handoff_blurb,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "app_connect_handoff_updated",
        Some(serde_json::json!({"client_id":id})),
    );
    Ok(Json(HandoffResponse {
        handoff_blurb: blurb,
    }))
}
