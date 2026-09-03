//! Wave-2 service-family assistant action effects (`service.update`,
//! `service.delete`, `service.route`, `service.rotate_credential`).
//!
//! Descriptors stay dormant under the current registry revision. Effects
//! reserve a secret-free receipt before mutating, replay exact retries, and
//! fail closed on identity reuse with different content. Evidence reads ride
//! `GET /keys/{id}/authorization`. `service.rotate_credential` proves rotation
//! through lineage ids and timestamps only — never credential material.

use std::sync::LazyLock;

use axum::{Json, Router, extract::State, routing::post};
use mongodb::bson::doc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_receipts::{
    ReceiptOutcome, fingerprint_canonical, fingerprint_sensitive_material, mark_completed,
    normalize_action_request_id, reserve_or_replay,
};
use crate::services::audit_service::AuditActor;
use crate::services::user_endpoint_service::{OpenApiSpecUrlUpdate, RecommendedSkillsUpdate};
use crate::services::{
    assistant_service, node_service, org_service, unified_key_service, user_api_key_service,
    user_endpoint_service, user_service_service,
};

const SERVICE_UPDATE_ACTION: &str = "service.update";
const SERVICE_DELETE_ACTION: &str = "service.delete";
const SERVICE_ROUTE_ACTION: &str = "service.route";
const SERVICE_ROTATE_ACTION: &str = "service.rotate_credential";
const MAX_LABEL_LEN: usize = 200;
const MAX_AUTH_KEY_NAME_LEN: usize = 256;
const MAX_CREDENTIAL_LEN: usize = 16_384;

static SECRET_SHAPED_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})")
        .expect("secret-shape regex")
});

