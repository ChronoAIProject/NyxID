//! Wave-3 node and device assistant action effects.
//!
//! One-time registration, rotation, and provisioning material is returned only
//! to the browser that commits the effect. Durable receipts and evidence carry
//! safe resource identities and mutation authority state only.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::{devices, node_admin};
use crate::models::assistant_action_receipt::{
    AssistantActionReceipt, COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
};
use crate::models::node_pending_credential::InjectionMethod;
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_receipts::{
    self, ReceiptOutcome, fingerprint_canonical, in_progress_conflict, normalize_action_request_id,
};
use crate::services::device_code_service::{DeviceOnboardInput, onboard_with_id};
use crate::services::{audit_service, node_pending_credential_service, node_service, org_service};

const NODE_REGISTER_TOKEN_ACTION: &str = "node.register_token";
const NODE_ROTATE_TOKEN_ACTION: &str = "node.rotate_token";
const NODE_DELETE_ACTION: &str = "node.delete";
const NODE_TRANSFER_ACTION: &str = "node.transfer";
const NODE_INJECT_CREDENTIAL_ACTION: &str = "node.inject_credential";
const PENDING_CREDENTIAL_PUSH_ACTION: &str = "pending_credential.push";
const PENDING_CREDENTIAL_CANCEL_ACTION: &str = "pending_credential.cancel";
const DEVICE_ONBOARD_ACTION: &str = "device.onboard";

