use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::mw::auth::AuthUser;
use crate::services::{
    credential_push_service, org_service, user_api_key_service, user_service_service,
};

/// Look up the external API key without an ownership filter and check
/// whether the actor may modify it (directly or as an org admin).
/// Returns the effective owner_id (which may be an org user_id) for
/// downstream service calls.
///
/// `OrgMembership.allowed_service_ids` lives in the `UserService.id`
/// space, so we translate by looking up every UserService that
/// references this credential and gating on `allows_any_resource`. An
/// orphan credential (referenced by zero services) is only writable by
/// Direct owners or unscoped admins.
async fn resolve_api_key_write_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<String> {
    let key = state
        .db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &key.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    let backing_service_ids =
        user_service_service::user_service_ids_for_api_key(&state.db, &key.user_id, &key.id)
            .await?;
    if !access.allows_any_resource(&backing_service_ids) {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this API key".to_string(),
        ));
    }
    Ok(key.user_id)
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateExternalApiKeyRequest {
    pub label: Option<String>,
    pub credential: Option<String>,
}

impl std::fmt::Debug for UpdateExternalApiKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateExternalApiKeyRequest")
            .field("label", &self.label)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExternalApiKeyResponse {
    pub id: String,
    pub label: String,
    pub credential_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExternalApiKeyListResponse {
    pub api_keys: Vec<ExternalApiKeyResponse>,
}

#[utoipa::path(
    get,
    path = "/api/v1/api-keys/external",
    responses(
        (status = 200, description = "List of user's external API keys", body = ExternalApiKeyListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "External API Keys"
)]
/// GET /api/v1/api-keys/external
pub async fn list_external_api_keys(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ExternalApiKeyListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let keys = user_api_key_service::list_api_keys(&state.db, &user_id_str).await?;
    let items = keys.into_iter().map(external_api_key_response).collect();
    Ok(Json(ExternalApiKeyListResponse { api_keys: items }))
}

#[utoipa::path(
    put,
    path = "/api/v1/api-keys/external/{key_id}",
    params(
        ("key_id" = String, Path, description = "External API key ID")
    ),
    request_body = UpdateExternalApiKeyRequest,
    responses(
        (status = 200, description = "Updated external API key", body = ExternalApiKeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "External API Keys"
)]
/// PUT /api/v1/api-keys/external/{key_id}
pub async fn update_external_api_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
    Json(body): Json<UpdateExternalApiKeyRequest>,
) -> AppResult<Json<ExternalApiKeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_api_key_write_owner(&state, &actor, &key_id).await?;

    // Preflight scope check for credential rotations: when this
    // external key is shared across multiple node-routed services and
    // any of them is outside the caller's API-key scope OR the caller
    // doesn't own the destination node, the follow-up fan-out would
    // silently skip those services — the new secret would be
    // persisted, but those nodes would keep serving the old one and
    // their services would start failing until someone with full
    // access re-pushed. Reject the PUT up front so the caller sees
    // an explicit error instead of a silent partial apply
    // (thirty-third-round Codex P2).
    let rotating_credential = body.credential.as_deref().is_some_and(|c| !c.is_empty());
    if rotating_credential {
        use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
        use futures::TryStreamExt;
        let routed_services: Vec<UserService> = state
            .db
            .collection::<UserService>(USER_SERVICES)
            .find(doc! {
                "user_id": &owner_id,
                "api_key_id": &key_id,
                "node_id": { "$ne": null },
                "is_active": true,
                "auth_method": { "$ne": "none" },
            })
            .await?
            .try_collect()
            .await?;
        for svc in &routed_services {
            if !auth_user.allow_all_services && !auth_user.allowed_service_ids.contains(&svc.id) {
                return Err(AppError::ApiKeyScopeForbidden(format!(
                    "API key scope does not cover service '{}', which shares this credential. \
                     Rotating here would leave that service's node on the old secret. \
                     Ask an operator with full service scope to rotate, or restrict the rotation \
                     to this service via `PUT /keys/{{id}}` instead.",
                    svc.slug
                )));
            }
            if let Some(node_id) = svc.node_id.as_deref().filter(|n| !n.is_empty()) {
                if !auth_user.allow_all_nodes
                    && !auth_user.allowed_node_ids.contains(&node_id.to_string())
                {
                    return Err(AppError::ApiKeyScopeForbidden(format!(
                        "API key scope does not include node '{}', which hosts service '{}'. \
                         Rotating here would leave that node on the old secret.",
                        node_id, svc.slug
                    )));
                }
                // Ownership check: mirrors the per-node guard the
                // push fan-out applies. Failing here is
                // Forbidden rather than silently-skipped.
                use crate::services::node_service;
                node_service::ensure_node_writable_by_actor(&state.db, &actor, node_id)
                    .await
                    .map_err(|_| {
                        AppError::Forbidden(format!(
                            "Actor does not own node '{}', which hosts service '{}' using this credential. \
                             Rotating here would leave that node on the old secret.",
                            node_id, svc.slug
                        ))
                    })?;
            }
        }
    }

    user_api_key_service::update_api_key(
        &state.db,
        &state.encryption_keys,
        &owner_id,
        &key_id,
        body.label.as_deref(),
        body.credential.as_deref(),
    )
    .await?;

    // When the caller rotated the secret and this external API key
    // backs one or more node-routed `UserService`s using the server-
    // held credential model (NyxID#418), fire-and-forget deliver the
    // new value to those nodes. Without this, rotating from the
    // External API Keys UI would leave the node serving the stale
    // credential until some unrelated `/keys` update next reconciled
    // (seventeenth-round Codex P1). Not strict: if the node is
    // offline, the server copy remains available to retry — the
    // canonical rotation happens in `PUT /keys/:id` anyway.
    //
    // Provider-backed keys (`provider_config_id.is_some()`) are
    // excluded: the `PUT /keys` + `promote_node_managed_api_key` path
    // explicitly forbids copying provider OAuth/API credentials onto
    // a node ("Node-routed provider-backed services must be authorized
    // on the node agent"). The External API Keys UI must honor the
    // same contract — otherwise it would be a back door for installing
    // a server-held provider secret on a node (eighteenth-round Codex
    // P2).
    //
    // Use the ownership-aware push variant so org admins or scoped key
    // editors can't rewrite credentials on nodes they don't own via
    // this endpoint — `PUT /keys` gates the same write with
    // `ensure_node_writable_by_actor` (twenty-first-round Codex P1).
    let refreshed = user_api_key_service::get_api_key(&state.db, &owner_id, &key_id).await?;
    if body.credential.as_deref().is_some_and(|c| !c.is_empty())
        && refreshed.provider_config_id.is_none()
    {
        let db = state.db.clone();
        let enc = state.encryption_keys.clone();
        let ws = state.node_ws_manager.clone();
        let uid = owner_id.clone();
        let act = actor.clone();
        let ak = key_id.clone();
        // Propagate the caller's API-key scope (both node and service
        // dims) into the background push. Without the service-dim
        // filter, a scoped key whose `allowed_service_ids` authorizes
        // only one of several siblings sharing this credential could
        // use external-key rotation to overwrite node-local secrets
        // for the out-of-scope siblings (thirty-first-round Codex
        // P1). The node-dim filter was added in the previous round.
        let scope = credential_push_service::ActorScope {
            allow_all_nodes: auth_user.allow_all_nodes,
            allowed_node_ids: auth_user.allowed_node_ids.clone(),
            allow_all_services: auth_user.allow_all_services,
            allowed_service_ids: auth_user.allowed_service_ids.clone(),
        };
        tokio::spawn(async move {
            credential_push_service::push_credential_to_node_if_owned(
                &db, &enc, &ws, &uid, &act, &ak, scope,
            )
            .await;
        });
    }

    Ok(Json(external_api_key_response(refreshed)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/api-keys/external/{key_id}",
    params(
        ("key_id" = String, Path, description = "External API key ID")
    ),
    responses(
        (status = 204, description = "External API key deleted"),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "External API Keys"
)]
/// DELETE /api/v1/api-keys/external/{key_id}
pub async fn delete_external_api_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_api_key_write_owner(&state, &actor, &key_id).await?;
    user_api_key_service::delete_api_key(&state.db, &owner_id, &key_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn external_api_key_response(key: UserApiKey) -> ExternalApiKeyResponse {
    ExternalApiKeyResponse {
        id: key.id,
        label: key.label,
        credential_type: key.credential_type,
        status: key.status,
        provider_config_id: key.provider_config_id,
        expires_at: key.expires_at.map(|dt| dt.to_rfc3339()),
        last_used_at: key.last_used_at.map(|dt| dt.to_rfc3339()),
        error_message: key.error_message,
        created_at: key.created_at.to_rfc3339(),
        updated_at: key.updated_at.to_rfc3339(),
    }
}
