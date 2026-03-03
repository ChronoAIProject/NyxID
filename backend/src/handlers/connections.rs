use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::crypto::aes;
use crate::errors::AppResult;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user_service_connection::{
    COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, connection_service};

// --- Request types ---

#[derive(Deserialize)]
pub struct ConnectRequest {
    /// The user's credential for this service.
    /// Required for "connection" category services.
    /// Must be None/absent for "internal" category services.
    pub credential: Option<String>,
    /// Optional label for the credential (e.g., "Production Key").
    pub credential_label: Option<String>,
}

impl std::fmt::Debug for ConnectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectRequest")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("credential_label", &self.credential_label)
            .finish()
    }
}

#[derive(Deserialize)]
pub struct UpdateCredentialRequest {
    pub credential: String,
    pub credential_label: Option<String>,
}

impl std::fmt::Debug for UpdateCredentialRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateCredentialRequest")
            .field("credential", &"[REDACTED]")
            .field("credential_label", &self.credential_label)
            .finish()
    }
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct ConnectionItem {
    pub service_id: String,
    pub service_name: String,
    pub service_category: String,
    pub auth_type: Option<String>,
    pub has_credential: bool,
    pub credential_label: Option<String>,
    pub connected_at: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectionListResponse {
    pub connections: Vec<ConnectionItem>,
}

#[derive(Debug, Serialize)]
pub struct ConnectResponse {
    pub service_id: String,
    pub service_name: String,
    pub connected_at: String,
}

#[derive(Debug, Serialize)]
pub struct DisconnectResponse {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct UpdateCredentialResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/connections
///
/// List all active connections for the authenticated user.
pub async fn list_connections(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ConnectionListResponse>> {
    let user_id = auth_user.user_id.to_string();

    let conns: Vec<UserServiceConnection> = state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find(doc! { "user_id": &user_id, "is_active": true })
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

    let items: Vec<ConnectionItem> = conns
        .iter()
        .map(|c| {
            let svc = service_map.get(c.service_id.as_str());
            ConnectionItem {
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

    Ok(Json(ConnectionListResponse { connections: items }))
}

/// POST /api/v1/connections/{service_id}
///
/// Connect the authenticated user to a downstream service.
/// For "connection" services, a credential must be provided in the JSON body.
/// For "internal" services, no credential is needed.
pub async fn connect_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<ConnectRequest>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = connection_service::connect_user(
        &state.db,
        &encryption_key,
        &user_id,
        &service_id,
        body.credential.as_deref(),
        body.credential_label.as_deref(),
    )
    .await?;

    tracing::info!(
        user_id = %user_id,
        service_id = %service_id,
        "User connected to service"
    );

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "connection_created".to_string(),
        Some(serde_json::json!({
            "service_id": &service_id,
            "has_credential": body.credential.is_some(),
        })),
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        service_id,
        service_name: result.service_name,
        connected_at: result.connected_at.to_rfc3339(),
    }))
}

/// PUT /api/v1/connections/{service_id}/credential
///
/// Update the credential on an existing connection.
pub async fn update_connection_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<UpdateCredentialRequest>,
) -> AppResult<Json<UpdateCredentialResponse>> {
    let user_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    connection_service::update_credential(
        &state.db,
        &encryption_key,
        &user_id,
        &service_id,
        &body.credential,
        body.credential_label.as_deref(),
    )
    .await?;

    tracing::info!(
        user_id = %user_id,
        service_id = %service_id,
        "Connection credential updated"
    );

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "connection_credential_updated".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
    );

    Ok(Json(UpdateCredentialResponse {
        message: "Credential updated".to_string(),
    }))
}

/// DELETE /api/v1/connections/{service_id}
///
/// Disconnect the authenticated user from a downstream service.
/// Securely clears the stored credential.
pub async fn disconnect_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<DisconnectResponse>> {
    let user_id = auth_user.user_id.to_string();

    connection_service::disconnect_user(&state.db, &user_id, &service_id).await?;

    tracing::info!(
        user_id = %user_id,
        service_id = %service_id,
        "User disconnected from service"
    );

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "connection_removed".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
    );

    Ok(Json(DisconnectResponse {
        message: "Disconnected from service".to_string(),
    }))
}