/// Effect routes mounted at `/api/v1/assistant/actions/nodes`.
///
/// The evidence aliases below are production-reachable immediately. Canonical
/// sibling mounts are intentionally owned by `routes.rs` and must be added by
/// its PM owner before the upstream reader is enabled.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register-token", post(register_node_token))
        .route("/rotate-token", post(rotate_node_token))
        .route("/delete", post(delete_node))
        .route("/transfer", post(transfer_node))
        .route("/inject-credential", post(inject_node_credential))
        .route("/pending-credential-push", post(push_pending_credential))
        .route(
            "/pending-credential-cancel",
            post(cancel_pending_credential),
        )
        .route("/device-onboard", post(onboard_device))
        .route(
            "/{node_id}/authorization",
            get(node_admin::get_node_authorization),
        )
        .route(
            "/{node_id}/pending/{pending_id}/authorization",
            get(node_admin::get_pending_credential_authorization),
        )
        .route(
            "/devices/{device_id}/authorization",
            get(devices::get_onboard_device_authorization),
        )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterNodeTokenRequest {
    pub action_request_id: String,
    pub name: String,
    pub target_org_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeIdentityRequest {
    pub action_request_id: String,
    pub node_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferNodeRequest {
    pub action_request_id: String,
    pub node_id: String,
    pub new_owner_user_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushCredentialRequest {
    pub action_request_id: String,
    pub node_id: String,
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelPendingCredentialRequest {
    pub action_request_id: String,
    pub node_id: String,
    pub pending_credential_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardDeviceRequest {
    pub action_request_id: String,
    pub label: String,
    pub target_org_id: Option<String>,
    pub default_service_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantNodeResource {
    pub node_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantPendingCredentialResource {
    pub pending_credential_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDeviceResource {
    pub device_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantNodeEffectResponse {
    pub resource: AssistantNodeResource,
    pub replayed: bool,
    pub requested_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantPendingCredentialEffectResponse {
    pub resource: AssistantPendingCredentialResource,
    pub replayed: bool,
    pub requested_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDeviceEffectResponse {
    pub resource: AssistantDeviceResource,
    pub replayed: bool,
    pub requested_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterNodeTokenFingerprint<'a> {
    action: &'static str,
    name: &'a str,
    target_owner_user_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeIdentityFingerprint<'a> {
    action: &'static str,
    node_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferNodeFingerprint<'a> {
    action: &'static str,
    node_id: &'a str,
    new_owner_user_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PushCredentialFingerprint<'a> {
    action: &'static str,
    node_id: &'a str,
    service_slug: &'a str,
    injection_method: &'a InjectionMethod,
    field_name: &'a str,
    target_url: Option<&'a str>,
    label: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelPendingCredentialFingerprint<'a> {
    action: &'static str,
    node_id: &'a str,
    pending_credential_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OnboardDeviceFingerprint<'a> {
    action: &'static str,
    label: &'a str,
    target_owner_user_id: &'a str,
    default_service_ids: &'a [String],
}

fn normalize_required(value: String, field: &str, max_len: usize) -> AppResult<String> {
    let value = value.trim().to_string();
    if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        return Err(AppError::ValidationError(format!(
            "{field} must be between 1 and {max_len} characters"
        )));
    }
    Ok(value)
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

fn normalize_owner_id(value: Option<String>, actor_user_id: &str) -> AppResult<String> {
    match normalize_optional(value) {
        Some(value) => normalize_action_request_id(value),
        None => Ok(actor_user_id.to_string()),
    }
}

fn normalize_default_service_ids(values: Option<Vec<String>>) -> AppResult<Vec<String>> {
    let mut values = values.unwrap_or_default();
    if values.len() > 64 {
        return Err(AppError::ValidationError(
            "defaultServiceIds must contain 64 entries or fewer".to_string(),
        ));
    }
    for value in &mut values {
        *value = normalize_action_request_id(std::mem::take(value))?;
    }
    values.sort();
    values.dedup();
    Ok(values)
}

async fn ensure_owner_writable(
    state: &AppState,
    actor_user_id: &str,
    owner_user_id: &str,
) -> AppResult<()> {
    let access = org_service::resolve_owner_access(&state.db, actor_user_id, owner_user_id).await?;
    if !access.can_write() {
        return Err(AppError::Forbidden(
            "You must be the owner or an org admin".to_string(),
        ));
    }
    Ok(())
}

async fn release_pending_receipt(state: &AppState, receipt: &AssistantActionReceipt) {
    if let Err(error) = state
        .db
        .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .delete_one(doc! {
            "_id": &receipt.id,
            "status": "pending",
        })
        .await
    {
        tracing::error!(
            receipt_id = %receipt.id,
            error = %error,
            "failed to release uncommitted assistant action receipt"
        );
    }
}

fn node_response(receipt: &AssistantActionReceipt, replayed: bool) -> AssistantNodeEffectResponse {
    AssistantNodeEffectResponse {
        resource: AssistantNodeResource {
            node_id: receipt.resource_id.clone(),
        },
        replayed,
        requested_at: receipt.created_at.to_rfc3339(),
        registration_token: None,
        auth_token: None,
        signing_secret: None,
        expires_at: None,
    }
}

fn pending_response(
    receipt: &AssistantActionReceipt,
    replayed: bool,
) -> AssistantPendingCredentialEffectResponse {
    AssistantPendingCredentialEffectResponse {
        resource: AssistantPendingCredentialResource {
            pending_credential_id: receipt.resource_id.clone(),
        },
        replayed,
        requested_at: receipt.created_at.to_rfc3339(),
    }
}

fn device_response(
    receipt: &AssistantActionReceipt,
    replayed: bool,
) -> AssistantDeviceEffectResponse {
    AssistantDeviceEffectResponse {
        resource: AssistantDeviceResource {
            device_id: receipt.resource_id.clone(),
        },
        replayed,
        requested_at: receipt.created_at.to_rfc3339(),
        qr_payload: None,
        expires_at: None,
    }
}

async fn replay_in_progress_registration(
    state: &AppState,
    actor_user_id: &str,
    receipt: AssistantActionReceipt,
) -> AppResult<Json<AssistantNodeEffectResponse>> {
    node_service::get_node_authorization_state(&state.db, actor_user_id, &receipt.resource_id)
        .await
        .map_err(|_| in_progress_conflict())?;
    assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
    Ok(Json(node_response(&receipt, true)))
}

async fn replay_in_progress_pending(
    state: &AppState,
    actor_user_id: &str,
    node_id: &str,
    receipt: AssistantActionReceipt,
) -> AppResult<Json<AssistantPendingCredentialEffectResponse>> {
    node_pending_credential_service::get_pending_credential_authorization_state(
        &state.db,
        actor_user_id,
        node_id,
        &receipt.resource_id,
    )
    .await
    .map_err(|_| in_progress_conflict())?;
    assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
    Ok(Json(pending_response(&receipt, true)))
}

async fn replay_in_progress_device(
    state: &AppState,
    actor_user_id: &str,
    receipt: AssistantActionReceipt,
) -> AppResult<Json<AssistantDeviceEffectResponse>> {
    crate::services::device_code_service::get_onboard_authorization_state(
        &state.db,
        actor_user_id,
        &receipt.resource_id,
    )
    .await
    .map_err(|_| in_progress_conflict())?;
    assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
    Ok(Json(device_response(&receipt, true)))
}

pub async fn register_node_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<RegisterNodeTokenRequest>,
) -> AppResult<Json<AssistantNodeEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let name = normalize_required(body.name, "name", 64)?;
    let owner_user_id = normalize_owner_id(body.target_org_id, &actor)?;
    ensure_owner_writable(&state, &actor, &owner_user_id).await?;
    let fingerprint = fingerprint_canonical(&RegisterNodeTokenFingerprint {
        action: NODE_REGISTER_TOKEN_ACTION,
        name: &name,
        target_owner_user_id: &owner_user_id,
    })?;
    let outcome = assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        NODE_REGISTER_TOKEN_ACTION,
        &action_request_id,
        &fingerprint,
        Uuid::new_v4().to_string(),
    )
    .await?;
    match outcome {
        ReceiptOutcome::Replay(receipt) => Ok(Json(node_response(&receipt, true))),
        ReceiptOutcome::InProgress(receipt) => {
            replay_in_progress_registration(&state, &actor, receipt).await
        }
        ReceiptOutcome::Reserved(receipt) => {
            let created = node_service::create_registration_token_with_id(
                &state.db,
                &owner_user_id,
                &name,
                state.config.node_max_per_user,
                state.config.node_registration_token_ttl_secs,
                &receipt.resource_id,
            )
            .await;
            let (_, raw_token, expires_at) = match created {
                Ok(created) => created,
                Err(error) => {
                    if !matches!(error, AppError::DatabaseError(_)) {
                        release_pending_receipt(&state, &receipt).await;
                    }
                    return Err(error);
                }
            };
            assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
            audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "assistant_node_registration_token_created",
                Some(serde_json::json!({
                    "node_id": &receipt.resource_id,
                    "owner_user_id": &owner_user_id,
                })),
            );
            let mut response = node_response(&receipt, false);
            response.registration_token = Some(raw_token);
            response.expires_at = Some(expires_at.to_rfc3339());
            Ok(Json(response))
        }
    }
}

pub async fn rotate_node_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<NodeIdentityRequest>,
) -> AppResult<Json<AssistantNodeEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let node_id = normalize_action_request_id(body.node_id)?;
    node_service::ensure_node_writable_by_actor(&state.db, &actor, &node_id).await?;
    let fingerprint = fingerprint_canonical(&NodeIdentityFingerprint {
        action: NODE_ROTATE_TOKEN_ACTION,
        node_id: &node_id,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        NODE_ROTATE_TOKEN_ACTION,
        &action_request_id,
        &fingerprint,
        node_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(node_response(&receipt, true))),
        ReceiptOutcome::InProgress(_) => Err(in_progress_conflict()),
        ReceiptOutcome::Reserved(receipt) => {
            let (raw_token, signing_secret) = node_service::rotate_auth_token(
                &state.db,
                &state.encryption_keys,
                &actor,
                &node_id,
            )
            .await?;
            assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
            if state.node_ws_manager.is_connected(&node_id) {
                state
                    .node_ws_manager
                    .disconnect_connection(&node_id, 4002, "node credentials rotated")
                    .await;
                if let Err(error) = node_service::set_node_status(
                    &state.db,
                    &node_id,
                    crate::models::node::NodeStatus::Offline,
                )
                .await
                {
                    tracing::warn!(node_id = %node_id, error = %error, "failed to persist rotated node disconnect status");
                }
            }
            audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "assistant_node_token_rotated",
                Some(serde_json::json!({ "node_id": &node_id })),
            );
            let mut response = node_response(&receipt, false);
            response.auth_token = Some(raw_token);
            response.signing_secret = Some(signing_secret);
            Ok(Json(response))
        }
    }
}

pub async fn delete_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<NodeIdentityRequest>,
) -> AppResult<Json<AssistantNodeEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let node_id = normalize_action_request_id(body.node_id)?;
    // Ownership is checked AFTER the reservation, not before. `ensure_node_
    // writable_by_actor` filters `is_active: true`, so running it first made a
    // retry of an already-committed delete return NotFound before the completed
    // receipt was ever consulted -- the Replay arm below was unreachable and a
    // succeeded delete reported "not found". Reserving first keeps replay
    // working; a caller with no claim to the node has its pending receipt
    // released immediately, so nothing is leaked or left behind.
    let fingerprint = fingerprint_canonical(&NodeIdentityFingerprint {
        action: NODE_DELETE_ACTION,
        node_id: &node_id,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        NODE_DELETE_ACTION,
        &action_request_id,
        &fingerprint,
        node_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(node_response(&receipt, true))),
        ReceiptOutcome::InProgress(_) => Err(in_progress_conflict()),
        ReceiptOutcome::Reserved(receipt) => {
            if let Err(error) =
                node_service::ensure_node_writable_by_actor(&state.db, &actor, &node_id).await
            {
                release_pending_receipt(&state, &receipt).await;
                return Err(error);
            }
            if let Err(error) = node_service::delete_node(&state.db, &actor, &node_id).await {
                if !matches!(error, AppError::DatabaseError(_)) {
                    release_pending_receipt(&state, &receipt).await;
                }
                return Err(error);
            }
            assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
            if state.node_ws_manager.is_connected(&node_id) {
                state
                    .node_ws_manager
                    .disconnect_connection(&node_id, 4006, "node deleted")
                    .await;
            }
            audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "assistant_node_deleted",
                Some(serde_json::json!({ "node_id": &node_id })),
            );
            Ok(Json(node_response(&receipt, false)))
        }
    }
}

pub async fn transfer_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<TransferNodeRequest>,
) -> AppResult<Json<AssistantNodeEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let node_id = normalize_action_request_id(body.node_id)?;
    let new_owner_user_id = normalize_action_request_id(body.new_owner_user_id)?;
    // Same ordering rule as delete: the pre-checks move into the Reserved arm
    // so a retry of a committed transfer replays instead of failing with
    // "node already belongs to that owner" -- which is the success condition,
    // reported as an error.
    let fingerprint = fingerprint_canonical(&TransferNodeFingerprint {
        action: NODE_TRANSFER_ACTION,
        node_id: &node_id,
        new_owner_user_id: &new_owner_user_id,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        NODE_TRANSFER_ACTION,
        &action_request_id,
        &fingerprint,
        node_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(node_response(&receipt, true))),
        ReceiptOutcome::InProgress(_) => Err(in_progress_conflict()),
        ReceiptOutcome::Reserved(receipt) => {
            // Both sides of the transfer are authorised here, inside the
            // reservation, so a replayed retry never re-runs them.
            match node_service::ensure_node_writable_by_actor(&state.db, &actor, &node_id).await {
                Ok(current) if current.user_id == new_owner_user_id => {
                    release_pending_receipt(&state, &receipt).await;
                    return Err(AppError::BadRequest(
                        "node already belongs to that owner".to_string(),
                    ));
                }
                Ok(_) => {}
                Err(error) => {
                    release_pending_receipt(&state, &receipt).await;
                    return Err(error);
                }
            }
            if let Err(error) = ensure_owner_writable(&state, &actor, &new_owner_user_id).await {
                release_pending_receipt(&state, &receipt).await;
                return Err(error);
            }
            let transfer = node_service::transfer_node_owner(
                &state.db,
                &actor,
                &node_id,
                &new_owner_user_id,
                state.config.node_max_per_user,
            )
            .await;
            if let Err(error) = transfer {
                if !matches!(error, AppError::DatabaseError(_)) {
                    release_pending_receipt(&state, &receipt).await;
                }
                return Err(error);
            }
            assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
            audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "assistant_node_transferred",
                Some(serde_json::json!({
                    "node_id": &node_id,
                    "new_owner_user_id": &new_owner_user_id,
                })),
            );
            Ok(Json(node_response(&receipt, false)))
        }
    }
}

