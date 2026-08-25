use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::mw::auth::AuthUser;
use crate::services::{
    credential_push_service, gcp_sa_service, org_service, unified_key_service,
    user_api_key_service, user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

/// Look up the external API key without an ownership filter and check
/// whether the actor may read it (directly or as an org member). Unauthorized
/// access is not-found-shaped so existence is not leaked.
///
/// `OrgMembership.allowed_service_ids` lives in the `UserService.id`
/// space, so we translate by looking up every UserService that
/// references this credential and gating on `allows_any_resource`. An
/// orphan credential (referenced by zero services) is only readable by
/// Direct owners or unscoped admins.
async fn load_readable_api_key(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<UserApiKey> {
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
    Ok(key)
}

/// Look up the external API key without an ownership filter and check
/// whether the actor may modify it (directly or as an org admin).
/// Returns the key so action handlers can reject unsupported mutation modes
/// before reserving an idempotency receipt.
pub(crate) async fn resolve_api_key_write_target(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<UserApiKey> {
    let key = load_readable_api_key(state, actor, key_id).await?;
    let access = org_service::resolve_owner_access(&state.db, actor, &key.user_id).await?;
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this API key".to_string(),
        ));
    }
    Ok(key)
}

/// Resolve the effective owner for service-layer mutations.
pub(crate) async fn resolve_api_key_write_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<String> {
    Ok(resolve_api_key_write_target(state, actor, key_id)
        .await?
        .user_id)
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

/// Closed credential_type set served as assistant-action evidence.
/// Adding a value is a breaking consumer change (`ParseCredentialStatus`-style
/// closed parsers throw on unknowns) — not additive. Keep in lockstep with
/// `user_api_key_service::VALID_CREDENTIAL_TYPES`.
pub(crate) const SERVED_EXTERNAL_KEY_CREDENTIAL_TYPES: &[&str] =
    user_api_key_service::VALID_CREDENTIAL_TYPES;

/// Closed status set served as assistant-action evidence.
/// Adding a value is a breaking consumer change — not additive. Keep in
/// lockstep with `user_api_key_service::VALID_STATUSES`.
pub(crate) const SERVED_EXTERNAL_KEY_STATUSES: &[&str] = user_api_key_service::VALID_STATUSES;

fn served_credential_type(value: String) -> AppResult<String> {
    // Telegram login rows predate the unified credential enum and are
    // migrated verbatim. They carry identity metadata, not a distinct
    // externally injectable credential; normalize without echoing the
    // legacy source value into the closed evidence contract.
    let value = if value == "telegram_identity" {
        "api_key".to_string()
    } else {
        value
    };
    if SERVED_EXTERNAL_KEY_CREDENTIAL_TYPES.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(AppError::Internal(
            "refusing to serve unknown credential_type as evidence".to_string(),
        ))
    }
}

fn served_credential_status(value: String) -> AppResult<String> {
    if SERVED_EXTERNAL_KEY_STATUSES.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(AppError::Internal(
            "refusing to serve unknown credential status as evidence".to_string(),
        ))
    }
}

/// Authorization evidence for one external API key — the minimal projection
/// of [`ExternalApiKeyResponse`] that an assistant-action postcondition
/// reader consumes.
///
/// The full detail response carries user-controlled `label` and
/// `error_message` (which echoes upstream text and can contain anything).
/// Either trips the reader's secret-shape scan, so a failed OAuth row or a
/// user-labelled key would become permanently unverifiable. This projection
/// keeps only the id, closed enums, and RFC 3339 timestamps.
///
/// Every property is serialized unconditionally. Readers distinguish an
/// explicit `null` from a missing property, so `expires_at` and
/// `last_used_at` are never `skip_serializing_if`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExternalApiKeyAuthorizationEvidenceResponse {
    pub id: String,
    pub credential_type: String,
    pub status: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub updated_at: String,
}

