//! Wave-2 endpoint- and external-key-family assistant action effects
//! (`endpoint.update`, `endpoint.delete`, `external_key.rotate`,
//! `external_key.delete`).
//!
//! Receipts use the shared reserve-then-commit helper in
//! `services/assistant_action_receipts.rs`. Evidence lives on the resource
//! handlers (`GET /endpoints/{id}/authorization` and
//! `GET /api-keys/external/{id}/authorization`). Destructive verbs confirm
//! every time and are never remember-eligible.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::user_api_keys_external::{self, UpdateExternalApiKeyRequest};
use crate::handlers::user_endpoints::{self, UpdateEndpointRequest};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_receipts::{
    self, ReceiptOutcome, fingerprint_canonical, fingerprint_sensitive_material, mark_completed,
    normalize_action_request_id,
};
use crate::services::{
    audit_service::AuditActor, org_service, unified_key_service, user_endpoint_service,
};
use crate::telemetry::TelemetryContext;

const ENDPOINT_UPDATE_ACTION: &str = "endpoint.update";
const ENDPOINT_DELETE_ACTION: &str = "endpoint.delete";
const EXTERNAL_KEY_ROTATE_ACTION: &str = "external_key.rotate";
const EXTERNAL_KEY_DELETE_ACTION: &str = "external_key.delete";