async fn create_pending_credential_effect(
    state: AppState,
    auth_user: AuthUser,
    body: PushCredentialRequest,
    action: &'static str,
    require_online: bool,
) -> AppResult<Json<AssistantPendingCredentialEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let node_id = normalize_action_request_id(body.node_id)?;
    let service_slug = normalize_required(body.service_slug, "serviceSlug", 64)?;
    let field_name = normalize_required(body.field_name, "fieldName", 128)?;
    let target_url = normalize_optional(body.target_url);
    let label = normalize_optional(body.label);
    node_service::ensure_node_writable_by_actor(&state.db, &actor, &node_id).await?;
    if require_online
        && (!state.node_ws_manager.is_connected(&node_id)
            || !state
                .node_ws_manager
                .supports_remote_credential_crypto(&node_id))
    {
        return Err(AppError::NodeOffline(
            "Node must be online with remote credential encryption support".to_string(),
        ));
    }
    let fingerprint = fingerprint_canonical(&PushCredentialFingerprint {
        action,
        node_id: &node_id,
        service_slug: &service_slug,
        injection_method: &body.injection_method,
        field_name: &field_name,
        target_url: target_url.as_deref(),
        label: label.as_deref(),
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        action,
        &action_request_id,
        &fingerprint,
        Uuid::new_v4().to_string(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(pending_response(&receipt, true))),
        ReceiptOutcome::InProgress(receipt) => {
            replay_in_progress_pending(&state, &actor, &node_id, receipt).await
        }
        ReceiptOutcome::Reserved(receipt) => {
            let created = node_pending_credential_service::create_pending_credential_with_id(
                &state.db,
                &actor,
                &node_id,
                &receipt.resource_id,
                node_pending_credential_service::CreatePendingCredentialInput {
                    service_slug,
                    injection_method: body.injection_method,
                    field_name,
                    target_url,
                    label,
                    ttl_secs: state.config.node_pending_credential_ttl_secs,
                    remote_crypto: true,
                },
            )
            .await;
            let pending = match created {
                Ok(pending) => pending,
                Err(error) => {
                    if !matches!(error, AppError::DatabaseError(_)) {
                        release_pending_receipt(&state, &receipt).await;
                    }
                    return Err(error);
                }
            };
            assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
            if state.node_ws_manager.is_connected(&node_id)
                && let Err(error) = state
                    .node_ws_manager
                    .send_pending_credentials_available(&node_id)
            {
                tracing::warn!(node_id = %node_id, error = %error, "failed to notify node about assistant credential push");
            }
            audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "assistant_node_credential_push_created",
                Some(serde_json::json!({
                    "node_id": &pending.node_id,
                    "pending_credential_id": &pending.id,
                    "action": action,
                })),
            );
            Ok(Json(pending_response(&receipt, false)))
        }
    }
}

