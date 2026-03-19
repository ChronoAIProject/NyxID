use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_endpoint::{COLLECTION_NAME as SERVICE_ENDPOINTS, ServiceEndpoint};
use crate::models::user_service_connection::{
    COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
};
use crate::services::content_type::{
    is_binary_content_type, is_json_content_type, normalize_content_type, schema_is_binary,
};
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::{connection_service, node_routing_service, proxy_service};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A downstream service with its active endpoints, ready for MCP tool generation.
pub struct McpToolService {
    pub service_id: String,
    pub service_name: String,
    pub service_slug: String,
    pub endpoints: Vec<McpToolEndpoint>,
}

/// A single endpoint within a service.
pub struct McpToolEndpoint {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub request_content_type: Option<String>,
    pub request_body_required: bool,
}

/// An MCP tool definition (name + description + JSON Schema input).
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Load user tools (shared by MCP transport + REST /api/v1/mcp/config)
// ---------------------------------------------------------------------------

/// Fetch the authenticated user's available MCP tools.
///
/// Includes:
/// - Services the user has explicitly connected to (with valid credentials)
/// - Auto-connected services (`requires_user_credential == false`) unless user opted out
///
/// Filters out provider services and connections with unsatisfied credentials.
pub async fn load_user_tools(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
) -> AppResult<Vec<McpToolService>> {
    // 1. All connections for this user (active and inactive, for opt-out detection)
    let connections: Vec<UserServiceConnection> = db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find(doc! { "user_id": user_id })
        .await?
        .try_collect()
        .await?;

    let conn_map: HashMap<&str, &UserServiceConnection> = connections
        .iter()
        .map(|c| (c.service_id.as_str(), c))
        .collect();

    let node_route_service_ids =
        node_routing_service::list_routable_service_ids(db, user_id, node_ws_manager).await?;
    let node_route_set: HashSet<&str> = node_route_service_ids
        .iter()
        .map(|service_id| service_id.as_str())
        .collect();

    // 2. Explicitly connected services (active connections)
    let connected_ids: Vec<&str> = connections
        .iter()
        .filter(|c| c.is_active)
        .map(|c| c.service_id.as_str())
        .collect();

    // 3. Auto-connect: services that don't require user credentials
    let auto_services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! {
            "is_active": true,
            "requires_user_credential": false,
            "service_category": { "$ne": "provider" },
        })
        .await?
        .try_collect()
        .await?;

    // 4. Explicitly connected services
    let connected_services: Vec<DownstreamService> = if connected_ids.is_empty() {
        vec![]
    } else {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": &connected_ids }, "is_active": true })
            .await?
            .try_collect()
            .await?
    };

    // 5. Merge and deduplicate, applying filters
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut valid_services: Vec<&DownstreamService> = Vec::new();

    // Add explicitly connected services (credential check)
    for svc in &connected_services {
        if svc.service_category == "provider" {
            continue;
        }
        if svc.requires_user_credential {
            // Must have credential in connection
            if let Some(conn) = conn_map.get(svc.id.as_str()) {
                if conn.credential_encrypted.is_none() && !node_route_set.contains(svc.id.as_str())
                {
                    continue;
                }
            } else {
                continue;
            }
        }
        if seen_ids.insert(svc.id.clone()) {
            valid_services.push(svc);
        }
    }

    // Add auto-connect services (skip if user opted out)
    for svc in &auto_services {
        if seen_ids.contains(&svc.id) {
            continue; // Already included from explicit connections
        }
        // Check if user has explicitly disconnected (opt-out)
        if let Some(conn) = conn_map.get(svc.id.as_str())
            && !conn.is_active
        {
            continue; // User opted out
        }
        if seen_ids.insert(svc.id.clone()) {
            valid_services.push(svc);
        }
    }

    // 6. Active endpoints for valid services (single batch query)
    let valid_ids: Vec<&str> = valid_services.iter().map(|s| s.id.as_str()).collect();
    let all_endpoints: Vec<ServiceEndpoint> = if valid_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .find(doc! {
                "service_id": { "$in": &valid_ids },
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?
    };

    // 7. Group endpoints by service_id
    let mut eps_by_svc: HashMap<&str, Vec<&ServiceEndpoint>> = HashMap::new();
    for ep in &all_endpoints {
        eps_by_svc
            .entry(ep.service_id.as_str())
            .or_default()
            .push(ep);
    }

    // 8. Assemble result
    let result = valid_services
        .into_iter()
        .map(|svc| {
            let endpoints = eps_by_svc
                .get(svc.id.as_str())
                .map(|eps| {
                    eps.iter()
                        .map(|ep| McpToolEndpoint {
                            name: ep.name.clone(),
                            description: ep.description.clone(),
                            method: ep.method.clone(),
                            path: ep.path.clone(),
                            parameters: ep.parameters.clone(),
                            request_body_schema: ep.request_body_schema.clone(),
                            request_content_type: ep.request_content_type.clone(),
                            request_body_required: ep.request_body_required,
                        })
                        .collect()
                })
                .unwrap_or_default();

            McpToolService {
                service_id: svc.id.clone(),
                service_name: svc.name.clone(),
                service_slug: svc.slug.clone(),
                endpoints,
            }
        })
        .collect();

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tool definition generation
// ---------------------------------------------------------------------------

/// Generate MCP tool definitions from loaded services.
/// Always includes the three `nyx__` meta-tools.
///
/// `activated_service_ids` controls which services' tools are included:
/// - `None` = include all services (backward compat for REST /mcp/config)
/// - `Some(set)` = include only services whose ID is in the set
pub fn generate_tool_definitions(
    services: &[McpToolService],
    activated_service_ids: Option<&HashSet<String>>,
) -> Vec<McpToolDefinition> {
    let mut tools = Vec::new();

    // -- Meta-tools (always present) --
    tools.push(McpToolDefinition {
        name: "nyx__search_tools".to_string(),
        description: "Search connected tools by keyword. Use this when you have many \
            tools and need to find a specific one."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query to filter tools by name or description"
                }
            },
            "required": ["query"]
        }),
    });

    tools.push(McpToolDefinition {
        name: "nyx__discover_services".to_string(),
        description: "Browse available services you can connect to on this NyxID instance. \
            Returns services you are NOT yet connected to."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional search query to filter services by name or description"
                },
                "category": {
                    "type": "string",
                    "enum": ["connection", "internal"],
                    "description": "Optional: filter by service category"
                }
            }
        }),
    });

    tools.push(McpToolDefinition {
        name: "nyx__connect_service".to_string(),
        description: "Connect to an available service. For services requiring credentials \
            (connection type), provide your API key or token."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "service_id": {
                    "type": "string",
                    "description": "The service ID to connect to (from discover_services results)"
                },
                "credential": {
                    "type": "string",
                    "description": "Your API key or credential (required for 'connection' type services)"
                },
                "credential_label": {
                    "type": "string",
                    "description": "Optional label for this credential (e.g., 'Production Key')"
                }
            },
            "required": ["service_id"]
        }),
    });

    tools.push(McpToolDefinition {
        name: "nyx__call_tool".to_string(),
        description: "Execute any connected tool by name. Use nyx__search_tools first to \
            discover available tools and their inputSchema, then invoke them through this \
            tool. Pass the tool_name and arguments_json (a JSON string containing all \
            required parameters from the tool's inputSchema)."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "The full tool name from search results (e.g., 'chrono-graph-service__get_api_graphs_by_graphid_snapshot')"
                },
                "arguments_json": {
                    "type": "string",
                    "description": "A JSON string containing all required arguments for the tool. Check the tool's inputSchema from nyx__search_tools results. Example: '{\"graphId\": \"dbeef00f-f2c7-4447-9686-3a6deba65a72\", \"depth\": 2}'. Pass '{}' if the tool takes no arguments."
                }
            },
            "required": ["tool_name", "arguments_json"]
        }),
    });

    // -- Per-service tools (filtered by activated set) --
    for service in services {
        let included = match activated_service_ids {
            Some(ids) => ids.contains(&service.service_id),
            None => true, // No filter = include all
        };
        if !included {
            continue;
        }
        for endpoint in &service.endpoints {
            let name = format!("{}__{}", service.service_slug, endpoint.name);
            let description = format!(
                "[{}] {}",
                service.service_name,
                endpoint.description.as_deref().unwrap_or(&endpoint.name)
            );
            let input_schema = build_input_schema(endpoint);
            tools.push(McpToolDefinition {
                name,
                description,
                input_schema,
            });
        }
    }

    tools
}

