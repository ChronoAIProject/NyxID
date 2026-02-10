use axum::{extract::State, Json};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Serialize;
use std::collections::HashMap;

use crate::errors::AppResult;
use crate::models::downstream_service::{DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES};
use crate::models::service_endpoint::{ServiceEndpoint, COLLECTION_NAME as SERVICE_ENDPOINTS};
use crate::models::user_service_connection::{
    UserServiceConnection, COLLECTION_NAME as CONNECTIONS,
};
use crate::mw::auth::AuthUser;
use crate::AppState;

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct McpConfigResponse {
    pub user_id: String,
    pub proxy_base_url: String,
    pub services: Vec<McpServiceConfig>,
}

#[derive(Debug, Serialize)]
pub struct McpServiceConfig {
    pub service_id: String,
    pub service_name: String,
    pub service_slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub endpoints: Vec<McpEndpointConfig>,
}

#[derive(Debug, Serialize)]
pub struct McpEndpointConfig {
    pub endpoint_id: String,
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub response_description: Option<String>,
}

// --- Handler ---

/// GET /api/v1/mcp/config
///
/// Returns the MCP tool configuration for the authenticated user.
/// Includes all services the user is connected to, along with their
/// registered endpoints (tools) and the proxy base URL.
pub async fn get_mcp_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<McpConfigResponse>> {
    let user_id = auth_user.user_id.to_string();

    // 1. Get user's active connections
    let connections: Vec<UserServiceConnection> = state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find(doc! { "user_id": &user_id, "is_active": true })
        .await?
        .try_collect()
        .await?;

    let service_ids: Vec<&str> = connections.iter().map(|c| c.service_id.as_str()).collect();

    if service_ids.is_empty() {
        return Ok(Json(McpConfigResponse {
            user_id,
            proxy_base_url: build_proxy_base_url(&state.config.base_url),
            services: vec![],
        }));
    }

    // 2. Fetch matching active downstream services
    let services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! { "_id": { "$in": &service_ids }, "is_active": true })
        .await?
        .try_collect()
        .await?;

    // 3. Fetch active endpoints for all connected services in one query
    let active_service_ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    let all_endpoints: Vec<ServiceEndpoint> = state
        .db
        .collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
        .find(doc! {
            "service_id": { "$in": &active_service_ids },
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;

    // Group endpoints by service_id
    let mut endpoints_by_service: HashMap<&str, Vec<&ServiceEndpoint>> = HashMap::new();
    for ep in &all_endpoints {
        endpoints_by_service
            .entry(ep.service_id.as_str())
            .or_default()
            .push(ep);
    }

    // 4. Build response
    let mcp_services: Vec<McpServiceConfig> = services
        .into_iter()
        .map(|svc| {
            let endpoints = endpoints_by_service
                .get(svc.id.as_str())
                .map(|eps| {
                    eps.iter()
                        .map(|ep| McpEndpointConfig {
                            endpoint_id: ep.id.clone(),
                            name: ep.name.clone(),
                            description: ep.description.clone(),
                            method: ep.method.clone(),
                            path: ep.path.clone(),
                            parameters: ep.parameters.clone(),
                            request_body_schema: ep.request_body_schema.clone(),
                            response_description: ep.response_description.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            McpServiceConfig {
                service_id: svc.id,
                service_name: svc.name,
                service_slug: svc.slug,
                description: svc.description,
                base_url: svc.base_url,
                endpoints,
            }
        })
        .collect();

    Ok(Json(McpConfigResponse {
        user_id,
        proxy_base_url: build_proxy_base_url(&state.config.base_url),
        services: mcp_services,
    }))
}

/// Build the proxy base URL from the backend's base_url config.
fn build_proxy_base_url(base_url: &str) -> String {
    format!("{}/api/v1/proxy", base_url.trim_end_matches('/'))
}
