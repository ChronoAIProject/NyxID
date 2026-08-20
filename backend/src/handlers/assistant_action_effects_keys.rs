//! Wave-2 key-family assistant action effects (`key.update`, `key.delete`,
//! `key.extend_scope`, `key.bind_credential`).
//!
//! Descriptors stay dormant under the current registry revision. Effects
//! reserve a secret-free receipt before mutating, replay exact retries, and
//! fail closed on identity reuse with different content. Evidence reads ride
//! `GET /api-keys/{id}/authorization` for update/extend/delete, and
//! `GET /api-keys/{id}/bindings/by-service/{user_service_id}/authorization`
//! for bind (addressable from the report's `{keyId, userServiceId}`).

use std::sync::LazyLock;

use axum::{Json, Router, extract::State, routing::post};
use regex::Regex;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::AgentServiceBinding;
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::assistant_action_receipt::AssistantActionReceipt;
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_receipts::{
    ReceiptOutcome, fingerprint_canonical, mark_completed, normalize_action_request_id,
    reserve_or_replay,
};
use crate::services::{agent_binding_service, key_service, org_service};

const KEY_UPDATE_ACTION: &str = "key.update";
const KEY_DELETE_ACTION: &str = "key.delete";
const KEY_EXTEND_SCOPE_ACTION: &str = "key.extend_scope";
const KEY_BIND_CREDENTIAL_ACTION: &str = "key.bind_credential";
const MAX_SERVICE_IDS: usize = 64;
const MAX_KEY_NAME_LEN: usize = 200;

static SECRET_SHAPED_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})")
        .expect("secret-shape regex")
});

