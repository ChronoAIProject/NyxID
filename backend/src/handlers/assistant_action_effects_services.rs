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
    ReceiptOutcome, fingerprint_canonical, in_progress_conflict, mark_completed,
    normalize_action_request_id, reserve_or_replay,
};
use crate::services::audit_service::AuditActor;
use crate::services::user_endpoint_service::{OpenApiSpecUrlUpdate, RecommendedSkillsUpdate};
use crate::services::{
    node_service, org_service, unified_key_service, user_api_key_service, user_endpoint_service,
    user_service_service,
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

fn utc_now_at_bson_precision() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(chrono::Utc::now().timestamp_millis())
        .expect("current UTC timestamp must fit BSON precision")
}

/// Atomically `$inc` `state_version` and `$set` `updated_at` plus extra fields.
async fn commit_service_mutation(
    db: &mongodb::Database,
    owner_id: &str,
    service_id: &str,
    extra_set: mongodb::bson::Document,
) -> AppResult<UserService> {
    let mut set_doc = extra_set;
    set_doc.insert(
        "updated_at",
        bson::DateTime::from_chrono(utc_now_at_bson_precision()),
    );
    let updated = db
        .collection::<UserService>(USER_SERVICES)
        .find_one_and_update(
            doc! { "_id": service_id, "user_id": owner_id },
            doc! {
                "$set": set_doc,
                "$inc": { "state_version": 1_i64 },
            },
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("User service not found".to_string()))?;
    Ok(updated)
}

fn completed_replay(outcome: &ReceiptOutcome) -> bool {
    // `Replay` is now completed-only; a pending reservation surfaces as
    // `InProgress` and must never reach the mutation branch.
    matches!(outcome, ReceiptOutcome::Replay(_))
}