/// Effect routes mounted at `/api/v1/assistant/actions/endpoints`.
///
/// Evidence GETs retain these action-router aliases for local action journeys;
/// the canonical sibling mounts live in `routes.rs` and are exercised through
/// the production-router regression test below.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/update", post(update_endpoint))
        .route("/delete", post(delete_endpoint))
        .route("/external-key-rotate", post(rotate_external_key))
        .route("/external-key-delete", post(delete_external_key))
        .route(
            "/{endpoint_id}/authorization",
            get(user_endpoints::get_endpoint_authorization),
        )
        .route(
            "/external-keys/{key_id}/authorization",
            get(user_api_keys_external::get_external_api_key_authorization),
        )
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAssistantEndpointRequest {
    pub action_request_id: String,
    pub endpoint_id: String,
    pub label: Option<String>,
    pub endpoint_url: Option<String>,
    pub openapi_spec_url: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAssistantEndpointRequest {
    pub action_request_id: String,
    pub endpoint_id: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RotateAssistantExternalKeyRequest {
    pub action_request_id: String,
    pub external_key_id: String,
    pub credential: String,
}

impl std::fmt::Debug for RotateAssistantExternalKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RotateAssistantExternalKeyRequest")
            .field("action_request_id", &self.action_request_id)
            .field("external_key_id", &self.external_key_id)
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAssistantExternalKeyRequest {
    pub action_request_id: String,
    pub external_key_id: String,
    #[serde(default)]
    pub cascade_grant: Option<bool>,
    #[serde(default)]
    pub grant_scope: Option<String>,
    #[serde(default)]
    pub cascade_siblings: Option<Vec<CascadeSiblingConfirmation>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEndpointResource {
    pub endpoint_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantExternalKeyResource {
    pub external_key_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEndpointEffectResponse {
    pub resource: AssistantEndpointResource,
    pub replayed: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantExternalKeyEffectResponse {
    pub resource: AssistantExternalKeyResource,
    pub replayed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EndpointUpdateFingerprint<'a> {
    action: &'static str,
    endpoint_id: &'a str,
    label: Option<&'a str>,
    endpoint_url: Option<&'a str>,
    openapi_spec_url: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EndpointDeleteFingerprint<'a> {
    action: &'static str,
    endpoint_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalKeyRotateFingerprint<'a> {
    action: &'static str,
    external_key_id: &'a str,
    credential_fingerprint: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalKeyDeleteFingerprint<'a> {
    action: &'static str,
    external_key_id: &'a str,
    cascade_grant: bool,
    grant_scope: Option<&'a str>,
    cascade_siblings: Option<Vec<CascadeSiblingFingerprint<'a>>>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CascadeSiblingConfirmation {
    pub user_service_id: String,
    pub name: String,
    pub slug: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CascadeSiblingFingerprint<'a> {
    user_service_id: &'a str,
    name: &'a str,
    slug: &'a str,
}

fn canonical_cascade_siblings<'a>(
    siblings: Option<&'a [CascadeSiblingConfirmation]>,
) -> Option<Vec<CascadeSiblingFingerprint<'a>>> {
    let mut fingerprints: Vec<_> = siblings?
        .iter()
        .map(|sibling| CascadeSiblingFingerprint {
            user_service_id: &sibling.user_service_id,
            name: &sibling.name,
            slug: &sibling.slug,
        })
        .collect();
    fingerprints.sort_by(|left, right| {
        left.user_service_id
            .cmp(right.user_service_id)
            .then_with(|| left.name.cmp(right.name))
            .then_with(|| left.slug.cmp(right.slug))
    });
    Some(fingerprints)
}

async fn release_receipt(
    db: &mongodb::Database,
    receipt: &crate::models::assistant_action_receipt::AssistantActionReceipt,
) -> AppResult<()> {
    db.collection::<crate::models::assistant_action_receipt::AssistantActionReceipt>(
        crate::models::assistant_action_receipt::COLLECTION_NAME,
    )
    .delete_one(mongodb::bson::doc! { "_id": &receipt.id })
    .await?;
    Ok(())
}

async fn receipt_exists(
    db: &mongodb::Database,
    actor: &str,
    action: &str,
    action_request_id: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<crate::models::assistant_action_receipt::AssistantActionReceipt>(
            crate::models::assistant_action_receipt::COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {
            "user_id": actor,
            "action": action,
            "action_request_id": action_request_id,
        })
        .await?
        .is_some())
}

async fn external_key_credential_matches(
    state: &AppState,
    key: &crate::models::user_api_key::UserApiKey,
    credential: &str,
) -> AppResult<bool> {
    let ciphertext = if key.credential_type == "oauth2" {
        key.access_token_encrypted.as_deref()
    } else {
        key.credential_encrypted.as_deref()
    };
    let Some(ciphertext) = ciphertext else {
        return Ok(false);
    };
    Ok(state.encryption_keys.decrypt(ciphertext).await? == credential.as_bytes())
}

async fn resolve_endpoint_owner(
    state: &AppState,
    actor: &str,
    endpoint_id: &str,
) -> AppResult<String> {
    let endpoint = state
        .db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(mongodb::bson::doc! { "_id": endpoint_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;
    let access = org_service::resolve_owner_access(&state.db, actor, &endpoint.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }
    let services = crate::services::user_service_service::user_service_ids_for_endpoint(
        &state.db,
        &endpoint.user_id,
        endpoint_id,
    )
    .await?;
    if !access.allows_any_resource(&services) {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this endpoint".to_string(),
        ));
    }
    Ok(endpoint.user_id)
}

#[allow(clippy::too_many_arguments)]
async fn commit_endpoint_update(
    state: &AppState,
    auth_user: &AuthUser,
    actor: &str,
    endpoint_id: &str,
    label: Option<String>,
    endpoint_url: Option<String>,
    openapi_spec_url: Option<String>,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let owner_id = resolve_endpoint_owner(state, actor, endpoint_id).await?;
    let current = user_endpoint_service::get_endpoint(&state.db, &owner_id, endpoint_id).await?;
    if was_in_progress
        && label.as_deref().is_none_or(|value| value == current.label)
        && endpoint_url
            .as_deref()
            .is_none_or(|value| value == current.url)
        && openapi_spec_url
            .as_deref()
            .is_none_or(|value| current.openapi_spec_url.as_deref() == Some(value))
    {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    let _updated = user_endpoints::update_endpoint(
        State(state.clone()),
        auth_user.clone(),
        TelemetryContext::default(),
        Path(endpoint_id.to_string()),
        Json(UpdateEndpointRequest {
            url: endpoint_url,
            label,
            openapi_spec_url,
            recommended_skills: None,
        }),
    )
    .await?;
    mark_completed(&state.db, &receipt).await?;
    Ok(false)
}

async fn commit_endpoint_delete(
    state: &AppState,
    auth_user: &AuthUser,
    endpoint_id: &str,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let exists = state
        .db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(mongodb::bson::doc! { "_id": endpoint_id })
        .await?
        .is_some();
    if !exists {
        if was_in_progress {
            mark_completed(&state.db, &receipt).await?;
            return Ok(true);
        }
        release_receipt(&state.db, &receipt).await?;
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }
    user_endpoints::delete_endpoint(
        State(state.clone()),
        auth_user.clone(),
        TelemetryContext::default(),
        Path(endpoint_id.to_string()),
    )
    .await?;
    mark_completed(&state.db, &receipt).await?;
    Ok(false)
}

async fn commit_external_key_rotate(
    state: &AppState,
    auth_user: &AuthUser,
    external_key_id: &str,
    credential: String,
    current_key: crate::models::user_api_key::UserApiKey,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    if was_in_progress && external_key_credential_matches(state, &current_key, &credential).await? {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    let _ = user_api_keys_external::update_external_api_key(
        State(state.clone()),
        auth_user.clone(),
        Path(external_key_id.to_string()),
        Json(UpdateExternalApiKeyRequest {
            label: None,
            credential: Some(credential),
        }),
    )
    .await?;
    mark_completed(&state.db, &receipt).await?;
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
async fn commit_external_key_delete(
    state: &AppState,
    actor: &str,
    audit_actor: &AuditActor,
    external_key_id: &str,
    options: unified_key_service::DisconnectOptions,
    expected_siblings: Option<&[(String, String, String)]>,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let exists = state
        .db
        .collection::<crate::models::user_api_key::UserApiKey>(
            crate::models::user_api_key::COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! { "_id": external_key_id })
        .await?
        .is_some();
    if !exists {
        if was_in_progress {
            mark_completed(&state.db, &receipt).await?;
            return Ok(true);
        }
        release_receipt(&state.db, &receipt).await?;
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    let owner_id =
        user_api_keys_external::resolve_api_key_write_owner(state, actor, external_key_id).await?;
    match unified_key_service::disconnect_credentials_with_expected_siblings(
        &state.db,
        &state.encryption_keys,
        &owner_id,
        audit_actor,
        unified_key_service::DisconnectTarget::UserApiKey(external_key_id),
        options,
        expected_siblings,
    )
    .await
    {
        Ok(_) => {
            mark_completed(&state.db, &receipt).await?;
            Ok(false)
        }
        Err(error @ AppError::NotFound(_)) => {
            if was_in_progress {
                mark_completed(&state.db, &receipt).await?;
                Ok(true)
            } else {
                release_receipt(&state.db, &receipt).await?;
                Err(error)
            }
        }
        Err(error @ AppError::GrantCascadeConfirmationRequired(_)) => {
            release_receipt(&state.db, &receipt).await?;
            Err(error)
        }
        Err(error)
            if matches!(
                &error,
                AppError::Conflict(message)
                    if message == "the grant-cascade sibling set changed; review the confirmation again"
            ) =>
        {
            release_receipt(&state.db, &receipt).await?;
            Err(error)
        }
        Err(error) => Err(error),
    }
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_string())
}

fn endpoint_effect_response(
    endpoint_id: String,
    replayed: bool,
) -> AssistantEndpointEffectResponse {
    AssistantEndpointEffectResponse {
        resource: AssistantEndpointResource { endpoint_id },
        replayed,
    }
}

fn external_key_effect_response(
    external_key_id: String,
    replayed: bool,
) -> AssistantExternalKeyEffectResponse {
    AssistantExternalKeyEffectResponse {
        resource: AssistantExternalKeyResource { external_key_id },
        replayed,
    }
}

/// Update one user endpoint. URL shape (no userinfo, no fragment) is enforced
/// before the receipt is reserved so a poisoned URL cannot occupy the identity.
pub async fn update_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateAssistantEndpointRequest>,
) -> AppResult<Json<AssistantEndpointEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let endpoint_id = normalize_action_request_id(body.endpoint_id)?;
    let label = optional_trimmed(body.label);
    let endpoint_url = optional_trimmed(body.endpoint_url);
    let openapi_spec_url = optional_trimmed(body.openapi_spec_url);

    if label.is_none() && endpoint_url.is_none() && openapi_spec_url.is_none() {
        return Err(AppError::BadRequest(
            "endpoint.update requires at least one of label, endpointUrl, or openapiSpecUrl"
                .to_string(),
        ));
    }
    if let Some(url) = endpoint_url.as_deref()
        && !url.is_empty()
        && !url.starts_with("ssh://")
    {
        crate::services::url_validation::validate_user_endpoint_url(
            url,
            state.config.is_production(),
            "endpointUrl",
        )
        .await?;
    }

    let fingerprint = fingerprint_canonical(&EndpointUpdateFingerprint {
        action: ENDPOINT_UPDATE_ACTION,
        endpoint_id: &endpoint_id,
        label: label.as_deref(),
        endpoint_url: endpoint_url.as_deref(),
        openapi_spec_url: openapi_spec_url.as_deref(),
    })?;
    let actor = auth_user.user_id.to_string();
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        ENDPOINT_UPDATE_ACTION,
        &action_request_id,
        &fingerprint,
        endpoint_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => {
            Ok(Json(endpoint_effect_response(receipt.resource_id, true)))
        }
        ReceiptOutcome::InProgress(receipt) => {
            let replayed = commit_endpoint_update(
                &state,
                &auth_user,
                &actor,
                &endpoint_id,
                label,
                endpoint_url,
                openapi_spec_url,
                receipt,
                true,
            )
            .await?;
            Ok(Json(endpoint_effect_response(endpoint_id, replayed)))
        }
        ReceiptOutcome::Reserved(receipt) => {
            let replayed = commit_endpoint_update(
                &state,
                &auth_user,
                &actor,
                &endpoint_id,
                label,
                endpoint_url,
                openapi_spec_url,
                receipt,
                false,
            )
            .await?;
            Ok(Json(endpoint_effect_response(endpoint_id, replayed)))
        }
    }
}

/// Permanently delete one user endpoint. A receipt is completed only after the
/// underlying delete succeeds; a missing or foreign id never becomes a
/// successful deletion on an exact retry.
pub async fn delete_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<DeleteAssistantEndpointRequest>,
) -> AppResult<Json<AssistantEndpointEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let endpoint_id = normalize_action_request_id(body.endpoint_id)?;
    let fingerprint = fingerprint_canonical(&EndpointDeleteFingerprint {
        action: ENDPOINT_DELETE_ACTION,
        endpoint_id: &endpoint_id,
    })?;
    let actor = auth_user.user_id.to_string();
    let already_reserved = receipt_exists(
        &state.db,
        &actor,
        ENDPOINT_DELETE_ACTION,
        &action_request_id,
    )
    .await?;
    if !already_reserved {
        let owner_id = resolve_endpoint_owner(&state, &actor, &endpoint_id).await?;
        crate::services::user_service_service::ensure_user_managed_endpoint(
            &state.db,
            &owner_id,
            &endpoint_id,
        )
        .await?;
    }
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        ENDPOINT_DELETE_ACTION,
        &action_request_id,
        &fingerprint,
        endpoint_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => {
            Ok(Json(endpoint_effect_response(receipt.resource_id, true)))
        }
        ReceiptOutcome::InProgress(receipt) => Ok(Json(endpoint_effect_response(
            endpoint_id.clone(),
            commit_endpoint_delete(&state, &auth_user, &endpoint_id, receipt, true).await?,
        ))),
        ReceiptOutcome::Reserved(receipt) => Ok(Json(endpoint_effect_response(
            endpoint_id.clone(),
            commit_endpoint_delete(&state, &auth_user, &endpoint_id, receipt, false).await?,
        ))),
    }
}