/// Effect routes mounted at `/api/v1/assistant/actions/keys`.
///
/// Binding evidence is served on the production API-key router:
/// `GET /api/v1/api-keys/{key_id}/bindings/by-service/{user_service_id}/authorization`
/// (and the binding-id sibling). This nest is effects only.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/update", post(update_key))
        .route("/delete", post(delete_key))
        .route("/extend-scope", post(extend_scope))
        .route("/bind-credential", post(bind_credential))
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantKeyResource {
    pub key_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAssistantKeyRequest {
    pub action_request_id: String,
    pub key_id: String,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub description: Option<String>,
    pub expected_state_version: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAssistantKeyResponse {
    pub resource: AssistantKeyResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAssistantKeyRequest {
    pub action_request_id: String,
    pub key_id: String,
    pub expected_state_version: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAssistantKeyResponse {
    pub resource: AssistantKeyResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtendAssistantKeyScopeRequest {
    pub action_request_id: String,
    pub key_id: String,
    pub add_service_ids: Vec<String>,
    pub expected_state_version: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExtendAssistantKeyScopeResponse {
    pub resource: AssistantKeyResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindAssistantKeyCredentialRequest {
    pub action_request_id: String,
    pub key_id: String,
    pub user_service_id: String,
    pub external_key_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BindAssistantKeyCredentialResponse {
    pub resource: AssistantKeyResource,
    pub binding_id: String,
    pub replayed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyUpdateFingerprint<'a> {
    action: &'static str,
    key_id: &'a str,
    name: Option<&'a str>,
    platform: Option<&'a str>,
    description: Option<&'a str>,
    expected_state_version: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyDeleteFingerprint<'a> {
    action: &'static str,
    key_id: &'a str,
    expected_state_version: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyExtendScopeFingerprint<'a> {
    action: &'static str,
    key_id: &'a str,
    add_service_ids: &'a [String],
    expected_state_version: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyBindFingerprint<'a> {
    action: &'static str,
    key_id: &'a str,
    user_service_id: &'a str,
    external_key_id: &'a str,
}

struct NormalizedUpdate {
    action_request_id: String,
    key_id: String,
    name: Option<String>,
    platform: Option<String>,
    description: Option<String>,
    expected_state_version: Option<i64>,
}

struct NormalizedDelete {
    action_request_id: String,
    key_id: String,
    expected_state_version: i64,
}

struct NormalizedExtend {
    action_request_id: String,
    key_id: String,
    add_service_ids: Vec<String>,
    expected_state_version: Option<i64>,
}

struct NormalizedBind {
    action_request_id: String,
    key_id: String,
    user_service_id: String,
    external_key_id: String,
}

fn reject_secret_shaped(field: &str, value: &str) -> AppResult<()> {
    if SECRET_SHAPED_VALUE.is_match(value) {
        return Err(AppError::ValidationError(format!(
            "{field} must not contain secret-shaped material"
        )));
    }
    Ok(())
}

fn parse_uuid_id(value: &str, field: &str) -> AppResult<String> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::ValidationError(format!("{field} must be a UUID")))
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_unique_ids(values: Vec<String>, field: &str) -> AppResult<Vec<String>> {
    let mut ids = values
        .into_iter()
        .map(|value| parse_uuid_id(&value, field))
        .collect::<AppResult<Vec<_>>>()?;
    if ids.is_empty() || ids.len() > MAX_SERVICE_IDS {
        return Err(AppError::ValidationError(format!(
            "{field} must contain 1 to {MAX_SERVICE_IDS} ids"
        )));
    }
    ids.sort();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(AppError::ValidationError(format!(
            "{field} must not contain duplicates"
        )));
    }
    Ok(ids)
}

fn normalize_update(body: UpdateAssistantKeyRequest) -> AppResult<NormalizedUpdate> {
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let key_id = parse_uuid_id(&body.key_id, "keyId")?;
    let name = optional_trimmed(body.name);
    let platform = optional_trimmed(body.platform);
    let description = body.description.map(|value| value.trim().to_string());
    if name.is_none() && platform.is_none() && description.is_none() {
        return Err(AppError::ValidationError(
            "key.update requires at least one of name, platform, or description".to_string(),
        ));
    }
    if let Some(name) = name.as_deref() {
        if name.is_empty() || name.len() > MAX_KEY_NAME_LEN {
            return Err(AppError::ValidationError(
                "API key name must be between 1 and 200 characters".to_string(),
            ));
        }
        reject_secret_shaped("name", name)?;
    }
    if let Some(platform) = platform.as_deref() {
        reject_secret_shaped("platform", platform)?;
    }
    if let Some(description) = description.as_deref() {
        reject_secret_shaped("description", description)?;
    }
    if body
        .expected_state_version
        .is_some_and(|version| version <= 0)
    {
        return Err(AppError::ValidationError(
            "expectedStateVersion must be a positive integer".to_string(),
        ));
    }
    Ok(NormalizedUpdate {
        action_request_id,
        key_id,
        name,
        platform,
        description,
        expected_state_version: body.expected_state_version,
    })
}

fn normalize_delete(body: DeleteAssistantKeyRequest) -> AppResult<NormalizedDelete> {
    if body.expected_state_version <= 0 {
        return Err(AppError::ValidationError(
            "expectedStateVersion must be a positive integer".to_string(),
        ));
    }
    Ok(NormalizedDelete {
        action_request_id: normalize_action_request_id(body.action_request_id)?,
        key_id: parse_uuid_id(&body.key_id, "keyId")?,
        expected_state_version: body.expected_state_version,
    })
}

fn normalize_extend(body: ExtendAssistantKeyScopeRequest) -> AppResult<NormalizedExtend> {
    if body
        .expected_state_version
        .is_some_and(|version| version <= 0)
    {
        return Err(AppError::ValidationError(
            "expectedStateVersion must be a positive integer".to_string(),
        ));
    }
    Ok(NormalizedExtend {
        action_request_id: normalize_action_request_id(body.action_request_id)?,
        key_id: parse_uuid_id(&body.key_id, "keyId")?,
        add_service_ids: normalize_unique_ids(body.add_service_ids, "addServiceIds")?,
        expected_state_version: body.expected_state_version,
    })
}

fn normalize_bind(body: BindAssistantKeyCredentialRequest) -> AppResult<NormalizedBind> {
    Ok(NormalizedBind {
        action_request_id: normalize_action_request_id(body.action_request_id)?,
        key_id: parse_uuid_id(&body.key_id, "keyId")?,
        user_service_id: parse_uuid_id(&body.user_service_id, "userServiceId")?,
        external_key_id: parse_uuid_id(&body.external_key_id, "externalKeyId")?,
    })
}

async fn resolve_write_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<(String, ApiKey)> {
    let key = state
        .db
        .collection::<ApiKey>(API_KEYS)
        .find_one(mongodb::bson::doc! { "_id": key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;
    let access = org_service::resolve_owner_access(&state.db, actor, &key.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this API key".to_string(),
        ));
    }
    Ok((key.user_id.clone(), key))
}

fn stale_if_conflict(error: &AppError) -> bool {
    matches!(error, AppError::Conflict(_))
}

fn update_already_applied(key: &ApiKey, request: &NormalizedUpdate) -> bool {
    request.name.as_ref().is_none_or(|name| key.name == *name)
        && request
            .platform
            .as_ref()
            .is_none_or(|platform| key.platform.as_deref() == Some(platform.as_str()))
        && request
            .description
            .as_ref()
            .is_none_or(|description| key.description.as_deref() == Some(description.as_str()))
}

fn extend_already_applied(key: &ApiKey, add_service_ids: &[String]) -> bool {
    add_service_ids
        .iter()
        .all(|id| key.allowed_service_ids.iter().any(|current| current == id))
}

async fn validate_personal_service_ids(
    db: &mongodb::Database,
    user_id: &str,
    service_ids: &[String],
) -> AppResult<()> {
    let matching = db
        .collection::<UserService>(USER_SERVICES)
        .count_documents(mongodb::bson::doc! {
            "_id": { "$in": service_ids },
            "user_id": user_id,
            "is_active": true,
        })
        .await?;
    if matching != service_ids.len() as u64 {
        return Err(AppError::ValidationError(
            "addServiceIds must identify active services owned by this account".to_string(),
        ));
    }
    Ok(())
}

fn union_service_ids(existing: &[String], added: &[String]) -> Vec<String> {
    let mut ids = existing.to_vec();
    for id in added {
        if !ids.iter().any(|current| current == id) {
            ids.push(id.clone());
        }
    }
    ids
}

/// POST /api/v1/assistant/actions/keys/update
pub async fn update_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateAssistantKeyRequest>,
) -> AppResult<Json<UpdateAssistantKeyResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_update(body)?;
    let fingerprint = fingerprint_canonical(&KeyUpdateFingerprint {
        action: KEY_UPDATE_ACTION,
        key_id: &request.key_id,
        name: request.name.as_deref(),
        platform: request.platform.as_deref(),
        description: request.description.as_deref(),
        expected_state_version: request.expected_state_version,
    })?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        KEY_UPDATE_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.key_id.clone(),
    )
    .await?;

    let replayed = match outcome {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) | ReceiptOutcome::Reserved(receipt) => {
            commit_key_update(&state, &actor, &request, receipt).await?
        }
    };

    Ok(Json(UpdateAssistantKeyResponse {
        resource: AssistantKeyResource {
            key_id: request.key_id,
        },
        replayed,
    }))
}

/// POST /api/v1/assistant/actions/keys/delete
pub async fn delete_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<DeleteAssistantKeyRequest>,
) -> AppResult<Json<DeleteAssistantKeyResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_delete(body)?;
    let fingerprint = fingerprint_canonical(&KeyDeleteFingerprint {
        action: KEY_DELETE_ACTION,
        key_id: &request.key_id,
        expected_state_version: request.expected_state_version,
    })?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        KEY_DELETE_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.key_id.clone(),
    )
    .await?;

    let replayed = match outcome {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) | ReceiptOutcome::Reserved(receipt) => {
            commit_key_delete(&state, &actor, &request, receipt).await?
        }
    };

    Ok(Json(DeleteAssistantKeyResponse {
        resource: AssistantKeyResource {
            key_id: request.key_id,
        },
        replayed,
    }))
}