impl ExternalApiKeyAuthorizationEvidenceResponse {
    /// Project the full detail response down to authorization evidence.
    ///
    /// Derived from [`ExternalApiKeyResponse`] so the two representations
    /// cannot drift on shared properties. Unknown enum values fail closed
    /// rather than being emitted into a consumer whose parsers throw.
    pub fn from_external_api_key_response(response: ExternalApiKeyResponse) -> AppResult<Self> {
        Ok(Self {
            id: response.id,
            credential_type: served_credential_type(response.credential_type)?,
            status: served_credential_status(response.status)?,
            expires_at: response.expires_at,
            last_used_at: response.last_used_at,
            updated_at: response.updated_at,
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExternalApiKeyListResponse {
    pub api_keys: Vec<ExternalApiKeyResponse>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteExternalApiKeyQuery {
    #[serde(default)]
    pub cascade_grant: Option<bool>,
    #[serde(default)]
    pub grant_scope: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteExternalApiKeyResponse {
    pub deleted: bool,
    pub upstream_revocation_scheduled: bool,
}

/// Register a Google Cloud service-account JSON key as a proxy
/// credential. NyxID mints short-lived access tokens from it via
/// JWT-bearer (no `invalid_rapt`, no human reauth — unlike user OAuth for
/// Cloud Platform scopes).
#[derive(Deserialize, ToSchema)]
pub struct CreateGcpServiceAccountRequest {
    /// Display label; defaults to the service-account `client_email`.
    #[serde(default)]
    pub label: Option<String>,
    /// Raw contents of the Google service-account JSON key file.
    pub key_json: String,
    /// OAuth scope(s) to request when minting (space-separated). Defaults
    /// to `https://www.googleapis.com/auth/cloud-platform`.
    #[serde(default)]
    pub scopes: Option<String>,
    /// Existing service slugs to (re)bind to this credential, e.g.
    /// `google-bigquery`, `google-cloud-billing`. Each is switched to
    /// `auth_method: "bearer"` against the new key.
    #[serde(default)]
    pub service_slugs: Vec<String>,
    /// Optional org owner for this credential. Omit for personal credentials.
    pub target_org_id: Option<String>,
}

impl std::fmt::Debug for CreateGcpServiceAccountRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key file embeds a private key — never log it.
        f.debug_struct("CreateGcpServiceAccountRequest")
            .field("label", &self.label)
            .field("key_json", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .field("service_slugs", &self.service_slugs)
            .field("target_org_id", &self.target_org_id)
            .finish()
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/api-keys/external/gcp-service-account",
    request_body = CreateGcpServiceAccountRequest,
    responses(
        (status = 201, description = "Created GCP service-account credential", body = ExternalApiKeyResponse),
        (status = 400, description = "Invalid service account key", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "External API Keys"
)]
/// POST /api/v1/api-keys/external/gcp-service-account
pub async fn create_gcp_service_account_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateGcpServiceAccountRequest>,
) -> AppResult<(StatusCode, Json<ExternalApiKeyResponse>)> {
    create_gcp_service_account_key_with_id(State(state), auth_user, Json(body), None).await
}

/// Internal assistant-action create path with a receipt-reserved API-key id.
pub(crate) async fn create_gcp_service_account_key_with_id(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateGcpServiceAccountRequest>,
    reserved_id: Option<&str>,
) -> AppResult<(StatusCode, Json<ExternalApiKeyResponse>)> {
    let actor = auth_user.user_id.to_string();

    // Validate JSON shape and derive a default label up front.
    let parsed: serde_json::Value = serde_json::from_str(&body.key_json)
        .map_err(|_| AppError::ValidationError("key_json is not valid JSON".to_string()))?;
    let client_email = parsed
        .get("client_email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let has_private_key = parsed
        .get("private_key")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if client_email.is_empty() || !has_private_key {
        return Err(AppError::ValidationError(
            "key_json must be a Google service account key file with client_email and private_key"
                .to_string(),
        ));
    }

    let owner_id = if let Some(target_org_id) =
        body.target_org_id.as_deref().filter(|id| !id.is_empty())
    {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create GCP service-account credentials under it"
                    .to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor.clone()
    };

    let scope = body
        .scopes
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(gcp_sa_service::DEFAULT_GCP_SA_SCOPE);

    // Mint once to validate the key actually works before storing it —
    // catches a disabled / deleted service account or a malformed key
    // immediately instead of on the user's first proxy request.
    let minted = gcp_sa_service::mint_access_token(&body.key_json, scope)
        .await
        .map_err(|e| {
            AppError::ValidationError(format!(
                "Could not mint a token from this service account: {e}"
            ))
        })?;
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(minted.expires_in_secs);

    let label = body
        .label
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| client_email.to_string());

    let params = user_api_key_service::CreateApiKeyParams {
        label: &label,
        credential_type: "gcp_service_account",
        credential: &body.key_json,
        access_token: Some(&minted.access_token),
        refresh_token: None,
        token_scopes: Some(scope),
        expires_at: Some(expires_at),
        provider_config_id: None,
        connection_id: None,
        oauth_client_id: None,
        oauth_client_secret: None,
        status: "active",
        source: Some("user_created"),
        source_id: None,
    };
    let key = match reserved_id {
        Some(id) => {
            user_api_key_service::create_api_key_with_id(
                &state.db,
                &state.encryption_keys,
                &owner_id,
                id,
                params,
            )
            .await?
        }
        None => {
            user_api_key_service::create_api_key(
                &state.db,
                &state.encryption_keys,
                &owner_id,
                params,
            )
            .await?
        }
    };

    // Optional: re-point existing services (e.g. google-bigquery,
    // google-cloud-billing) at this credential.
    for slug in &body.service_slugs {
        user_service_service::rebind_user_service_api_key(&state.db, &owner_id, slug, &key.id)
            .await?;
    }

    Ok((StatusCode::CREATED, Json(external_api_key_response(key))))
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
        ("key_id" = String, Path, description = "External API key ID"),
        ("cascade_grant" = Option<bool>, Query, description = "Confirm deletion of all same-owner connections sharing an upstream OAuth grant"),
        ("grant_scope" = Option<String>, Query, description = "Set to token to delete only this credential without revoking the upstream grant")
    ),
    responses(
        (status = 200, description = "External API key deleted", body = DeleteExternalApiKeyResponse),
        (status = 400, description = "Invalid or conflicting revocation options", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse),
        (status = 409, description = "OAuth grant cascade confirmation required", body = crate::errors::ErrorResponse)
    ),
    tag = "External API Keys"
)]
/// DELETE /api/v1/api-keys/external/{key_id}
pub async fn delete_external_api_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(key_id): Path<String>,
    Query(query): Query<DeleteExternalApiKeyQuery>,
) -> AppResult<Json<DeleteExternalApiKeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let options = unified_key_service::DisconnectOptions {
        cascade_grant: query.cascade_grant.unwrap_or(false),
        grant_scope: query.grant_scope,
    };
    options.validate()?;
    let owner_id = resolve_api_key_write_owner(&state, &actor, &key_id).await?;
    let audit_actor = crate::services::audit_service::AuditActor::from_auth_user(&auth_user);
    let result = unified_key_service::disconnect_credentials(
        &state.db,
        &state.encryption_keys,
        &owner_id,
        &audit_actor,
        unified_key_service::DisconnectTarget::UserApiKey(&key_id),
        options,
    )
    .await?;
    for event in &result.deleted_services {
        emit_event(
            state.telemetry.as_deref(),
            &actor,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::KeyDeleted {
                source: event.source.clone(),
            },
        );
    }
    Ok(Json(DeleteExternalApiKeyResponse {
        deleted: true,
        upstream_revocation_scheduled: result.upstream_revocation_scheduled,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/api-keys/external/{key_id}/authorization",
    params(
        ("key_id" = String, Path, description = "External API key ID")
    ),
    responses(
        (status = 200, description = "Authorization evidence", body = ExternalApiKeyAuthorizationEvidenceResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "External API Keys"
)]
/// GET /api/v1/api-keys/external/{key_id}/authorization
///
/// Same ACL as the external-key write sibling, projected to the properties
/// an assistant-action postcondition reader consumes. Delete-shaped verbs
/// prove absence through this route returning 404.
pub async fn get_external_api_key_authorization(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<ExternalApiKeyAuthorizationEvidenceResponse>> {
    let actor = auth_user.user_id.to_string();
    let key = load_readable_api_key(&state, &actor, &key_id).await?;
    Ok(Json(
        ExternalApiKeyAuthorizationEvidenceResponse::from_external_api_key_response(
            external_api_key_response(key),
        )?,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::UserType;
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::user_api_key_service;
    use crate::test_utils::*;
    use axum::extract::{Path, State};
    use chrono::Utc;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn fixture_external_key(key_id: &str, user_id: &str, label: &str) -> UserApiKey {
        UserApiKey {
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
            credential_epoch: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    async fn seed_service_for_key(
        db: &mongodb::Database,
        owner_id: &str,
        service_id: &str,
        key_id: &str,
    ) {
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                owner_id,
                "External Key Endpoint",
                "https://service.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        let mut service = test_user_service(
            service_id,
            owner_id,
            "external-key-service",
            &endpoint_id,
            None,
            None,
        );
        service.api_key_id = Some(key_id.to_string());
        service.auth_method = "bearer".to_string();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service)
            .await
            .unwrap();
    }

    async fn seed_org_actor(
        db: &mongodb::Database,
        org_id: &str,
        actor_id: &str,
        role: OrgRole,
        allowed_service_ids: Option<Vec<String>>,
    ) {
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_many([
                test_user(actor_id, UserType::Person),
                test_user(org_id, UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(org_id, actor_id, role, allowed_service_ids))
            .await
            .unwrap();
    }

    async fn seed_unbound_service(
        db: &mongodb::Database,
        owner_id: &str,
        service_id: &str,
        slug: &str,
    ) -> UserService {
        let service = test_user_service(
            service_id,
            owner_id,
            slug,
            &uuid::Uuid::new_v4().to_string(),
            None,
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        service
    }

    fn gcp_sa_body(
        label: Option<&str>,
        key_json: String,
        service_slugs: &[&str],
        target_org_id: Option<&str>,
    ) -> CreateGcpServiceAccountRequest {
        CreateGcpServiceAccountRequest {
            label: label.map(str::to_string),
            key_json,
            scopes: None,
            service_slugs: service_slugs.iter().map(|slug| slug.to_string()).collect(),
            target_org_id: target_org_id.map(str::to_string),
        }
    }

    async fn spawn_gcp_token_server(access_token: &str) -> (String, tokio::task::JoinHandle<()>) {
        spawn_mock_token_server(
            serde_json::json!({ "access_token": access_token, "expires_in": 3600 }),
            StatusCode::OK,
        )
        .await
    }

    async fn spawn_counting_gcp_token_server(
        access_token: &str,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        spawn_counting_token_server(
            serde_json::json!({ "access_token": access_token, "expires_in": 3600 }),
            StatusCode::OK,
        )
        .await
    }

    async fn spawn_counting_token_server(
        response: serde_json::Value,
        status: axum::http::StatusCode,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_route = calls.clone();
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(move || {
                let resp = response.clone();
                let calls = calls_for_route.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (status, axum::Json(resp))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/token"), calls, handle)
    }

    #[tokio::test]
    async fn test_list_external_api_keys_empty() {
        let Some(db) = connect_test_database("h_ext_keys_list_empty").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(resp) = list_external_api_keys(State(state), auth).await.unwrap();
        assert!(resp.api_keys.is_empty());
    }

    #[tokio::test]
    async fn test_list_external_api_keys_returns_user_keys() {
        let Some(db) = connect_test_database("h_ext_keys_list_ok").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db.clone());

        user_api_key_service::create_api_key(
            &db,
            &state.encryption_keys,
            &user_id,
            user_api_key_service::CreateApiKeyParams {
                label: "Test Key",
                credential_type: "api_key",
                credential: "sk-test-1234",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();

        let Json(resp) = list_external_api_keys(State(state), test_auth_user(&user_id))
            .await
            .unwrap();
        assert_eq!(resp.api_keys.len(), 1);
        assert_eq!(resp.api_keys[0].label, "Test Key");
        assert_eq!(resp.api_keys[0].credential_type, "api_key");
        assert_eq!(resp.api_keys[0].status, "active");
    }

    #[tokio::test]
    async fn test_list_external_api_keys_excludes_other_users() {
        let Some(db) = connect_test_database("h_ext_keys_list_iso").await else {
            return;
        };
        let user_a = uuid::Uuid::new_v4().to_string();
        let user_b = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db.clone());

        user_api_key_service::create_api_key(
            &db,
            &state.encryption_keys,
            &user_a,
            user_api_key_service::CreateApiKeyParams {
                label: "A key",
                credential_type: "bearer",
                credential: "bearer-token-a",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();

        let Json(resp) = list_external_api_keys(State(state), test_auth_user(&user_b))
            .await
            .unwrap();
        assert!(resp.api_keys.is_empty());
    }

    #[tokio::test]
    async fn test_update_external_api_key_label() {
        let Some(db) = connect_test_database("h_ext_keys_update_label").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();

        let state = test_app_state(db.clone());

        let created = user_api_key_service::create_api_key(
            &db,
            &state.encryption_keys,
            &user_id,
            user_api_key_service::CreateApiKeyParams {
                label: "Old Label",
                credential_type: "api_key",
                credential: "sk-old-value",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();

        let Json(resp) = update_external_api_key(
            State(state),
            test_auth_user(&user_id),
            Path(created.id.clone()),
            Json(UpdateExternalApiKeyRequest {
                label: Some("New Label".to_string()),
                credential: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(resp.label, "New Label");
        assert_eq!(resp.id, created.id);
    }

    #[tokio::test]
    async fn test_delete_external_api_key_success() {
        let Some(db) = connect_test_database("h_ext_keys_delete_ok").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();

        let state = test_app_state(db.clone());

        let created = user_api_key_service::create_api_key(
            &db,
            &state.encryption_keys,
            &user_id,
            user_api_key_service::CreateApiKeyParams {
                label: "Ephemeral",
                credential_type: "bearer",
                credential: "bearer-123",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        seed_service_for_key(&db, &user_id, &service_id, &created.id).await;
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let resp = delete_external_api_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(created.id.clone()),
            Query(DeleteExternalApiKeyQuery::default()),
        )
        .await;
        assert!(resp.is_ok());

        let remaining = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .count_documents(doc! { "_id": &created.id })
            .await
            .unwrap();
        assert_eq!(remaining, 0);

        let view = crate::services::unified_key_service::get_key(
            &db,
            &state.encryption_keys,
            &user_id,
            &service_id,
        )
        .await
        .unwrap();
        assert!(view.credential_missing);
        assert_eq!(view.auth_method, "bearer");

        let enable_error = crate::services::user_service_service::update_user_service(
            &db,
            &user_id,
            &user_id,
            &service_id,
            None,
            None,
            None,
            None,
            Some(true),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("deleting the credential must keep Enable blocked");
        assert!(matches!(
            enable_error,
            AppError::BadRequest(message) if message.contains("Reconnect or delete")
        ));
    }

    #[tokio::test]
    async fn resolve_api_key_write_owner_allows_scoped_org_admin_for_bound_service() {
        let Some(db) = connect_test_database("h_ext_keys_org_admin_scoped").await else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        seed_org_actor(
            &db,
            &org_id,
            &actor_id,
            OrgRole::Admin,
            Some(vec![service_id.clone()]),
        )
        .await;
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_external_key(&key_id, &org_id, "Org Credential"))
            .await
            .unwrap();
        seed_service_for_key(&db, &org_id, &service_id, &key_id).await;

        let state = test_app_state(db);
        let owner_id = resolve_api_key_write_owner(&state, &actor_id, &key_id)
            .await
            .unwrap();

        assert_eq!(owner_id, org_id);
    }

    #[tokio::test]
    async fn resolve_api_key_write_owner_hides_scoped_org_orphan_key() {
        let Some(db) = connect_test_database("h_ext_keys_org_orphan_scoped").await else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        seed_org_actor(
            &db,
            &org_id,
            &actor_id,
            OrgRole::Admin,
            Some(vec![uuid::Uuid::new_v4().to_string()]),
        )
        .await;
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_external_key(
                &key_id,
                &org_id,
                "Unbound Org Credential",
            ))
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = resolve_api_key_write_owner(&state, &actor_id, &key_id)
            .await
            .expect_err("scoped admin has no resource claim on orphan credential");

        assert!(matches!(
            err,
            AppError::NotFound(message) if message == "API key not found"
        ));
    }

    #[tokio::test]
    async fn update_external_api_key_denies_org_viewer_write() {
        let Some(db) = connect_test_database("h_ext_keys_org_viewer_denied").await else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        seed_org_actor(&db, &org_id, &actor_id, OrgRole::Viewer, None).await;
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_external_key(&key_id, &org_id, "Org Credential"))
            .await
            .unwrap();
        seed_service_for_key(&db, &org_id, &service_id, &key_id).await;

        let state = test_app_state(db);
        let err = update_external_api_key(
            State(state),
            test_auth_user(&actor_id),
            Path(key_id),
            Json(UpdateExternalApiKeyRequest {
                label: Some("Viewer Edit".to_string()),
                credential: None,
            }),
        )
        .await
        .expect_err("org viewer cannot update external API keys");

        assert!(matches!(
            err,
            AppError::OrgRoleInsufficient(message)
                if message == "you do not have permission to modify this API key"
        ));
    }

    #[tokio::test]
    async fn delete_external_api_key_returns_conflict_when_active_service_uses_key() {
        let Some(db) = connect_test_database("h_ext_keys_delete_in_use").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_external_key(&key_id, &user_id, "In Use"))
            .await
            .unwrap();
        seed_service_for_key(&db, &user_id, &service_id, &key_id).await;

        let state = test_app_state(db);
        let err = match delete_external_api_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(key_id),
            Query(DeleteExternalApiKeyQuery::default()),
        )
        .await
        {
            Ok(_) => panic!("active service reference should prevent credential delete"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            AppError::Conflict(message) if message == "API key is in use by active services"
        ));
    }

    #[tokio::test]
    async fn delete_external_oauth_key_uses_grant_cascade_contract() {
        let Some(db) = connect_test_database("h_ext_keys_oauth_cascade").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        db.collection::<mongodb::bson::Document>(crate::models::provider_config::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &provider_id,
                "slug": "github",
                "name": "GitHub",
                "provider_type": "oauth2",
                "revocation": {
                    "style": "github",
                    "url": "https://api.github.com/applications",
                    "auth": "basic",
                    "revokes_grant": true,
                },
                "is_active": true,
                "created_by": "system",
                "created_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
                "updated_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
            })
            .await
            .unwrap();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        for suffix in ["primary", "sibling"] {
            let key_id = format!("key-{suffix}");
            let service_id = format!("service-{suffix}");
            let mut key = fixture_external_key(&key_id, &user_id, suffix);
            key.credential_type = "oauth2".to_string();
            key.credential_encrypted = None;
            key.provider_config_id = Some(provider_id.clone());
            key.connection_id = Some(uuid::Uuid::new_v4().to_string());
            key.credential_source = Some("platform".to_string());
            db.collection::<UserApiKey>(USER_API_KEYS)
                .insert_one(key)
                .await
                .unwrap();
            seed_service_for_key(&db, &user_id, &service_id, &key_id).await;
        }
        let state = test_app_state(db.clone());

        let err = delete_external_api_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path("key-primary".to_string()),
            Query(DeleteExternalApiKeyQuery::default()),
        )
        .await
        .expect_err("external OAuth key must not bypass grant confirmation");
        assert!(matches!(err, AppError::GrantCascadeConfirmationRequired(_)));
        assert_eq!(
            db.collection::<UserApiKey>(USER_API_KEYS)
                .count_documents(doc! { "user_id": &user_id })
                .await
                .unwrap(),
            2
        );

        let Json(response) = delete_external_api_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path("key-primary".to_string()),
            Query(DeleteExternalApiKeyQuery {
                cascade_grant: Some(true),
                grant_scope: None,
            }),
        )
        .await
        .unwrap();
        assert!(response.deleted);
        assert!(!response.upstream_revocation_scheduled);
        assert_eq!(
            db.collection::<UserApiKey>(USER_API_KEYS)
                .count_documents(doc! { "user_id": &user_id })
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn test_create_gcp_service_account_stores_and_rebinds() {
        let Some(db) = connect_test_database("h_ext_gcp_sa_create").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db.clone());

        let (token_uri, _handle) = spawn_gcp_token_server("ya29.minted").await;
        seed_unbound_service(
            &db,
            &user_id,
            &uuid::Uuid::new_v4().to_string(),
            "google-bigquery",
        )
        .await;
        let body = gcp_sa_body(
            Some("GCP Cost Reader"),
            test_gcp_sa_json(&token_uri),
            &["google-bigquery"],
            None,
        );

        let (status, Json(resp)) =
            create_gcp_service_account_key(State(state), test_auth_user(&user_id), Json(body))
                .await
                .unwrap();

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(resp.credential_type, "gcp_service_account");
        assert_eq!(resp.status, "active");

        // The durable SA key and the minted token are both stored.
        let stored = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &resp.id })
            .await
            .unwrap()
            .unwrap();
        assert!(stored.credential_encrypted.is_some());
        assert!(stored.access_token_encrypted.is_some());
        assert_eq!(
            stored.token_scopes.as_deref(),
            Some("https://www.googleapis.com/auth/cloud-platform")
        );

        // The named service was re-pointed onto the new key as bearer.
        let rebound = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "slug": "google-bigquery", "user_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebound.api_key_id.as_deref(), Some(resp.id.as_str()));
        assert_eq!(rebound.auth_method, "bearer");
    }

    #[tokio::test]
    async fn org_admin_create_gcp_sa_stores_under_org_and_rebinds_org_service() {
        let Some(db) = connect_test_database("h_ext_gcp_sa_org_admin").await else {
            return;
        };
        let admin_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &admin_id, OrgRole::Admin, None).await;

        let service_id = uuid::Uuid::new_v4().to_string();
        seed_unbound_service(&db, &org_id, &service_id, "google-bigquery").await;

        let state = test_app_state(db.clone());
        let (token_uri, _handle) = spawn_gcp_token_server("ya29.org-admin").await;
        let key_json = test_gcp_sa_json(&token_uri);
        let body = gcp_sa_body(
            Some("Org GCP Cost Reader"),
            key_json.clone(),
            &["google-bigquery"],
            Some(&org_id),
        );

        let (status, Json(resp)) = create_gcp_service_account_key(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(body),
        )
        .await
        .unwrap();

        assert_eq!(status, StatusCode::CREATED);
        let stored = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &resp.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.user_id, org_id);
        assert_eq!(stored.credential_type, "gcp_service_account");
        let decrypted_key = state
            .encryption_keys
            .decrypt(stored.credential_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        let decrypted_token = state
            .encryption_keys
            .decrypt(stored.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(decrypted_key).unwrap(), key_json);
        assert_eq!(
            String::from_utf8(decrypted_token).unwrap(),
            "ya29.org-admin"
        );

        let rebound = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebound.api_key_id.as_deref(), Some(resp.id.as_str()));
        assert_eq!(rebound.auth_method, "bearer");
    }

    #[tokio::test]
    async fn org_member_create_gcp_sa_rejected_before_mint_or_store() {
        let Some(db) = connect_test_database("h_ext_gcp_sa_org_member_denied").await else {
            return;
        };
        let member_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &member_id, OrgRole::Member, None).await;
        let state = test_app_state(db.clone());
        let (token_uri, calls, _handle) =
            spawn_counting_gcp_token_server("ya29.must-not-mint").await;
        let body = gcp_sa_body(
            None,
            test_gcp_sa_json(&token_uri),
            &["google-bigquery"],
            Some(&org_id),
        );

        let err =
            create_gcp_service_account_key(State(state), test_auth_user(&member_id), Json(body))
                .await
                .expect_err("org member cannot create org-owned GCP SA credentials");

        assert!(matches!(
            &err,
            AppError::OrgRoleInsufficient(message)
                if message == "you must be an admin of the target org to create GCP service-account credentials under it"
        ));
        assert_eq!(err.error_code(), 8103);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let key_count = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .count_documents(doc! {})
            .await
            .unwrap();
        assert_eq!(key_count, 0);
    }

    #[tokio::test]
    async fn scoped_admin_create_gcp_sa_allows_empty_and_out_of_scope_service_slugs() {
        let Some(db) = connect_test_database("h_ext_gcp_sa_scoped_admin_allow").await else {
            return;
        };
        let admin_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let in_scope_service_id = uuid::Uuid::new_v4().to_string();
        let out_of_scope_service_id = uuid::Uuid::new_v4().to_string();
        seed_org_actor(
            &db,
            &org_id,
            &admin_id,
            OrgRole::Admin,
            Some(vec![in_scope_service_id]),
        )
        .await;

        seed_unbound_service(
            &db,
            &org_id,
            &out_of_scope_service_id,
            "google-out-of-scope",
        )
        .await;

        let state = test_app_state(db.clone());
        let (token_uri, _handle) = spawn_gcp_token_server("ya29.scoped-admin").await;

        let empty_body = gcp_sa_body(
            Some("Scoped Admin Empty"),
            test_gcp_sa_json(&token_uri),
            &[],
            Some(&org_id),
        );
        let (_, Json(empty_resp)) = create_gcp_service_account_key(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(empty_body),
        )
        .await
        .expect("scoped admin may create an unbound org credential");
        let empty_key = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &empty_resp.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(empty_key.user_id, org_id);

        let out_of_scope_body = gcp_sa_body(
            Some("Scoped Admin Out Of Scope"),
            test_gcp_sa_json(&token_uri),
            &["google-out-of-scope"],
            Some(&org_id),
        );
        let (_, Json(bound_resp)) = create_gcp_service_account_key(
            State(state),
            test_auth_user(&admin_id),
            Json(out_of_scope_body),
        )
        .await
        .expect("scoped admin create does not precheck service_slugs scope");

        let rebound = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &out_of_scope_service_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebound.api_key_id.as_deref(), Some(bound_resp.id.as_str()));
        assert_eq!(rebound.auth_method, "bearer");
    }

    #[tokio::test]
    async fn test_create_gcp_service_account_rejects_invalid_json() {
        let Some(db) = connect_test_database("h_ext_gcp_sa_badjson").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);

        let body = gcp_sa_body(None, "not json".to_string(), &[], None);

        let result =
            create_gcp_service_account_key(State(state), test_auth_user(&user_id), Json(body))
                .await;
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    fn poisoned_external_key_response() -> ExternalApiKeyResponse {
        ExternalApiKeyResponse {
            id: uuid::Uuid::new_v4().to_string(),
            label: "Bearer nyxid_ag_abcdefghijklmnopqrst".to_string(),
            credential_type: "api_key".to_string(),
            status: "failed".to_string(),
            provider_config_id: None,
            expires_at: None,
            last_used_at: None,
            error_message: Some("upstream said Bearer nyxid_ag_abcdefghijklmnopqrst".to_string()),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn external_key_evidence_projection_has_no_error_message() {
        let Some(db) = connect_test_database("h_ext_keys_evidence_no_err").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let mut key =
            fixture_external_key(&key_id, &user_id, "Bearer nyxid_ag_abcdefghijklmnopqrst");
        key.error_message = Some("upstream said Bearer nyxid_ag_abcdefghijklmnopqrst".to_string());
        key.status = "failed".to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(key)
            .await
            .unwrap();
        let state = test_app_state(db);

        let Json(evidence) = get_external_api_key_authorization(
            State(state),
            test_auth_user(&user_id),
            Path(key_id.clone()),
        )
        .await
        .unwrap();
        let json = serde_json::to_value(&evidence).unwrap();

        assert_eq!(json["id"], key_id);
        assert_eq!(json["credential_type"], "api_key");
        assert_eq!(json["status"], "failed");
        assert!(
            json.get("error_message").is_none(),
            "evidence must never carry error_message"
        );
        assert!(
            json.get("label").is_none(),
            "evidence must never carry label"
        );
        assert!(json.get("created_at").is_none());
        assert_eq!(json["expires_at"], serde_json::Value::Null);
        assert_eq!(json["last_used_at"], serde_json::Value::Null);
        assert!(json.as_object().unwrap().contains_key("expires_at"));
        assert!(json.as_object().unwrap().contains_key("last_used_at"));
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&json),
            None,
            "external-key evidence must never carry a secret-shaped value"
        );
        let properties: std::collections::BTreeSet<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            properties,
            [
                "credential_type",
                "expires_at",
                "id",
                "last_used_at",
                "status",
                "updated_at",
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<&str>>()
        );
    }

    #[test]
    fn telegram_identity_evidence_uses_pinned_api_key_type() {
        let mut response = poisoned_external_key_response();
        response.credential_type = "telegram_identity".to_string();
        response.label = "safe-label".to_string();
        response.error_message = None;
        let evidence =
            ExternalApiKeyAuthorizationEvidenceResponse::from_external_api_key_response(response)
                .expect("legacy telegram identity rows normalize safely");
        assert_eq!(evidence.credential_type, "api_key");
        assert_eq!(evidence.status, "failed");
        let json = serde_json::to_value(evidence).unwrap();
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&json),
            None
        );
    }

    #[test]
    fn full_external_key_response_trips_the_aevatar_secret_scan() {
        let json = serde_json::to_value(poisoned_external_key_response()).unwrap();
        assert!(
            crate::test_utils::aevatar_secret_free_violation(&json).is_some(),
            "full external-key detail must trip the evidence scan"
        );
    }

    #[test]
    fn external_key_evidence_emits_only_pinned_credential_type_and_status_values() {
        assert_eq!(
            SERVED_EXTERNAL_KEY_CREDENTIAL_TYPES,
            [
                "api_key",
                "oauth2",
                "bearer",
                "basic",
                "ssh_certificate",
                "node_managed",
                "gcp_service_account",
            ]
        );
        assert_eq!(
            SERVED_EXTERNAL_KEY_STATUSES,
            [
                "active",
                "expired",
                "revoked",
                "failed",
                "refresh_failed",
                "pending_auth",
            ]
        );

        let mut seen_types = std::collections::BTreeSet::new();
        let mut seen_statuses = std::collections::BTreeSet::new();
        for credential_type in SERVED_EXTERNAL_KEY_CREDENTIAL_TYPES {
            for status in SERVED_EXTERNAL_KEY_STATUSES {
                let mut response = poisoned_external_key_response();
                response.credential_type = credential_type.to_string();
                response.status = status.to_string();
                response.label = "safe-label".to_string();
                response.error_message = None;
                let evidence =
                    ExternalApiKeyAuthorizationEvidenceResponse::from_external_api_key_response(
                        response,
                    )
                    .unwrap();
                let json = serde_json::to_value(&evidence).unwrap();
                assert_eq!(json["credential_type"], *credential_type);
                assert_eq!(json["status"], *status);
                assert_eq!(
                    crate::test_utils::aevatar_secret_free_violation(&json),
                    None
                );
                seen_types.insert(json["credential_type"].as_str().unwrap().to_string());
                seen_statuses.insert(json["status"].as_str().unwrap().to_string());
            }
        }
        assert_eq!(
            seen_types,
            SERVED_EXTERNAL_KEY_CREDENTIAL_TYPES
                .iter()
                .map(|value| value.to_string())
                .collect()
        );
        assert_eq!(
            seen_statuses,
            SERVED_EXTERNAL_KEY_STATUSES
                .iter()
                .map(|value| value.to_string())
                .collect()
        );

        let mut unknown = poisoned_external_key_response();
        unknown.credential_type = "device_code".to_string();
        unknown.label = "safe".to_string();
        unknown.error_message = None;
        assert!(
            ExternalApiKeyAuthorizationEvidenceResponse::from_external_api_key_response(unknown)
                .is_err(),
            "unknown credential_type must fail closed rather than be emitted"
        );

        let mut unknown_status = poisoned_external_key_response();
        unknown_status.status = "disabled".to_string();
        unknown_status.label = "safe".to_string();
        unknown_status.error_message = None;
        assert!(
            ExternalApiKeyAuthorizationEvidenceResponse::from_external_api_key_response(
                unknown_status
            )
            .is_err(),
            "unknown status must fail closed rather than be emitted"
        );
    }
}