/// Build a JSON Schema `inputSchema` from endpoint parameters and body schema.
/// Ported from the TypeScript `buildInputSchema()` in `mcp-proxy/src/tools.ts`.
fn build_input_schema(endpoint: &McpToolEndpoint) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required: Vec<serde_json::Value> = Vec::new();

    // -- Path / query / header / cookie parameters --
    if let Some(params_value) = &endpoint.parameters
        && let Some(params) = params_value.as_array()
    {
        for param in params {
            let name = match supported_parameter_name_for_mcp(param) {
                Some(n) if !n.is_empty() => n,
                _ => continue,
            };

            let mut schema = serde_json::Map::new();

            if let Some(param_schema) = param.get("schema") {
                let typ = param_schema
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("string");
                schema.insert("type".into(), serde_json::Value::String(typ.into()));

                if let Some(desc) = param_schema.get("description").and_then(|v| v.as_str()) {
                    schema.insert("description".into(), serde_json::Value::String(desc.into()));
                }
                if let Some(fmt) = param_schema.get("format").and_then(|v| v.as_str()) {
                    schema.insert("format".into(), serde_json::Value::String(fmt.into()));
                }
                if let Some(enums) = param_schema.get("enum") {
                    schema.insert("enum".into(), enums.clone());
                }
                if let Some(default) = param_schema.get("default") {
                    schema.insert("default".into(), default.clone());
                }
            }

            // Param-level description overrides schema-level
            if let Some(desc) = param.get("description").and_then(|v| v.as_str()) {
                schema.insert("description".into(), serde_json::Value::String(desc.into()));
            }

            properties.insert(name.to_string(), serde_json::Value::Object(schema));

            if param
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                required.push(serde_json::Value::String(name.to_string()));
            }
        }
    }

    // -- Request body schema --
    let body_mode = request_body_mode(endpoint);
    if let Some(body_schema) = &endpoint.request_body_schema {
        if json_body_is_flattened(endpoint, body_mode, body_schema) {
            // Merge object properties directly into the tool's inputSchema
            if let Some(props) = body_schema.get("properties").and_then(|v| v.as_object()) {
                for (key, value) in props {
                    properties.insert(key.clone(), value.clone());
                }
            }
            if let Some(req_arr) = body_schema.get("required").and_then(|v| v.as_array()) {
                for r in req_arr {
                    if let Some(s) = r.as_str() {
                        push_required(&mut required, s);
                    }
                }
            }
        } else {
            // Non-object body: wrap as a single `body` property
            let body_field_name = request_body_field_name(endpoint);
            let body_prop = build_body_property(endpoint, body_schema, body_mode);
            properties.insert(body_field_name.clone(), body_prop);
            if request_body_is_required(endpoint) {
                push_required(&mut required, &body_field_name);
            }
        }
    } else if endpoint.request_content_type.is_some() {
        let body_field_name = request_body_field_name(endpoint);
        properties.insert(
            body_field_name.clone(),
            build_default_body_property(endpoint, body_mode),
        );
        if request_body_is_required(endpoint) {
            push_required(&mut required, &body_field_name);
        }
    }

    let mut schema = serde_json::json!({
        "type": "object",
        "properties": serde_json::Value::Object(properties),
    });

    if !required.is_empty() {
        schema
            .as_object_mut()
            .unwrap()
            .insert("required".into(), serde_json::Value::Array(required));
    }

    schema
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestBodyMode {
    Json,
    Raw,
    Binary,
    Multipart,
}

fn request_body_mode(endpoint: &McpToolEndpoint) -> RequestBodyMode {
    request_body_mode_for(
        endpoint.request_content_type.as_deref(),
        endpoint.request_body_schema.as_ref(),
    )
}

fn request_body_mode_for(
    content_type: Option<&str>,
    body_schema: Option<&serde_json::Value>,
) -> RequestBodyMode {
    let Some(content_type) = content_type else {
        return if schema_is_binary(body_schema) {
            RequestBodyMode::Binary
        } else {
            RequestBodyMode::Json
        };
    };

    let normalized = normalize_content_type(content_type);
    if normalized.starts_with("multipart/") {
        RequestBodyMode::Multipart
    } else if is_binary_content_type(content_type) || schema_is_binary(body_schema) {
        RequestBodyMode::Binary
    } else if normalized.is_empty() || normalized == "*/*" || is_json_content_type(content_type) {
        RequestBodyMode::Json
    } else {
        RequestBodyMode::Raw
    }
}

fn build_body_property(
    endpoint: &McpToolEndpoint,
    body_schema: &serde_json::Value,
    body_mode: RequestBodyMode,
) -> serde_json::Value {
    match body_mode {
        RequestBodyMode::Json => {
            let mut body_prop = body_schema.clone();
            if let Some(obj) = body_prop.as_object_mut() {
                obj.insert(
                    "description".into(),
                    serde_json::Value::String("Request body".into()),
                );
            }
            body_prop
        }
        RequestBodyMode::Binary => {
            let body_prop = serde_json::json!({
                "type": "string",
                "description": format!(
                    "Base64-encoded binary content for {} request body",
                    request_content_type_or_default(endpoint)
                ),
                "contentEncoding": "base64",
                "contentMediaType": request_content_type_or_default(endpoint),
            });
            body_prop
        }
        RequestBodyMode::Raw => {
            let body_prop = serde_json::json!({
                "type": "string",
                "description": format!(
                    "Raw request body for {}",
                    request_content_type_or_default(endpoint)
                ),
                "contentMediaType": request_content_type_or_default(endpoint),
            });
            body_prop
        }
        RequestBodyMode::Multipart => {
            let body_prop = serde_json::json!({
                "type": "string",
                "description": format!(
                    "multipart/form-data request body for {}. Multipart bodies are not yet supported by the NyxID MCP proxy.",
                    request_content_type_or_default(endpoint)
                ),
                "contentMediaType": request_content_type_or_default(endpoint),
            });
            body_prop
        }
    }
}

fn build_default_body_property(
    endpoint: &McpToolEndpoint,
    body_mode: RequestBodyMode,
) -> serde_json::Value {
    match body_mode {
        RequestBodyMode::Json => serde_json::json!({
            "description": format!(
                "Request body for {}",
                request_content_type_or_default(endpoint)
            ),
        }),
        RequestBodyMode::Binary | RequestBodyMode::Raw | RequestBodyMode::Multipart => {
            build_body_property(endpoint, &serde_json::Value::Null, body_mode)
        }
    }
}

fn push_required(required: &mut Vec<serde_json::Value>, name: &str) {
    let required_value = serde_json::Value::String(name.to_string());
    if !required.contains(&required_value) {
        required.push(required_value);
    }
}

const REQUEST_BODY_FIELD_CANDIDATES: &[&str] = &["body", "request_body", "requestBody", "payload"];

const BLOCKED_MCP_HEADER_PARAMETER_NAMES: &[&str] = &[
    "host",
    "authorization",
    "cookie",
    "set-cookie",
    "transfer-encoding",
    "content-length",
    "connection",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
    "content-type",
    "accept",
];

fn normalize_header_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn is_blocked_mcp_header_parameter(name: &str) -> bool {
    let normalized = normalize_header_name(name);
    normalized.starts_with("x-nyxid-")
        || BLOCKED_MCP_HEADER_PARAMETER_NAMES.contains(&normalized.as_str())
}

fn request_argument_parameter_name(param: &serde_json::Value) -> Option<&str> {
    let name = param.get("name").and_then(|v| v.as_str())?;
    if name.is_empty() {
        return None;
    }

    match param.get("in").and_then(|v| v.as_str()) {
        Some("path" | "query" | "header" | "cookie") => Some(name),
        _ => None,
    }
}

fn request_argument_name_conflicts(param: &serde_json::Value, candidate: &str) -> bool {
    let Some(name) = request_argument_parameter_name(param) else {
        return false;
    };

    match param.get("in").and_then(|v| v.as_str()) {
        Some("header") => normalize_header_name(name) == normalize_header_name(candidate),
        _ => name == candidate,
    }
}

fn supported_parameter_name_for_mcp(param: &serde_json::Value) -> Option<&str> {
    let name = request_argument_parameter_name(param)?;

    match param.get("in").and_then(|v| v.as_str()) {
        Some("header") if is_blocked_mcp_header_parameter(name) => None,
        _ => Some(name),
    }
}

fn request_body_field_name(endpoint: &McpToolEndpoint) -> String {
    for candidate in REQUEST_BODY_FIELD_CANDIDATES {
        let has_collision = endpoint
            .parameters
            .as_ref()
            .and_then(|params| params.as_array())
            .into_iter()
            .flatten()
            .any(|param| request_argument_name_conflicts(param, candidate));

        if !has_collision {
            return (*candidate).to_string();
        }
    }

    let mut suffix = 2;
    loop {
        let candidate = format!("body_{suffix}");
        let has_collision = endpoint
            .parameters
            .as_ref()
            .and_then(|params| params.as_array())
            .into_iter()
            .flatten()
            .any(|param| request_argument_name_conflicts(param, &candidate));

        if !has_collision {
            return candidate;
        }
        suffix += 1;
    }
}

fn endpoint_has_request_body(endpoint: &McpToolEndpoint) -> bool {
    endpoint.request_body_schema.is_some() || endpoint.request_content_type.is_some()
}

fn request_body_is_required(endpoint: &McpToolEndpoint) -> bool {
    endpoint.request_body_required && endpoint_has_request_body(endpoint)
}

