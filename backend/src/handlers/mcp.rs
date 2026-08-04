use axum::{Json, extract::State};
use serde::Serialize;

use crate::AppState;
use crate::errors::AppResult;
use crate::mw::auth::AuthUser;
use crate::services::mcp_service;

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
    pub service_category: String,
    /// true if this is a user-managed service (from AI Services / keys page)
    pub is_user_service: bool,
    /// true if this is a custom endpoint with only a generic proxy tool
    pub is_generic_proxy: bool,
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
    pub request_body_required: bool,
    pub response_description: Option<String>,
}

// --- Handler ---

/// GET /api/v1/mcp/config
///
/// Returns the MCP tool configuration for the authenticated user.
/// Includes both platform services (DownstreamService with valid connections)
/// and user-managed services (UserService from the AI Services / keys page).
pub async fn get_mcp_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<McpConfigResponse>> {
    auth_user.ensure_rest_proxy_access()?;

    let user_id = auth_user.user_id.to_string();

    // Honor the caller's API-key node scope when discovering tools —
    // otherwise a scoped key bootstrapping from this REST endpoint
    // would see a different tool set than the JSON-RPC `tools/list`,
    // and could be handed tools whose only viable routes point at
    // disallowed nodes (twenty-ninth-round Codex P2).
    let scope = if auth_user.allow_all_nodes {
        mcp_service::NodeScope::Unrestricted
    } else {
        mcp_service::NodeScope::Allowed(auth_user.allowed_node_ids.as_slice())
    };

    let tool_services = mcp_service::load_user_tools_scoped(
        &state.db,
        state.node_ws_manager.as_ref(),
        &user_id,
        scope,
    )
    .await?;

    let mcp_services: Vec<McpServiceConfig> = tool_services
        .iter()
        .map(|svc| {
            let endpoints = svc
                .endpoints
                .iter()
                .map(|ep| McpEndpointConfig {
                    endpoint_id: ep.endpoint_id.clone(),
                    name: ep.name.clone(),
                    description: ep.description.clone(),
                    method: ep.method.clone(),
                    path: ep.path.clone(),
                    parameters: ep.parameters.clone(),
                    request_body_schema: ep.request_body_schema.clone(),
                    request_content_type: ep.request_content_type.clone(),
                    request_body_required: ep.request_body_required,
                    response_description: ep.response_description.clone(),
                })
                .collect();

            McpServiceConfig {
                service_id: svc.service_id.clone(),
                service_name: svc.service_name.clone(),
                service_slug: svc.service_slug.clone(),
                description: svc.description.clone(),
                service_category: svc.service_category.clone(),
                is_user_service: svc.source.is_user_service(),
                is_generic_proxy: svc.is_generic_proxy,
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

#[cfg(test)]
mod tests {
    use super::build_proxy_base_url;

    #[test]
    fn proxy_base_url_strips_trailing_slash() {
        assert_eq!(
            build_proxy_base_url("http://localhost:3001/"),
            "http://localhost:3001/api/v1/proxy"
        );
        assert_eq!(
            build_proxy_base_url("http://localhost:3001"),
            "http://localhost:3001/api/v1/proxy"
        );
    }
}
