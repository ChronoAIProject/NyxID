use axum::{Json, extract::State};
use serde::Serialize;

use crate::AppState;
use crate::errors::AppResult;
use crate::mw::auth::AuthUser;
use crate::services::mcp_service;

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct McpConfigResponse {
    pub contract_version: &'static str,
    pub catalog_digest: String,
    pub user_id: String,
    pub proxy_base_url: String,
    pub schema_contract: McpSchemaContract,
    pub services: Vec<McpServiceConfig>,
    pub total_services: usize,
    pub total_endpoints: usize,
    pub diagnostics: McpCatalogDiagnostics,
}

#[derive(Debug, Serialize)]
pub struct McpSchemaContract {
    pub parameters_format: &'static str,
    pub parameter_locations: [&'static str; 4],
    pub parameter_schema_keywords: [&'static str; 5],
    pub request_body_schema_format: &'static str,
    pub request_body_schema_keywords: [&'static str; 16],
    pub required_headers: &'static str,
    pub managed_headers: &'static [&'static str],
    pub managed_header_prefixes: [&'static str; 1],
}

#[derive(Debug, Serialize)]
pub struct McpCatalogDiagnostics {
    pub no_visible_connections: bool,
    pub unavailable_services: usize,
    pub generic_only_services: usize,
    pub invalid_contract_services: usize,
    pub service_scope_restricted: bool,
    pub node_scope_restricted: bool,
    pub node_scope_exclusions_present: bool,
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
    /// Skill names (Ornn or bundled) that teach an agent how to use this
    /// service. Empty when none are recorded.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub recommended_skills: Vec<String>,
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
    pub response: crate::models::service_endpoint::OperationResponseContract,
    pub operation_digest: String,
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
    // and could be handed tools whose only dispatchable routes point at
    // disallowed nodes (twenty-ninth-round Codex P2).
    let scope = if auth_user.allow_all_nodes {
        mcp_service::NodeScope::Unrestricted
    } else {
        mcp_service::NodeScope::Allowed(auth_user.allowed_node_ids.as_slice())
    };

    let service_scope = if auth_user.allow_all_services {
        mcp_service::ServiceScope::Unrestricted
    } else {
        mcp_service::ServiceScope::Allowed(auth_user.allowed_service_ids.as_slice())
    };
    let catalog = mcp_service::load_operation_catalog(
        &state.db,
        state.node_ws_manager.as_ref(),
        &user_id,
        scope,
        service_scope,
    )
    .await?;

    let mcp_services = config_services(&catalog.services);

    let total_endpoints: usize = mcp_services.iter().map(|s| s.endpoints.len()).sum();
    let total_services = mcp_services.len();
    let catalog_digest = mcp_service::operation_catalog_digest(&catalog.services);

    Ok(Json(McpConfigResponse {
        contract_version: "1.0",
        catalog_digest,
        user_id,
        proxy_base_url: build_proxy_base_url(&state.config.base_url),
        schema_contract: McpSchemaContract {
            parameters_format: "openapi-parameter-array-v1",
            parameter_locations: ["path", "query", "header", "cookie"],
            parameter_schema_keywords: ["type", "description", "format", "enum", "default"],
            request_body_schema_format: "json-schema-subset-v1",
            request_body_schema_keywords: [
                "type",
                "properties",
                "required",
                "items",
                "additionalProperties",
                "description",
                "format",
                "enum",
                "default",
                "minimum",
                "maximum",
                "minLength",
                "maxLength",
                "allOf",
                "anyOf",
                "oneOf",
            ],
            required_headers: "parameters entries with in=header and required=true",
            managed_headers: mcp_service::BLOCKED_MCP_HEADER_PARAMETER_NAMES,
            managed_header_prefixes: ["x-nyxid-"],
        },
        services: mcp_services,
        total_services,
        total_endpoints,
        diagnostics: McpCatalogDiagnostics {
            no_visible_connections: catalog.diagnostics.no_visible_connections,
            unavailable_services: catalog.diagnostics.unavailable_services,
            generic_only_services: catalog.diagnostics.generic_only_services,
            invalid_contract_services: catalog.diagnostics.invalid_contract_services,
            service_scope_restricted: catalog.diagnostics.service_scope_restricted,
            node_scope_restricted: catalog.diagnostics.node_scope_restricted,
            node_scope_exclusions_present: catalog.diagnostics.node_scope_exclusions_present,
        },
    }))
}

fn config_services(tool_services: &[mcp_service::McpToolService]) -> Vec<McpServiceConfig> {
    tool_services
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
                    response: ep.response.clone(),
                    operation_digest: mcp_service::endpoint_contract_digest(ep),
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
                recommended_skills: svc.recommended_skills.clone(),
                endpoints,
            }
        })
        .collect()
}