fn request_content_type_or_default(endpoint: &McpToolEndpoint) -> &str {
    endpoint
        .request_content_type
        .as_deref()
        .filter(|content_type| has_concrete_content_type(content_type))
        .unwrap_or_else(|| default_content_type_for_mode(request_body_mode(endpoint)))
}

fn default_content_type_for_mode(mode: RequestBodyMode) -> &'static str {
    match mode {
        RequestBodyMode::Json => "application/json",
        RequestBodyMode::Raw => "text/plain",
        RequestBodyMode::Binary => "application/octet-stream",
        RequestBodyMode::Multipart => "multipart/form-data",
    }
}

fn has_concrete_content_type(content_type: &str) -> bool {
    let normalized = normalize_content_type(content_type);
    !normalized.is_empty() && normalized != "*/*"
}

fn request_content_type_header_value(endpoint: &McpToolEndpoint, has_body: bool) -> Option<&str> {
    if has_body {
        Some(request_content_type_or_default(endpoint))
    } else {
        None
    }
}

fn build_downstream_request_headers(
    endpoint: &McpToolEndpoint,
    has_body: bool,
) -> AppResult<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(content_type) = request_content_type_header_value(endpoint, has_body) {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            content_type.parse().map_err(|e| {
                AppError::Internal(format!(
                    "Invalid request content type configured for endpoint {}: {e}",
                    endpoint.name
                ))
            })?,
        );
    }

    Ok(headers)
}

// ---------------------------------------------------------------------------
// Tool resolution
// ---------------------------------------------------------------------------

/// Parse a tool name (`{slug}__{endpoint_name}`) and find the matching
/// service + endpoint from the loaded services.
pub fn resolve_tool_call<'a>(
    name: &str,
    services: &'a [McpToolService],
) -> Option<(&'a McpToolService, &'a McpToolEndpoint)> {
    let separator = name.find("__")?;
    let service_slug = &name[..separator];
    let endpoint_name = &name[separator + 2..];

    let service = services.iter().find(|s| s.service_slug == service_slug)?;
    let endpoint = service.endpoints.iter().find(|e| e.name == endpoint_name)?;

    Some((service, endpoint))
}

// ---------------------------------------------------------------------------
// Proxy argument building (ported from TypeScript buildProxyArgs)
// ---------------------------------------------------------------------------

type ProxyArgs = (
    reqwest::Method,
    String,
    Option<String>,
    Vec<(String, String)>,
    Option<bytes::Bytes>,
);