/// Replace the secret of one stored external credential in place. The
/// replacement material is never stored on the receipt; exact retries replay
/// only the durable resource id.
pub async fn rotate_external_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<RotateAssistantExternalKeyRequest>,
) -> AppResult<Json<AssistantExternalKeyEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let external_key_id = normalize_action_request_id(body.external_key_id)?;
    let credential = body.credential;
    if credential.trim().is_empty() {
        return Err(AppError::ValidationError(
            "credential must not be empty".to_string(),
        ));
    }
    let existing_key = user_api_keys_external::resolve_api_key_write_target(
        &state,
        &auth_user.user_id.to_string(),
        &external_key_id,
    )
    .await?;
    if existing_key.credential_type == "ssh_certificate" {
        return Err(AppError::SshAuthModeUnsupportedForOperation(
            "ssh_certificate credentials are rotated through the SSH certificate flow".to_string(),
        ));
    }
    let fingerprint = fingerprint_canonical(&ExternalKeyRotateFingerprint {
        action: EXTERNAL_KEY_ROTATE_ACTION,
        external_key_id: &external_key_id,
        credential_fingerprint: fingerprint_sensitive_material(&credential),
    })?;
    let actor = auth_user.user_id.to_string();
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        EXTERNAL_KEY_ROTATE_ACTION,
        &action_request_id,
        &fingerprint,
        external_key_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(external_key_effect_response(
            receipt.resource_id,
            true,
        ))),
        ReceiptOutcome::InProgress(receipt) => {
            let replayed = commit_external_key_rotate(
                &state,
                &auth_user,
                &external_key_id,
                credential,
                existing_key,
                receipt,
                true,
            )
            .await?;
            Ok(Json(external_key_effect_response(
                external_key_id,
                replayed,
            )))
        }
        ReceiptOutcome::Reserved(receipt) => {
            let replayed = commit_external_key_rotate(
                &state,
                &auth_user,
                &external_key_id,
                credential,
                existing_key,
                receipt,
                false,
            )
            .await?;
            Ok(Json(external_key_effect_response(
                external_key_id,
                replayed,
            )))
        }
    }
}

