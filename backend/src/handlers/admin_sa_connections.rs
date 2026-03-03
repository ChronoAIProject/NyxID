use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent, require_admin};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user_service_connection::{
    COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, connection_service, service_account_service};

// --- Request types ---

#[derive(Deserialize)]
pub struct AdminSaConnectRequest {
    /// Required for "connection" category services.
    /// Must be None/absent for "internal" services.
    pub credential: Option<String>,
    pub credential_label: Option<String>,
}

impl std::fmt::Debug for AdminSaConnectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminSaConnectRequest")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("credential_label", &self.credential_label)
            .finish()
    }
}

#[derive(Deserialize)]
pub struct AdminSaUpdateCredentialRequest {
    pub credential: String,
    pub credential_label: Option<String>,
}

impl std::fmt::Debug for AdminSaUpdateCredentialRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminSaUpdateCredentialRequest")
            .field("credential", &"[REDACTED]")
            .field("credential_label", &self.credential_label)
            .finish()
    }
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct AdminSaConnectionItem {
    pub service_id: String,
    pub service_name: String,
    pub service_category: String,
    pub auth_type: Option<String>,
    pub has_credential: bool,
    pub credential_label: Option<String>,
    pub connected_at: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaConnectionListResponse {
    pub connections: Vec<AdminSaConnectionItem>,
}

#[derive(Debug, Serialize)]
pub struct AdminSaConnectResponse {
    pub service_id: String,
    pub service_name: String,
    pub connected_at: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaConnectionActionResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/admin/service-accounts/{sa_id}/connections
///
/// List all active service connections for a service account.
pub async fn list_sa_connections(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(sa_id): Path<String>,
) -> AppResult<Json<AdminSaConnectionListResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    // Query active connections for the SA (same pattern as handlers/connections.rs)
    let conns: Vec<UserServiceConnection> = state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find(doc! { "user_id": &sa_id, "is_active": true })
        .await?
        .try_collect()
        .await?;

    // Gather service details
    let service_ids: Vec<&str> = conns.iter().map(|c| c.service_id.as_str()).collect();
    let services: Vec<DownstreamService> = if service_ids.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": &service_ids } })
            .await?
            .try_collect()
            .await?
    };

    let service_map: std::collections::HashMap<&str, &DownstreamService> =
        services.iter().map(|s| (s.id.as_str(), s)).collect();

    let items: Vec<AdminSaConnectionItem> = conns
        .iter()
        .map(|c| {
            let svc = service_map.get(c.service_id.as_str());
            AdminSaConnectionItem {
                service_id: c.service_id.clone(),
                service_name: svc.map_or("Unknown".to_string(), |s| s.name.clone()),
                service_category: svc
                    .map_or("connection".to_string(), |s| s.service_category.clone()),
                auth_type: svc.and_then(|s| s.auth_type.clone()),
                has_credential: c.credential_encrypted.is_some(),
                credential_label: c.credential_label.clone(),
                connected_at: c.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(AdminSaConnectionListResponse { connections: items }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/connections/{service_id}
///
/// Connect a service account to a downstream service.
/// For "connection" services: credential is required in body.
/// For "internal" services: no credential needed.
pub async fn connect_sa_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, service_id)): Path<(String, String)>,
    Json(body): Json<AdminSaConnectRequest>,
) -> AppResult<Json<AdminSaConnectResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists and is active
    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect services to an inactive service account".to_string(),
        ));
    }

    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    // Reuse existing connection_service -- pass sa.id as user_id
    let result = connection_service::connect_user(
        &state.db,
        &encryption_key,
        &sa_id,
        &service_id,
        body.credential.as_deref(),
        body.credential_label.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.service_connected".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "service_id": &service_id,
            "has_credential": body.credential.is_some(),
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaConnectResponse {
        service_id,
        service_name: result.service_name,
        connected_at: result.connected_at.to_rfc3339(),
    }))
}

/// PUT /api/v1/admin/service-accounts/{sa_id}/connections/{service_id}/credential
///
/// Update the credential on an existing SA service connection.
pub async fn update_sa_connection_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, service_id)): Path<(String, String)>,
    Json(body): Json<AdminSaUpdateCredentialRequest>,
) -> AppResult<Json<AdminSaConnectionActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists and is active (consistent with connect_sa_service)
    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot update credentials on an inactive service account".to_string(),
        ));
    }

    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    connection_service::update_credential(
        &state.db,
        &encryption_key,
        &sa_id,
        &service_id,
        &body.credential,
        body.credential_label.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.service_credential_updated".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "service_id": &service_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaConnectionActionResponse {
        message: "Credential updated".to_string(),
    }))
}

/// DELETE /api/v1/admin/service-accounts/{sa_id}/connections/{service_id}
///
/// Disconnect a service account from a downstream service.
pub async fn disconnect_sa_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, service_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaConnectionActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists and is active (consistent with connect_sa_service
    // and update_sa_connection_credential -- all mutation endpoints on SA
    // connections require an active service account)
    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot disconnect services from an inactive service account".to_string(),
        ));
    }

    connection_service::disconnect_user(&state.db, &sa_id, &service_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.service_disconnected".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "service_id": &service_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaConnectionActionResponse {
        message: "Service disconnected from service account".to_string(),
    }))
}