/// Build the HTTP method, path, query string, and body for a proxy request
/// from the endpoint definition and the MCP tool call arguments.
pub fn build_proxy_args(
    endpoint: &McpToolEndpoint,
    args: &serde_json::Value,
) -> AppResult<ProxyArgs> {
    let mut path = endpoint.path.trim_start_matches('/').to_string();
    let mut query_params: Vec<(String, String)> = Vec::new();
    let mut header_params: Vec<(String, String)> = Vec::new();
    let mut cookie_params: Vec<(String, String)> = Vec::new();
    let mut body_fields: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();

    // Classify parameters
    let mut path_params = HashSet::new();
    let mut query_param_names = HashSet::new();
    let mut header_param_names = HashSet::new();
    let mut header_param_lookup: HashMap<String, String> = HashMap::new();
    let mut cookie_param_names = HashSet::new();
    let mut blocked_header_param_names = HashSet::new();
    let mut required_path_params = HashSet::new();
    let mut required_query_params = HashSet::new();
    let mut required_header_params = HashSet::new();
    let mut required_cookie_params = HashSet::new();
    let mut provided_path_params = HashSet::new();
    let mut provided_query_params = HashSet::new();
    let mut provided_header_params = HashSet::new();
    let mut provided_cookie_params = HashSet::new();

    if let Some(params_value) = &endpoint.parameters
        && let Some(params) = params_value.as_array()
    {
        for param in params {
            let name = param.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let is_required = param
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match param.get("in").and_then(|v| v.as_str()).unwrap_or("") {
                "path" => {
                    path_params.insert(name.to_string());
                    if is_required {
                        required_path_params.insert(name.to_string());
                    }
                }
                "query" => {
                    query_param_names.insert(name.to_string());
                    if is_required {
                        required_query_params.insert(name.to_string());
                    }
                }
                "header" => {
                    if is_blocked_mcp_header_parameter(name) {
                        blocked_header_param_names.insert(normalize_header_name(name));
                    } else {
                        reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
                            AppError::Internal(format!(
                                "Invalid header parameter configured for endpoint {}: {} ({e})",
                                endpoint.name, name
                            ))
                        })?;
                        header_param_names.insert(name.to_string());
                        header_param_lookup.insert(normalize_header_name(name), name.to_string());
                        if is_required {
                            required_header_params.insert(name.to_string());
                        }
                    }
                }
                "cookie" => {
                    cookie_param_names.insert(name.to_string());
                    if is_required {
                        required_cookie_params.insert(name.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(args_obj) = args.as_object() {
        for (key, value) in args_obj {
            let str_value = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let normalized_header_key = normalize_header_name(key);

            if path_params.contains(key.as_str()) {
                path = path.replace(&format!("{{{key}}}"), &urlencoding::encode(&str_value));
                provided_path_params.insert(key.clone());
            } else if query_param_names.contains(key.as_str()) {
                query_params.push((key.clone(), str_value));
                provided_query_params.insert(key.clone());
            } else if header_param_names.contains(key.as_str()) {
                header_params.push((key.clone(), str_value));
                provided_header_params.insert(key.clone());
            } else if let Some(header_name) = header_param_lookup.get(&normalized_header_key) {
                header_params.push((header_name.clone(), str_value));
                provided_header_params.insert(header_name.clone());
            } else if cookie_param_names.contains(key.as_str()) {
                cookie_params.push((key.clone(), str_value));
                provided_cookie_params.insert(key.clone());
            } else if blocked_header_param_names.contains(&normalized_header_key) {
                return Err(AppError::BadRequest(format!(
                    "Header parameter `{key}` is reserved and cannot be set through the NyxID MCP proxy"
                )));
            } else {
                body_fields.insert(key.clone(), value.clone());
            }
        }
    }

    let query = if query_params.is_empty() {
        None
    } else {
        let qs: Vec<String> = query_params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect();
        Some(qs.join("&"))
    };

    if let Some(error) = missing_required_parameter_error(
        endpoint,
        &required_path_params,
        &provided_path_params,
        "path",
    )
    .or_else(|| {
        missing_required_parameter_error(
            endpoint,
            &required_query_params,
            &provided_query_params,
            "query",
        )
    })
    .or_else(|| {
        missing_required_parameter_error(
            endpoint,
            &required_header_params,
            &provided_header_params,
            "header",
        )
    })
    .or_else(|| {
        missing_required_parameter_error(
            endpoint,
            &required_cookie_params,
            &provided_cookie_params,
            "cookie",
        )
    }) {
        return Err(error);
    }

    if let Some(error) = unresolved_path_parameter_error(endpoint, &path) {
        return Err(error);
    }

    let method = match endpoint.method.to_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => reqwest::Method::GET,
    };

    let parameter_headers = build_parameter_headers(endpoint, header_params, cookie_params)?;
    let body = build_request_body(endpoint, body_fields)?;

    Ok((method, path, query, parameter_headers, body))
}

fn build_request_body(
    endpoint: &McpToolEndpoint,
    body_fields: serde_json::Map<String, serde_json::Value>,
) -> AppResult<Option<bytes::Bytes>> {
    if body_fields.is_empty() {
        if request_body_is_required(endpoint) {
            return Err(missing_required_request_body_error(endpoint));
        }
        return Ok(None);
    }

    if !endpoint_has_request_body(endpoint) {
        return Err(unexpected_request_body_error(endpoint, &body_fields));
    }

    match request_body_mode(endpoint) {
        RequestBodyMode::Json => {
            let body_value = if json_body_uses_wrapper(endpoint) {
                extract_body_field(body_fields, endpoint, "a JSON value")?
            } else {
                serde_json::Value::Object(body_fields)
            };
            let bytes = serde_json::to_vec(&body_value).map_err(|e| {
                AppError::BadRequest(format!("Failed to serialize request body as JSON: {e}"))
            })?;
            Ok(Some(bytes::Bytes::from(bytes)))
        }
        RequestBodyMode::Binary => {
            let body = extract_body_field(body_fields, endpoint, "a base64-encoded string")?;
            let body_field_name = request_body_field_name(endpoint);
            let encoded = body.as_str().ok_or_else(|| {
                AppError::BadRequest(format!(
                    "Request body for {} must be a base64-encoded string in the `{}` field",
                    request_content_type_or_default(endpoint),
                    body_field_name
                ))
            })?;

            use base64::Engine as _;

            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|e| {
                    AppError::BadRequest(format!(
                        "Failed to decode base64 body for {}: {e}",
                        request_content_type_or_default(endpoint)
                    ))
                })?;

            Ok(Some(bytes::Bytes::from(decoded)))
        }
        RequestBodyMode::Raw => {
            let body = extract_body_field(body_fields, endpoint, "a raw string")?;
            let body_field_name = request_body_field_name(endpoint);
            let text = body.as_str().ok_or_else(|| {
                AppError::BadRequest(format!(
                    "Request body for {} must be a raw string in the `{}` field",
                    request_content_type_or_default(endpoint),
                    body_field_name
                ))
            })?;

            Ok(Some(bytes::Bytes::from(text.to_owned())))
        }
        RequestBodyMode::Multipart => {
            let body_field_name = request_body_field_name(endpoint);
            Err(AppError::BadRequest(format!(
                "multipart/form-data request bodies are not yet supported by the NyxID MCP proxy for {}. Use the `{}` field for the body payload when support is added.",
                request_content_type_or_default(endpoint),
                body_field_name
            )))
        }
    }
}

fn missing_required_request_body_error(endpoint: &McpToolEndpoint) -> AppError {
    match request_body_mode(endpoint) {
        RequestBodyMode::Json if !json_body_uses_wrapper(endpoint) => {
            AppError::BadRequest(format!(
                "Request body for {} must include at least one body field",
                request_content_type_or_default(endpoint)
            ))
        }
        RequestBodyMode::Json => AppError::BadRequest(format!(
            "Request body for {} must be provided as a JSON value in the `{}` field",
            request_content_type_or_default(endpoint),
            request_body_field_name(endpoint)
        )),
        RequestBodyMode::Binary => AppError::BadRequest(format!(
            "Request body for {} must be provided as a base64-encoded string in the `{}` field",
            request_content_type_or_default(endpoint),
            request_body_field_name(endpoint)
        )),
        RequestBodyMode::Raw => AppError::BadRequest(format!(
            "Request body for {} must be provided as a raw string in the `{}` field",
            request_content_type_or_default(endpoint),
            request_body_field_name(endpoint)
        )),
        RequestBodyMode::Multipart => AppError::BadRequest(format!(
            "multipart/form-data request bodies are not yet supported by the NyxID MCP proxy for {}. Use the `{}` field for the body payload when support is added.",
            request_content_type_or_default(endpoint),
            request_body_field_name(endpoint)
        )),
    }
}

fn unexpected_request_body_error(
    endpoint: &McpToolEndpoint,
    body_fields: &serde_json::Map<String, serde_json::Value>,
) -> AppError {
    let mut field_names: Vec<&str> = body_fields.keys().map(String::as_str).collect();
    field_names.sort_unstable();

    AppError::BadRequest(format!(
        "Endpoint {} does not define a request body, but received unexpected argument(s): {}",
        endpoint.name,
        field_names.join(", ")
    ))
}

fn missing_required_parameter_error(
    endpoint: &McpToolEndpoint,
    required_params: &HashSet<String>,
    provided_params: &HashSet<String>,
    location: &str,
) -> Option<AppError> {
    let mut missing_params: Vec<&str> = required_params
        .iter()
        .filter(|name| !provided_params.contains(name.as_str()))
        .map(String::as_str)
        .collect();

    if missing_params.is_empty() {
        return None;
    }

    missing_params.sort_unstable();
    Some(AppError::BadRequest(format!(
        "Endpoint {} is missing required {} parameter(s): {}",
        endpoint.name,
        location,
        missing_params.join(", ")
    )))
}

fn unresolved_path_parameter_error(endpoint: &McpToolEndpoint, path: &str) -> Option<AppError> {
    let mut remaining = path;
    let mut unresolved_params = Vec::new();

    while let Some(start) = remaining.find('{') {
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find('}') else {
            break;
        };

        let name = after_start[..end].trim();
        if !name.is_empty() && !name.contains('/') {
            unresolved_params.push(name.to_string());
        }

        remaining = &after_start[end + 1..];
    }

    if unresolved_params.is_empty() {
        return None;
    }

    unresolved_params.sort_unstable();
    unresolved_params.dedup();

    Some(AppError::BadRequest(format!(
        "Endpoint {} has unresolved path parameter(s): {}",
        endpoint.name,
        unresolved_params.join(", ")
    )))
}

fn extract_body_field(
    mut body_fields: serde_json::Map<String, serde_json::Value>,
    endpoint: &McpToolEndpoint,
    expected_shape: &str,
) -> AppResult<serde_json::Value> {
    let body_field_name = request_body_field_name(endpoint);
    if body_fields.len() == 1 && body_fields.contains_key(&body_field_name) {
        return Ok(body_fields.remove(&body_field_name).unwrap());
    }

    Err(AppError::BadRequest(format!(
        "Request body for {} must be provided as {} in the `{}` field",
        request_content_type_or_default(endpoint),
        expected_shape,
        body_field_name
    )))
}

fn build_parameter_headers(
    endpoint: &McpToolEndpoint,
    header_params: Vec<(String, String)>,
    cookie_params: Vec<(String, String)>,
) -> AppResult<Vec<(String, String)>> {
    let mut headers =
        Vec::with_capacity(header_params.len() + usize::from(!cookie_params.is_empty()));

    for (name, value) in header_params {
        reqwest::header::HeaderValue::from_str(&value).map_err(|e| {
            AppError::BadRequest(format!(
                "Invalid value for header parameter `{name}` on endpoint {}: {e}",
                endpoint.name
            ))
        })?;
        headers.push((name, value));
    }

    if !cookie_params.is_empty() {
        let mut cookie_pairs = Vec::with_capacity(cookie_params.len());
        for (name, value) in cookie_params {
            if value.contains(';') {
                return Err(AppError::BadRequest(format!(
                    "Cookie parameter `{name}` on endpoint {} cannot contain `;`",
                    endpoint.name
                )));
            }
            cookie_pairs.push(format!("{name}={value}"));
        }

        let cookie_header = cookie_pairs.join("; ");
        reqwest::header::HeaderValue::from_str(&cookie_header).map_err(|e| {
            AppError::BadRequest(format!(
                "Invalid cookie parameters for endpoint {}: {e}",
                endpoint.name
            ))
        })?;
        headers.push((reqwest::header::COOKIE.as_str().to_string(), cookie_header));
    }

    Ok(headers)
}

fn json_body_is_flattened(
    endpoint: &McpToolEndpoint,
    body_mode: RequestBodyMode,
    body_schema: &serde_json::Value,
) -> bool {
    if !request_body_is_required(endpoint)
        || !matches!(body_mode, RequestBodyMode::Json)
        || body_schema.get("type").and_then(|v| v.as_str()) != Some("object")
    {
        return false;
    }

    let Some(properties) = body_schema.get("properties").and_then(|v| v.as_object()) else {
        return false;
    };

    let has_param_collision = endpoint
        .parameters
        .as_ref()
        .and_then(|params| params.as_array())
        .into_iter()
        .flatten()
        .any(|param| {
            properties
                .keys()
                .any(|name| request_argument_name_conflicts(param, name))
        });

    !has_param_collision
}

fn json_body_uses_wrapper(endpoint: &McpToolEndpoint) -> bool {
    let body_mode = request_body_mode(endpoint);

    if let Some(body_schema) = endpoint.request_body_schema.as_ref() {
        !json_body_is_flattened(endpoint, body_mode, body_schema)
    } else {
        endpoint.request_content_type.is_some() && matches!(body_mode, RequestBodyMode::Json)
    }
}

// ---------------------------------------------------------------------------
// Tool execution
// ---------------------------------------------------------------------------

/// Execute a resolved tool by calling `proxy_service` directly (no HTTP self-call).
/// Returns (http_status, response_body).
///
/// Builds identity headers and resolves delegated credentials (CR-8),
/// matching the behavior of `handlers/proxy.rs`.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool(
    http_client: &reqwest::Client,
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
    arguments: &serde_json::Value,
    jwt_keys: &crate::crypto::jwt::JwtKeys,
    config: &crate::config::AppConfig,
) -> AppResult<(u16, String)> {
    use crate::models::user::{COLLECTION_NAME as USERS, User};
    use crate::services::{delegation_service, identity_service};
    use mongodb::bson::doc;

    let (method, path, query, parameter_headers, body) = build_proxy_args(endpoint, arguments)?;

    let target =
        proxy_service::resolve_proxy_target(db, encryption_keys, user_id, &service.service_id)
            .await?;

    // Build identity headers if configured on the service (CR-8)
    let mut identity_headers = Vec::new();
    if target.service.identity_propagation_mode != "none" {
        let user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": user_id })
            .await?;

        if let Some(ref user) = user {
            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "headers" | "both"
            ) {
                identity_headers = identity_service::build_identity_headers(user, &target.service);
            }

            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "jwt" | "both"
            ) {
                match identity_service::generate_identity_assertion(
                    jwt_keys,
                    config,
                    user,
                    &target.service,
                ) {
                    Ok(assertion) => {
                        identity_headers.push(("X-NyxID-Identity-Token".to_string(), assertion));
                    }
                    Err(e) => {
                        tracing::warn!(
                            service_id = %service.service_id,
                            error = %e,
                            "Failed to generate identity assertion for MCP tool"
                        );
                    }
                }
            }
        }

        // Resolve user RBAC and inject as headers so downstream services can
        // enforce permission checks without needing JWT verification.
        match crate::services::rbac_helpers::resolve_user_rbac(db, user_id).await {
            Ok(rbac) => {
                if !rbac.role_slugs.is_empty() {
                    identity_headers
                        .push(("X-NyxID-User-Roles".to_string(), rbac.role_slugs.join(",")));
                }
                if !rbac.permissions.is_empty() {
                    identity_headers.push((
                        "X-NyxID-User-Permissions".to_string(),
                        rbac.permissions.join(","),
                    ));
                }
                if !rbac.group_slugs.is_empty() {
                    identity_headers.push((
                        "X-NyxID-User-Groups".to_string(),
                        rbac.group_slugs.join(","),
                    ));
                }
            }
            Err(e) => {
                tracing::warn!(
                    user_id = %user_id,
                    error = %e,
                    "Failed to resolve RBAC for delegation headers"
                );
            }
        }
    }

    identity_headers.extend(parameter_headers);

    // Resolve delegated credentials. Required provider connections must succeed.
    let delegated = delegation_service::resolve_delegated_credentials(
        db,
        encryption_keys,
        user_id,
        &service.service_id,
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("Provider credentials not available: {e}")))?;

    // Only set Content-Type when a payload is present. Do not force Accept:
    // downstream content negotiation is endpoint-specific, and MCP tool
    // results are already normalized back into text by the transport layer.
    let headers = build_downstream_request_headers(endpoint, body.is_some())?;

    let response = proxy_service::forward_request(
        http_client,
        &target,
        method,
        &path,
        query.as_deref(),
        headers,
        body,
        identity_headers,
        delegated,
    )
    .await?;

    let status = response.status().as_u16();
    let body_text = response.text().await.map_err(|e| {
        tracing::error!("Failed to read downstream response: {e}");
        crate::errors::AppError::Internal("Failed to read downstream response".to_string())
    })?;

    Ok((status, body_text))
}

