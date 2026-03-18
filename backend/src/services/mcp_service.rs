use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, legacy_http_service_type_filter,
};
use crate::models::service_endpoint::{COLLECTION_NAME as SERVICE_ENDPOINTS, ServiceEndpoint};
use crate::models::user_service_connection::{
    COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
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
    let mut auto_services_filter = doc! {
        "is_active": true,
        "requires_user_credential": false,
        "service_category": { "$ne": "provider" },
    };
    auto_services_filter.extend(legacy_http_service_type_filter());

    let auto_services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(auto_services_filter)
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
        if svc.service_type != "http" || svc.service_category == "provider" {
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

    // -- URL / query / header parameters --
    if let Some(params_value) = &endpoint.parameters
        && let Some(params) = params_value.as_array()
    {
        for param in params {
            let name = match param.get("name").and_then(|v| v.as_str()) {
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
    if let Some(body_schema) = &endpoint.request_body_schema {
        let is_object = body_schema.get("type").and_then(|v| v.as_str()) == Some("object");
        let has_props = body_schema
            .get("properties")
            .and_then(|v| v.as_object())
            .is_some();

        if is_object && has_props {
            // Merge object properties directly into the tool's inputSchema
            if let Some(props) = body_schema.get("properties").and_then(|v| v.as_object()) {
                for (key, value) in props {
                    properties.insert(key.clone(), value.clone());
                }
            }
            if let Some(req_arr) = body_schema.get("required").and_then(|v| v.as_array()) {
                for r in req_arr {
                    if let Some(s) = r.as_str() {
                        required.push(serde_json::Value::String(s.to_string()));
                    }
                }
            }
        } else {
            // Non-object body: wrap as a single `body` property
            let mut body_prop = body_schema.clone();
            if let Some(obj) = body_prop.as_object_mut() {
                obj.insert(
                    "description".into(),
                    serde_json::Value::String("Request body".into()),
                );
            }
            properties.insert("body".to_string(), body_prop);
            required.push(serde_json::Value::String("body".to_string()));
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

/// Build the HTTP method, path, query string, and body for a proxy request
/// from the endpoint definition and the MCP tool call arguments.
pub fn build_proxy_args(
    endpoint: &McpToolEndpoint,
    args: &serde_json::Value,
) -> (
    reqwest::Method,
    String,
    Option<String>,
    Option<bytes::Bytes>,
) {
    let mut path = endpoint.path.trim_start_matches('/').to_string();
    let mut query_params: Vec<(String, String)> = Vec::new();
    let mut body_fields: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();

    // Classify parameters
    let mut path_params = HashSet::new();
    let mut query_param_names = HashSet::new();

    if let Some(params_value) = &endpoint.parameters
        && let Some(params) = params_value.as_array()
    {
        for param in params {
            let name = param.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match param.get("in").and_then(|v| v.as_str()).unwrap_or("") {
                "path" => {
                    path_params.insert(name.to_string());
                }
                "query" => {
                    query_param_names.insert(name.to_string());
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

            if path_params.contains(key.as_str()) {
                path = path.replace(&format!("{{{key}}}"), &urlencoding::encode(&str_value));
            } else if query_param_names.contains(key.as_str()) {
                query_params.push((key.clone(), str_value));
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

    let body = if body_fields.is_empty() {
        None
    } else {
        // If only a `body` key exists, unwrap it (same logic as TS buildProxyArgs)
        let body_value = if body_fields.len() == 1 && body_fields.contains_key("body") {
            body_fields.remove("body").unwrap()
        } else {
            serde_json::Value::Object(body_fields)
        };
        Some(bytes::Bytes::from(
            serde_json::to_vec(&body_value).unwrap_or_default(),
        ))
    };

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

    (method, path, query, body)
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

    let (method, path, query, body) = build_proxy_args(endpoint, arguments);

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

    // Resolve delegated credentials. Required provider connections must succeed.
    let delegated = delegation_service::resolve_delegated_credentials(
        db,
        encryption_keys,
        user_id,
        &service.service_id,
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("Provider credentials not available: {e}")))?;

    // Minimal headers for the downstream request.
    // Always set Content-Type for methods that typically carry a body, even
    // when the body is empty -- some APIs (e.g. ASP.NET) return 415 without it.
    let mut headers = reqwest::header::HeaderMap::new();
    if body.is_some()
        || matches!(
            method,
            reqwest::Method::POST | reqwest::Method::PUT | reqwest::Method::PATCH
        )
    {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
    }
    headers.insert(reqwest::header::ACCEPT, "application/json".parse().unwrap());

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

    let mut filter = doc! {
        "is_active": true,
        "service_category": { "$ne": "provider" },
    };
    filter.extend(legacy_http_service_type_filter());
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
}