/// POST /api/v1/assistant/actions/keys/extend-scope
pub async fn extend_scope(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<ExtendAssistantKeyScopeRequest>,
) -> AppResult<Json<ExtendAssistantKeyScopeResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_extend(body)?;
    let fingerprint = fingerprint_canonical(&KeyExtendScopeFingerprint {
        action: KEY_EXTEND_SCOPE_ACTION,
        key_id: &request.key_id,
        add_service_ids: &request.add_service_ids,
        expected_state_version: request.expected_state_version,
    })?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        KEY_EXTEND_SCOPE_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.key_id.clone(),
    )
    .await?;

    let replayed = match outcome {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) | ReceiptOutcome::Reserved(receipt) => {
            commit_key_extend(&state, &actor, &request, receipt).await?
        }
    };

    Ok(Json(ExtendAssistantKeyScopeResponse {
        resource: AssistantKeyResource {
            key_id: request.key_id,
        },
        replayed,
    }))
}

/// POST /api/v1/assistant/actions/keys/bind-credential
pub async fn bind_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<BindAssistantKeyCredentialRequest>,
) -> AppResult<Json<BindAssistantKeyCredentialResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_bind(body)?;
    let fingerprint = fingerprint_canonical(&KeyBindFingerprint {
        action: KEY_BIND_CREDENTIAL_ACTION,
        key_id: &request.key_id,
        user_service_id: &request.user_service_id,
        external_key_id: &request.external_key_id,
    })?;
    let (owner_id, current) = resolve_write_owner(&state, &actor, &request.key_id).await?;
    if !current.is_active {
        return Err(AppError::NotFound("API key not found".to_string()));
    }

    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        KEY_BIND_CREDENTIAL_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.key_id.clone(),
    )
    .await?;

    let (binding_id, replayed) = match outcome {
        ReceiptOutcome::Replay(_) => {
            let binding = existing_binding(
                &state.db,
                &owner_id,
                &request.key_id,
                &request.user_service_id,
            )
            .await?
            .ok_or_else(|| {
                AppError::Conflict(
                    "the binding created for this action is no longer present".to_string(),
                )
            })?;
            (binding.id, true)
        }
        ReceiptOutcome::InProgress(receipt) | ReceiptOutcome::Reserved(receipt) => {
            commit_key_bind(&state, &owner_id, &actor, &request, receipt).await?
        }
    };

    Ok(Json(BindAssistantKeyCredentialResponse {
        resource: AssistantKeyResource {
            key_id: request.key_id,
        },
        binding_id,
        replayed,
    }))
}