// ---------------------------------------------------------------------------
// Meta-tool: nyx__search_tools
// ---------------------------------------------------------------------------

const MAX_SEARCH_RESULTS: usize = 25;

/// Result of searching all tools across all services.
pub struct SearchResult {
    pub matches: Vec<McpToolDefinition>,
    /// Service IDs that had matching tools (for activation).
    pub matched_service_ids: Vec<String>,
}

/// Search ALL user tools (regardless of activation state) and return matches
/// plus the service IDs they belong to.
pub fn search_all_tools(services: &[McpToolService], query: &str) -> SearchResult {
    let q_lower = query.to_lowercase();
    let mut matches = Vec::new();
    let mut matched_ids: HashSet<String> = HashSet::new();

    for service in services {
        for endpoint in &service.endpoints {
            let name = format!("{}__{}", service.service_slug, endpoint.name);
            let description = format!(
                "[{}] {}",
                service.service_name,
                endpoint.description.as_deref().unwrap_or(&endpoint.name),
            );

            if name.to_lowercase().contains(&q_lower)
                || description.to_lowercase().contains(&q_lower)
            {
                matched_ids.insert(service.service_id.clone());
                matches.push(McpToolDefinition {
                    name,
                    description,
                    input_schema: build_input_schema(endpoint),
                });
                if matches.len() >= MAX_SEARCH_RESULTS {
                    break;
                }
            }
        }
        if matches.len() >= MAX_SEARCH_RESULTS {
            break;
        }
    }

    SearchResult {
        matches,
        matched_service_ids: matched_ids.into_iter().collect(),
    }
}

// ---------------------------------------------------------------------------
// Meta-tool: nyx__discover_services
// ---------------------------------------------------------------------------

/// List services the user has NOT yet connected to.
pub async fn discover_services(
    db: &mongodb::Database,
    user_id: &str,
    query: Option<&str>,
    category: Option<&str>,
) -> AppResult<serde_json::Value> {
    let connections: Vec<UserServiceConnection> = db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find(doc! { "user_id": user_id, "is_active": true })
        .await?
        .try_collect()
        .await?;

    let connected_ids: HashSet<&str> = connections.iter().map(|c| c.service_id.as_str()).collect();

    let mut filter = doc! { "is_active": true, "service_category": { "$ne": "provider" } };
    if let Some(cat) = category {
        if cat == "provider" {
            return Ok(serde_json::json!({ "services": [], "count": 0 }));
        }
        filter.insert("service_category", cat);
    }

    let all_services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .await?
        .try_collect()
        .await?;

    let results: Vec<serde_json::Value> = all_services
        .iter()
        .filter(|svc| {
            if connected_ids.contains(svc.id.as_str()) {
                return false;
            }
            match query {
                None => true,
                Some(q) => {
                    let q_lower = q.to_lowercase();
                    svc.name.to_lowercase().contains(&q_lower)
                        || svc.slug.to_lowercase().contains(&q_lower)
                        || svc
                            .description
                            .as_deref()
                            .is_some_and(|d| d.to_lowercase().contains(&q_lower))
                }
            }
        })
        .map(|svc| {
            serde_json::json!({
                "service_id": svc.id,
                "name": svc.name,
                "slug": svc.slug,
                "description": svc.description,
                "category": svc.service_category,
                "requires_credential": svc.requires_user_credential,
            })
        })
        .collect();

    let count = results.len();
    Ok(serde_json::json!({ "services": results, "count": count }))
}

// ---------------------------------------------------------------------------
// Meta-tool: nyx__connect_service
// ---------------------------------------------------------------------------