pub async fn inject_node_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<PushCredentialRequest>,
) -> AppResult<Json<AssistantPendingCredentialEffectResponse>> {
    create_pending_credential_effect(state, auth_user, body, NODE_INJECT_CREDENTIAL_ACTION, true)
        .await
}

pub async fn push_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<PushCredentialRequest>,
) -> AppResult<Json<AssistantPendingCredentialEffectResponse>> {
    create_pending_credential_effect(
        state,
        auth_user,
        body,
        PENDING_CREDENTIAL_PUSH_ACTION,
        false,
    )
    .await
}

pub async fn cancel_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CancelPendingCredentialRequest>,
) -> AppResult<Json<AssistantPendingCredentialEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let node_id = normalize_action_request_id(body.node_id)?;
    let pending_credential_id = normalize_action_request_id(body.pending_credential_id)?;
    node_pending_credential_service::get_pending_credential_authorization_state(
        &state.db,
        &actor,
        &node_id,
        &pending_credential_id,
    )
    .await?;
    let fingerprint = fingerprint_canonical(&CancelPendingCredentialFingerprint {
        action: PENDING_CREDENTIAL_CANCEL_ACTION,
        node_id: &node_id,
        pending_credential_id: &pending_credential_id,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        PENDING_CREDENTIAL_CANCEL_ACTION,
        &action_request_id,
        &fingerprint,
        pending_credential_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(pending_response(&receipt, true))),
        ReceiptOutcome::InProgress(_) => Err(in_progress_conflict()),
        ReceiptOutcome::Reserved(receipt) => {
            let cancelled = node_pending_credential_service::cancel_pending_credential(
                &state.db,
                &actor,
                &node_id,
                &pending_credential_id,
            )
            .await;
            if let Err(error) = cancelled {
                if !matches!(error, AppError::DatabaseError(_)) {
                    release_pending_receipt(&state, &receipt).await;
                }
                return Err(error);
            }
            assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
            audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "assistant_pending_credential_cancelled",
                Some(serde_json::json!({
                    "node_id": &node_id,
                    "pending_credential_id": &pending_credential_id,
                })),
            );
            Ok(Json(pending_response(&receipt, false)))
        }
    }
}

