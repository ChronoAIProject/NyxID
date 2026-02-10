use axum::{
    extract::{Path, State},
    Json,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Serialize;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES,
};
use crate::models::user_service_connection::{
    UserServiceConnection, COLLECTION_NAME as CONNECTIONS,
};
use crate::mw::auth::AuthUser;
use crate::AppState;

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct ConnectionItem {
    pub service_id: String,
    pub service_name: String,
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

    // Gather service names
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

    let service_map: std::collections::HashMap<&str, &str> = services
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str()))
        .collect();

    let items: Vec<ConnectionItem> = conns
        .iter()
        .map(|c| ConnectionItem {
            service_id: c.service_id.clone(),
            service_name: service_map
                .get(c.service_id.as_str())
                .unwrap_or(&"Unknown")
                .to_string(),
            connected_at: c.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ConnectionListResponse { connections: items }))
}

/// POST /api/v1/connections/{service_id}
///
/// Connect the authenticated user to a downstream service.
pub async fn connect_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id = auth_user.user_id.to_string();

    // Verify service exists and is active
    let service = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": &service_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    // Check if already connected
    let existing = state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find_one(doc! {
            "user_id": &user_id,
            "service_id": &service_id,
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "Already connected to this service".to_string(),
        ));
    }

    let now = Utc::now();
    let conn = UserServiceConnection {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.clone(),
        service_id: service_id.clone(),
        credential_encrypted: None,
        metadata: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .insert_one(&conn)
        .await?;

    tracing::info!(
        user_id = %user_id,
        service_id = %service_id,
        "User connected to service"
    );

    Ok(Json(ConnectResponse {
        service_id,
        service_name: service.name,
        connected_at: now.to_rfc3339(),
    }))
}

/// DELETE /api/v1/connections/{service_id}
///
/// Disconnect the authenticated user from a downstream service.
pub async fn disconnect_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<DisconnectResponse>> {
    let user_id = auth_user.user_id.to_string();
    let now = Utc::now();

    let result = state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .update_one(
            doc! {
                "user_id": &user_id,
                "service_id": &service_id,
                "is_active": true,
            },
            doc! { "$set": {
                "is_active": false,
                "updated_at": mongodb::bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Connection not found".to_string()));
    }

    tracing::info!(
        user_id = %user_id,
        service_id = %service_id,
        "User disconnected from service"
    );

    Ok(Json(DisconnectResponse {
        message: "Disconnected from service".to_string(),
    }))
}