/// Connect the user to a service from within the MCP client.
pub async fn connect_service(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &crate::services::node_ws_manager::NodeWsManager,
    user_id: &str,
    service_id: &str,
    credential: Option<&str>,
    credential_label: Option<&str>,
) -> AppResult<serde_json::Value> {
    let result = connection_service::connect_user(
        db,
        encryption_keys,
        node_ws_manager,
        user_id,
        service_id,
        credential,
        credential_label,
    )
    .await?;

    Ok(serde_json::json!({
        "status": "connected",
        "service_name": result.service_name,
        "connected_at": result.connected_at.to_rfc3339(),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_endpoint(name: &str, description: &str) -> McpToolEndpoint {
        McpToolEndpoint {
            name: name.to_string(),
            description: Some(description.to_string()),
            method: "GET".to_string(),
            path: format!("/{name}"),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        }
    }

    fn make_service(
        id: &str,
        name: &str,
        slug: &str,
        endpoints: Vec<McpToolEndpoint>,
    ) -> McpToolService {
        McpToolService {
            service_id: id.to_string(),
            service_name: name.to_string(),
            service_slug: slug.to_string(),
            endpoints,
        }
    }

    // -- search_all_tools tests --

    #[test]
    fn search_all_tools_empty_query_matches_everything() {
        let services = vec![make_service(
            "svc-1",
            "Weather",
            "weather",
            vec![make_endpoint("get_forecast", "Get weather forecast")],
        )];

        let result = search_all_tools(&services, "");
        // Empty string is contained in everything, so all tools should match
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matched_service_ids.len(), 1);
    }

    #[test]
    fn search_all_tools_respects_max_results() {
        let endpoints: Vec<McpToolEndpoint> = (0..30)
            .map(|i| make_endpoint(&format!("ep_{i}"), &format!("Endpoint {i} does stuff")))
            .collect();
        let services = vec![make_service("svc-1", "BigService", "big", endpoints)];

        let result = search_all_tools(&services, "stuff");
        assert_eq!(result.matches.len(), MAX_SEARCH_RESULTS);
    }

    #[test]
    fn search_all_tools_multi_service_matching() {
        let services = vec![
            make_service(
                "svc-1",
                "Weather",
                "weather",
                vec![make_endpoint("get_forecast", "Get weather forecast")],
            ),
            make_service(
                "svc-2",
                "News",
                "news",
                vec![make_endpoint(
                    "get_weather_news",
                    "Get weather-related news",
                )],
            ),
        ];

        let result = search_all_tools(&services, "weather");
        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.matched_service_ids.len(), 2);
        assert!(result.matched_service_ids.contains(&"svc-1".to_string()));
        assert!(result.matched_service_ids.contains(&"svc-2".to_string()));
    }

    #[test]
    fn search_all_tools_no_match() {
        let services = vec![make_service(
            "svc-1",
            "Weather",
            "weather",
            vec![make_endpoint("get_forecast", "Get weather forecast")],
        )];

        let result = search_all_tools(&services, "zzz_nonexistent_zzz");
        assert!(result.matches.is_empty());
        assert!(result.matched_service_ids.is_empty());
    }

    // -- generate_tool_definitions tests --

    #[test]
    fn generate_tool_definitions_with_empty_activation_set() {
        let services = vec![make_service(
            "svc-1",
            "Weather",
            "weather",
            vec![make_endpoint("get_forecast", "Get weather forecast")],
        )];

        let empty_set = HashSet::new();
        let tools = generate_tool_definitions(&services, Some(&empty_set));

        // Should only have the 4 meta-tools
        assert_eq!(tools.len(), 4);
        assert!(tools.iter().all(|t| t.name.starts_with("nyx__")));
    }

    #[test]
    fn generate_tool_definitions_with_subset_activation() {
        let services = vec![
            make_service(
                "svc-1",
                "Weather",
                "weather",
                vec![make_endpoint("get_forecast", "Get weather forecast")],
            ),
            make_service(
                "svc-2",
                "News",
                "news",
                vec![make_endpoint("headlines", "Get headlines")],
            ),
        ];

        let mut activated = HashSet::new();
        activated.insert("svc-1".to_string());
        let tools = generate_tool_definitions(&services, Some(&activated));

        // 4 meta-tools + 1 weather tool (news excluded)
        assert_eq!(tools.len(), 5);
        assert!(tools.iter().any(|t| t.name == "weather__get_forecast"));
        assert!(!tools.iter().any(|t| t.name == "news__headlines"));
    }

    #[test]
    fn generate_tool_definitions_with_none_returns_all() {
        let services = vec![
            make_service(
                "svc-1",
                "Weather",
                "weather",
                vec![make_endpoint("get_forecast", "Get weather forecast")],
            ),
            make_service(
                "svc-2",
                "News",
                "news",
                vec![make_endpoint("headlines", "Get headlines")],
            ),
        ];

        let tools = generate_tool_definitions(&services, None);

        // 4 meta-tools + 2 service tools
        assert_eq!(tools.len(), 6);
        assert!(tools.iter().any(|t| t.name == "weather__get_forecast"));
        assert!(tools.iter().any(|t| t.name == "news__headlines"));
    }

    #[test]
    fn build_input_schema_uses_base64_string_for_binary_bodies() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(schema["properties"]["body"]["type"], "string");
        assert_eq!(schema["properties"]["body"]["contentEncoding"], "base64");
        assert_eq!(
            schema["properties"]["body"]["contentMediaType"],
            "application/zip"
        );
        assert!(
            schema["properties"]["body"]["description"]
                .as_str()
                .unwrap()
                .contains("Base64-encoded binary")
        );
        assert_eq!(schema["required"], serde_json::json!(["body"]));
    }

    #[test]
    fn build_input_schema_wraps_non_json_object_bodies() {
        let endpoint = McpToolEndpoint {
            name: "submit_xml".to_string(),
            description: Some("Submit XML".to_string()),
            method: "POST".to_string(),
            path: "/xml".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "note": { "type": "string" }
                },
                "required": ["note"]
            })),
            request_content_type: Some("application/xml".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert!(schema["properties"].get("note").is_none());
        assert_eq!(schema["properties"]["body"]["type"], "string");
        assert_eq!(
            schema["properties"]["body"]["contentMediaType"],
            "application/xml"
        );
        assert_eq!(schema["required"], serde_json::json!(["body"]));
    }

    #[test]
    fn build_input_schema_exposes_body_when_content_type_has_no_schema() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(schema["properties"]["body"]["type"], "string");
        assert_eq!(schema["properties"]["body"]["contentEncoding"], "base64");
        assert_eq!(
            schema["properties"]["body"]["contentMediaType"],
            "application/zip"
        );
        assert_eq!(schema["required"], serde_json::json!(["body"]));
    }

    #[test]
    fn build_input_schema_treats_unknown_application_uploads_as_binary() {
        let endpoint = McpToolEndpoint {
            name: "upload_tarball".to_string(),
            description: Some("Upload a tarball".to_string()),
            method: "POST".to_string(),
            path: "/archives".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/x-tar".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(schema["properties"]["body"]["type"], "string");
        assert_eq!(schema["properties"]["body"]["contentEncoding"], "base64");
        assert_eq!(
            schema["properties"]["body"]["contentMediaType"],
            "application/x-tar"
        );
        assert_eq!(schema["required"], serde_json::json!(["body"]));
    }

    #[test]
    fn build_input_schema_includes_supported_header_and_cookie_params() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users/{id}".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "session_id",
                    "in": "cookie",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "Authorization",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(schema["properties"]["X-Api-Version"]["type"], "string");
        assert_eq!(schema["properties"]["session_id"]["type"], "string");
        assert!(schema["properties"].get("Authorization").is_none());
        assert_eq!(
            schema["required"],
            serde_json::json!(["X-Api-Version", "session_id"])
        );
    }

    #[test]
    fn build_input_schema_uses_alternate_body_field_when_body_param_exists() {
        let endpoint = McpToolEndpoint {
            name: "upload_archive".to_string(),
            description: Some("Upload an archive".to_string()),
            method: "POST".to_string(),
            path: "/archives".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "body",
                    "in": "query",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(schema["properties"]["body"]["type"], "string");
        assert_eq!(schema["properties"]["request_body"]["type"], "string");
        assert_eq!(
            schema["properties"]["request_body"]["contentEncoding"],
            "base64"
        );
        assert_eq!(
            schema["required"],
            serde_json::json!(["body", "request_body"])
        );
    }

    #[test]
    fn build_input_schema_wraps_json_body_when_properties_collide_with_params() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users/{id}".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "id",
                    "in": "path",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "session_id",
                    "in": "cookie",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "display_name": { "type": "string" }
                },
                "required": ["id", "display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(schema["properties"]["id"]["type"], "string");
        assert_eq!(schema["properties"]["body"]["type"], "object");
        assert_eq!(
            schema["properties"]["body"]["properties"]["id"]["type"],
            "string"
        );
        assert_eq!(
            schema["required"],
            serde_json::json!(["id", "X-Api-Version", "session_id", "body"])
        );
    }

    #[test]
    fn build_input_schema_wraps_json_body_when_properties_collide_with_blocked_header_params() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "accept",
                    "in": "header",
                    "required": false,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "accept": { "type": "string" },
                    "display_name": { "type": "string" }
                },
                "required": ["accept", "display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert!(schema["properties"].get("accept").is_none());
        assert_eq!(schema["properties"]["body"]["type"], "object");
        assert_eq!(schema["required"], serde_json::json!(["body"]));
    }

    #[test]
    fn build_input_schema_wraps_json_body_when_properties_collide_with_header_params_case_insensitively()
     {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "x-api-version": { "type": "string" },
                    "display_name": { "type": "string" }
                },
                "required": ["x-api-version", "display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert!(schema["properties"].get("x-api-version").is_none());
        assert_eq!(schema["properties"]["body"]["type"], "object");
        assert_eq!(
            schema["properties"]["body"]["properties"]["x-api-version"]["type"],
            "string"
        );
        assert_eq!(
            schema["required"],
            serde_json::json!(["X-Api-Version", "body"])
        );
    }

    #[test]
    fn build_input_schema_wraps_optional_json_body_without_requiring_it() {
        let endpoint = McpToolEndpoint {
            name: "update_profile".to_string(),
            description: Some("Update a profile".to_string()),
            method: "PATCH".to_string(),
            path: "/profile".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "display_name": { "type": "string" }
                },
                "required": ["display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: false,
        };

        let schema = build_input_schema(&endpoint);
        assert!(schema["properties"].get("display_name").is_none());
        assert_eq!(schema["properties"]["body"]["type"], "object");
        assert_eq!(
            schema["properties"]["body"]["required"],
            serde_json::json!(["display_name"])
        );
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn build_input_schema_defaults_binary_media_type_when_missing() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: None,
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(
            schema["properties"]["body"]["contentMediaType"],
            "application/octet-stream"
        );
        assert_eq!(schema["properties"]["body"]["contentEncoding"], "base64");
    }

    #[test]
    fn build_input_schema_defaults_wildcard_binary_media_type_to_octet_stream() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: Some("*/*".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(
            schema["properties"]["body"]["contentMediaType"],
            "application/octet-stream"
        );
        assert_eq!(schema["properties"]["body"]["contentEncoding"], "base64");
    }

    #[test]
    fn build_input_schema_uses_alternate_body_field_when_body_header_param_exists_case_insensitively()
     {
        let endpoint = McpToolEndpoint {
            name: "submit_message".to_string(),
            description: Some("Submit a message".to_string()),
            method: "POST".to_string(),
            path: "/messages".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "Body",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: Some("text/plain".to_string()),
            request_body_required: true,
        };

        let schema = build_input_schema(&endpoint);
        assert_eq!(schema["properties"]["Body"]["type"], "string");
        assert!(schema["properties"].get("body").is_none());
        assert_eq!(schema["properties"]["request_body"]["type"], "string");
        assert_eq!(
            schema["required"],
            serde_json::json!(["Body", "request_body"])
        );
    }

    #[test]
    fn build_proxy_args_decodes_binary_body_from_base64() {
        use base64::Engine as _;

        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        let (_, _, _, _, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04")
            }),
        )
        .expect("binary body should decode");

        assert_eq!(body.unwrap().as_ref(), b"PK\x03\x04");
    }

    #[test]
    fn build_proxy_args_decodes_binary_body_without_explicit_content_type() {
        use base64::Engine as _;

        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: None,
            request_body_required: true,
        };

        let (_, _, _, _, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04")
            }),
        )
        .expect("binary body should decode");

        assert_eq!(body.unwrap().as_ref(), b"PK\x03\x04");
    }

    #[test]
    fn build_proxy_args_decodes_unknown_application_binary_body() {
        use base64::Engine as _;

        let endpoint = McpToolEndpoint {
            name: "upload_tarball".to_string(),
            description: Some("Upload a tarball".to_string()),
            method: "POST".to_string(),
            path: "/archives".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/x-tar".to_string()),
            request_body_required: true,
        };

        let (_, _, _, _, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": base64::engine::general_purpose::STANDARD.encode(b"ustar")
            }),
        )
        .expect("binary body should decode");

        assert_eq!(body.unwrap().as_ref(), b"ustar");
    }

    #[test]
    fn build_proxy_args_preserves_flattened_json_body_named_body_property() {
        let endpoint = McpToolEndpoint {
            name: "submit_payload".to_string(),
            description: Some("Submit a JSON object with a body field".to_string()),
            method: "POST".to_string(),
            path: "/payloads".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "body": { "type": "string" }
                },
                "required": ["body"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let (_, _, _, _, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": "hello"
            }),
        )
        .expect("flattened JSON body should serialize as an object");

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body.unwrap().as_ref()).unwrap(),
            serde_json::json!({ "body": "hello" })
        );
    }

    #[test]
    fn build_proxy_args_rejects_missing_required_flattened_json_body() {
        let endpoint = McpToolEndpoint {
            name: "update_profile".to_string(),
            description: Some("Update a profile".to_string()),
            method: "PATCH".to_string(),
            path: "/profile".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "display_name": { "type": "string" }
                }
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let error = build_proxy_args(&endpoint, &serde_json::json!({}))
            .expect_err("required flattened JSON body should be rejected when omitted");

        assert!(
            matches!(error, AppError::BadRequest(message) if message.contains("must include at least one body field"))
        );
    }

    #[test]
    fn build_proxy_args_routes_header_and_cookie_params_out_of_body() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users/{id}".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "id",
                    "in": "path",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "session_id",
                    "in": "cookie",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "display_name": { "type": "string" }
                },
                "required": ["display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let (_, path, _, headers, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "id": "path-user",
                "X-Api-Version": "2025-01-01",
                "session_id": "abc123",
                "display_name": "Nyx"
            }),
        )
        .expect("header and cookie params should be routed out of the body");

        assert_eq!(path, "users/path-user");
        assert!(
            headers
                .iter()
                .any(|(name, value)| { name == "X-Api-Version" && value == "2025-01-01" })
        );
        assert!(headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("cookie") && value == "session_id=abc123"
        }));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body.unwrap().as_ref()).unwrap(),
            serde_json::json!({
                "display_name": "Nyx"
            })
        );
    }

    #[test]
    fn build_proxy_args_accepts_header_parameters_case_insensitively() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "display_name": { "type": "string" }
                }
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let (_, _, _, headers, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "x-api-version": "2025-01-01",
                "display_name": "Nyx"
            }),
        )
        .expect("header params should match case-insensitively");

        assert!(
            headers
                .iter()
                .any(|(name, value)| { name == "X-Api-Version" && value == "2025-01-01" })
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body.unwrap().as_ref()).unwrap(),
            serde_json::json!({
                "display_name": "Nyx"
            })
        );
    }

    #[test]
    fn build_proxy_args_allows_missing_optional_wrapped_json_body() {
        let endpoint = McpToolEndpoint {
            name: "update_profile".to_string(),
            description: Some("Update a profile".to_string()),
            method: "PATCH".to_string(),
            path: "/profile".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "display_name": { "type": "string" }
                },
                "required": ["display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: false,
        };

        let (_, _, _, _, body) = build_proxy_args(&endpoint, &serde_json::json!({}))
            .expect("optional wrapped JSON body should be allowed");

        assert!(body.is_none());
    }

    #[test]
    fn build_proxy_args_uses_alternate_body_field_when_body_param_exists() {
        use base64::Engine as _;

        let endpoint = McpToolEndpoint {
            name: "upload_archive".to_string(),
            description: Some("Upload an archive".to_string()),
            method: "POST".to_string(),
            path: "/archives".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "body",
                    "in": "query",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        let (_, _, query, _, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": "metadata",
                "request_body": base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04")
            }),
        )
        .expect("alternate body field should be accepted");

        assert_eq!(query.as_deref(), Some("body=metadata"));
        assert_eq!(body.unwrap().as_ref(), b"PK\x03\x04");
    }

    #[test]
    fn build_proxy_args_wraps_json_body_when_properties_collide_with_params() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users/{id}".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "id",
                    "in": "path",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "session_id",
                    "in": "cookie",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "display_name": { "type": "string" }
                },
                "required": ["id", "display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let (_, path, _, headers, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "id": "path-user",
                "X-Api-Version": "2025-01-01",
                "session_id": "abc123",
                "body": {
                    "id": "body-user",
                    "display_name": "Nyx"
                }
            }),
        )
        .expect("wrapped JSON body should serialize with path param intact");

        assert_eq!(path, "users/path-user");
        assert!(
            headers
                .iter()
                .any(|(name, value)| { name == "X-Api-Version" && value == "2025-01-01" })
        );
        assert!(headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("cookie") && value == "session_id=abc123"
        }));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body.unwrap().as_ref()).unwrap(),
            serde_json::json!({
                "id": "body-user",
                "display_name": "Nyx"
            })
        );
    }

    #[test]
    fn build_proxy_args_wraps_json_body_when_properties_collide_with_blocked_header_params() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "accept",
                    "in": "header",
                    "required": false,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "accept": { "type": "string" },
                    "display_name": { "type": "string" }
                },
                "required": ["accept", "display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let (_, _, _, headers, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": {
                    "accept": "application/json",
                    "display_name": "Nyx"
                }
            }),
        )
        .expect("wrapped JSON body should not collide with blocked header params");

        assert!(headers.is_empty());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body.unwrap().as_ref()).unwrap(),
            serde_json::json!({
                "accept": "application/json",
                "display_name": "Nyx"
            })
        );
    }

    #[test]
    fn build_proxy_args_wraps_json_body_when_properties_collide_with_header_params_case_insensitively()
     {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "x-api-version": { "type": "string" },
                    "display_name": { "type": "string" }
                },
                "required": ["x-api-version", "display_name"]
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let (_, _, _, headers, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "X-Api-Version": "2025-01-01",
                "body": {
                    "x-api-version": "body-version",
                    "display_name": "Nyx"
                }
            }),
        )
        .expect("wrapped JSON body should not collide with header params case-insensitively");

        assert!(
            headers
                .iter()
                .any(|(name, value)| { name == "X-Api-Version" && value == "2025-01-01" })
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body.unwrap().as_ref()).unwrap(),
            serde_json::json!({
                "x-api-version": "body-version",
                "display_name": "Nyx"
            })
        );
    }

    #[test]
    fn build_proxy_args_rejects_missing_required_binary_body() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        let error = build_proxy_args(&endpoint, &serde_json::json!({}))
            .expect_err("required binary body should be rejected when omitted");

        assert!(
            matches!(error, AppError::BadRequest(message) if message.contains("base64-encoded string"))
        );
    }

    #[test]
    fn build_proxy_args_rejects_reserved_header_parameters() {
        let endpoint = McpToolEndpoint {
            name: "submit_message".to_string(),
            description: Some("Submit a message".to_string()),
            method: "POST".to_string(),
            path: "/messages".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "Authorization",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: Some("text/plain".to_string()),
            request_body_required: true,
        };

        let error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "Authorization": "Bearer secret",
                "body": "hello"
            }),
        )
        .expect_err("reserved header params should be rejected");

        assert!(
            matches!(error, AppError::BadRequest(message) if message.contains("is reserved and cannot be set"))
        );
    }

    #[test]
    fn build_proxy_args_rejects_reserved_header_parameters_case_insensitively() {
        let endpoint = McpToolEndpoint {
            name: "submit_message".to_string(),
            description: Some("Submit a message".to_string()),
            method: "POST".to_string(),
            path: "/messages".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "Authorization",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: Some("text/plain".to_string()),
            request_body_required: true,
        };

        let error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "authorization": "Bearer secret",
                "body": "hello"
            }),
        )
        .expect_err("reserved header params should be rejected case-insensitively");

        assert!(
            matches!(error, AppError::BadRequest(message) if message.contains("is reserved and cannot be set"))
        );
    }

    #[test]
    fn build_proxy_args_uses_alternate_body_field_when_body_header_param_exists_case_insensitively()
    {
        let endpoint = McpToolEndpoint {
            name: "submit_message".to_string(),
            description: Some("Submit a message".to_string()),
            method: "POST".to_string(),
            path: "/messages".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "Body",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: Some("text/plain".to_string()),
            request_body_required: true,
        };

        let (_, _, _, headers, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "Body": "metadata",
                "request_body": "hello"
            }),
        )
        .expect("alternate body field should avoid case-insensitive header collisions");

        assert!(
            headers
                .iter()
                .any(|(name, value)| { name == "Body" && value == "metadata" })
        );
        assert_eq!(
            std::str::from_utf8(body.unwrap().as_ref()).unwrap(),
            "hello"
        );
    }

    #[test]
    fn build_proxy_args_rejects_extra_fields_for_wrapped_json_body() {
        let endpoint = McpToolEndpoint {
            name: "submit_message".to_string(),
            description: Some("Submit a JSON string body".to_string()),
            method: "POST".to_string(),
            path: "/messages".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string"
            })),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };

        let error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": "hello",
                "extra": true
            }),
        )
        .expect_err("wrapped JSON body should reject extra fields");

        assert!(
            matches!(error, AppError::BadRequest(message) if message.contains("must be provided as a JSON value in the `body` field"))
        );
    }

    #[test]
    fn build_proxy_args_preserves_urlencoded_body_as_raw_text() {
        let endpoint = McpToolEndpoint {
            name: "submit_form".to_string(),
            description: Some("Submit a urlencoded form".to_string()),
            method: "POST".to_string(),
            path: "/forms".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/x-www-form-urlencoded".to_string()),
            request_body_required: true,
        };

        let (_, _, _, _, body) = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": "message=hello%20world&count=2"
            }),
        )
        .expect("raw body should pass through");

        assert_eq!(
            std::str::from_utf8(body.unwrap().as_ref()).unwrap(),
            "message=hello%20world&count=2"
        );
    }

    #[test]
    fn build_proxy_args_rejects_unknown_args_when_endpoint_has_no_request_body() {
        let endpoint = McpToolEndpoint {
            name: "list_users".to_string(),
            description: Some("List users".to_string()),
            method: "GET".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "limit",
                    "in": "query",
                    "required": false,
                    "schema": { "type": "integer" }
                }
            ])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };

        let error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "limit": 10,
                "unexpected": { "send": "body" }
            }),
        )
        .expect_err("unknown args should not become an undeclared request body");

        assert!(matches!(
            error,
            AppError::BadRequest(message)
                if message.contains("does not define a request body")
                    && message.contains("unexpected")
        ));
    }

    #[test]
    fn build_proxy_args_rejects_body_for_bodyless_post_endpoint() {
        let endpoint = McpToolEndpoint {
            name: "create_session".to_string(),
            description: Some("Create a session without a request body".to_string()),
            method: "POST".to_string(),
            path: "/sessions".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };

        let error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "payload": { "hello": "world" }
            }),
        )
        .expect_err("bodyless endpoints should reject undeclared payloads");

        assert!(matches!(
            error,
            AppError::BadRequest(message)
                if message.contains("does not define a request body")
                    && message.contains("payload")
        ));
    }

    #[test]
    fn build_proxy_args_rejects_missing_required_path_parameter() {
        let endpoint = McpToolEndpoint {
            name: "get_user".to_string(),
            description: Some("Get a user".to_string()),
            method: "GET".to_string(),
            path: "/users/{id}".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "id",
                    "in": "path",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };

        let error = build_proxy_args(&endpoint, &serde_json::json!({}))
            .expect_err("missing required path params should be rejected");

        assert!(matches!(
            error,
            AppError::BadRequest(message)
                if message.contains("missing required path parameter(s)")
                    && message.contains("id")
        ));
    }

    #[test]
    fn build_proxy_args_rejects_unresolved_path_templates_without_required_metadata() {
        let endpoint = McpToolEndpoint {
            name: "get_user".to_string(),
            description: Some("Get a user".to_string()),
            method: "GET".to_string(),
            path: "/users/{id}".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "id",
                    "in": "path",
                    "required": false,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };

        let error = build_proxy_args(&endpoint, &serde_json::json!({}))
            .expect_err("unresolved path templates should be rejected");

        assert!(matches!(
            error,
            AppError::BadRequest(message)
                if message.contains("unresolved path parameter(s)")
                    && message.contains("id")
        ));
    }

    #[test]
    fn build_proxy_args_rejects_missing_required_non_body_parameters() {
        let endpoint = McpToolEndpoint {
            name: "update_user".to_string(),
            description: Some("Update a user".to_string()),
            method: "POST".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "limit",
                    "in": "query",
                    "required": true,
                    "schema": { "type": "integer" }
                },
                {
                    "name": "X-Api-Version",
                    "in": "header",
                    "required": true,
                    "schema": { "type": "string" }
                },
                {
                    "name": "session_id",
                    "in": "cookie",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };

        let query_error = build_proxy_args(&endpoint, &serde_json::json!({}))
            .expect_err("missing required query params should be rejected");
        assert!(matches!(
            query_error,
            AppError::BadRequest(message)
                if message.contains("missing required query parameter(s)")
                    && message.contains("limit")
        ));

        let header_error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "limit": 10
            }),
        )
        .expect_err("missing required header params should be rejected");
        assert!(matches!(
            header_error,
            AppError::BadRequest(message)
                if message.contains("missing required header parameter(s)")
                    && message.contains("X-Api-Version")
        ));

        let cookie_error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "limit": 10,
                "X-Api-Version": "2025-01-01"
            }),
        )
        .expect_err("missing required cookie params should be rejected");
        assert!(matches!(
            cookie_error,
            AppError::BadRequest(message)
                if message.contains("missing required cookie parameter(s)")
                    && message.contains("session_id")
        ));
    }

    #[test]
    fn build_proxy_args_rejects_multipart_body() {
        let endpoint = McpToolEndpoint {
            name: "upload_form".to_string(),
            description: Some("Upload multipart form".to_string()),
            method: "POST".to_string(),
            path: "/form".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string" }
                }
            })),
            request_content_type: Some("multipart/form-data".to_string()),
            request_body_required: true,
        };

        let error = build_proxy_args(&endpoint, &serde_json::json!({ "body": "ignored" }))
            .expect_err("multipart should be rejected");

        assert!(matches!(error, AppError::BadRequest(_)));
        assert!(
            error
                .to_string()
                .contains("multipart/form-data request bodies are not yet supported")
        );
    }

    #[test]
    fn build_proxy_args_error_mentions_alternate_body_field_name() {
        let endpoint = McpToolEndpoint {
            name: "submit_text".to_string(),
            description: Some("Submit text".to_string()),
            method: "POST".to_string(),
            path: "/texts".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "body",
                    "in": "query",
                    "required": true,
                    "schema": { "type": "string" }
                }
            ])),
            request_body_schema: None,
            request_content_type: Some("text/plain".to_string()),
            request_body_required: true,
        };

        let error = build_proxy_args(
            &endpoint,
            &serde_json::json!({
                "body": "metadata",
                "payload": "hello"
            }),
        )
        .expect_err("missing alternate body field should be rejected");

        assert!(
            matches!(error, AppError::BadRequest(message) if message.contains("`request_body` field"))
        );
    }

    #[test]
    fn request_content_type_header_value_defaults_binary_schema_to_octet_stream() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: None,
            request_body_required: true,
        };

        assert_eq!(
            request_content_type_header_value(&endpoint, true),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn request_content_type_header_value_defaults_wildcard_binary_schema_to_octet_stream() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: Some("*/*".to_string()),
            request_body_required: true,
        };

        assert_eq!(
            request_content_type_header_value(&endpoint, true),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn request_content_type_header_value_uses_endpoint_content_type() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        assert_eq!(
            request_content_type_header_value(&endpoint, true),
            Some("application/zip")
        );
    }

    #[test]
    fn request_content_type_header_value_omits_optional_body_without_payload() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: false,
        };

        assert_eq!(request_content_type_header_value(&endpoint, false), None);
    }

    #[test]
    fn request_content_type_header_value_omits_default_json_without_payload() {
        let endpoint = McpToolEndpoint {
            name: "create_session".to_string(),
            description: Some("Create a session".to_string()),
            method: "POST".to_string(),
            path: "/sessions".to_string(),
            parameters: Some(serde_json::json!([
                {
                    "name": "ttl",
                    "in": "query",
                    "required": false,
                    "schema": { "type": "integer" }
                }
            ])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };

        assert_eq!(request_content_type_header_value(&endpoint, false), None);
    }

    #[test]
    fn build_downstream_request_headers_sets_content_type_without_forcing_accept() {
        let endpoint = McpToolEndpoint {
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({
                "type": "string",
                "format": "binary"
            })),
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
        };

        let headers =
            build_downstream_request_headers(&endpoint, true).expect("headers should build");

        assert_eq!(
            headers.get(reqwest::header::CONTENT_TYPE).unwrap(),
            "application/zip"
        );
        assert!(headers.get(reqwest::header::ACCEPT).is_none());
    }
}