async fn commit_key_update(
    state: &AppState,
    actor: &str,
    request: &NormalizedUpdate,
    receipt: AssistantActionReceipt,
) -> AppResult<bool> {
    let (owner_id, current) = resolve_write_owner(state, actor, &request.key_id).await?;
    if !current.is_active {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    if update_already_applied(&current, request) {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    match key_service::update_api_key_scope_with_expected_state_version(
        &state.db,
        &owner_id,
        Some(actor),
        &request.key_id,
        request.name.as_deref(),
        request.description.as_deref(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        request.platform.as_deref().map(Some),
        None,
        None,
        request.expected_state_version,
    )
    .await
    {
        Ok(_) => {
            mark_completed(&state.db, &receipt).await?;
            Ok(false)
        }
        Err(error) if stale_if_conflict(&error) => {
            let (_, latest) = resolve_write_owner(state, actor, &request.key_id).await?;
            if update_already_applied(&latest, request) {
                mark_completed(&state.db, &receipt).await?;
                Ok(true)
            } else {
                Err(AppError::Conflict(
                    "the API key changed since this action was prepared".to_string(),
                ))
            }
        }
        Err(error) => Err(error),
    }
}

async fn commit_key_delete(
    state: &AppState,
    actor: &str,
    request: &NormalizedDelete,
    receipt: AssistantActionReceipt,
) -> AppResult<bool> {
    let (owner_id, current) = resolve_write_owner(state, actor, &request.key_id).await?;
    if !current.is_active {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    match key_service::delete_api_key_with_expected_state_version(
        &state.db,
        &owner_id,
        &request.key_id,
        Some(request.expected_state_version),
    )
    .await
    {
        Ok(()) => {
            mark_completed(&state.db, &receipt).await?;
            Ok(false)
        }
        Err(error) if stale_if_conflict(&error) => {
            let (_, latest) = resolve_write_owner(state, actor, &request.key_id).await?;
            if !latest.is_active {
                mark_completed(&state.db, &receipt).await?;
                Ok(true)
            } else {
                Err(AppError::Conflict(
                    "the API key changed since this action was prepared".to_string(),
                ))
            }
        }
        Err(AppError::NotFound(_)) => {
            mark_completed(&state.db, &receipt).await?;
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

async fn commit_key_extend(
    state: &AppState,
    actor: &str,
    request: &NormalizedExtend,
    receipt: AssistantActionReceipt,
) -> AppResult<bool> {
    let (owner_id, current) = resolve_write_owner(state, actor, &request.key_id).await?;
    if !current.is_active {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    if current.allow_all_services {
        return Err(AppError::ValidationError(
            "cannot extend scope on a key that already allows all services".to_string(),
        ));
    }
    if extend_already_applied(&current, &request.add_service_ids) {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    validate_personal_service_ids(&state.db, &owner_id, &request.add_service_ids).await?;
    let merged = union_service_ids(&current.allowed_service_ids, &request.add_service_ids);
    match key_service::update_api_key_scope_with_expected_state_version(
        &state.db,
        &owner_id,
        Some(actor),
        &request.key_id,
        None,
        None,
        None,
        Some(&merged),
        None,
        Some(false),
        None,
        None,
        None,
        None,
        None,
        None,
        request.expected_state_version,
    )
    .await
    {
        Ok(_) => {
            mark_completed(&state.db, &receipt).await?;
            Ok(false)
        }
        Err(error) if stale_if_conflict(&error) => {
            let (_, latest) = resolve_write_owner(state, actor, &request.key_id).await?;
            if extend_already_applied(&latest, &request.add_service_ids) {
                mark_completed(&state.db, &receipt).await?;
                Ok(true)
            } else {
                Err(AppError::Conflict(
                    "the API key changed since this action was prepared".to_string(),
                ))
            }
        }
        Err(error) => Err(error),
    }
}

async fn commit_key_bind(
    state: &AppState,
    owner_id: &str,
    actor: &str,
    request: &NormalizedBind,
    receipt: AssistantActionReceipt,
) -> AppResult<(String, bool)> {
    let binding = match existing_binding(
        &state.db,
        owner_id,
        &request.key_id,
        &request.user_service_id,
    )
    .await?
    {
        Some(existing) if existing.user_api_key_id == request.external_key_id => {
            mark_completed(&state.db, &receipt).await?;
            return Ok((existing.id, true));
        }
        Some(_) => {
            return Err(AppError::Conflict(
                "Binding already exists for this API key and service".to_string(),
            ));
        }
        None => {
            agent_binding_service::create_binding_with_scope_authorization(
                &state.db,
                owner_id,
                Some(actor),
                &request.key_id,
                &request.user_service_id,
                &request.external_key_id,
            )
            .await?
        }
    };
    mark_completed(&state.db, &receipt).await?;
    Ok((binding.id, false))
}

async fn existing_binding(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    user_service_id: &str,
) -> AppResult<Option<AgentServiceBinding>> {
    Ok(
        agent_binding_service::list_bindings(db, user_id, api_key_id)
            .await?
            .into_iter()
            .find(|binding| binding.user_service_id == user_service_id),
    )
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use mongodb::{IndexModel, bson::doc, options::IndexOptions};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::models::api_key::{ApiKeyPurpose, COLLECTION_NAME as API_KEYS};
    use crate::models::assistant_action_receipt::{
        AssistantActionReceiptStatus, COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
    use crate::test_utils::{
        connect_test_database, connect_transaction_test_database, test_app_state, test_user,
        test_user_endpoint, test_user_service,
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

    async fn ensure_receipt_index(db: &mongodb::Database) {
        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "action": 1, "action_request_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create receipt uniqueness index");
    }

    fn test_api_key(
        id: &str,
        user_id: &str,
        name: &str,
        allowed_service_ids: Vec<String>,
    ) -> ApiKey {
        let now = chrono::Utc::now() - chrono::Duration::minutes(5);
        ApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            name: name.to_string(),
            key_prefix: "safe-prefix".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "proxy".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: now,
            rotation_predecessor_id: None,
            state_version: 1,
            updated_at: Some(now),
            description: Some("assistant key fixture".to_string()),
            allowed_service_ids,
            allowed_node_ids: Vec::new(),
            allow_all_services: false,
            allow_all_nodes: false,
            rate_limit_per_second: Some(10),
            rate_limit_burst: Some(20),
            platform: Some("codex".to_string()),
            callback_url: None,
            purpose: ApiKeyPurpose::General,
            scheduled_write_enabled: false,
        }
    }

    fn fixture_user_api_key(id: &str, user_id: &str) -> UserApiKey {
        UserApiKey {
            credential_source: None,
            id: id.to_string(),
            user_id: user_id.to_string(),
            label: "test-credential".to_string(),
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
            source: None,
            source_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    async fn prepare_key_database(
        prefix: &str,
    ) -> Option<(mongodb::Database, String, String, UserService, UserService)> {
        let db = connect_test_database(prefix).await?;
        ensure_receipt_index(&db).await;

        let actor_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&other_id, UserType::Person),
            ])
            .await
            .expect("insert users");

        let own_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &actor_id,
            "own-service",
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        let extra_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &actor_id,
            "extra-service",
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        let other_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &other_id,
            "other-service",
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([
                own_service.clone(),
                extra_service.clone(),
                other_service.clone(),
            ])
            .await
            .expect("insert services");

        let key_id = Uuid::new_v4().to_string();
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(test_api_key(
                &key_id,
                &actor_id,
                "coding-agent",
                vec![own_service.id.clone()],
            ))
            .await
            .expect("insert api key");

        Some((db, actor_id, key_id, own_service, extra_service))
    }

    #[test]
    fn wave2_key_descriptors_present_and_dormant_in_manifest() {
        let manifest: Value =
            serde_json::from_str(crate::handlers::assistant_actions::manifest_body()).unwrap();
        assert_eq!(
            manifest["revision"],
            crate::handlers::assistant_actions::ASSISTANT_ACTIONS_REVISION
        );
        assert_eq!(manifest["revision"], "nyxid-assistant-actions.v8");

        let actions = manifest["actions"].as_array().unwrap();
        let pinned: Vec<&str> = actions
            .iter()
            .take(4)
            .map(|entry| entry["action"].as_str().unwrap())
            .collect();
        assert_eq!(
            pinned,
            [
                "service.connect",
                "service.reauthorize",
                "key.create",
                "key.rotate"
            ]
        );

        for (name, risk) in [
            ("key.update", "grant"),
            ("key.delete", "destructive"),
            ("key.extend_scope", "grant"),
            ("key.bind_credential", "grant"),
        ] {
            let entry = actions
                .iter()
                .find(|item| item["action"] == name)
                .unwrap_or_else(|| panic!("missing dormant descriptor {name}"));
            assert_eq!(
                entry["remember_eligible"], false,
                "{name} must never be remember_eligible"
            );
            assert_eq!(entry["risk"], risk, "wrong risk for {name}");
            assert!(
                !pinned.contains(&name),
                "{name} must stay outside the v8 pinned set"
            );
        }
    }

    #[tokio::test]
    async fn key_update_effect_reserves_receipt_and_replays_exact_retry() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_update_replay").await
        else {
            return;
        };
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "update-exact",
            "keyId": key_id,
            "name": "renamed-agent",
            "expectedStateVersion": 1,
        });

        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["replayed"], false);
        assert_eq!(first["resource"]["keyId"], key_id);

        let (status, second) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_eq!(second["replayed"], true);
        assert_eq!(second["resource"]["keyId"], key_id);
        assert!(second.get("fullKey").is_none());

        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{evidence}");
        assert_eq!(evidence["name"], "renamed-agent");
        assert_eq!(evidence["state_version"], 2);
    }

    #[tokio::test]
    async fn key_update_rejects_secret_shaped_name() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_update_secret_name").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, response) = request(
            app(state),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(json!({
                "actionRequestId": "update-secret",
                "keyId": key_id,
                "name": "Bearer sk-live-abc123def456",
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");

        let stored = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &key_id })
            .await
            .expect("load key")
            .expect("key exists");
        assert_eq!(stored.name, "coding-agent");
        assert_eq!(stored.state_version, 1);
    }

    #[tokio::test]
    async fn key_update_evidence_reflects_state_version_advance() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_update_evidence").await
        else {
            return;
        };
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, before) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{before}");
        let before_version = before["state_version"].as_i64().expect("lineage version");
        assert!(before_version >= 1);

        let (status, updated) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(json!({
                "actionRequestId": "update-evidence",
                "keyId": key_id,
                "platform": "claude-code",
                "expectedStateVersion": before_version,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        assert_eq!(updated["replayed"], false);

        let (status, after) = request(
            app(state),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{after}");
        assert_eq!(after["id"], key_id);
        assert_eq!(after["platform"], "claude-code");
        assert!(
            after["state_version"]
                .as_i64()
                .is_some_and(|version| version > before_version),
            "state_version did not advance: {after}"
        );
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&after),
            None
        );
    }

    #[tokio::test]
    async fn key_extend_scope_widens_allowed_service_ids_only_with_valid_ids() {
        let Some((db, actor_id, key_id, own_service, extra_service)) =
            prepare_key_database("assistant_key_extend_scope").await
        else {
            return;
        };
        let other_service_id = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "slug": "other-service" })
            .await
            .expect("load other service")
            .expect("other service")
            .id;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, rejected) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/extend-scope",
            Some(json!({
                "actionRequestId": "extend-cross-owner",
                "keyId": key_id,
                "addServiceIds": [other_service_id],
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");

        let (status, rejected_unknown) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/extend-scope",
            Some(json!({
                "actionRequestId": "extend-unknown",
                "keyId": key_id,
                "addServiceIds": [Uuid::new_v4().to_string()],
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected_unknown}");

        let (status, widened) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/extend-scope",
            Some(json!({
                "actionRequestId": "extend-valid",
                "keyId": key_id,
                "addServiceIds": [extra_service.id],
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{widened}");
        assert_eq!(widened["replayed"], false);

        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{evidence}");
        let allowed = evidence["allowed_service_ids"]
            .as_array()
            .expect("allowed_service_ids");
        let allowed: Vec<&str> = allowed.iter().filter_map(Value::as_str).collect();
        assert!(allowed.contains(&own_service.id.as_str()));
        assert!(allowed.contains(&extra_service.id.as_str()));
        assert!(!allowed.contains(&other_service_id.as_str()));
        assert_eq!(evidence["allow_all_services"], false);
        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .find_one(doc! { "_id": &key_id })
                .await
                .expect("load key")
                .expect("key")
                .allow_all_services,
            false
        );
    }

    #[tokio::test]
    async fn key_extend_scope_conflicting_content_replay_fails_closed() {
        let Some((db, actor_id, key_id, _, extra_service)) =
            prepare_key_database("assistant_key_extend_conflict").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/extend-scope",
            Some(json!({
                "actionRequestId": "extend-conflict",
                "keyId": key_id,
                "addServiceIds": [extra_service.id],
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");

        let (status, conflict) = request(
            app(state),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/extend-scope",
            Some(json!({
                "actionRequestId": "extend-conflict",
                "keyId": key_id,
                "addServiceIds": [Uuid::new_v4().to_string()],
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    }

    #[tokio::test]
    async fn key_delete_stale_state_version_rejected() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_delete_stale").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, rejected) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/delete",
            Some(json!({
                "actionRequestId": "delete-stale",
                "keyId": key_id,
                "expectedStateVersion": 99,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{rejected}");

        let stored = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &key_id })
            .await
            .expect("load key")
            .expect("key exists");
        assert!(stored.is_active);
        assert_eq!(stored.state_version, 1);
    }

    #[tokio::test]
    async fn key_delete_evidence_read_returns_404_after_delete() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_delete_evidence").await
        else {
            return;
        };
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, before) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{before}");

        let (status, deleted) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/delete",
            Some(json!({
                "actionRequestId": "delete-exact",
                "keyId": key_id,
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{deleted}");
        assert_eq!(deleted["replayed"], false);

        let (status, evidence) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{evidence}");
        assert!(
            evidence.get("allowed_service_ids").is_none(),
            "delete evidence must be absence (404), not a projected body: {evidence}"
        );

        let (status, replayed) = request(
            app(state),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/delete",
            Some(json!({
                "actionRequestId": "delete-exact",
                "keyId": key_id,
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed["replayed"], true);
    }

    #[tokio::test]
    async fn key_bind_credential_rejects_cross_owner_credential() {
        let db = connect_transaction_test_database("assistant_key_bind_cross_owner").await;
        ensure_receipt_index(&db).await;

        let actor_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&other_id, UserType::Person),
            ])
            .await
            .expect("insert users");

        let endpoint_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();
        let own_credential_id = Uuid::new_v4().to_string();
        let other_credential_id = Uuid::new_v4().to_string();

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &actor_id,
                "Own EP",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .expect("insert endpoint");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &service_id,
                &actor_id,
                "own-svc",
                &endpoint_id,
                None,
                None,
            ))
            .await
            .expect("insert service");
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(test_api_key(
                &key_id,
                &actor_id,
                "coding-agent",
                vec![service_id.clone()],
            ))
            .await
            .expect("insert api key");
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_many([
                fixture_user_api_key(&own_credential_id, &actor_id),
                fixture_user_api_key(&other_credential_id, &other_id),
            ])
            .await
            .expect("insert credentials");

        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, rejected) = request(
            app(state),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/bind-credential",
            Some(json!({
                "actionRequestId": "bind-cross-owner",
                "keyId": key_id,
                "userServiceId": service_id,
                "externalKeyId": other_credential_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{rejected}");
    }

    #[tokio::test]
    async fn binding_evidence_projection_contains_no_label_properties() {
        let db = connect_transaction_test_database("assistant_binding_evidence").await;
        ensure_receipt_index(&db).await;

        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let endpoint_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();
        let credential_id = Uuid::new_v4().to_string();

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &actor_id,
                "Bearer Bot",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .expect("insert endpoint");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &service_id,
                &actor_id,
                "own-svc",
                &endpoint_id,
                None,
                None,
            ))
            .await
            .expect("insert service");
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(test_api_key(
                &key_id,
                &actor_id,
                "coding-agent",
                vec![service_id.clone()],
            ))
            .await
            .expect("insert api key");
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_user_api_key(&credential_id, &actor_id))
            .await
            .expect("insert credential");

        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, bound) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/bind-credential",
            Some(json!({
                "actionRequestId": "bind-evidence",
                "keyId": key_id,
                "userServiceId": service_id,
                "externalKeyId": credential_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{bound}");
        let binding_id = bound["bindingId"].as_str().expect("binding id");

        let (status, by_id) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/bindings/{binding_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{by_id}");

        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/api/v1/api-keys/{key_id}/bindings/by-service/{service_id}/authorization"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{evidence}");
        assert_eq!(evidence, by_id);
        assert_eq!(evidence["id"], binding_id);
        assert_eq!(evidence["api_key_id"], key_id);
        assert_eq!(evidence["user_service_id"], service_id);
        assert_eq!(evidence["user_api_key_id"], credential_id);
        for forbidden in [
            "service_label",
            "credential_label",
            "service_slug",
            "is_invalid",
            "invalid_reason",
            "label",
        ] {
            assert!(
                evidence.get(forbidden).is_none(),
                "evidence leaked {forbidden}: {evidence}"
            );
        }
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&evidence),
            None,
            "binding evidence must be secret-free: {evidence}"
        );
    }

    #[test]
    fn key_update_fingerprint_includes_expected_state_version() {
        let version_one = fingerprint_canonical(&KeyUpdateFingerprint {
            action: KEY_UPDATE_ACTION,
            key_id: "key-1",
            name: Some("renamed"),
            platform: None,
            description: None,
            expected_state_version: Some(1),
        })
        .unwrap();
        let version_ninety_nine = fingerprint_canonical(&KeyUpdateFingerprint {
            action: KEY_UPDATE_ACTION,
            key_id: "key-1",
            name: Some("renamed"),
            platform: None,
            description: None,
            expected_state_version: Some(99),
        })
        .unwrap();
        let omitted = fingerprint_canonical(&KeyUpdateFingerprint {
            action: KEY_UPDATE_ACTION,
            key_id: "key-1",
            name: Some("renamed"),
            platform: None,
            description: None,
            expected_state_version: None,
        })
        .unwrap();
        assert_ne!(version_one, version_ninety_nine);
        assert_ne!(version_one, omitted);
    }

    #[tokio::test]
    async fn key_update_identity_reuse_with_different_expected_state_version_conflicts() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_update_version_fp").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, stale) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(json!({
                "actionRequestId": "update-version-reuse",
                "keyId": key_id,
                "name": "renamed-agent",
                "expectedStateVersion": 99,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{stale}");

        let (status, reuse) = request(
            app(state),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(json!({
                "actionRequestId": "update-version-reuse",
                "keyId": key_id,
                "name": "renamed-agent",
                "expectedStateVersion": 1,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{reuse}");
        let stored = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &key_id })
            .await
            .expect("load key")
            .expect("key");
        assert_eq!(stored.name, "coding-agent");
        assert_eq!(stored.state_version, 1);
    }

    #[tokio::test]
    async fn key_update_interrupted_retry_does_not_apply_twice() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_update_interrupted").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "update-interrupted",
            "keyId": key_id,
            "name": "renamed-agent",
            "expectedStateVersion": 1,
        });

        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["replayed"], false);

        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .update_one(
                doc! {
                    "user_id": &actor_id,
                    "action": KEY_UPDATE_ACTION,
                    "action_request_id": "update-interrupted",
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
            "/api/v1/assistant/actions/keys/update",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(retry["replayed"], true);

        let stored = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &key_id })
            .await
            .expect("load key")
            .expect("key");
        assert_eq!(stored.name, "renamed-agent");
        assert_eq!(
            stored.state_version, 2,
            "interrupted retry must not apply the update twice"
        );
        let receipt = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! {
                "user_id": &actor_id,
                "action": KEY_UPDATE_ACTION,
                "action_request_id": "update-interrupted",
            })
            .await
            .expect("load receipt")
            .expect("receipt");
        assert_eq!(receipt.status, AssistantActionReceiptStatus::Completed);
    }

    #[tokio::test]
    async fn key_update_concurrent_exact_requests_apply_once() {
        let Some((db, actor_id, key_id, _, _)) =
            prepare_key_database("assistant_key_update_concurrent").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "update-concurrent",
            "keyId": key_id,
            "name": "renamed-agent",
            "expectedStateVersion": 1,
        });

        let first = request(
            app(state.clone()),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(body.clone()),
        );
        let second = request(
            app(state),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/update",
            Some(body),
        );
        let ((status_a, body_a), (status_b, body_b)) = tokio::join!(first, second);

        for (status, body) in [(status_a, &body_a), (status_b, &body_b)] {
            assert!(
                status == StatusCode::OK || status == StatusCode::CONFLICT,
                "concurrent exact request returned {status}: {body}"
            );
        }
        let successes = [(status_a, &body_a), (status_b, &body_b)]
            .into_iter()
            .filter(|(status, _)| *status == StatusCode::OK)
            .collect::<Vec<_>>();
        assert!(
            !successes.is_empty(),
            "at least one concurrent request must commit: {body_a} / {body_b}"
        );
        let first_commits = successes
            .iter()
            .filter(|(_, body)| body["replayed"] == false)
            .count();
        assert!(
            first_commits <= 1,
            "both concurrent requests claimed a first commit: {body_a} / {body_b}"
        );

        let stored = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &key_id })
            .await
            .expect("load key")
            .expect("key");
        assert_eq!(stored.name, "renamed-agent");
        assert_eq!(
            stored.state_version, 2,
            "concurrent exact requests must advance state_version once, got {}",
            stored.state_version
        );
    }

    #[tokio::test]
    async fn key_bind_credential_rejects_inactive_credential() {
        let db = connect_transaction_test_database("assistant_key_bind_inactive").await;
        ensure_receipt_index(&db).await;

        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let endpoint_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();
        let credential_id = Uuid::new_v4().to_string();

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &actor_id,
                "Own EP",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .expect("insert endpoint");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &service_id,
                &actor_id,
                "own-svc",
                &endpoint_id,
                None,
                None,
            ))
            .await
            .expect("insert service");
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(test_api_key(
                &key_id,
                &actor_id,
                "coding-agent",
                vec![service_id.clone()],
            ))
            .await
            .expect("insert api key");
        let mut credential = fixture_user_api_key(&credential_id, &actor_id);
        credential.status = "revoked".to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(credential)
            .await
            .expect("insert credential");

        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, rejected) = request(
            app(state),
            &token,
            "POST",
            "/api/v1/assistant/actions/keys/bind-credential",
            Some(json!({
                "actionRequestId": "bind-revoked",
                "keyId": key_id,
                "userServiceId": service_id,
                "externalKeyId": credential_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        let message = rejected["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("not active"),
            "journey must receive a typed inactive-credential error: {rejected}"
        );
    }
}