fn receipt_from_outcome(
    outcome: ReceiptOutcome,
) -> AppResult<crate::models::assistant_action_receipt::AssistantActionReceipt> {
    match outcome {
        ReceiptOutcome::Reserved(receipt) => Ok(receipt),
        ReceiptOutcome::Replay(receipt) => Ok(receipt),
        ReceiptOutcome::InProgress(_) => Err(in_progress_conflict()),
    }
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
    let replayed = completed_replay(&outcome);
    if !replayed {
        let receipt = receipt_from_outcome(outcome)?;
        let access = resolve_write_owner(&state, &actor, &request.user_service_id).await?;
        if let Some(url) = request.endpoint_url.as_deref() {
            crate::services::url_validation::validate_user_endpoint_url(
                url,
                state.config.is_production(),
                "endpointUrl",
            )
            .await?;
        }
        if request.name.is_some() || request.endpoint_url.is_some() {
            user_endpoint_service::update_endpoint(
                &state.db,
                &access.owner_id,
                &access.service.endpoint_id,
                request.endpoint_url.as_deref(),
                request.name.as_deref(),
                OpenApiSpecUrlUpdate::Leave,
                RecommendedSkillsUpdate::Leave,
            )
            .await?;
        }
        if request.auth_method.is_some() || request.auth_key_name.is_some() {
            user_service_service::update_user_service(
                &state.db,
                &access.owner_id,
                &actor,
                &request.user_service_id,
                request.auth_method.as_deref(),
                request.auth_key_name.as_deref(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
        } else {
            commit_service_mutation(
                &state.db,
                &access.owner_id,
                &request.user_service_id,
                doc! {},
            )
            .await?;
        }
        mark_completed(&state.db, &receipt).await?;
    }

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
    let options = unified_key_service::DisconnectOptions {
        cascade_grant: request.cascade_grant,
        grant_scope: request.grant_scope.clone(),
    };
    options.validate()?;
    let fingerprint = fingerprint_canonical(&ServiceDeleteFingerprint {
        action: SERVICE_DELETE_ACTION,
        user_service_id: &request.user_service_id,
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
    let replayed = completed_replay(&outcome);
    if !replayed {
        let receipt = receipt_from_outcome(outcome)?;
        let access = resolve_write_owner(&state, &actor, &request.user_service_id).await?;
        if access.service.source.as_deref()
            == Some(crate::models::user_service::AUTO_PROVISION_SOURCE)
        {
            return Err(AppError::Forbidden(
                "Auto-connected services cannot be deleted".to_string(),
            ));
        }
        let audit_actor = AuditActor::from_auth_user(&auth_user);
        unified_key_service::disconnect_credentials(
            &state.db,
            &state.encryption_keys,
            &access.owner_id,
            &audit_actor,
            unified_key_service::DisconnectTarget::UserService(&request.user_service_id),
            options,
        )
        .await?;
        mark_completed(&state.db, &receipt).await?;
    }

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
    let replayed = completed_replay(&outcome);
    if !replayed {
        let receipt = receipt_from_outcome(outcome)?;
        let access = resolve_write_owner(&state, &actor, &request.user_service_id).await?;
        let mut extra = doc! {};
        match request.via_node_id.as_deref() {
            Some(node_id) => {
                node_service::ensure_node_writable_by_actor(&state.db, &actor, node_id).await?;
                extra.insert("node_id", node_id);
            }
            None => {
                extra.insert("node_id", mongodb::bson::Bson::Null);
            }
        }
        commit_service_mutation(&state.db, &access.owner_id, &request.user_service_id, extra)
            .await?;
        mark_completed(&state.db, &receipt).await?;
    }

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
    let fingerprint = fingerprint_canonical(&ServiceRotateFingerprint {
        action: SERVICE_ROTATE_ACTION,
        user_service_id: &request.user_service_id,
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
    let replayed = completed_replay(&outcome);
    if !replayed {
        let receipt = receipt_from_outcome(outcome)?;
        let access = resolve_write_owner(&state, &actor, &request.user_service_id).await?;
        let predecessor_id = access.service.api_key_id.clone().ok_or_else(|| {
            AppError::BadRequest("This service has no stored credential to rotate".to_string())
        })?;
        let existing =
            user_api_key_service::get_api_key(&state.db, &access.owner_id, &predecessor_id).await?;
        if existing.credential_type == "node_managed" {
            return Err(AppError::BadRequest(
                "Credential is managed on the node agent. Update it on the node instead."
                    .to_string(),
            ));
        }
        let successor = user_api_key_service::create_api_key(
            &state.db,
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
        commit_service_mutation(
            &state.db,
            &access.owner_id,
            &request.user_service_id,
            doc! {
                "api_key_id": &successor.id,
                "rotation_predecessor_id": &predecessor_id,
            },
        )
        .await?;
        mark_completed(&state.db, &receipt).await?;
    }

    Ok(Json(RotateAssistantServiceCredentialResponse {
        resource: AssistantServiceResource {
            user_service_id: request.user_service_id,
        },
        replayed,
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
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::handlers::assistant_actions;
    use crate::handlers::keys;
    use crate::models::assistant_action_receipt::COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
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
        connect_test_database, test_app_state, test_encryption_keys, test_user, test_user_endpoint,
        test_user_service,
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
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn fixture_api_key(id: &str, user_id: &str) -> UserApiKey {
        UserApiKey {
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
        assert!(stored.state_version >= 1);
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
        assert!(stored.state_version >= 1);

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
    async fn service_delete_cascade_prompts_grant_confirmation() {
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

        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let (status, body) = request(
            app(state),
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
    async fn service_rotate_stale_identity_replay_rejected() {
        let Some((db, actor_id)) = prepare_actor("svc_rotate_stale_id").await else {
            return;
        };
        let (first, _) = seed_service_named(&db, &actor_id, "svc-one", true, None).await;
        let (second, _) = seed_service_named(&db, &actor_id, "svc-two", true, None).await;
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
                "userServiceId": second.id,
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