/// Effect routes mounted at `/api/v1/assistant/actions/services`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/update", post(update_service))
        .route("/delete", post(delete_service))
        .route("/route", post(route_service))
        .route("/rotate-credential", post(rotate_credential))
        .route("/access-review", post(access_review))
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantServiceResource {
    pub user_service_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAssistantServiceRequest {
    pub action_request_id: String,
    pub user_service_id: String,
    pub name: Option<String>,
    pub endpoint_url: Option<String>,
    pub auth_method: Option<String>,
    pub auth_key_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAssistantServiceResponse {
    pub resource: AssistantServiceResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAssistantServiceRequest {
    pub action_request_id: String,
    pub user_service_id: String,
    pub cascade_grant: Option<bool>,
    pub grant_scope: Option<String>,
    #[serde(default)]
    pub cascade_siblings: Option<Vec<CascadeSiblingConfirmation>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAssistantServiceResponse {
    pub resource: AssistantServiceResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteAssistantServiceRequest {
    pub action_request_id: String,
    pub user_service_id: String,
    /// Omit or JSON `null` to clear node routing. A UUID routes through that node.
    pub via_node_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RouteAssistantServiceResponse {
    pub resource: AssistantServiceResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RotateAssistantServiceCredentialRequest {
    pub action_request_id: String,
    pub user_service_id: String,
    pub credential: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RotateAssistantServiceCredentialResponse {
    pub resource: AssistantServiceResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessReviewAssistantServiceRequest {
    pub action_request_id: String,
    pub user_service_id: String,
    pub service_slug: String,
    pub resource_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccessReviewAssistantServiceResponse {
    pub resource: AssistantServiceResource,
    pub replayed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceUpdateFingerprint<'a> {
    action: &'static str,
    user_service_id: &'a str,
    name: Option<&'a str>,
    endpoint_url: Option<&'a str>,
    auth_method: Option<&'a str>,
    auth_key_name: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceDeleteFingerprint<'a> {
    action: &'static str,
    user_service_id: &'a str,
    cascade_grant: bool,
    grant_scope: Option<&'a str>,
    cascade_siblings: Option<Vec<CascadeSiblingFingerprint<'a>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceRouteFingerprint<'a> {
    action: &'static str,
    user_service_id: &'a str,
    via_node_id: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceRotateFingerprint<'a> {
    action: &'static str,
    user_service_id: &'a str,
    credential_fingerprint: &'a str,
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

struct NormalizedUpdate {
    action_request_id: String,
    user_service_id: String,
    name: Option<String>,
    endpoint_url: Option<String>,
    auth_method: Option<String>,
    auth_key_name: Option<String>,
}

struct NormalizedDelete {
    action_request_id: String,
    user_service_id: String,
    cascade_grant: bool,
    grant_scope: Option<String>,
    cascade_siblings: Option<Vec<CascadeSiblingConfirmation>>,
}

struct NormalizedRoute {
    action_request_id: String,
    user_service_id: String,
    /// `None` clears routing; `Some(id)` sets it.
    via_node_id: Option<String>,
}

struct NormalizedRotate {
    action_request_id: String,
    user_service_id: String,
    credential: String,
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

fn normalize_update(body: UpdateAssistantServiceRequest) -> AppResult<NormalizedUpdate> {
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let user_service_id = parse_uuid_id(&body.user_service_id, "userServiceId")?;
    let name = optional_trimmed(body.name);
    let endpoint_url = optional_trimmed(body.endpoint_url);
    let auth_method = optional_trimmed(body.auth_method);
    let auth_key_name = optional_trimmed(body.auth_key_name);
    if name.is_none() && endpoint_url.is_none() && auth_method.is_none() && auth_key_name.is_none()
    {
        return Err(AppError::ValidationError(
            "service.update requires at least one of name, endpointUrl, authMethod, or authKeyName"
                .to_string(),
        ));
    }
    if let Some(name) = name.as_deref() {
        if name.len() > MAX_LABEL_LEN {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 200 characters".to_string(),
            ));
        }
        reject_secret_shaped("name", name)?;
    }
    if let Some(url) = endpoint_url.as_deref() {
        reject_secret_shaped("endpointUrl", url)?;
    }
    if let Some(method) = auth_method.as_deref() {
        reject_secret_shaped("authMethod", method)?;
    }
    if let Some(key_name) = auth_key_name.as_deref() {
        if key_name.len() > MAX_AUTH_KEY_NAME_LEN {
            return Err(AppError::ValidationError(
                "authKeyName must be at most 256 characters".to_string(),
            ));
        }
        reject_secret_shaped("authKeyName", key_name)?;
    }
    Ok(NormalizedUpdate {
        action_request_id,
        user_service_id,
        name,
        endpoint_url,
        auth_method,
        auth_key_name,
    })
}

fn normalize_delete(body: DeleteAssistantServiceRequest) -> AppResult<NormalizedDelete> {
    Ok(NormalizedDelete {
        action_request_id: normalize_action_request_id(body.action_request_id)?,
        user_service_id: parse_uuid_id(&body.user_service_id, "userServiceId")?,
        cascade_grant: body.cascade_grant.unwrap_or(false),
        grant_scope: optional_trimmed(body.grant_scope),
        cascade_siblings: body.cascade_siblings,
    })
}

fn normalize_route(body: RouteAssistantServiceRequest) -> AppResult<NormalizedRoute> {
    let via_node_id = match optional_trimmed(body.via_node_id) {
        Some(id) => Some(parse_uuid_id(&id, "viaNodeId")?),
        None => None,
    };
    Ok(NormalizedRoute {
        action_request_id: normalize_action_request_id(body.action_request_id)?,
        user_service_id: parse_uuid_id(&body.user_service_id, "userServiceId")?,
        via_node_id,
    })
}

fn normalize_rotate(body: RotateAssistantServiceCredentialRequest) -> AppResult<NormalizedRotate> {
    let credential = body.credential.trim().to_string();
    if credential.is_empty() {
        return Err(AppError::ValidationError(
            "credential must not be empty".to_string(),
        ));
    }
    if credential.len() > MAX_CREDENTIAL_LEN {
        return Err(AppError::ValidationError(format!(
            "credential exceeds maximum length of {MAX_CREDENTIAL_LEN} bytes"
        )));
    }
    Ok(NormalizedRotate {
        action_request_id: normalize_action_request_id(body.action_request_id)?,
        user_service_id: parse_uuid_id(&body.user_service_id, "userServiceId")?,
        credential,
    })
}

fn normalize_access_review(
    body: AccessReviewAssistantServiceRequest,
) -> AppResult<assistant_service::ServiceAccessReviewCommand> {
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    reject_secret_shaped("actionRequestId", &action_request_id)?;
    let user_service_id = parse_uuid_id(&body.user_service_id, "userServiceId")?;
    if body.service_slug.is_empty() || body.resource_uri.is_empty() {
        return Err(AppError::ValidationError(
            "serviceSlug and resourceUri must not be empty".to_string(),
        ));
    }
    reject_secret_shaped("serviceSlug", &body.service_slug)?;
    reject_secret_shaped("resourceUri", &body.resource_uri)?;
    Ok(assistant_service::ServiceAccessReviewCommand {
        action_request_id,
        user_service_id,
        service_slug: body.service_slug,
        resource_uri: body.resource_uri,
    })
}

struct ServiceWriteAccess {
    owner_id: String,
    service: UserService,
}

async fn resolve_write_owner(
    state: &AppState,
    actor: &str,
    service_id: &str,
) -> AppResult<ServiceWriteAccess> {
    let service = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User service not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &service.user_id).await?;
    if !access.can_read() || !access.allows_resource(&service.id) {
        return Err(AppError::NotFound("User service not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this service".to_string(),
        ));
    }
    Ok(ServiceWriteAccess {
        owner_id: service.user_id.clone(),
        service,
    })
}

async fn release_receipt(
    db: &mongodb::Database,
    receipt: &crate::models::assistant_action_receipt::AssistantActionReceipt,
) -> AppResult<()> {
    db.collection::<crate::models::assistant_action_receipt::AssistantActionReceipt>(
        crate::models::assistant_action_receipt::COLLECTION_NAME,
    )
    .delete_one(doc! { "_id": &receipt.id })
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
        .find_one(doc! {
            "user_id": actor,
            "action": action,
            "action_request_id": action_request_id,
        })
        .await?
        .is_some())
}

async fn service_update_already_applied(
    state: &AppState,
    owner_id: &str,
    service: &UserService,
    request: &NormalizedUpdate,
) -> AppResult<bool> {
    if request
        .auth_method
        .as_deref()
        .is_some_and(|value| value != service.auth_method)
        || request
            .auth_key_name
            .as_deref()
            .is_some_and(|value| value != service.auth_key_name)
    {
        return Ok(false);
    }
    let endpoint =
        user_endpoint_service::get_endpoint(&state.db, owner_id, &service.endpoint_id).await?;
    Ok(request
        .name
        .as_deref()
        .is_none_or(|value| value == endpoint.label)
        && request
            .endpoint_url
            .as_deref()
            .is_none_or(|value| value == endpoint.url))
}

async fn credential_matches(
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

async fn service_rotate_already_applied(
    state: &AppState,
    owner_id: &str,
    service: &UserService,
    credential: &str,
) -> AppResult<bool> {
    let Some(key_id) = service.api_key_id.as_deref() else {
        return Ok(false);
    };
    if service.rotation_predecessor_id.is_none() {
        return Ok(false);
    }
    let key = user_api_key_service::get_api_key(&state.db, owner_id, key_id).await?;
    credential_matches(state, &key, credential).await
}

async fn commit_service_update(
    state: &AppState,
    actor: &str,
    request: &NormalizedUpdate,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let access = resolve_write_owner(state, actor, &request.user_service_id).await?;
    if was_in_progress
        && service_update_already_applied(state, &access.owner_id, &access.service, request).await?
    {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    if let Some(url) = request.endpoint_url.as_deref() {
        crate::services::url_validation::validate_user_endpoint_url(
            url,
            state.config.is_production(),
            "endpointUrl",
        )
        .await?;
    }
    user_service_service::validate_service_auth_update(
        &access.service,
        request.auth_method.as_deref(),
        request.auth_key_name.as_deref(),
    )?;
    let db = state.db.clone();
    let owner_id = access.owner_id.clone();
    let endpoint_id = access.service.endpoint_id.clone();
    let service_id = request.user_service_id.clone();
    let endpoint_url = request.endpoint_url.clone();
    let name = request.name.clone();
    let auth_method = request.auth_method.clone();
    let auth_key_name = request.auth_key_name.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = apply_update_in_session(
                &db,
                &mut *session,
                &owner_id,
                &endpoint_id,
                &service_id,
                endpoint_url.as_deref(),
                name.as_deref(),
                auth_method.as_deref(),
                auth_key_name.as_deref(),
            )
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    mark_completed(&state.db, &receipt).await?;
    Ok(false)
}

async fn commit_service_delete(
    state: &AppState,
    auth_user: &AuthUser,
    actor: &str,
    request: &NormalizedDelete,
    options: unified_key_service::DisconnectOptions,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let current = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": &request.user_service_id })
        .await?;
    if current.is_none() {
        if was_in_progress {
            mark_completed(&state.db, &receipt).await?;
            return Ok(true);
        }
        release_receipt(&state.db, &receipt).await?;
        return Err(AppError::NotFound("User service not found".to_string()));
    }
    let access = resolve_write_owner(state, actor, &request.user_service_id).await?;
    if access.service.source.as_deref() == Some(crate::models::user_service::AUTO_PROVISION_SOURCE)
    {
        return Err(AppError::Forbidden(
            "Auto-connected services cannot be deleted".to_string(),
        ));
    }
    let audit_actor = AuditActor::from_auth_user(auth_user);
    let expected_siblings = request.cascade_siblings.as_ref().map(|siblings| {
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
    match unified_key_service::disconnect_credentials_with_expected_siblings(
        &state.db,
        &state.encryption_keys,
        &access.owner_id,
        &audit_actor,
        unified_key_service::DisconnectTarget::UserService(&request.user_service_id),
        options,
        expected_siblings.as_deref(),
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

async fn commit_service_route(
    state: &AppState,
    actor: &str,
    request: &NormalizedRoute,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let access = resolve_write_owner(state, actor, &request.user_service_id).await?;
    if was_in_progress && access.service.node_id == request.via_node_id {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    if let Some(node_id) = request.via_node_id.as_deref() {
        node_service::ensure_node_writable_by_actor(&state.db, actor, node_id).await?;
    }
    let mut extra = doc! {};
    match request.via_node_id.as_deref() {
        Some(node_id) => {
            extra.insert("node_id", node_id);
        }
        None => {
            extra.insert("node_id", mongodb::bson::Bson::Null);
        }
    }
    let db = state.db.clone();
    let owner_id = access.owner_id.clone();
    let service_id = request.user_service_id.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let db = db.clone();
            let owner_id = owner_id.clone();
            let service_id = service_id.clone();
            let extra = extra.clone();
            let operation = user_service_service::commit_user_service_mutation(
                &db,
                &mut *session,
                &owner_id,
                &service_id,
                extra,
            )
            .await
            .map(|_| ());
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    mark_completed(&state.db, &receipt).await?;
    Ok(false)
}

async fn commit_service_rotate(
    state: &AppState,
    actor: &str,
    request: &NormalizedRotate,
    receipt: crate::models::assistant_action_receipt::AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let access = resolve_write_owner(state, actor, &request.user_service_id).await?;
    if was_in_progress
        && service_rotate_already_applied(
            state,
            &access.owner_id,
            &access.service,
            &request.credential,
        )
        .await?
    {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    let predecessor_id = access.service.api_key_id.clone().ok_or_else(|| {
        AppError::BadRequest("This service has no stored credential to rotate".to_string())
    })?;
    let existing =
        user_api_key_service::get_api_key(&state.db, &access.owner_id, &predecessor_id).await?;
    if existing.credential_type == "node_managed" {
        return Err(AppError::BadRequest(
            "Credential is managed on the node agent. Update it on the node instead.".to_string(),
        ));
    }
    let successor = user_api_key_service::build_api_key(
        &state.encryption_keys,
        &access.owner_id,
        user_api_key_service::CreateApiKeyParams {
            label: &existing.label,
            credential_type: &existing.credential_type,
            credential: &request.credential,
            access_token: None,
            refresh_token: None,
            token_scopes: existing.token_scopes.as_deref(),
            expires_at: None,
            provider_config_id: existing.provider_config_id.as_deref(),
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: Some("assistant_rotate"),
            source_id: Some(&predecessor_id),
        },
    )
    .await?;
    let db = state.db.clone();
    let owner_id = access.owner_id.clone();
    let service_id = request.user_service_id.clone();
    let predecessor_id_for_tx = predecessor_id.clone();
    let successor_for_tx = successor.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<()> = async {
                user_api_key_service::insert_api_key_in_session(
                    &db,
                    &mut *session,
                    &successor_for_tx,
                )
                .await?;
                user_service_service::commit_user_service_mutation(
                    &db,
                    &mut *session,
                    &owner_id,
                    &service_id,
                    doc! {
                        "api_key_id": &successor_for_tx.id,
                        "rotation_predecessor_id": &predecessor_id_for_tx,
                    },
                )
                .await?;
                Ok(())
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    mark_completed(&state.db, &receipt).await?;
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
async fn apply_update_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    owner_id: &str,
    endpoint_id: &str,
    service_id: &str,
    endpoint_url: Option<&str>,
    name: Option<&str>,
    auth_method: Option<&str>,
    auth_key_name: Option<&str>,
) -> AppResult<()> {
    if endpoint_url.is_some() || name.is_some() {
        user_endpoint_service::update_endpoint_in_session(
            db,
            session,
            owner_id,
            endpoint_id,
            endpoint_url,
            name,
            OpenApiSpecUrlUpdate::Leave,
            RecommendedSkillsUpdate::Leave,
        )
        .await?;
    }
    let mut service_set = doc! {};
    if let Some(auth_method) = auth_method {
        service_set.insert("auth_method", auth_method);
    }
    if let Some(auth_key_name) = auth_key_name {
        service_set.insert("auth_key_name", auth_key_name);
    }
    user_service_service::commit_user_service_mutation(
        db,
        session,
        owner_id,
        service_id,
        service_set,
    )
    .await?;
    Ok(())
}

/// POST /api/v1/assistant/actions/services/update
pub async fn update_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateAssistantServiceRequest>,
) -> AppResult<Json<UpdateAssistantServiceResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_update(body)?;
    let fingerprint = fingerprint_canonical(&ServiceUpdateFingerprint {
        action: SERVICE_UPDATE_ACTION,
        user_service_id: &request.user_service_id,
        name: request.name.as_deref(),
        endpoint_url: request.endpoint_url.as_deref(),
        auth_method: request.auth_method.as_deref(),
        auth_key_name: request.auth_key_name.as_deref(),
    })?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_UPDATE_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.user_service_id.clone(),
    )
    .await?;
    let replayed = match outcome {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            commit_service_update(&state, &actor, &request, receipt, true).await?
        }
        ReceiptOutcome::Reserved(receipt) => {
            commit_service_update(&state, &actor, &request, receipt, false).await?
        }
    };

    Ok(Json(UpdateAssistantServiceResponse {
        resource: AssistantServiceResource {
            user_service_id: request.user_service_id,
        },
        replayed,
    }))
}

/// POST /api/v1/assistant/actions/services/delete
pub async fn delete_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<DeleteAssistantServiceRequest>,
) -> AppResult<Json<DeleteAssistantServiceResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_delete(body)?;
    if request.cascade_grant && request.cascade_siblings.is_none() {
        return Err(AppError::ValidationError(
            "cascadeSiblings is required when cascadeGrant is true".to_string(),
        ));
    }
    let options = unified_key_service::DisconnectOptions {
        cascade_grant: request.cascade_grant,
        grant_scope: request.grant_scope.clone(),
    };
    options.validate()?;
    let already_reserved = receipt_exists(
        &state.db,
        &actor,
        SERVICE_DELETE_ACTION,
        &request.action_request_id,
    )
    .await?;
    if !already_reserved {
        let access = resolve_write_owner(&state, &actor, &request.user_service_id).await?;
        if access.service.source.as_deref()
            == Some(crate::models::user_service::AUTO_PROVISION_SOURCE)
        {
            return Err(AppError::Forbidden(
                "Auto-connected services cannot be deleted".to_string(),
            ));
        }
        if !request.cascade_grant && request.grant_scope.as_deref() != Some("token") {
            unified_key_service::ensure_disconnect_confirmation(
                &state.db,
                &state.encryption_keys,
                &access.owner_id,
                unified_key_service::DisconnectTarget::UserService(&request.user_service_id),
                options.clone(),
            )
            .await?;
        }
    }
    let fingerprint = fingerprint_canonical(&ServiceDeleteFingerprint {
        action: SERVICE_DELETE_ACTION,
        user_service_id: &request.user_service_id,
        cascade_grant: request.cascade_grant,
        grant_scope: request.grant_scope.as_deref(),
        cascade_siblings: canonical_cascade_siblings(request.cascade_siblings.as_deref()),
    })?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_DELETE_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.user_service_id.clone(),
    )
    .await?;
    let replayed = match outcome {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            commit_service_delete(&state, &auth_user, &actor, &request, options, receipt, true)
                .await?
        }
        ReceiptOutcome::Reserved(receipt) => {
            commit_service_delete(
                &state, &auth_user, &actor, &request, options, receipt, false,
            )
            .await?
        }
    };

    Ok(Json(DeleteAssistantServiceResponse {
        resource: AssistantServiceResource {
            user_service_id: request.user_service_id,
        },
        replayed,
    }))
}

/// POST /api/v1/assistant/actions/services/route
pub async fn route_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<RouteAssistantServiceRequest>,
) -> AppResult<Json<RouteAssistantServiceResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_route(body)?;
    let fingerprint = fingerprint_canonical(&ServiceRouteFingerprint {
        action: SERVICE_ROUTE_ACTION,
        user_service_id: &request.user_service_id,
        via_node_id: request.via_node_id.as_deref(),
    })?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_ROUTE_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.user_service_id.clone(),
    )
    .await?;
    let replayed = match outcome {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            commit_service_route(&state, &actor, &request, receipt, true).await?
        }
        ReceiptOutcome::Reserved(receipt) => {
            commit_service_route(&state, &actor, &request, receipt, false).await?
        }
    };

    Ok(Json(RouteAssistantServiceResponse {
        resource: AssistantServiceResource {
            user_service_id: request.user_service_id,
        },
        replayed,
    }))
}

/// POST /api/v1/assistant/actions/services/rotate-credential
pub async fn rotate_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<RotateAssistantServiceCredentialRequest>,
) -> AppResult<Json<RotateAssistantServiceCredentialResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let request = normalize_rotate(body)?;
    let credential_fingerprint = fingerprint_sensitive_material(&request.credential);
    let fingerprint = fingerprint_canonical(&ServiceRotateFingerprint {
        action: SERVICE_ROTATE_ACTION,
        user_service_id: &request.user_service_id,
        credential_fingerprint: &credential_fingerprint,
    })?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_ROTATE_ACTION,
        &request.action_request_id,
        &fingerprint,
        request.user_service_id.clone(),
    )
    .await?;
    let replayed = match outcome {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            commit_service_rotate(&state, &actor, &request, receipt, true).await?
        }
        ReceiptOutcome::Reserved(receipt) => {
            commit_service_rotate(&state, &actor, &request, receipt, false).await?
        }
    };

    Ok(Json(RotateAssistantServiceCredentialResponse {
        resource: AssistantServiceResource {
            user_service_id: request.user_service_id,
        },
        replayed,
    }))
}

/// POST /api/v1/assistant/actions/services/access-review
pub async fn access_review(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AccessReviewAssistantServiceRequest>,
) -> AppResult<Json<AccessReviewAssistantServiceResponse>> {
    if auth_user.auth_method != crate::mw::auth::AuthMethod::Session
        || auth_user.session_id.is_none()
    {
        return Err(AppError::Forbidden(
            "A browser session is required for service access review".to_string(),
        ));
    }
    auth_user.ensure_write_scope()?;
    let command = normalize_access_review(body)?;
    let actor = AuditActor::from_auth_user(&auth_user);
    let outcome = assistant_service::apply_service_access_review(
        &state.db,
        &state.config,
        &actor,
        &command,
    )
    .await?;
    Ok(Json(AccessReviewAssistantServiceResponse {
        resource: AssistantServiceResource {
            user_service_id: outcome.user_service_id,
        },
        replayed: outcome.replayed,
    }))
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        routing::get,
    };
    use mongodb::{
        IndexModel,
        bson::{self, doc},
        options::IndexOptions,
    };
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::handlers::assistant_actions;
    use crate::handlers::keys;
    use crate::models::assistant_action_receipt::{
        AssistantActionReceipt, AssistantActionReceiptStatus,
        COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
    };
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOGS};
    use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
    use crate::models::provider_config::{
        COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig, RevocationConfig,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
    use crate::models::user_endpoint::UserEndpoint;
    use crate::models::ws_frame_injection::{
        WsFrameDirection, WsFrameInjection, WsFrameKind, WsFrameTrigger,
    };
    use crate::test_utils::{
        connect_test_database, connect_transaction_test_database, test_app_config, test_app_state,
        test_app_state_with_config, test_auth_user, test_encryption_keys, test_user,
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
        Router::new()
            .nest("/assistant/actions/services", super::router())
            .route(
                "/keys/{key_id}/authorization",
                get(keys::get_key_authorization),
            )
            .route(
                "/assistant/actions",
                get(assistant_actions::get_assistant_actions),
            )
            .with_state(state)
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

    async fn prepare_receipts(db: &mongodb::Database) {
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

    fn test_node(id: &str, owner_id: &str) -> Node {
        let now = chrono::Utc::now();
        Node {
            id: id.to_string(),
            user_id: owner_id.to_string(),
            name: "route-node".to_string(),
            status: NodeStatus::Online,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: Some(now),
            connected_at: Some(now),
            metadata: None,
            metrics: NodeMetrics::default(),
            connection_owner: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn fixture_api_key(id: &str, user_id: &str) -> UserApiKey {
        UserApiKey {
            credential_epoch: 1,
            credential_source: None,
            id: id.to_string(),
            user_id: user_id.to_string(),
            label: "svc-cred".to_string(),
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

    async fn seed_service(
        db: &mongodb::Database,
        actor_id: &str,
        with_credential: bool,
        node_id: Option<&str>,
    ) -> (UserService, UserEndpoint) {
        seed_service_named(db, actor_id, "example-svc", with_credential, node_id).await
    }

    async fn seed_service_named(
        db: &mongodb::Database,
        actor_id: &str,
        slug: &str,
        with_credential: bool,
        node_id: Option<&str>,
    ) -> (UserService, UserEndpoint) {
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            actor_id,
            "Example",
            "https://api.example.com",
            None,
            None,
        );
        let mut service = test_user_service(
            &Uuid::new_v4().to_string(),
            actor_id,
            slug,
            &endpoint.id,
            None,
            node_id,
        );
        if with_credential {
            let key_id = Uuid::new_v4().to_string();
            db.collection::<UserApiKey>(USER_API_KEYS)
                .insert_one(fixture_api_key(&key_id, actor_id))
                .await
                .expect("insert api key");
            service.api_key_id = Some(key_id);
            service.auth_method = "bearer".to_string();
            service.auth_key_name = "Authorization".to_string();
        }
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(&endpoint)
            .await
            .expect("insert endpoint");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert service");
        (service, endpoint)
    }

    async fn prepare_actor(prefix: &str) -> Option<(mongodb::Database, String)> {
        let db = connect_test_database(prefix).await?;
        prepare_receipts(&db).await;
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");
        Some((db, actor_id))
    }

    async fn prepare_transaction_actor(prefix: &str) -> (mongodb::Database, String) {
        let db = connect_transaction_test_database(prefix).await;
        prepare_receipts(&db).await;
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");
        (db, actor_id)
    }

    fn delegated_authority_client(id: &str) -> OauthClient {
        let now = chrono::Utc::now();
        OauthClient {
            id: id.to_string(),
            client_name: "Aevatar".to_string(),
            client_secret_hash: String::new(),
            redirect_uris: vec!["https://aevatar.example/callback".to_string()],
            allowed_scopes: "openid".to_string(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: crate::services::assistant_service::ASSISTANT_DELEGATED_SCOPE
                .to_string(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: None,
            created_at: now,
            updated_at: now,
        }
    }

    async fn seed_delegated_authority_link(db: &mongodb::Database) -> String {
        let client_id = Uuid::new_v4().to_string();
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(delegated_authority_client(&client_id))
            .await
            .unwrap();
        let mut service =
            crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = crate::services::assistant_service::AEVATAR_SLUG.to_string();
        service.delegated_authority_client_id = Some(client_id.clone());
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(service)
            .await
            .unwrap();
        client_id
    }

    fn session_auth(actor_id: &str) -> AuthUser {
        let mut auth = test_auth_user(actor_id);
        auth.session_id = Some(Uuid::new_v4());
        auth
    }

    fn access_review_body(
        action_request_id: &str,
        service: &UserService,
        resource_uri: &str,
    ) -> AccessReviewAssistantServiceRequest {
        AccessReviewAssistantServiceRequest {
            action_request_id: action_request_id.to_string(),
            user_service_id: service.id.clone(),
            service_slug: service.slug.clone(),
            resource_uri: resource_uri.to_string(),
        }
    }

    async fn prepare_access_review(
        prefix: &str,
    ) -> Option<(AppState, String, String, UserService)> {
        let (db, actor_id) = prepare_actor(prefix).await?;
        let client_id = seed_delegated_authority_link(&db).await;
        let (service, _) = seed_service_named(&db, &actor_id, "github", false, None).await;
        let mut config = test_app_config();
        config.base_url = "https://nyxid.test".to_string();
        let state = test_app_state_with_config(db, config);
        Some((state, actor_id, client_id, service))
    }

    fn grant_provider_for_test() -> ProviderConfig {
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
                url: "https://api.github.com/applications".to_string(),
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

    async fn insert_oauth_service_for_test(
        db: &mongodb::Database,
        encryption_keys: &crate::crypto::aes::EncryptionKeys,
        user_id: &str,
        provider_id: &str,
        slug: &str,
    ) -> UserService {
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            user_id,
            slug,
            "https://api.github.com",
            None,
            None,
        );
        let mut key = fixture_api_key(&Uuid::new_v4().to_string(), user_id);
        key.credential_type = "oauth2".to_string();
        key.provider_config_id = Some(provider_id.to_string());
        key.connection_id = Some(Uuid::new_v4().to_string());
        key.credential_source = Some("platform".to_string());
        key.access_token_encrypted = Some(encryption_keys.encrypt(b"access-token").await.unwrap());
        key.label = slug.to_string();
        let mut service = test_user_service(
            &Uuid::new_v4().to_string(),
            user_id,
            slug,
            &endpoint.id,
            None,
            None,
        );
        service.api_key_id = Some(key.id.clone());
        service.auth_method = "oauth2".to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(&endpoint)
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        service
    }

    async fn prepare_grant_services(
        prefix: &str,
    ) -> Option<(
        mongodb::Database,
        String,
        ProviderConfig,
        UserService,
        UserService,
    )> {
        let (db, actor_id) = prepare_actor(prefix).await?;
        let encryption_keys = test_encryption_keys();
        let provider = grant_provider_for_test();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .expect("insert provider");
        let primary = insert_oauth_service_for_test(
            &db,
            &encryption_keys,
            &actor_id,
            &provider.id,
            "svc-primary",
        )
        .await;
        let sibling = insert_oauth_service_for_test(
            &db,
            &encryption_keys,
            &actor_id,
            &provider.id,
            "svc-sibling",
        )
        .await;
        Some((db, actor_id, provider, primary, sibling))
    }

    #[test]
    fn service_delete_fingerprint_includes_cascade_semantics() {
        let first = fingerprint_canonical(&ServiceDeleteFingerprint {
            action: SERVICE_DELETE_ACTION,
            user_service_id: "service",
            cascade_grant: false,
            grant_scope: None,
            cascade_siblings: None,
        })
        .expect("fingerprint");
        let second = fingerprint_canonical(&ServiceDeleteFingerprint {
            action: SERVICE_DELETE_ACTION,
            user_service_id: "service",
            cascade_grant: true,
            grant_scope: Some("repo"),
            cascade_siblings: Some(vec![]),
        })
        .expect("fingerprint");
        assert_ne!(first, second);
    }

    #[test]
    fn service_rotate_fingerprint_includes_credential_content() {
        // Falsifier: revert the keyed helper to a bare SHA-256 digest; the
        // serialized fingerprint then equals the offline-oracle digest below.
        let first_credential = fingerprint_sensitive_material("credential-a");
        let first = fingerprint_canonical(&ServiceRotateFingerprint {
            action: SERVICE_ROTATE_ACTION,
            user_service_id: "service",
            credential_fingerprint: &first_credential,
        })
        .expect("fingerprint");
        let second_credential = fingerprint_sensitive_material("credential-b");
        let second = fingerprint_canonical(&ServiceRotateFingerprint {
            action: SERVICE_ROTATE_ACTION,
            user_service_id: "service",
            credential_fingerprint: &second_credential,
        })
        .expect("fingerprint");
        assert_ne!(first, second);
        let first_json = serde_json::to_value(ServiceRotateFingerprint {
            action: SERVICE_ROTATE_ACTION,
            user_service_id: "service",
            credential_fingerprint: &first_credential,
        })
        .expect("serialize fingerprint");
        assert_ne!(
            first_json["credentialFingerprint"],
            hex::encode(Sha256::digest(b"credential-a"))
        );
    }

    #[tokio::test]
    async fn service_access_review_merges_consent_once_and_replays_completed_receipt() {
        let Some((state, actor_id, client_id, service)) =
            prepare_access_review("svc_access_review_replay").await
        else {
            return;
        };
        let resource_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &service.slug,
        );
        let auth = session_auth(&actor_id);

        let first = access_review(
            State(state.clone()),
            auth.clone(),
            Json(access_review_body("review-1", &service, &resource_uri)),
        )
        .await
        .unwrap()
        .0;
        assert!(!first.replayed);
        assert_eq!(first.resource.user_service_id, service.id);
        let consent = state
            .db
            .collection::<Consent>(CONSENTS)
            .find_one(doc! { "user_id": &actor_id, "client_id": &client_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(consent.scopes, "openid");
        assert!(!consent.allow_all_services);
        assert_eq!(consent.allowed_service_ids, Some(vec![service.id.clone()]));

        let second = access_review(
            State(state.clone()),
            auth,
            Json(access_review_body("review-1", &service, &resource_uri)),
        )
        .await
        .unwrap()
        .0;
        assert!(second.replayed);
        let after = state
            .db
            .collection::<Consent>(CONSENTS)
            .find_one(doc! { "_id": &consent.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after, consent);
        assert_eq!(
            state
                .db
                .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {
                    "user_id": &actor_id,
                    "action": "service.access_review",
                    "action_request_id": "review-1",
                })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .db
                .collection::<AuditLog>(AUDIT_LOGS)
                .count_documents(doc! { "event_type": "assistant_service_access_grant_merged" })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .db
                .collection::<AuditLog>(AUDIT_LOGS)
                .count_documents(doc! { "event_type": "assistant_service_access_grant_replayed" })
                .await
                .unwrap(),
            1
        );

        for event_type in [
            "assistant_service_access_review_requested",
            "assistant_service_access_grant_merged",
            "assistant_service_access_grant_replayed",
        ] {
            let entry = state
                .db
                .collection::<AuditLog>(AUDIT_LOGS)
                .find_one(doc! { "event_type": event_type })
                .await
                .unwrap()
                .unwrap();
            let keys = entry
                .event_data
                .as_ref()
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(
                keys,
                std::collections::HashSet::from([
                    "actor_id",
                    "owner_id",
                    "service_id",
                    "client_id",
                    "action_request_id",
                    "outcome",
                ]),
                "{event_type} audit key allowlist"
            );
        }
    }

    #[tokio::test]
    async fn service_access_review_preserves_unrestricted_consent() {
        let Some((state, actor_id, client_id, service)) =
            prepare_access_review("svc_access_review_unrestricted").await
        else {
            return;
        };
        crate::services::consent_service::grant_consent_with_services(
            &state.db,
            &actor_id,
            &client_id,
            "openid email",
            None,
        )
        .await
        .unwrap();
        let resource_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &service.slug,
        );

        access_review(
            State(state.clone()),
            session_auth(&actor_id),
            Json(access_review_body(
                "review-unrestricted",
                &service,
                &resource_uri,
            )),
        )
        .await
        .unwrap();

        let consent = state
            .db
            .collection::<Consent>(CONSENTS)
            .find_one(doc! { "user_id": &actor_id, "client_id": &client_id })
            .await
            .unwrap()
            .unwrap();
        assert!(consent.allow_all_services);
        assert_eq!(consent.scopes, "openid email");
        assert_eq!(consent.allowed_service_ids, Some(vec![service.id]));
    }

    #[tokio::test]
    async fn service_access_review_recovers_a_pending_receipt_idempotently() {
        let Some((state, actor_id, client_id, service)) =
            prepare_access_review("svc_access_review_pending").await
        else {
            return;
        };
        let resource_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &service.slug,
        );
        let body = || access_review_body("review-pending", &service, &resource_uri);
        access_review(
            State(state.clone()),
            session_auth(&actor_id),
            Json(body()),
        )
        .await
        .unwrap();
        state
            .db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .update_one(
                doc! {
                    "user_id": &actor_id,
                    "action": "service.access_review",
                    "action_request_id": "review-pending",
                },
                doc! {
                    "$set": { "status": "pending" },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .unwrap();
        state
            .db
            .collection::<Consent>(CONSENTS)
            .update_one(
                doc! { "user_id": &actor_id, "client_id": &client_id },
                doc! { "$set": { "allowed_service_ids": [] } },
            )
            .await
            .unwrap();

        let recovered = access_review(
            State(state.clone()),
            session_auth(&actor_id),
            Json(body()),
        )
        .await
        .unwrap()
        .0;
        assert!(recovered.replayed);
        let consent = state
            .db
            .collection::<Consent>(CONSENTS)
            .find_one(doc! { "user_id": &actor_id, "client_id": &client_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(consent.allowed_service_ids, Some(vec![service.id.clone()]));
        let receipt = state
            .db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! {
                "user_id": &actor_id,
                "action": "service.access_review",
                "action_request_id": "review-pending",
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.status, AssistantActionReceiptStatus::Completed);
        assert!(receipt.completed_at.is_some());
    }

    #[tokio::test]
    async fn service_access_review_rejects_action_id_reuse_with_different_content() {
        let Some((state, actor_id, client_id, first_service)) =
            prepare_access_review("svc_access_review_conflict").await
        else {
            return;
        };
        let (second_service, _) =
            seed_service_named(&state.db, &actor_id, "linear", false, None).await;
        let first_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &first_service.slug,
        );
        let second_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &second_service.slug,
        );
        access_review(
            State(state.clone()),
            session_auth(&actor_id),
            Json(access_review_body(
                "review-conflict",
                &first_service,
                &first_uri,
            )),
        )
        .await
        .unwrap();

        let result = access_review(
            State(state.clone()),
            session_auth(&actor_id),
            Json(access_review_body(
                "review-conflict",
                &second_service,
                &second_uri,
            )),
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
        let consent = state
            .db
            .collection::<Consent>(CONSENTS)
            .find_one(doc! { "user_id": &actor_id, "client_id": &client_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            consent.allowed_service_ids,
            Some(vec![first_service.id.clone()])
        );
        assert_eq!(
            state
                .db
                .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {
                    "user_id": &actor_id,
                    "action": "service.access_review",
                    "action_request_id": "review-conflict",
                })
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn service_access_review_fails_closed_for_invalid_link_states() {
        let Some((state, actor_id, client_id, service)) =
            prepare_access_review("svc_access_review_link_states").await
        else {
            return;
        };
        let resource_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &service.slug,
        );
        let downstream = state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        let clients = state.db.collection::<OauthClient>(OAUTH_CLIENTS);

        downstream
            .update_one(
                doc! { "slug": crate::services::assistant_service::AEVATAR_SLUG },
                doc! { "$unset": { "delegated_authority_client_id": "" } },
            )
            .await
            .unwrap();
        assert!(
            access_review(
                State(state.clone()),
                session_auth(&actor_id),
                Json(access_review_body("review-no-link", &service, &resource_uri)),
            )
            .await
            .is_err()
        );
        downstream
            .update_one(
                doc! { "slug": crate::services::assistant_service::AEVATAR_SLUG },
                doc! { "$set": { "delegated_authority_client_id": &client_id } },
            )
            .await
            .unwrap();
        for (case, scopes) in [
            ("missing-proxy", "mcp:catalog:read"),
            ("missing-catalog", "proxy:*"),
        ] {
            clients
                .update_one(
                    doc! { "_id": &client_id },
                    doc! { "$set": { "delegation_scopes": scopes } },
                )
                .await
                .unwrap();
            assert!(
                access_review(
                    State(state.clone()),
                    session_auth(&actor_id),
                    Json(access_review_body(
                        &format!("review-{case}"),
                        &service,
                        &resource_uri,
                    )),
                )
                .await
                .is_err(),
                "{case}"
            );
        }
        clients
            .update_one(
                doc! { "_id": &client_id },
                doc! {
                    "$set": {
                        "delegation_scopes": crate::services::assistant_service::ASSISTANT_DELEGATED_SCOPE,
                        "is_active": false,
                    }
                },
            )
            .await
            .unwrap();
        assert!(
            access_review(
                State(state.clone()),
                session_auth(&actor_id),
                Json(access_review_body(
                    "review-inactive-client",
                    &service,
                    &resource_uri,
                )),
            )
            .await
            .is_err()
        );
        assert_eq!(
            state
                .db
                .collection::<Consent>(CONSENTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            state
                .db
                .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn service_access_review_rejects_cross_user_before_receipt_or_consent() {
        let Some((state, actor_id, _client_id, _)) =
            prepare_access_review("svc_access_review_cross_user").await
        else {
            return;
        };
        let other_user_id = Uuid::new_v4().to_string();
        state
            .db
            .collection::<User>(USERS)
            .insert_one(test_user(&other_user_id, UserType::Person))
            .await
            .unwrap();
        let (foreign, _) =
            seed_service_named(&state.db, &other_user_id, "foreign", false, None).await;
        let resource_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &foreign.slug,
        );

        let result = access_review(
            State(state.clone()),
            session_auth(&actor_id),
            Json(access_review_body("review-cross-user", &foreign, &resource_uri)),
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
        assert_eq!(
            state
                .db
                .collection::<Consent>(CONSENTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            state
                .db
                .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn service_access_review_resource_matrix_fails_before_mutation() {
        let Some((state, actor_id, _client_id, service)) =
            prepare_access_review("svc_access_review_resource_matrix").await
        else {
            return;
        };
        let canonical = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &service.slug,
        );
        let cases = [
            ("wrong-slug", "other", canonical.clone()),
            (
                "http",
                service.slug.as_str(),
                canonical.replacen("https://", "http://", 1),
            ),
            (
                "userinfo",
                service.slug.as_str(),
                canonical.replacen("https://", "https://user@", 1),
            ),
            ("query", service.slug.as_str(), format!("{canonical}?admin=1")),
            ("fragment", service.slug.as_str(), format!("{canonical}#grant")),
            (
                "encoded-path",
                service.slug.as_str(),
                canonical.replace("/proxy/s/", "/proxy%2fs/"),
            ),
        ];
        for (case, slug, resource_uri) in cases {
            let mut body = access_review_body(&format!("review-{case}"), &service, &resource_uri);
            body.service_slug = slug.to_string();
            assert!(
                access_review(
                    State(state.clone()),
                    session_auth(&actor_id),
                    Json(body),
                )
                .await
                .is_err(),
                "{case}"
            );
        }

        state
            .db
            .collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();
        assert!(
            access_review(
                State(state.clone()),
                session_auth(&actor_id),
                Json(access_review_body("review-inactive", &service, &canonical)),
            )
            .await
            .is_err()
        );
        assert_eq!(
            state
                .db
                .collection::<Consent>(CONSENTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            state
                .db
                .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let denied = state
            .db
            .collection::<AuditLog>(AUDIT_LOGS)
            .find_one(doc! { "event_type": "assistant_service_access_grant_denied" })
            .await
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&denied.event_data).unwrap();
        for forbidden in ["resource_uri", "authorization", "cookie", "token", "credential"] {
            assert!(!serialized.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[tokio::test]
    async fn service_access_review_requires_browser_session() {
        let Some((state, actor_id, _client_id, service)) =
            prepare_access_review("svc_access_review_session_only").await
        else {
            return;
        };
        let resource_uri = crate::services::oauth_resource_service::user_service_resource_uri(
            &state.config,
            &service.slug,
        );
        let mut auth = test_auth_user(&actor_id);
        auth.auth_method = crate::mw::auth::AuthMethod::AccessToken;

        let result = access_review(
            State(state.clone()),
            auth,
            Json(access_review_body("review-access-token", &service, &resource_uri)),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
        assert_eq!(
            state
                .db
                .collection::<Consent>(CONSENTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn service_update_effect_normalizes_and_receipts() {
        let Some((db, actor_id)) = prepare_actor("svc_update_normalizes").await else {
            return;
        };
        let (service, _) = seed_service(&db, &actor_id, false, None).await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "update-normalize",
            "userServiceId": service.id,
            "name": "  Trimmed Label  ",
            "endpointUrl": "https://api.example.com/v2",
        });

        let (status, created) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/update",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["replayed"], false);
        assert_eq!(created["resource"]["userServiceId"], service.id);

        let stored = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
            .expect("load service")
            .expect("service exists");
        assert_eq!(stored.state_version, 2);
        let endpoint = db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(doc! { "_id": &service.endpoint_id })
            .await
            .expect("load endpoint")
            .expect("endpoint exists");
        assert_eq!(endpoint.label, "Trimmed Label");
        assert_eq!(endpoint.url, "https://api.example.com/v2");

        let (status, replayed) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/services/update",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed["replayed"], true);
        assert_eq!(replayed["resource"]["userServiceId"], service.id);
        assert!(replayed.get("credential").is_none());
        assert!(replayed.get("fullKey").is_none());
    }

    #[tokio::test]
    async fn service_update_interrupted_retry_resumes_and_completes() {
        // Falsifier: restore the unconditional InProgress 409 (or remove the
        // already-applied check) and this retry either fails or increments twice.
        let Some((db, actor_id)) = prepare_actor("svc_update_interrupted").await else {
            return;
        };
        let (service, _) = seed_service(&db, &actor_id, false, None).await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "update-interrupted",
            "userServiceId": service.id,
            "name": "Interrupted Rename",
        });
        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/update",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["replayed"], false);
        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .update_one(
                doc! {
                    "user_id": &actor_id,
                    "action": SERVICE_UPDATE_ACTION,
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
            "/assistant/actions/services/update",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(retry["replayed"], true);
        let stored = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
            .expect("load service")
            .expect("service");
        assert_eq!(stored.state_version, 2, "retry must not apply twice");
        let endpoint = db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(doc! { "_id": &service.endpoint_id })
            .await
            .expect("load endpoint")
            .expect("endpoint");
        assert_eq!(endpoint.label, "Interrupted Rename");
        let receipt = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! {
                "user_id": &actor_id,
                "action": SERVICE_UPDATE_ACTION,
                "action_request_id": "update-interrupted",
            })
            .await
            .expect("load receipt")
            .expect("receipt");
        assert_eq!(receipt.status, AssistantActionReceiptStatus::Completed);
    }

    #[tokio::test]
    async fn service_route_sets_and_clears_node_id_atomically() {
        let Some((db, actor_id)) = prepare_actor("svc_route_atomic").await else {
            return;
        };
        let node_id = Uuid::new_v4().to_string();
        db.collection::<Node>(NODES)
            .insert_one(test_node(&node_id, &actor_id))
            .await
            .expect("insert node");
        let (service, _) = seed_service(&db, &actor_id, false, None).await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, set) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/route",
            Some(json!({
                "actionRequestId": "route-set",
                "userServiceId": service.id,
                "viaNodeId": node_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{set}");
        let stored = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
            .expect("load")
            .expect("service");
        assert_eq!(stored.node_id.as_deref(), Some(node_id.as_str()));
        let version_after_set = stored.state_version;

        let (status, cleared) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/services/route",
            Some(json!({
                "actionRequestId": "route-clear",
                "userServiceId": service.id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cleared}");
        let stored = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
            .expect("load")
            .expect("service");
        assert!(stored.node_id.is_none());
        assert!(stored.state_version > version_after_set);
    }

    #[tokio::test]
    async fn service_rotate_credential_never_returns_credential_material() {
        let Some((db, actor_id)) = prepare_actor("svc_rotate_secret_free").await else {
            return;
        };
        let (service, _) = seed_service(&db, &actor_id, true, None).await;
        let predecessor = service.api_key_id.clone().expect("credential");
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "rotate-once",
            "userServiceId": service.id,
            "credential": "sk-live-replacement-credential",
        });

        let (status, rotated) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/rotate-credential",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{rotated}");
        assert_eq!(rotated["replayed"], false);
        let serialized = rotated.to_string();
        assert!(!serialized.contains("sk-live-replacement-credential"));
        for forbidden in ["credential", "fullKey", "full_key", "secret", "token"] {
            assert!(
                rotated.get(forbidden).is_none(),
                "leaked field: {forbidden}"
            );
        }

        let stored = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
            .expect("load")
            .expect("service");
        assert_ne!(stored.api_key_id.as_deref(), Some(predecessor.as_str()));
        assert_eq!(
            stored.rotation_predecessor_id.as_deref(),
            Some(predecessor.as_str())
        );
        assert_eq!(stored.state_version, 2);

        let (status, replayed) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/services/rotate-credential",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed["replayed"], true);
        assert!(
            !replayed
                .to_string()
                .contains("sk-live-replacement-credential")
        );
    }

    #[tokio::test]
    async fn service_evidence_projection_gains_no_free_text_fields() {
        let Some((db, actor_id)) = prepare_actor("svc_evidence_no_freetext").await else {
            return;
        };
        let (service, _) = seed_service(&db, &actor_id, true, None).await;
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$set": {
                    "ws_frame_injections": bson::to_bson(&vec![WsFrameInjection {
                        trigger: WsFrameTrigger::FirstFrameFromDownstream,
                        template: r#"{"headers":{"Authorization":"Bearer ${credential}"}}"#.to_string(),
                        frame_kind: WsFrameKind::Text,
                        consume_trigger: true,
                        direction: WsFrameDirection::Downstream,
                    }]).unwrap(),
                } },
            )
            .await
            .expect("poison ws template");
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/keys/{}/authorization", service.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{evidence}");
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&evidence),
            None,
            "{evidence}"
        );
        for forbidden in [
            "label",
            "name",
            "ws_frame_injections",
            "default_request_headers",
        ] {
            assert!(
                evidence.get(forbidden).is_none(),
                "free-text carrier leaked: {forbidden}"
            );
        }
    }

    #[tokio::test]
    async fn service_delete_cascade_grant_retry_completes() {
        // Falsifier: reserving before the 11500 preflight, or omitting the sibling
        // set from the retry, makes the second request fail with a fingerprint
        // conflict instead of completing the cascade.
        let Some((db, actor_id)) = prepare_actor("svc_delete_cascade").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let provider = ProviderConfig {
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
                url: "https://api.github.com/applications".to_string(),
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
        };
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .expect("insert provider");

        async fn insert_oauth_svc(
            db: &mongodb::Database,
            encryption_keys: &crate::crypto::aes::EncryptionKeys,
            user_id: &str,
            provider_id: &str,
            slug: &str,
        ) -> UserService {
            let endpoint = test_user_endpoint(
                &Uuid::new_v4().to_string(),
                user_id,
                slug,
                "https://api.github.com",
                None,
                None,
            );
            let mut key = fixture_api_key(&Uuid::new_v4().to_string(), user_id);
            key.credential_type = "oauth2".to_string();
            key.provider_config_id = Some(provider_id.to_string());
            key.connection_id = Some(Uuid::new_v4().to_string());
            key.credential_source = Some("platform".to_string());
            key.access_token_encrypted =
                Some(encryption_keys.encrypt(b"access-token").await.unwrap());
            key.label = slug.to_string();
            let mut service = test_user_service(
                &Uuid::new_v4().to_string(),
                user_id,
                slug,
                &endpoint.id,
                None,
                None,
            );
            service.api_key_id = Some(key.id.clone());
            service.auth_method = "oauth2".to_string();
            db.collection::<UserEndpoint>(USER_ENDPOINTS)
                .insert_one(&endpoint)
                .await
                .unwrap();
            db.collection::<UserApiKey>(USER_API_KEYS)
                .insert_one(&key)
                .await
                .unwrap();
            db.collection::<UserService>(USER_SERVICES)
                .insert_one(&service)
                .await
                .unwrap();
            service
        }

        let primary = insert_oauth_svc(
            &db,
            &encryption_keys,
            &actor_id,
            &provider.id,
            "svc-primary",
        )
        .await;
        let _sibling = insert_oauth_svc(
            &db,
            &encryption_keys,
            &actor_id,
            &provider.id,
            "svc-sibling",
        )
        .await;

        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let (status, body) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/delete",
            Some(json!({
                "actionRequestId": "delete-cascade",
                "userServiceId": primary.id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error_code"], 11500);
        assert_eq!(body["error"], "grant_cascade_confirmation_required");
        assert!(body.get("details").is_some());
        assert_eq!(
            db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {
                    "user_id": &actor_id,
                    "action": SERVICE_DELETE_ACTION,
                    "action_request_id": "delete-cascade",
                })
                .await
                .expect("count preflight receipts"),
            0,
            "11500 preflight must not reserve an action receipt"
        );
        let (status, completed) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/delete",
            Some(json!({
                "actionRequestId": "delete-cascade",
                "userServiceId": primary.id,
                "cascadeGrant": true,
                "cascadeSiblings": body["details"]["siblings"],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{completed}");
        assert_eq!(completed["replayed"], false);
        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/keys/{}/authorization", primary.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{evidence}");
    }

    #[tokio::test]
    async fn service_delete_rejects_changed_sibling_set() {
        // Falsifier: deleting a newly-created dependent instead of rejecting the
        // stale confirmation makes this mutation test return 200.
        let Some((db, actor_id, provider, primary, _sibling)) =
            prepare_grant_services("svc_delete_changed_siblings").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/delete",
            Some(json!({
                "actionRequestId": "delete-changed-siblings",
                "userServiceId": primary.id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{first}");
        let expected = first["details"]["siblings"].clone();
        let _new_sibling = insert_oauth_service_for_test(
            &db,
            &test_encryption_keys(),
            &actor_id,
            &provider.id,
            "svc-new-sibling",
        )
        .await;
        let (status, changed) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/services/delete",
            Some(json!({
                "actionRequestId": "delete-changed-siblings",
                "userServiceId": primary.id,
                "cascadeGrant": true,
                "cascadeSiblings": expected,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{changed}");
        assert_eq!(changed["error_code"], 1004);
        assert!(
            db.collection::<UserService>(USER_SERVICES)
                .find_one(doc! { "_id": &primary.id })
                .await
                .expect("load primary")
                .is_some(),
            "stale confirmation must leave the primary service intact"
        );
    }

    #[tokio::test]
    async fn service_route_evidence_shows_node_id_after_route() {
        let Some((db, actor_id)) = prepare_actor("svc_route_evidence").await else {
            return;
        };
        let node_id = Uuid::new_v4().to_string();
        db.collection::<Node>(NODES)
            .insert_one(test_node(&node_id, &actor_id))
            .await
            .expect("insert node");
        let (service, _) = seed_service(&db, &actor_id, false, None).await;
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, routed) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/route",
            Some(json!({
                "actionRequestId": "route-evidence",
                "userServiceId": service.id,
                "viaNodeId": node_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{routed}");

        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/keys/{}/authorization", service.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{evidence}");
        assert_eq!(evidence["node_id"], node_id);
        assert!(
            evidence["state_version"]
                .as_i64()
                .is_some_and(|value| value >= 1)
        );
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&evidence),
            None
        );
    }

    #[tokio::test]
    async fn service_update_rejects_ws_template_in_evidence_path() {
        let Some((db, actor_id)) = prepare_actor("svc_update_ws_template").await else {
            return;
        };
        let (service, _) = seed_service(&db, &actor_id, false, None).await;
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$set": {
                    "ws_frame_injections": bson::to_bson(&vec![WsFrameInjection {
                        trigger: WsFrameTrigger::FirstFrameFromDownstream,
                        template: r#"{"headers":{"Authorization":"Bearer ${credential}"}}"#.to_string(),
                        frame_kind: WsFrameKind::Text,
                        consume_trigger: true,
                        direction: WsFrameDirection::Downstream,
                    }]).unwrap(),
                } },
            )
            .await
            .expect("set ws template");
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let (status, updated) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/update",
            Some(json!({
                "actionRequestId": "update-ws",
                "userServiceId": service.id,
                "name": "Still Poisoned",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{updated}");

        let (status, evidence) = request(
            app(state),
            &token,
            "GET",
            &format!("/keys/{}/authorization", service.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{evidence}");
        assert!(evidence.get("ws_frame_injections").is_none());
        assert!(evidence.get("label").is_none());
        assert!(evidence.get("name").is_none());
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&evidence),
            None,
            "{evidence}"
        );
    }

    #[tokio::test]
    async fn service_update_validation_does_not_mutate_endpoint_before_rejection() {
        let (db, actor_id) = prepare_transaction_actor("svc_update_atomic_validation").await;
        let (service, endpoint_before) = seed_service(&db, &actor_id, false, None).await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let (status, body) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/services/update",
            Some(json!({
                "actionRequestId": "update-invalid-auth",
                "userServiceId": service.id,
                "name": "Changed before rejection",
                "authMethod": "bearer",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        let endpoint = db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(doc! { "_id": &endpoint_before.id })
            .await
            .expect("load endpoint")
            .expect("endpoint exists");
        assert_eq!(endpoint.label, endpoint_before.label);
        let stored = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
            .expect("load service")
            .expect("service exists");
        assert_eq!(stored.state_version, 1);
    }

    #[tokio::test]
    async fn service_rotate_interrupted_retry_resumes_and_completes() {
        // Falsifier: restore the unconditional InProgress 409 or skip the
        // credential-match check; the retry fails or creates a third key.
        let (db, actor_id) = prepare_transaction_actor("svc_rotate_interrupted").await;
        let (service, _) = seed_service(&db, &actor_id, true, None).await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "rotate-interrupted",
            "userServiceId": service.id,
            "credential": "replacement-exactly-once",
        });
        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/rotate-credential",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["replayed"], false);
        assert_eq!(
            db.collection::<UserApiKey>(USER_API_KEYS)
                .count_documents(doc! { "user_id": &actor_id })
                .await
                .expect("count keys after rotate"),
            2
        );
        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .update_one(
                doc! {
                    "user_id": &actor_id,
                    "action": SERVICE_ROTATE_ACTION,
                    "action_request_id": "rotate-interrupted",
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
            "/assistant/actions/services/rotate-credential",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(retry["replayed"], true);
        assert_eq!(
            db.collection::<UserApiKey>(USER_API_KEYS)
                .count_documents(doc! { "user_id": &actor_id })
                .await
                .expect("count keys after retry"),
            2,
            "interrupted retry must not create another successor"
        );
        let stored = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
            .expect("load service")
            .expect("service");
        assert_eq!(stored.state_version, 2);
        let receipt = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! {
                "user_id": &actor_id,
                "action": SERVICE_ROTATE_ACTION,
                "action_request_id": "rotate-interrupted",
            })
            .await
            .expect("load receipt")
            .expect("receipt");
        assert_eq!(receipt.status, AssistantActionReceiptStatus::Completed);
    }

    #[tokio::test]
    async fn service_rotate_stale_identity_replay_rejected() {
        let Some((db, actor_id)) = prepare_actor("svc_rotate_stale_id").await else {
            return;
        };
        let (first, _) = seed_service_named(&db, &actor_id, "svc-one", true, None).await;
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);

        let (status, first_rotate) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/services/rotate-credential",
            Some(json!({
                "actionRequestId": "shared-identity",
                "userServiceId": first.id,
                "credential": "replacement-one",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first_rotate}");

        let (status, conflict) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/services/rotate-credential",
            Some(json!({
                "actionRequestId": "shared-identity",
                "userServiceId": first.id,
                "credential": "replacement-two",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    }

    #[tokio::test]
    async fn wave2_service_descriptors_dormant() {
        let Some((db, actor_id)) = prepare_actor("svc_descriptors_dormant").await else {
            return;
        };
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let (status, body) = request(app(state), &token, "GET", "/assistant/actions", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["revision"], "nyxid-assistant-actions.v8");
        let names: Vec<&str> = body["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .filter_map(|action| action["action"].as_str())
            .collect();
        for verb in [
            "service.update",
            "service.delete",
            "service.route",
            "service.rotate_credential",
        ] {
            assert!(names.contains(&verb), "missing dormant descriptor {verb}");
        }
        assert_eq!(
            crate::handlers::assistant_actions::ASSISTANT_ACTIONS_REVISION,
            "nyxid-assistant-actions.v8"
        );
    }
}