/// Delete one stored external credential. `cascade_grant` / `grant_scope`
/// are confirmation protocol for the existing 11500 contract, not receipt
/// identity — the same actionRequestId may retry with cascade after a 409.
pub async fn delete_external_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<DeleteAssistantExternalKeyRequest>,
) -> AppResult<Json<AssistantExternalKeyEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let external_key_id = normalize_action_request_id(body.external_key_id)?;
    let cascade_grant = body.cascade_grant.unwrap_or(false);
    let grant_scope = body.grant_scope.clone();
    if cascade_grant && body.cascade_siblings.is_none() {
        return Err(AppError::ValidationError(
            "cascadeSiblings is required when cascadeGrant is true".to_string(),
        ));
    }
    let expected_siblings = body.cascade_siblings.as_ref().map(|siblings| {
        siblings
            .iter()
            .map(|sibling| {
                (
                    sibling.user_service_id.clone(),
                    sibling.name.clone(),
                    sibling.slug.clone(),
                )
            })
            .collect::<Vec<_>>()
    });
    let actor = auth_user.user_id.to_string();
    let options = unified_key_service::DisconnectOptions {
        cascade_grant,
        grant_scope: grant_scope.clone(),
    };
    let already_reserved = receipt_exists(
        &state.db,
        &actor,
        EXTERNAL_KEY_DELETE_ACTION,
        &action_request_id,
    )
    .await?;
    if !already_reserved {
        let owner_id =
            user_api_keys_external::resolve_api_key_write_owner(&state, &actor, &external_key_id)
                .await?;
        if !cascade_grant && grant_scope.as_deref() != Some("token") {
            unified_key_service::ensure_disconnect_confirmation(
                &state.db,
                &state.encryption_keys,
                &owner_id,
                unified_key_service::DisconnectTarget::UserApiKey(&external_key_id),
                options.clone(),
            )
            .await?;
        }
    }
    let fingerprint = fingerprint_canonical(&ExternalKeyDeleteFingerprint {
        action: EXTERNAL_KEY_DELETE_ACTION,
        external_key_id: &external_key_id,
        cascade_grant,
        grant_scope: grant_scope.as_deref(),
        cascade_siblings: canonical_cascade_siblings(body.cascade_siblings.as_deref()),
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        EXTERNAL_KEY_DELETE_ACTION,
        &action_request_id,
        &fingerprint,
        external_key_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(external_key_effect_response(
            receipt.resource_id,
            true,
        ))),
        ReceiptOutcome::InProgress(receipt) => {
            let audit_actor = AuditActor::from_auth_user(&auth_user);
            let replayed = commit_external_key_delete(
                &state,
                &actor,
                &audit_actor,
                &external_key_id,
                options,
                expected_siblings.as_deref(),
                receipt,
                true,
            )
            .await?;
            Ok(Json(external_key_effect_response(
                external_key_id,
                replayed,
            )))
        }
        ReceiptOutcome::Reserved(receipt) => {
            let audit_actor = AuditActor::from_auth_user(&auth_user);
            let replayed = commit_external_key_delete(
                &state,
                &actor,
                &audit_actor,
                &external_key_id,
                options,
                expected_siblings.as_deref(),
                receipt,
                false,
            )
            .await?;
            Ok(Json(external_key_effect_response(
                external_key_id,
                replayed,
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        routing::get,
    };
    use futures::TryStreamExt;
    use mongodb::{IndexModel, bson::doc, options::IndexOptions};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::handlers::user_api_keys_external;
    use crate::handlers::user_endpoints;
    use crate::models::assistant_action_receipt::{
        AssistantActionReceipt, AssistantActionReceiptStatus,
        COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
    };
    use crate::models::provider_config::{
        COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig, RevocationConfig,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_user, test_user_endpoint, test_user_service,
    };

    fn access_token(state: &AppState, user_id: &str) -> String {
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(user_id).expect("valid user id"),
            "",
            None,
            None,
            None,
            None,
            None,
        )
        .expect("sign test access token")
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .nest("/assistant/actions/endpoints", super::router())
            .route(
                "/endpoints/{endpoint_id}/authorization",
                get(user_endpoints::get_endpoint_authorization),
            )
            .route(
                "/api-keys/external/{key_id}/authorization",
                get(user_api_keys_external::get_external_api_key_authorization),
            )
            .with_state(state)
    }

    fn production_app(state: AppState) -> Router {
        let (_, private) = crate::routes::build_router();
        private.with_state(state)
    }

    async fn request(
        app: Router,
        token: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = app
            .oneshot(builder.body(body).expect("build request"))
            .await
            .expect("route response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response");
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes).into_owned() }));
        (status, value)
    }

    async fn prepare_database(prefix: &str) -> Option<(mongodb::Database, String)> {
        let db = connect_test_database(prefix).await?;
        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "action": 1, "action_request_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create receipt uniqueness index");
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");
        Some((db, actor_id))
    }

    async fn insert_endpoint(
        db: &mongodb::Database,
        user_id: &str,
        label: &str,
        url: &str,
    ) -> String {
        let ep_id = Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(&ep_id, user_id, label, url, None, None))
            .await
            .expect("insert endpoint");
        ep_id
    }

    fn fixture_external_key(key_id: &str, user_id: &str, label: &str) -> UserApiKey {
        UserApiKey {
            credential_epoch: 1,
            credential_source: None,
            id: key_id.to_string(),
            user_id: user_id.to_string(),
            label: label.to_string(),
            credential_type: "api_key".to_string(),
            credential_encrypted: Some(vec![1, 2, 3]),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn grant_provider() -> ProviderConfig {
        ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "github".to_string(),
            name: "GitHub".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://example.com/authorize".to_string()),
            token_url: Some("https://example.com/token".to_string()),
            revocation_url: None,
            revocation: Some(RevocationConfig {
                style: "github".to_string(),
                url: "https://example.com/revoke".to_string(),
                auth: "basic".to_string(),
                revokes_grant: true,
            }),
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            revocation_seed_version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn endpoint_update_rejects_url_with_userinfo_or_fragment() {
        let Some((db, actor_id)) = prepare_database("asst_ep_update_rejects_userinfo").await else {
            return;
        };
        let ep_id = insert_endpoint(&db, &actor_id, "API", "https://api.example.com/v1").await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        for (label, url) in [
            ("userinfo", "https://user:pass@api.example.com/v1"),
            ("fragment", "https://api.example.com/v1#token"),
        ] {
            let (status, response) = request(
                app(state.clone()),
                &token,
                "POST",
                "/assistant/actions/endpoints/update",
                Some(json!({
                    "actionRequestId": format!("update-{label}"),
                    "endpointId": ep_id,
                    "endpointUrl": url,
                })),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {response}");
            assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    #[tokio::test]
    async fn endpoint_update_interrupted_retry_resumes_and_completes() {
        // Falsifier: restore the unconditional InProgress 409 or reapply the
        // endpoint mutation; the retry then fails or overwrites `sentinel`.
        let Some((db, actor_id)) = prepare_database("asst_ep_update_interrupted").await else {
            return;
        };
        let endpoint_id =
            insert_endpoint(&db, &actor_id, "Before", "https://api.example.com").await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "endpoint-update-interrupted",
            "endpointId": endpoint_id,
            "label": "After",
        });
        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/update",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["replayed"], false);
        let sentinel = mongodb::bson::DateTime::from_millis(2_000_000_000_000);
        db.collection::<mongodb::bson::Document>(USER_ENDPOINTS)
            .update_one(
                doc! { "_id": &endpoint_id },
                doc! { "$set": { "updated_at": sentinel } },
            )
            .await
            .expect("set sentinel timestamp");
        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .update_one(
                doc! {
                    "user_id": &actor_id,
                    "action": ENDPOINT_UPDATE_ACTION,
                    "action_request_id": "endpoint-update-interrupted",
                },
                doc! {
                    "$set": { "status": "pending" },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .expect("reopen receipt as pending");
        let (status, retry) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/endpoints/update",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(retry["replayed"], true);
        let endpoint = db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(doc! { "_id": &endpoint_id })
            .await
            .expect("load endpoint")
            .expect("endpoint");
        assert_eq!(endpoint.label, "After");
        assert_eq!(endpoint.updated_at, sentinel.to_chrono());
        let receipt = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! {
                "user_id": &actor_id,
                "action": ENDPOINT_UPDATE_ACTION,
                "action_request_id": "endpoint-update-interrupted",
            })
            .await
            .expect("load receipt")
            .expect("receipt");
        assert_eq!(receipt.status, AssistantActionReceiptStatus::Completed);
    }

    #[tokio::test]
    async fn production_router_mounts_endpoint_and_external_key_evidence() {
        let Some((db, actor_id)) = prepare_database("asst_production_evidence_routes").await else {
            return;
        };
        let endpoint_id = insert_endpoint(
            &db,
            &actor_id,
            "Production route endpoint",
            "https://api.example.com",
        )
        .await;
        let key_id = Uuid::new_v4().to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_external_key(
                &key_id,
                &actor_id,
                "Production route key",
            ))
            .await
            .expect("insert external key");
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, endpoint_evidence) = request(
            production_app(state.clone()),
            &token,
            "GET",
            &format!("/api/v1/endpoints/{endpoint_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{endpoint_evidence}");
        assert_eq!(endpoint_evidence["id"], endpoint_id);

        let (status, external_key_evidence) = request(
            production_app(state),
            &token,
            "GET",
            &format!("/api/v1/api-keys/external/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{external_key_evidence}");
        assert_eq!(external_key_evidence["id"], key_id);
    }

    #[tokio::test]
    async fn external_key_rotate_reserves_successor_receipt() {
        let Some((db, actor_id)) = prepare_database("asst_ext_rotate_receipt").await else {
            return;
        };
        let key_id = Uuid::new_v4().to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_external_key(&key_id, &actor_id, "OpenAI"))
            .await
            .expect("insert external key");
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "rotate-successor",
            "externalKeyId": key_id,
            "credential": "sk-replacement-secret",
        });

        let (status, rotated) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-rotate",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{rotated}");
        assert_eq!(rotated["replayed"], false);
        assert_eq!(rotated["resource"]["externalKeyId"], key_id);
        assert!(rotated.get("credential").is_none());
        assert!(rotated.get("fullKey").is_none());

        let receipts: Vec<AssistantActionReceipt> = db
            .collection(ASSISTANT_ACTION_RECEIPTS)
            .find(doc! {
                "user_id": &actor_id,
                "action": EXTERNAL_KEY_ROTATE_ACTION,
                "action_request_id": "rotate-successor",
            })
            .await
            .expect("find receipts")
            .try_collect()
            .await
            .expect("collect receipts");
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].resource_id, key_id);
        assert_eq!(receipts[0].status, AssistantActionReceiptStatus::Completed);

        let (status, replayed) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-rotate",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed["replayed"], true);
        assert_eq!(replayed["resource"]["externalKeyId"], key_id);
        assert!(replayed.get("credential").is_none());

        let (status, evidence) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/api-keys/external/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{evidence}");
        assert_eq!(evidence["id"], key_id);
        assert_eq!(evidence["status"], "active");
        assert!(evidence.get("error_message").is_none());
        assert!(evidence.get("label").is_none());
    }

    #[tokio::test]
    async fn external_key_rotate_conflicting_replay_fails_closed() {
        let Some((db, actor_id)) = prepare_database("asst_ext_rotate_conflict").await else {
            return;
        };
        let first_id = Uuid::new_v4().to_string();
        let second_id = Uuid::new_v4().to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_many([
                fixture_external_key(&first_id, &actor_id, "First"),
                fixture_external_key(&second_id, &actor_id, "Second"),
            ])
            .await
            .expect("insert keys");
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-rotate",
            Some(json!({
                "actionRequestId": "rotate-conflict",
                "externalKeyId": first_id,
                "credential": "sk-one",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");

        let (status, conflict) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-rotate",
            Some(json!({
                "actionRequestId": "rotate-conflict",
                "externalKeyId": first_id,
                "credential": "sk-two",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn external_key_rotate_rejects_ssh_certificate_before_receipt() {
        let Some((db, actor_id)) = prepare_database("asst_ext_rotate_ssh_certificate").await else {
            return;
        };
        let key_id = Uuid::new_v4().to_string();
        let mut key = fixture_external_key(&key_id, &actor_id, "SSH certificate");
        key.credential_type = "ssh_certificate".to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(key)
            .await
            .expect("insert SSH certificate key");
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, response) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-rotate",
            Some(json!({
                "actionRequestId": "rotate-ssh-certificate",
                "externalKeyId": key_id,
                "credential": "replacement",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["error_code"], 1015);
        assert_eq!(
            state
                .db
                .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {
                    "user_id": &actor_id,
                    "action": EXTERNAL_KEY_ROTATE_ACTION,
                    "action_request_id": "rotate-ssh-certificate",
                })
                .await
                .expect("count SSH rotation receipts"),
            0,
            "unsupported rotation must not reserve a receipt"
        );
    }

    #[tokio::test]
    async fn endpoint_delete_stale_id_404_not_500() {
        let Some((db, actor_id)) = prepare_database("asst_ep_delete_stale").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let stale_id = Uuid::new_v4().to_string();

        let (status, deleted) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/delete",
            Some(json!({
                "actionRequestId": "delete-stale",
                "endpointId": stale_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{deleted}");
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let (status, replay) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/delete",
            Some(json!({
                "actionRequestId": "delete-stale",
                "endpointId": stale_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{replay}");
        assert_ne!(status, StatusCode::OK);

        let (status, evidence) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/endpoints/{stale_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{evidence}");
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let foreign_user_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&foreign_user_id, UserType::Person))
            .await
            .expect("insert foreign user");
        let foreign_endpoint_id =
            insert_endpoint(&db, &foreign_user_id, "Foreign", "https://foreign.example").await;
        let foreign_body = json!({
            "actionRequestId": "delete-foreign",
            "endpointId": foreign_endpoint_id,
        });
        let (status, first_foreign) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/delete",
            Some(foreign_body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{first_foreign}");
        let (status, replay_foreign) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/endpoints/delete",
            Some(foreign_body),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{replay_foreign}");
        assert_ne!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn external_key_delete_missing_or_foreign_id_never_replays_success() {
        let Some((db, actor_id)) = prepare_database("asst_ext_delete_absence").await else {
            return;
        };
        let foreign_user_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&foreign_user_id, UserType::Person))
            .await
            .expect("insert foreign user");
        let foreign_key_id = Uuid::new_v4().to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_external_key(
                &foreign_key_id,
                &foreign_user_id,
                "Foreign",
            ))
            .await
            .expect("insert foreign key");
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        for (action_request_id, external_key_id) in [
            ("delete-missing", Uuid::new_v4().to_string()),
            ("delete-foreign", foreign_key_id),
        ] {
            let body = json!({
                "actionRequestId": action_request_id,
                "externalKeyId": external_key_id,
            });
            let (status, first) = request(
                app(state.clone()),
                &token,
                "POST",
                "/assistant/actions/endpoints/external-key-delete",
                Some(body.clone()),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{first}");
            let (status, retry) = request(
                app(state.clone()),
                &token,
                "POST",
                "/assistant/actions/endpoints/external-key-delete",
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{retry}");
            assert_ne!(status, StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn external_key_delete_with_cascade_grant_confirms() {
        let Some((db, actor_id)) = prepare_database("asst_ext_delete_cascade").await else {
            return;
        };
        let provider = grant_provider();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .expect("insert provider");

        let mut keys = Vec::new();
        for slug in ["svc-primary", "svc-sibling"] {
            let key_id = Uuid::new_v4().to_string();
            let endpoint_id = Uuid::new_v4().to_string();
            let service_id = Uuid::new_v4().to_string();
            let mut key = fixture_external_key(&key_id, &actor_id, slug);
            key.credential_type = "oauth2".to_string();
            key.provider_config_id = Some(provider.id.clone());
            key.connection_id = Some(Uuid::new_v4().to_string());
            key.credential_source = Some("platform".to_string());
            key.access_token_encrypted = Some(vec![9, 8, 7]);
            db.collection::<UserApiKey>(USER_API_KEYS)
                .insert_one(&key)
                .await
                .expect("insert oauth key");
            db.collection::<UserEndpoint>(USER_ENDPOINTS)
                .insert_one(test_user_endpoint(
                    &endpoint_id,
                    &actor_id,
                    slug,
                    "https://api.github.com",
                    None,
                    None,
                ))
                .await
                .expect("insert endpoint");
            let mut service =
                test_user_service(&service_id, &actor_id, slug, &endpoint_id, None, None);
            service.api_key_id = Some(key_id.clone());
            service.auth_method = "bearer".to_string();
            db.collection::<UserService>(USER_SERVICES)
                .insert_one(service)
                .await
                .expect("insert service");
            keys.push(key_id);
        }
        let primary_key = keys[0].clone();
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-delete",
            Some(json!({
                "actionRequestId": "delete-cascade",
                "externalKeyId": primary_key,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{first}");
        assert_eq!(first["error_code"], 11500);
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            state
                .db
                .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {
                    "user_id": &actor_id,
                    "action": EXTERNAL_KEY_DELETE_ACTION,
                    "action_request_id": "delete-cascade",
                })
                .await
                .expect("count confirmation receipts"),
            0,
            "the preflight 11500 must not reserve a receipt"
        );

        let cascade_siblings = first["details"]["siblings"].clone();
        assert!(cascade_siblings.is_array());
        let (status, changed) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-delete",
            Some(json!({
                "actionRequestId": "delete-cascade",
                "externalKeyId": primary_key,
                "cascadeGrant": true,
                "cascadeSiblings": [],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{changed}");
        assert_eq!(changed["error_code"], 1004);

        let (status, confirmed) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/endpoints/external-key-delete",
            Some(json!({
                "actionRequestId": "delete-cascade",
                "externalKeyId": primary_key,
                "cascadeGrant": true,
                "cascadeSiblings": cascade_siblings,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        assert_eq!(confirmed["replayed"], false);
        assert_eq!(confirmed["resource"]["externalKeyId"], primary_key);

        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/api-keys/external/{primary_key}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{evidence}");
    }

    #[test]
    fn rotate_request_debug_redacts_credential() {
        let request = RotateAssistantExternalKeyRequest {
            action_request_id: "act-1".to_string(),
            external_key_id: "key-1".to_string(),
            credential: "sk-live-secret".to_string(),
        };
        let rendered = format!("{request:?}");
        assert!(rendered.contains("[REDACTED]"));
        assert!(!rendered.contains("sk-live-secret"));
    }

    #[test]
    fn replay_responses_are_secret_free() {
        let endpoint = endpoint_effect_response("endpoint-1".to_string(), true);
        assert_eq!(
            serde_json::to_value(endpoint).unwrap(),
            json!({
                "resource": { "endpointId": "endpoint-1" },
                "replayed": true,
            })
        );
        let key = external_key_effect_response("external-1".to_string(), true);
        assert_eq!(
            serde_json::to_value(key).unwrap(),
            json!({
                "resource": { "externalKeyId": "external-1" },
                "replayed": true,
            })
        );
    }

    #[test]
    fn external_key_rotate_fingerprint_is_keyed() {
        // Falsifier: revert the rotate call site to plain SHA-256; the stored
        // field then equals the offline-oracle digest of caller material.
        let fingerprint = fingerprint_sensitive_material("credential-a");
        let serialized = serde_json::to_value(ExternalKeyRotateFingerprint {
            action: EXTERNAL_KEY_ROTATE_ACTION,
            external_key_id: "external-1",
            credential_fingerprint: fingerprint,
        })
        .expect("serialize fingerprint");
        assert_ne!(
            serialized["credentialFingerprint"],
            hex::encode(Sha256::digest(b"credential-a"))
        );
    }
}