/// Build the proxy base URL from the backend's base_url config.
fn build_proxy_base_url(base_url: &str) -> String {
    format!("{}/api/v1/proxy", base_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::{
        McpCatalogDiagnostics, McpEndpointConfig, McpServiceConfig, build_proxy_base_url,
        config_services,
    };
    use crate::models::service_endpoint::OperationResponseContract;
    use crate::services::mcp_service::{self, McpToolEndpoint, McpToolService, McpToolSource};

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

    fn service_with_schema(schema: serde_json::Value) -> McpServiceConfig {
        McpServiceConfig {
            service_id: "service-1".to_string(),
            service_name: "Example".to_string(),
            service_slug: "example".to_string(),
            description: None,
            service_category: "user_service".to_string(),
            is_user_service: true,
            is_generic_proxy: false,
            recommended_skills: Vec::new(),
            endpoints: vec![McpEndpointConfig {
                endpoint_id: "endpoint-1".to_string(),
                name: "get_item".to_string(),
                description: None,
                method: "GET".to_string(),
                path: "/items/{id}".to_string(),
                parameters: None,
                request_body_schema: Some(schema),
                request_content_type: Some("application/json".to_string()),
                request_body_required: false,
                response_description: None,
                response: OperationResponseContract {
                    content_types: vec!["application/json".to_string()],
                    binary_artifact: Some(false),
                },
                operation_digest: "sha256:endpoint".to_string(),
            }],
        }
    }

    #[test]
    fn catalog_digest_is_canonical_across_object_key_order() {
        let first = service_with_schema(serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}, "b": {"type": "integer"}}
        }));
        let second = service_with_schema(serde_json::json!({
            "properties": {"b": {"type": "integer"}, "a": {"type": "string"}},
            "type": "object"
        }));

        assert_eq!(
            mcp_service::canonical_sha256(serde_json::json!(first)),
            mcp_service::canonical_sha256(serde_json::json!(second))
        );
    }

    #[test]
    fn rest_and_mcp_tool_generation_share_the_same_operation_set() {
        let services = vec![McpToolService {
            service_id: "user-service-1".to_string(),
            service_name: "Example".to_string(),
            service_slug: "example".to_string(),
            description: None,
            service_category: "user_service".to_string(),
            recommended_skills: Vec::new(),
            endpoints: vec![McpToolEndpoint {
                endpoint_id: "endpoint-1".to_string(),
                name: "get_item".to_string(),
                description: None,
                method: "GET".to_string(),
                path: "/items/{id}".to_string(),
                ..Default::default()
            }],
            durable_endpoint_metadata: Default::default(),
            source: McpToolSource::UserManaged {
                user_service_id: "user-service-1".to_string(),
                catalog_service_id: None,
                effective_owner_id: "owner-1".to_string(),
                node_id: None,
                has_server_credential: true,
            },
            executable: true,
            is_generic_proxy: false,
            invalid_openapi_contract: false,
            proxy_operation_policy: None,
        }];

        let rest_operations: Vec<String> = config_services(&services)
            .into_iter()
            .flat_map(|service| {
                service
                    .endpoints
                    .into_iter()
                    .map(move |endpoint| format!("{}__{}", service.service_slug, endpoint.name))
            })
            .collect();
        let mcp_operations: Vec<String> =
            mcp_service::generate_tool_definitions(&services, None, &[])
                .into_iter()
                .filter(|tool| !tool.name.starts_with("nyx__"))
                .map(|tool| tool.name)
                .collect();

        assert_eq!(rest_operations, mcp_operations);
    }

    #[test]
    fn diagnostics_are_typed_and_do_not_serialize_sensitive_details() {
        let diagnostics = McpCatalogDiagnostics {
            no_visible_connections: false,
            unavailable_services: 1,
            generic_only_services: 2,
            invalid_contract_services: 3,
            service_scope_restricted: true,
            node_scope_restricted: true,
            node_scope_exclusions_present: true,
        };

        let value = serde_json::to_value(diagnostics).expect("serialize diagnostics");
        assert_eq!(value["node_scope_exclusions_present"], true);
        assert!(value.get("excluded_service_ids").is_none());
        assert!(value.get("excluded_node_ids").is_none());
        assert!(value.get("credential").is_none());
        assert!(value.get("token").is_none());
    }
}
