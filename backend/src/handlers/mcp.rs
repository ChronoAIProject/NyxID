use axum::{Json, extract::State};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

use crate::AppState;
use crate::errors::AppResult;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_endpoint::{COLLECTION_NAME as SERVICE_ENDPOINTS, ServiceEndpoint};
use crate::models::user_service_connection::{
    COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
};
use crate::mw::auth::AuthUser;
use crate::services::node_routing_service;

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct McpConfigResponse {
    pub user_id: String,
    pub proxy_base_url: String,
    pub services: Vec<McpServiceConfig>,
    pub total_services: usize,
    pub total_endpoints: usize,
}

#[derive(Debug, Serialize)]
pub struct McpServiceConfig {
    pub service_id: String,
    pub service_name: String,
    pub service_slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub service_category: String,
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
    pub request_content_type: Option<String>,
    pub response_description: Option<String>,
}

// --- Handler ---

/// GET /api/v1/mcp/config
///
/// Returns the MCP tool configuration for the authenticated user.
/// Only includes services where the user has a valid connection with
/// satisfied credentials.
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
            total_services: 0,
            total_endpoints: 0,
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

    let node_route_service_ids = node_routing_service::list_routable_service_ids(
        &state.db,
        &user_id,
        state.node_ws_manager.as_ref(),
    )
    .await?;
    let node_route_set: HashSet<&str> = node_route_service_ids
        .iter()
        .map(|service_id| service_id.as_str())
        .collect();

    // 3. Filter: only include services where credentials are satisfied
    let conn_map: HashMap<&str, &UserServiceConnection> = connections
        .iter()
        .map(|c| (c.service_id.as_str(), c))
        .collect();

    let valid_services: Vec<&DownstreamService> = services
        .iter()
        .filter(|svc| {
            // Skip provider services
            if svc.service_category == "provider" {
                return false;
            }
            match conn_map.get(svc.id.as_str()) {
                Some(conn) => {
                    if svc.requires_user_credential {
                        conn.credential_encrypted.is_some()
                            || node_route_set.contains(svc.id.as_str())
                    } else {
                        true
                    }
                }
                None => false,
            }
        })
        .collect();

    // 4. Fetch active endpoints for valid services in one query
    let valid_service_ids: Vec<&str> = valid_services.iter().map(|s| s.id.as_str()).collect();
    let all_endpoints: Vec<ServiceEndpoint> = if valid_service_ids.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .find(doc! {
                "service_id": { "$in": &valid_service_ids },
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?
    };

    // Group endpoints by service_id
    let mut endpoints_by_service: HashMap<&str, Vec<&ServiceEndpoint>> = HashMap::new();
    for ep in &all_endpoints {
        endpoints_by_service
            .entry(ep.service_id.as_str())
            .or_default()
            .push(ep);
    }

    // 5. Build response
    let mcp_services: Vec<McpServiceConfig> = valid_services
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
                            request_content_type: ep.request_content_type.clone(),
                            response_description: ep.response_description.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            McpServiceConfig {
                service_id: svc.id.clone(),
                service_name: svc.name.clone(),
                service_slug: svc.slug.clone(),
                description: svc.description.clone(),
                // TODO(SEC-L1): Consider whether base_url needs to be exposed in the MCP config.
                // The MCP proxy routes through NyxID's proxy endpoint anyway, so base_url may
                // not be needed. Removing it would prevent leaking internal service URLs.
                base_url: svc.base_url.clone(),
                service_category: svc.service_category.clone(),
                endpoints,
            }
        })
        .collect();

    let total_endpoints: usize = mcp_services.iter().map(|s| s.endpoints.len()).sum();
    let total_services = mcp_services.len();

    Ok(Json(McpConfigResponse {
        user_id,
        proxy_base_url: build_proxy_base_url(&state.config.base_url),
        services: mcp_services,
        total_services,
        total_endpoints,
    }))
}

/// Build the proxy base URL from the backend's base_url config.
fn build_proxy_base_url(base_url: &str) -> String {
    format!("{}/api/v1/proxy", base_url.trim_end_matches('/'))
}
