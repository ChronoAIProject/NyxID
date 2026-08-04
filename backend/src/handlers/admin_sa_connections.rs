use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
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
    require_admin_or_operator(
        &state,
        &auth_user,
        "admin.service_accounts.connections.list",
    )
    .await?;

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
    _headers: HeaderMap,
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

    // Reuse existing connection_service -- pass sa.id as user_id
    let result = connection_service::connect_user(
        &state.db,
        &state.encryption_keys,
        state.node_ws_manager.as_ref(),
        &sa_id,
        &service_id,
        body.credential.as_deref(),
        body.credential_label.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.sa.service_connected",
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "service_id": &service_id,
            "has_credential": body.credential.is_some(),
        })),
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
    _headers: HeaderMap,
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

    connection_service::update_credential(
        &state.db,
        &state.encryption_keys,
        &sa_id,
        &service_id,
        &body.credential,
        body.credential_label.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.sa.service_credential_updated",
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "service_id": &service_id,
        })),
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
    _headers: HeaderMap,
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.sa.service_disconnected",
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "service_id": &service_id,
        })),
    );

    Ok(Json(AdminSaConnectionActionResponse {
        message: "Service disconnected from service account".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::DownstreamService;
    use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{Path, State};
    use axum::http::HeaderMap;
    use chrono::Utc;
    use uuid::Uuid;

    async fn seed_admin(db: &mongodb::Database) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed roles");
        let ids = role_service::get_platform_role_ids(db)
            .await
            .expect("role ids");
        let id = Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.role_ids.push(ids.admin);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert admin");
        id
    }

    fn fixture_sa(admin_id: &str, active: bool) -> ServiceAccount {
        let now = Utc::now();
        ServiceAccount {
            id: Uuid::new_v4().to_string(),
            name: "Test SA".to_string(),
            description: None,
            client_id: format!("sa_{}", hex::encode([2u8; 12])),
            client_secret_hash: "0".repeat(64),
            secret_prefix: "sas_test".to_string(),
            role_ids: vec![],
            allowed_scopes: "proxy:*".to_string(),
            is_active: active,
            rate_limit_override: None,
            created_by: admin_id.to_string(),
            owner_user_id: None,
            created_at: now,
            updated_at: now,
            last_authenticated_at: None,
        }
    }

    fn fixture_downstream_service(svc_id: &str) -> DownstreamService {
        let mut svc = crate::models::downstream_service::test_helpers::dummy_service();
        svc.id = svc_id.to_string();
        svc.slug = format!("svc-{}", &svc_id[..8]);
        svc.name = "Test Downstream".to_string();
        svc.service_category = "internal".to_string();
        svc
    }

    #[tokio::test]
    async fn test_list_sa_connections_empty() {
        let Some(db) = connect_test_database("h_sa_conn_list").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let sa = fixture_sa(&admin_id, true);
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let Json(resp) = list_sa_connections(State(state), auth, Path(sa_id))
            .await
            .expect("list should succeed");

        assert!(resp.connections.is_empty());
    }

    #[tokio::test]
    async fn test_connect_sa_service_internal() {
        let Some(db) = connect_test_database("h_sa_conn_connect").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let sa = fixture_sa(&admin_id, true);
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let svc_id = Uuid::new_v4().to_string();
        let svc = fixture_downstream_service(&svc_id);
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&svc)
            .await
            .expect("insert downstream");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let Json(resp) = connect_sa_service(
            State(state),
            auth,
            HeaderMap::new(),
            Path((sa_id, svc_id)),
            Json(AdminSaConnectRequest {
                credential: None,
                credential_label: None,
            }),
        )
        .await
        .expect("connect should succeed");

        assert_eq!(resp.service_name, "Test Downstream");
    }

    #[tokio::test]
    async fn test_connect_sa_service_inactive_rejected() {
        let Some(db) = connect_test_database("h_sa_conn_inactive").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let sa = fixture_sa(&admin_id, false);
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let err = connect_sa_service(
            State(state),
            auth,
            HeaderMap::new(),
            Path((sa_id, Uuid::new_v4().to_string())),
            Json(AdminSaConnectRequest {
                credential: None,
                credential_label: None,
            }),
        )
        .await
        .expect_err("inactive SA should be rejected");

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_disconnect_sa_service_inactive_rejected() {
        let Some(db) = connect_test_database("h_sa_conn_disc_inactive").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let sa = fixture_sa(&admin_id, false);
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let err = disconnect_sa_service(
            State(state),
            auth,
            HeaderMap::new(),
            Path((sa_id, Uuid::new_v4().to_string())),
        )
        .await
        .expect_err("disconnect on inactive SA should be rejected");

        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
