use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
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

    let result = connection_service::connect_user(
        &state.db,
        &state.encryption_keys,
        state.node_ws_manager.as_ref(),
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "connection_created",
        Some(serde_json::json!({
            "service_id": &service_id,
            "has_credential": body.credential.is_some(),
        })),
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

    connection_service::update_credential(
        &state.db,
        &state.encryption_keys,
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "connection_credential_updated",
        Some(serde_json::json!({ "service_id": &service_id })),
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "connection_removed",
        Some(serde_json::json!({ "service_id": &service_id })),
    );

    Ok(Json(DisconnectResponse {
        message: "Disconnected from service".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;
    use crate::test_utils::*;
    use axum::extract::{Path, State};

    async fn insert_internal_service(db: &mongodb::Database, service_id: &str) {
        let mut svc = dummy_service();
        svc.id = service_id.to_string();
        svc.name = "Internal Test".to_string();
        svc.service_category = "internal".to_string();
        svc.requires_user_credential = false;
        svc.service_type = "http".to_string();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&svc)
            .await
            .unwrap();
    }

    async fn insert_connection_service(db: &mongodb::Database, service_id: &str) {
        let mut svc = dummy_service();
        svc.id = service_id.to_string();
        svc.name = "Connection Test".to_string();
        svc.service_category = "connection".to_string();
        svc.requires_user_credential = true;
        svc.service_type = "http".to_string();
        svc.auth_type = Some("api_key".to_string());
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&svc)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_list_connections_empty() {
        let Some(db) = connect_test_database("h_connections_list_empty").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);

        let Json(resp) = list_connections(State(state), test_auth_user(&user_id))
            .await
            .unwrap();
        assert!(resp.connections.is_empty());
    }

    #[tokio::test]
    async fn test_connect_internal_service_no_credential() {
        let Some(db) = connect_test_database("h_connections_connect_internal").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_internal_service(&db, &service_id).await;

        let state = test_app_state(db);

        let Json(resp) = connect_service(
            State(state),
            test_auth_user(&user_id),
            Path(service_id.clone()),
            Json(ConnectRequest {
                credential: None,
                credential_label: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(resp.service_id, service_id);
        assert_eq!(resp.service_name, "Internal Test");
    }

    #[tokio::test]
    async fn test_connect_and_list_connections() {
        let Some(db) = connect_test_database("h_connections_connect_list").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id).await;

        let state = test_app_state(db);

        let _ = connect_service(
            State(state.clone()),
            test_auth_user(&user_id),
            Path(service_id.clone()),
            Json(ConnectRequest {
                credential: Some("sk-test-api-key".to_string()),
                credential_label: Some("Prod Key".to_string()),
            }),
        )
        .await
        .unwrap();

        let Json(list_resp) = list_connections(State(state), test_auth_user(&user_id))
            .await
            .unwrap();
        assert_eq!(list_resp.connections.len(), 1);
        assert_eq!(list_resp.connections[0].service_id, service_id);
        assert!(list_resp.connections[0].has_credential);
        assert_eq!(
            list_resp.connections[0].credential_label.as_deref(),
            Some("Prod Key")
        );
    }

    #[tokio::test]
    async fn test_disconnect_service_success() {
        let Some(db) = connect_test_database("h_connections_disconnect").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_internal_service(&db, &service_id).await;

        let state = test_app_state(db);

        let _ = connect_service(
            State(state.clone()),
            test_auth_user(&user_id),
            Path(service_id.clone()),
            Json(ConnectRequest {
                credential: None,
                credential_label: None,
            }),
        )
        .await
        .unwrap();

        let Json(disc_resp) = disconnect_service(
            State(state.clone()),
            test_auth_user(&user_id),
            Path(service_id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(disc_resp.message, "Disconnected from service");

        let Json(list_resp) = list_connections(State(state), test_auth_user(&user_id))
            .await
            .unwrap();
        assert!(list_resp.connections.is_empty());
    }

    // --- Pure function tests: Debug impls for request types ---

    #[test]
    fn connect_request_debug_redacts_credential() {
        let req = ConnectRequest {
            credential: Some("super-secret-api-key".to_string()),
            credential_label: Some("Production Key".to_string()),
        };
        let debug_output = format!("{:?}", req);

        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-api-key"));
        assert!(debug_output.contains("Production Key"));
    }

    #[test]
    fn connect_request_debug_none_credential_shows_none() {
        let req = ConnectRequest {
            credential: None,
            credential_label: None,
        };
        let debug_output = format!("{:?}", req);

        assert!(debug_output.contains("None"));
        assert!(!debug_output.contains("[REDACTED]"));
    }

    #[test]
    fn update_credential_request_debug_redacts_credential() {
        let req = UpdateCredentialRequest {
            credential: "sk-very-secret".to_string(),
            credential_label: Some("Test Key".to_string()),
        };
        let debug_output = format!("{:?}", req);

        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("sk-very-secret"));
        assert!(debug_output.contains("Test Key"));
    }

    // --- Serialization tests: ConnectionItem ---

    #[test]
    fn connection_item_serialization() {
        let item = ConnectionItem {
            service_id: "svc-1".to_string(),
            service_name: "OpenAI".to_string(),
            service_category: "connection".to_string(),
            auth_type: Some("api_key".to_string()),
            has_credential: true,
            credential_label: Some("Prod Key".to_string()),
            connected_at: "2025-06-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&item).unwrap();

        assert_eq!(json["service_id"], "svc-1");
        assert_eq!(json["service_name"], "OpenAI");
        assert_eq!(json["service_category"], "connection");
        assert_eq!(json["auth_type"], "api_key");
        assert_eq!(json["has_credential"], true);
        assert_eq!(json["credential_label"], "Prod Key");
        assert_eq!(json["connected_at"], "2025-06-01T00:00:00+00:00");
    }

    #[test]
    fn connection_item_serialization_with_null_optionals() {
        let item = ConnectionItem {
            service_id: "svc-2".to_string(),
            service_name: "Internal Service".to_string(),
            service_category: "internal".to_string(),
            auth_type: None,
            has_credential: false,
            credential_label: None,
            connected_at: "2025-01-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&item).unwrap();

        assert_eq!(json["service_category"], "internal");
        assert!(json["auth_type"].is_null());
        assert_eq!(json["has_credential"], false);
        assert!(json["credential_label"].is_null());
    }

    // --- Serialization tests: ConnectionListResponse ---

    #[test]
    fn connection_list_response_serialization_empty() {
        let resp = ConnectionListResponse {
            connections: vec![],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["connections"].as_array().unwrap().is_empty());
    }

    #[test]
    fn connection_list_response_serialization_with_items() {
        let resp = ConnectionListResponse {
            connections: vec![
                ConnectionItem {
                    service_id: "svc-1".to_string(),
                    service_name: "Service A".to_string(),
                    service_category: "connection".to_string(),
                    auth_type: Some("bearer".to_string()),
                    has_credential: true,
                    credential_label: None,
                    connected_at: "2025-01-01T00:00:00+00:00".to_string(),
                },
                ConnectionItem {
                    service_id: "svc-2".to_string(),
                    service_name: "Service B".to_string(),
                    service_category: "internal".to_string(),
                    auth_type: None,
                    has_credential: false,
                    credential_label: None,
                    connected_at: "2025-02-01T00:00:00+00:00".to_string(),
                },
            ],
        };
        let json = serde_json::to_value(&resp).unwrap();
        let connections = json["connections"].as_array().unwrap();
        assert_eq!(connections.len(), 2);
        assert_eq!(connections[0]["service_id"], "svc-1");
        assert_eq!(connections[1]["service_id"], "svc-2");
    }

    // --- Serialization tests: ConnectResponse ---

    #[test]
    fn connect_response_serialization() {
        let resp = ConnectResponse {
            service_id: "svc-abc".to_string(),
            service_name: "Anthropic".to_string(),
            connected_at: "2025-06-15T12:30:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["service_id"], "svc-abc");
        assert_eq!(json["service_name"], "Anthropic");
        assert_eq!(json["connected_at"], "2025-06-15T12:30:00+00:00");
    }

    // --- Serialization tests: DisconnectResponse ---

    #[test]
    fn disconnect_response_serialization() {
        let resp = DisconnectResponse {
            message: "Disconnected from service".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["message"], "Disconnected from service");
    }

    // --- Serialization tests: UpdateCredentialResponse ---

    #[test]
    fn update_credential_response_serialization() {
        let resp = UpdateCredentialResponse {
            message: "Credential updated".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["message"], "Credential updated");
    }

    // --- Deserialization tests: ConnectRequest ---

    #[test]
    fn connect_request_deserialization_with_credential() {
        let json = serde_json::json!({
            "credential": "sk-test-key",
            "credential_label": "My Key"
        });
        let req: ConnectRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.credential.as_deref(), Some("sk-test-key"));
        assert_eq!(req.credential_label.as_deref(), Some("My Key"));
    }

    #[test]
    fn connect_request_deserialization_without_optional_fields() {
        let json = serde_json::json!({});
        let req: ConnectRequest = serde_json::from_value(json).unwrap();
        assert!(req.credential.is_none());
        assert!(req.credential_label.is_none());
    }

    // --- Deserialization tests: UpdateCredentialRequest ---

    #[test]
    fn update_credential_request_deserialization() {
        let json = serde_json::json!({
            "credential": "new-secret-key",
            "credential_label": "Updated Key"
        });
        let req: UpdateCredentialRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.credential, "new-secret-key");
        assert_eq!(req.credential_label.as_deref(), Some("Updated Key"));
    }

    #[test]
    fn update_credential_request_deserialization_without_label() {
        let json = serde_json::json!({
            "credential": "another-key"
        });
        let req: UpdateCredentialRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.credential, "another-key");
        assert!(req.credential_label.is_none());
    }
}