pub async fn onboard_device(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<OnboardDeviceRequest>,
) -> AppResult<Json<AssistantDeviceEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let actor = auth_user.user_id.to_string();
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let label = normalize_required(body.label, "label", 128)?;
    let owner_user_id = normalize_owner_id(body.target_org_id, &actor)?;
    let default_service_ids = normalize_default_service_ids(body.default_service_ids)?;
    ensure_owner_writable(&state, &actor, &owner_user_id).await?;
    let fingerprint = fingerprint_canonical(&OnboardDeviceFingerprint {
        action: DEVICE_ONBOARD_ACTION,
        label: &label,
        target_owner_user_id: &owner_user_id,
        default_service_ids: &default_service_ids,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &actor,
        DEVICE_ONBOARD_ACTION,
        &action_request_id,
        &fingerprint,
        Uuid::new_v4().to_string(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(device_response(&receipt, true))),
        ReceiptOutcome::InProgress(receipt) => {
            replay_in_progress_device(&state, &actor, receipt).await
        }
        ReceiptOutcome::Reserved(receipt) => {
            let created = onboard_with_id(
                &state.db,
                &actor,
                &receipt.resource_id,
                DeviceOnboardInput {
                    org_id: (owner_user_id != actor).then_some(owner_user_id.clone()),
                    label,
                    default_services: Some(default_service_ids),
                    base_url: state.config.base_url.clone(),
                },
            )
            .await;
            let onboarded = match created {
                Ok(onboarded) => onboarded,
                Err(error) => {
                    if !matches!(error, AppError::DatabaseError(_)) {
                        release_pending_receipt(&state, &receipt).await;
                    }
                    return Err(error);
                }
            };
            assistant_action_receipts::mark_completed(&state.db, &receipt).await?;
            audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "assistant_device_onboard_created",
                Some(serde_json::json!({
                    "device_id": &receipt.resource_id,
                    "owner_user_id": &owner_user_id,
                })),
            );
            let mut response = device_response(&receipt, false);
            response.qr_payload = Some(onboarded.qr_payload);
            response.expires_at = Some(onboarded.expires_at.to_rfc3339());
            Ok(Json(response))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit finding F4. `ensure_node_writable_by_actor` filters
    /// `is_active: true`, so running it BEFORE `reserve_or_replay` made a
    /// retry of an already-committed delete return NotFound before the
    /// completed receipt was ever consulted -- the Replay arm was dead code
    /// and a succeeded delete reported "not found / not yours".
    ///
    /// This fails against that ordering: the second request 404s instead of
    /// replaying 200.
    #[tokio::test]
    async fn node_delete_retry_replays_instead_of_denying_a_committed_delete() {
        use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use crate::test_utils::{connect_test_database, test_app_state, test_user};
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode};
        use serde_json::{Value, json};
        use tower::ServiceExt;

        let Some(db) = connect_test_database("assistant_node_delete_retry").await else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert actor");

        let now = chrono::Utc::now();
        let node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: actor_id.clone(),
            name: "retry-node".to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());
        let token = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &uuid::Uuid::parse_str(&actor_id).expect("actor uuid"),
            "",
            None,
            None,
            None,
            None,
            None,
        )
        .expect("sign token");

        let body = json!({ "actionRequestId": "node-delete-retry", "nodeId": node_id });
        let send = |st: AppState| {
            let token = token.clone();
            let body = body.clone();
            async move {
                let (_, private) = crate::routes::build_router();
                let app = private.with_state(st);
                let response = app
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/v1/assistant/actions/nodes/delete")
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .body(Body::from(body.to_string()))
                            .expect("build request"),
                    )
                    .await
                    .expect("router responds");
                let status = response.status();
                let bytes = to_bytes(response.into_body(), 65536).await.expect("body");
                let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                (status, value)
            }
        };

        let (first_status, first) = send(state.clone()).await;
        assert_eq!(first_status, StatusCode::OK, "first delete: {first}");
        assert_eq!(first["replayed"], false);

        let (retry_status, retry) = send(state).await;
        assert_eq!(
            retry_status,
            StatusCode::OK,
            "retry of a committed delete must replay, not deny it: {retry}"
        );
        assert_eq!(retry["replayed"], true);
    }

    #[test]
    fn fingerprints_include_every_semantic_field() {
        let first = fingerprint_canonical(&PushCredentialFingerprint {
            action: PENDING_CREDENTIAL_PUSH_ACTION,
            node_id: "node-a",
            service_slug: "github",
            injection_method: &InjectionMethod::Header,
            field_name: "Authorization",
            target_url: Some("https://example.test"),
            label: Some("deploy"),
        })
        .unwrap();
        let changed = fingerprint_canonical(&PushCredentialFingerprint {
            action: PENDING_CREDENTIAL_PUSH_ACTION,
            node_id: "node-a",
            service_slug: "github",
            injection_method: &InjectionMethod::Header,
            field_name: "X-Api-Key",
            target_url: Some("https://example.test"),
            label: Some("deploy"),
        })
        .unwrap();
        assert_ne!(first, changed);
    }

    #[test]
    fn replay_responses_never_serialize_one_time_material() {
        let receipt = AssistantActionReceipt {
            id: "receipt".to_string(),
            user_id: "actor".to_string(),
            action: NODE_ROTATE_TOKEN_ACTION.to_string(),
            action_request_id: "action".to_string(),
            request_fingerprint: "fingerprint".to_string(),
            resource_id: Uuid::new_v4().to_string(),
            status:
                crate::models::assistant_action_receipt::AssistantActionReceiptStatus::Completed,
            created_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
        };
        let node = serde_json::to_value(node_response(&receipt, true)).unwrap();
        let device = serde_json::to_value(device_response(&receipt, true)).unwrap();
        for forbidden in [
            "registrationToken",
            "authToken",
            "signingSecret",
            "qrPayload",
        ] {
            assert!(node.get(forbidden).is_none(), "node leaked {forbidden}");
            assert!(device.get(forbidden).is_none(), "device leaked {forbidden}");
        }
    }
}
