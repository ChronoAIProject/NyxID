use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, legacy_http_service_type_filter,
};
use crate::models::service_endpoint::{COLLECTION_NAME as SERVICE_ENDPOINTS, ServiceEndpoint};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::models::user_service_connection::{
    COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
};
use crate::services::content_type::{
    is_binary_content_type, is_json_content_type, normalize_content_type, schema_is_binary,
};
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::{
    api_docs_service, connection_service, node_routing_service, openapi_parser, proxy_service,
};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// How the service was resolved -- carries enough identity for unambiguous execution.
#[allow(dead_code)]
pub enum McpToolSource {
    /// Platform service (DownstreamService)
    Platform { downstream_service_id: String },
    /// User-managed service (UserService -- personal or org-shared)
    UserManaged {
        user_service_id: String,
        /// The user who owns this service (actor for personal, org user_id for org-shared)
        effective_owner_id: String,
        /// Node routing -- when set, requests go through the node agent
        node_id: Option<String>,
        /// Whether the server-side credential is available (false = node-managed only)
        has_server_credential: bool,
    },
}

impl McpToolSource {
    pub fn is_user_service(&self) -> bool {
        matches!(self, McpToolSource::UserManaged { .. })
    }
}

/// Agent/scope context carried into [`execute_tool`].
///
/// Mirrors the agent-isolation fields already honored by the REST proxy
/// (`execute_proxy_inner`): per-agent credential binding resolution and
/// node allow-list enforcement. OAuth and session callers pass `api_key_id:
/// None` and `allow_all_nodes: true`, preserving their existing behavior.
pub struct McpExecContext<'a> {
    /// API key ID that is acting on behalf of the user. Enables per-agent
    /// credential override via [`proxy_service::resolve_agent_credential_override`].
    pub api_key_id: Option<&'a str>,
    /// When true, node routing and fallbacks are not filtered.
    pub allow_all_nodes: bool,
    /// Permitted node IDs when `allow_all_nodes` is false.
    pub allowed_node_ids: &'a [String],
}

/// A downstream service with its active endpoints, ready for MCP tool generation.
pub struct McpToolService {
    pub service_id: String,
    pub service_name: String,
    pub service_slug: String,
    pub description: Option<String>,
    pub service_category: String,
    pub endpoints: Vec<McpToolEndpoint>,
    pub source: McpToolSource,
    /// true if this service has only a generic proxy tool (custom endpoint, no predefined endpoints)
    pub is_generic_proxy: bool,
}

/// A single endpoint within a service.
#[derive(Default)]
pub struct McpToolEndpoint {
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
/// - Platform services the user has connected to (DownstreamService + UserServiceConnection)
/// - Auto-connected platform services (`requires_user_credential == false`)
/// - User-managed services (UserService -- personal and org-shared where callable)
///
/// Dedup: UserService takes priority over a platform DownstreamService for the
/// same catalog entry, but only when the UserService is actually executable.
/// Load all user tools (platform + user-managed).
///
/// When `include_non_executable` is true, user services whose credentials are
/// currently unavailable (node offline, key inactive) are still included so that
/// search results show them. When false, only callable services are returned.
#[allow(dead_code)]
pub async fn load_user_tools(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
) -> AppResult<Vec<McpToolService>> {
    load_user_tools_inner(db, node_ws_manager, user_id, false, NodeScope::Unrestricted).await
}

/// Like [`load_user_tools`] but honors an API-key node scope during
/// discovery: node-routed tools whose only viable routes (primary +
/// failovers) are all out of scope get classified as non-executable so
/// MCP doesn't advertise tools the caller can't actually invoke. Matches
/// the scope enforcement in `execute_tool` (seventeenth-round Codex
/// review P2).
pub async fn load_user_tools_scoped(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    scope: NodeScope<'_>,
) -> AppResult<Vec<McpToolService>> {
    load_user_tools_inner(db, node_ws_manager, user_id, false, scope).await
}

/// Like [`load_user_tools`] but includes non-executable user services for
/// discovery via `nyx__search_tools`. Prefer [`load_user_tools_all_scoped`]
/// when the caller's API-key node scope is available — MCP transport
/// has migrated to the scoped variant. This function is retained as the
/// unrestricted form for parity with [`load_user_tools`] in case other
/// callers need it, and to keep the public API symmetric.
#[allow(dead_code)]
pub async fn load_user_tools_all(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
) -> AppResult<Vec<McpToolService>> {
    load_user_tools_inner(db, node_ws_manager, user_id, true, NodeScope::Unrestricted).await
}

/// Scoped variant of [`load_user_tools_all`]. Search discovery honors
/// the caller's API-key node allow-list so `nyx__search_tools` can't
/// surface tools whose only viable routes fall outside the caller's
/// scope — otherwise a scoped agent would find tools it can never
/// successfully invoke (twentieth-round Codex P2).
pub async fn load_user_tools_all_scoped(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    scope: NodeScope<'_>,
) -> AppResult<Vec<McpToolService>> {
    load_user_tools_inner(db, node_ws_manager, user_id, true, scope).await
}

/// Node-scope filter used by the MCP discovery chain. `Unrestricted`
/// means every connected node counts; `Allowed(set)` only counts node
/// ids present in the set — the same semantic `execute_tool` enforces
/// when trimming fallback routes for scoped API keys.
#[derive(Clone, Copy)]
pub enum NodeScope<'a> {
    Unrestricted,
    Allowed(&'a [String]),
}

impl<'a> NodeScope<'a> {
    fn permits(&self, node_id: &str) -> bool {
        match self {
            NodeScope::Unrestricted => true,
            NodeScope::Allowed(ids) => ids.iter().any(|id| id == node_id),
        }
    }
}

async fn load_user_tools_inner(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    include_non_executable: bool,
    scope: NodeScope<'_>,
) -> AppResult<Vec<McpToolService>> {
    // -----------------------------------------------------------------------
    // Phase 1: Load platform (DownstreamService) services
    // -----------------------------------------------------------------------

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

    // Scope-aware routable-service lookup: a scoped API key's platform
    // tools shouldn't be surfaced when the only viable bindings point
    // to nodes outside its allow-list — `execute_tool` would later
    // reject every call with `ApiKeyScopeForbidden` (eighteenth-round
    // Codex P2).
    let node_route_service_ids = match scope {
        NodeScope::Unrestricted => {
            node_routing_service::list_routable_service_ids(db, user_id, node_ws_manager).await?
        }
        NodeScope::Allowed(allowed) => {
            node_routing_service::list_routable_service_ids_filtered(
                db,
                user_id,
                node_ws_manager,
                |nid| allowed.iter().any(|id| id == nid),
            )
            .await?
        }
    };
    let node_route_set: HashSet<&str> = node_route_service_ids
        .iter()
        .map(|service_id| service_id.as_str())
        .collect();

    let connected_ids: Vec<&str> = connections
        .iter()
        .filter(|c| c.is_active)
        .map(|c| c.service_id.as_str())
        .collect();

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

    let connected_services: Vec<DownstreamService> = if connected_ids.is_empty() {
        vec![]
    } else {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": &connected_ids }, "is_active": true })
            .await?
            .try_collect()
            .await?
    };

    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut valid_platform_services: Vec<&DownstreamService> = Vec::new();

    for svc in &connected_services {
        if svc.service_type != "http" || svc.service_category == "provider" {
            continue;
        }
        if svc.requires_user_credential {
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
            valid_platform_services.push(svc);
        }
    }

    for svc in &auto_services {
        if seen_ids.contains(&svc.id) {
            continue;
        }
        if let Some(conn) = conn_map.get(svc.id.as_str())
            && !conn.is_active
        {
            continue;
        }
        if seen_ids.insert(svc.id.clone()) {
            valid_platform_services.push(svc);
        }
    }

    // -----------------------------------------------------------------------
    // Phase 2: Load UserService tools (personal + org-shared)
    // -----------------------------------------------------------------------

    let all_user_services =
        load_callable_user_services(db, node_ws_manager, user_id, include_non_executable, scope)
            .await?;

    // Collect catalog IDs and slugs from *executable* user services for dedup
    let executable_catalog_ids: HashSet<&str> = all_user_services
        .iter()
        .filter_map(|r| r.service.catalog_service_id.as_deref())
        .collect();
    let executable_slugs: HashSet<&str> = all_user_services
        .iter()
        .map(|r| r.service.slug.as_str())
        .collect();

    // Also block platform tools whose catalog id / slug is claimed by a
    // node-pinned `UserService`, regardless of current executability.
    // Without this, a user-pinned service whose node is offline would
    // be dropped from the user-tool list, letting the auto-connected
    // platform copy of the same catalog entry fall through the dedup
    // and expose `execute_tool` as a direct HTTP route — defeating the
    // user's "route via node" choice (twelfth-round Codex review P1).
    //
    // Scope: personal services always, org services only when the
    // caller's effective membership scope actually allows them.
    // Previously this bulk-loaded the org owner's entire service table,
    // which could suppress a platform tool for a scoped member just
    // because some *other* service in the same org was pinned to a
    // node — even though the member couldn't access it (thirteenth-
    // round Codex P1). Must match `load_callable_user_services`'s
    // scope handling.
    let mut blocked_catalog_ids: HashSet<String> = executable_catalog_ids
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let mut blocked_slugs: HashSet<String> =
        executable_slugs.iter().map(|s| (*s).to_string()).collect();
    let personal_pinned: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "user_id": user_id,
            "is_active": true,
            "service_type": "http",
            "node_id": { "$type": "string", "$ne": "" },
        })
        .await?
        .try_collect()
        .await?;
    for svc in &personal_pinned {
        // Always block, even when the pinned `node_id` is out of the
        // caller's API-key scope. `execute_tool` for the platform copy
        // of the same catalog entry resolves the user's pinned
        // `UserService` via `resolve_from_user_service` and then
        // rejects with `ApiKeyScopeForbidden` on the out-of-scope
        // node — so listing the platform copy here would surface a
        // tool the scoped agent could never actually call
        // (thirtieth-round Codex P2, reverting the twenty-ninth-
        // round change).
        if let Some(cat) = svc.catalog_service_id.clone() {
            blocked_catalog_ids.insert(cat);
        }
        blocked_slugs.insert(svc.slug.clone());
    }
    {
        use crate::services::org_service;
        let memberships = org_service::list_memberships_for_member(db, user_id, false).await?;
        for m in &memberships {
            if !m.role.can_proxy() {
                continue;
            }
            let effective_scope =
                crate::services::org_role_scope_service::effective_scope_for_membership(db, m)
                    .await?;
            let org_pinned: Vec<UserService> = db
                .collection::<UserService>(USER_SERVICES)
                .find(doc! {
                    "user_id": &m.org_user_id,
                    "is_active": true,
                    "service_type": "http",
                    "node_id": { "$type": "string", "$ne": "" },
                })
                .await?
                .try_collect()
                .await?;
            for svc in org_pinned {
                if !crate::services::org_role_scope_service::scope_allows(&effective_scope, &svc.id)
                {
                    continue;
                }
                // Mirror the personal-pinned loop: always block the
                // platform copy even when the org-pinned node is out
                // of the caller's API-key scope. `execute_tool` will
                // reject with `ApiKeyScopeForbidden` on the out-of-
                // scope node, so the platform copy cannot actually be
                // called in that case either (thirtieth-round Codex
                // P2, reverting the twenty-ninth-round change).
                if let Some(cat) = svc.catalog_service_id {
                    blocked_catalog_ids.insert(cat);
                }
                blocked_slugs.insert(svc.slug);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase 3: Load ServiceEndpoints for both platform and user services
    // -----------------------------------------------------------------------

    // Collect all catalog/downstream IDs that need endpoints
    let mut endpoint_service_ids: Vec<&str> = valid_platform_services
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    for r in &all_user_services {
        if let Some(catalog_id) = r.service.catalog_service_id.as_deref() {
            endpoint_service_ids.push(catalog_id);
        }
    }
    endpoint_service_ids.sort_unstable();
    endpoint_service_ids.dedup();

    let all_endpoints: Vec<ServiceEndpoint> = if endpoint_service_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .find(doc! {
                "service_id": { "$in": &endpoint_service_ids },
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?
    };

    let mut eps_by_svc: HashMap<&str, Vec<&ServiceEndpoint>> = HashMap::new();
    for ep in &all_endpoints {
        eps_by_svc
            .entry(ep.service_id.as_str())
            .or_default()
            .push(ep);
    }

    // Load UserEndpoints for label info
    let user_endpoint_ids: Vec<&str> = all_user_services
        .iter()
        .map(|r| r.service.endpoint_id.as_str())
        .collect();
    let user_endpoints: Vec<UserEndpoint> = if user_endpoint_ids.is_empty() {
        vec![]
    } else {
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": &user_endpoint_ids } })
            .await?
            .try_collect()
            .await?
    };
    let endpoints_by_id: HashMap<&str, &UserEndpoint> = user_endpoints
        .iter()
        .map(|ep| (ep.id.as_str(), ep))
        .collect();

    // -----------------------------------------------------------------------
    // Phase 4: Assemble results -- user services first, dedup platform after
    // -----------------------------------------------------------------------

    let mut result: Vec<McpToolService> = Vec::new();

    // 4a. User-managed services
    for r in &all_user_services {
        let us = &r.service;
        let user_endpoint = endpoints_by_id.get(us.endpoint_id.as_str()).copied();
        let endpoint_label = user_endpoint
            .map(|ep| ep.label.as_str())
            .unwrap_or(&us.slug);

        let (endpoints, is_generic) = if let Some(catalog_id) = us.catalog_service_id.as_deref() {
            // Catalog-backed: use the ServiceEndpoint rows pre-parsed at catalog
            // registration time. Unchanged path.
            let eps = eps_by_svc
                .get(catalog_id)
                .map(|eps| service_endpoints_to_mcp(eps))
                .unwrap_or_default();
            (eps, false)
        } else if let Some(spec_url) = user_endpoint.and_then(|ep| ep.openapi_spec_url.as_deref()) {
            // Custom endpoint with a user-supplied OpenAPI spec: fetch + parse
            // through the hardened cache (scoped by owner) and surface each
            // operation as a tool. On any failure we silently fall back to the
            // generic proxy tool so a broken spec URL never takes the service
            // offline for the agent.
            match fetch_and_parse_user_spec(spec_url, &r.effective_owner_id).await {
                Ok(mcp_endpoints) if !mcp_endpoints.is_empty() => (mcp_endpoints, false),
                Ok(_) => {
                    tracing::debug!(
                        user_service_id = %us.id,
                        spec_url = %api_docs_service::redact_url_for_logs(spec_url),
                        "Parsed user OpenAPI spec contained no operations; falling back to generic proxy tool"
                    );
                    (vec![build_generic_proxy_endpoint(endpoint_label)], true)
                }
                Err(error) => {
                    tracing::warn!(
                        user_service_id = %us.id,
                        spec_url = %api_docs_service::redact_url_for_logs(spec_url),
                        %error,
                        "Failed to parse user OpenAPI spec; falling back to generic proxy tool"
                    );
                    (vec![build_generic_proxy_endpoint(endpoint_label)], true)
                }
            }
        } else {
            let generic_ep = build_generic_proxy_endpoint(endpoint_label);
            (vec![generic_ep], true)
        };

        result.push(McpToolService {
            service_id: us.id.clone(),
            service_name: endpoint_label.to_string(),
            service_slug: us.slug.clone(),
            description: None,
            service_category: "user_service".to_string(),
            endpoints,
            source: McpToolSource::UserManaged {
                user_service_id: us.id.clone(),
                effective_owner_id: r.effective_owner_id.clone(),
                node_id: us.node_id.clone(),
                has_server_credential: r.has_server_credential,
            },
            is_generic_proxy: is_generic,
        });
    }

    // 4b. Platform services (skip those covered by an executable user
    // service OR by a node-pinned UserService — see blocked_* sets).
    for svc in valid_platform_services {
        if blocked_catalog_ids.contains(svc.id.as_str()) {
            continue;
        }
        if blocked_slugs.contains(svc.slug.as_str()) {
            continue;
        }

        let endpoints = eps_by_svc
            .get(svc.id.as_str())
            .map(|eps| service_endpoints_to_mcp(eps))
            .unwrap_or_default();

        result.push(McpToolService {
            service_id: svc.id.clone(),
            service_name: svc.name.clone(),
            service_slug: svc.slug.clone(),
            description: svc.description.clone(),
            service_category: svc.service_category.clone(),
            endpoints,
            source: McpToolSource::Platform {
                downstream_service_id: svc.id.clone(),
            },
            is_generic_proxy: false,
        });
    }

    Ok(result)
}

/// Convert ServiceEndpoints to McpToolEndpoints.
fn service_endpoints_to_mcp(eps: &[&ServiceEndpoint]) -> Vec<McpToolEndpoint> {
    eps.iter()
        .map(|ep| McpToolEndpoint {
            endpoint_id: ep.id.clone(),
            name: ep.name.clone(),
            description: ep.description.clone(),
            method: ep.method.clone(),
            path: ep.path.clone(),
            parameters: ep.parameters.clone(),
            request_body_schema: ep.request_body_schema.clone(),
            request_content_type: ep.request_content_type.clone(),
            request_body_required: ep.effective_request_body_required(),
            response_description: ep.response_description.clone(),
        })
        .collect()
}

/// Fetch the user-supplied OpenAPI spec through the hardened cache (scoped
/// by the owning user id), parse it, and convert to MCP tool endpoints.
async fn fetch_and_parse_user_spec(
    spec_url: &str,
    owner_id: &str,
) -> AppResult<Vec<McpToolEndpoint>> {
    let spec = api_docs_service::fetch_spec_json_scoped(spec_url, owner_id).await?;
    let parsed = openapi_parser::parse_openapi_spec_value(&spec)?;
    Ok(parsed
        .into_iter()
        .map(|p| McpToolEndpoint {
            // User-endpoint operations have no persistent ID; synthesise a
            // stable one from method+path so downstream logging / metrics can
            // distinguish tools.
            endpoint_id: format!("{}:{}", p.method, p.path),
            name: p.name,
            description: p.description,
            method: p.method,
            path: p.path,
            parameters: p.parameters,
            request_body_schema: p.request_body_schema,
            request_content_type: p.request_content_type,
            request_body_required: p.request_body_required,
            response_description: None,
        })
        .collect())
}

/// A resolved user service ready for MCP tool generation.
struct ResolvedUserService {
    service: UserService,
    effective_owner_id: String,
    has_server_credential: bool,
}

/// Load all callable UserServices for the user: personal + org-shared (where
/// the membership allows proxy access). Filters out services with unsatisfied
/// credentials unless they are node-routed with an online node.
async fn load_callable_user_services(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    include_non_executable: bool,
    scope: NodeScope<'_>,
) -> AppResult<Vec<ResolvedUserService>> {
    use crate::services::org_service;

    // -- Personal services --
    let personal_services: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! { "user_id": user_id, "is_active": true, "service_type": "http" })
        .await?
        .try_collect()
        .await?;

    // Collect all api_key_ids from personal + org services for batch lookup
    let mut all_api_key_ids: Vec<String> = personal_services
        .iter()
        .filter_map(|us| us.api_key_id.clone())
        .collect();

    // -- Org-shared services --
    let memberships = org_service::list_memberships_for_member(db, user_id, false).await?;
    let mut org_services: Vec<(UserService, String)> = Vec::new(); // (service, org_user_id)

    for m in &memberships {
        if !m.role.can_proxy() {
            continue; // Viewers cannot call MCP tools
        }
        let effective_scope =
            crate::services::org_role_scope_service::effective_scope_for_membership(db, m).await?;

        let org_svcs: Vec<UserService> = db
            .collection::<UserService>(USER_SERVICES)
            .find(doc! {
                "user_id": &m.org_user_id,
                "is_active": true,
                "service_type": "http",
            })
            .await?
            .try_collect()
            .await?;

        for svc in org_svcs {
            if !crate::services::org_role_scope_service::scope_allows(&effective_scope, &svc.id) {
                continue;
            }
            if let Some(ak_id) = &svc.api_key_id {
                all_api_key_ids.push(ak_id.clone());
            }
            org_services.push((svc, m.org_user_id.clone()));
        }
    }

    // Batch-load active API keys
    all_api_key_ids.sort_unstable();
    all_api_key_ids.dedup();
    let active_api_keys: Vec<UserApiKey> = if all_api_key_ids.is_empty() {
        vec![]
    } else {
        db.collection::<UserApiKey>(USER_API_KEYS)
            .find(doc! { "_id": { "$in": &all_api_key_ids }, "status": "active" })
            .await?
            .try_collect()
            .await?
    };
    // Map key ID -> credential_type for distinguishing node_managed from real keys
    let active_key_map: HashMap<&str, &str> = active_api_keys
        .iter()
        .map(|k| (k.id.as_str(), k.credential_type.as_str()))
        .collect();

    // Precompute "is any routing candidate online?" per node-routed
    // service so `classify_credential` can report a tool as executable
    // when the pinned node is offline but a viable fallback binding is
    // available (tenth-round Codex P2). Without this, agents lose
    // access to a service during exactly the kind of failover that
    // NodeServiceBinding was designed to enable. Cost is an extra DB
    // round-trip per node-routed service; acceptable on the discovery
    // path (not the hot proxy path).
    let mut service_has_viable_route: HashMap<String, bool> = HashMap::new();
    let node_routed_refs: Vec<(&UserService, &str)> = personal_services
        .iter()
        .map(|s| (s, user_id))
        .chain(
            org_services
                .iter()
                .map(|(s, org_user_id)| (s, org_user_id.as_str())),
        )
        .filter(|(s, _)| s.node_id.as_deref().is_some_and(|n| !n.is_empty()))
        .collect();
    for (svc, effective_owner) in node_routed_refs {
        // Primary must be in-scope. `execute_tool`'s UserManaged path
        // hard-rejects with `ApiKeyScopeForbidden` when the resolved
        // primary `node_id` is not in the API key's allow-list —
        // before trying any fallback. Advertising a tool as executable
        // when the primary is out of scope would make every invocation
        // 403 (eighteenth-round Codex P2). Fallbacks alone can't rescue
        // that configuration for a scoped key.
        let primary_in_scope = svc.node_id.as_deref().is_some_and(|nid| scope.permits(nid));
        if !primary_in_scope {
            service_has_viable_route.insert(svc.id.clone(), false);
            continue;
        }
        let primary_online = svc
            .node_id
            .as_deref()
            .is_some_and(|nid| node_ws_manager.is_connected(nid));
        if primary_online {
            service_has_viable_route.insert(svc.id.clone(), true);
            continue;
        }
        // `NodeServiceBinding.service_id` is keyed by the catalog
        // (`DownstreamService`) id, not the `UserService` id. Custom
        // services have no catalog id and therefore no failover
        // bindings — their only routing option is the primary node
        // (eleventh-round Codex review P2).
        //
        // Scope-aware: fallback nodes outside the caller's API-key
        // node allow-list are excluded here, matching the scope filter
        // `execute_tool` applies at invocation time (seventeenth-round
        // Codex review P2). Without this, a scoped key could see a
        // tool as executable solely because an out-of-scope fallback
        // is online, then every call would fail after the scope filter
        // trimmed the fallback chain away.
        let any_online = if let Some(ref catalog_service_id) = svc.catalog_service_id {
            let fallbacks = node_routing_service::list_viable_binding_node_ids(
                db,
                effective_owner,
                catalog_service_id,
                node_ws_manager,
            )
            .await
            .unwrap_or_default();
            fallbacks
                .iter()
                .any(|nid| node_ws_manager.is_connected(nid) && scope.permits(nid))
        } else {
            false
        };
        service_has_viable_route.insert(svc.id.clone(), any_online);
    }

    // Filter and assemble
    let mut result = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();

    // Personal first (takes priority over org for same slug)
    for us in personal_services {
        let has_route = service_has_viable_route
            .get(&us.id)
            .copied()
            .unwrap_or(false);
        let cred_info = classify_credential(&us, &active_key_map, node_ws_manager, has_route);
        if !include_non_executable && !cred_info.is_executable {
            continue;
        }
        // Scope filter applies even in `include_non_executable` search
        // mode. A tool whose primary node is outside the caller's
        // allow-list will hard-fail with `ApiKeyScopeForbidden` on
        // every invocation, so surfacing it in `nyx__search_tools`
        // misleads scoped agents into trying and retrying tools they
        // can never call (twenty-eighth-round Codex P2). This is the
        // pinned-primary scope check; failover nodes are already
        // gated by `has_route` via the scope-aware
        // `service_has_viable_route` precompute above.
        let primary_in_scope = us
            .node_id
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|nid| scope.permits(nid))
            .unwrap_or(true);
        if !primary_in_scope {
            continue;
        }
        seen_slugs.insert(us.slug.clone());
        result.push(ResolvedUserService {
            service: us,
            effective_owner_id: user_id.to_string(),
            has_server_credential: cred_info.has_server_credential,
        });
    }

    // Org services (skip slug collisions with personal)
    for (us, org_user_id) in org_services {
        if seen_slugs.contains(&us.slug) {
            continue;
        }
        let has_route = service_has_viable_route
            .get(&us.id)
            .copied()
            .unwrap_or(false);
        let cred_info = classify_credential(&us, &active_key_map, node_ws_manager, has_route);
        if !include_non_executable && !cred_info.is_executable {
            continue;
        }
        // Scope filter applies even in `include_non_executable` search
        // mode. A tool whose primary node is outside the caller's
        // allow-list will hard-fail with `ApiKeyScopeForbidden` on
        // every invocation, so surfacing it in `nyx__search_tools`
        // misleads scoped agents into trying and retrying tools they
        // can never call (twenty-eighth-round Codex P2). This is the
        // pinned-primary scope check; failover nodes are already
        // gated by `has_route` via the scope-aware
        // `service_has_viable_route` precompute above.
        let primary_in_scope = us
            .node_id
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|nid| scope.permits(nid))
            .unwrap_or(true);
        if !primary_in_scope {
            continue;
        }
        seen_slugs.insert(us.slug.clone());
        result.push(ResolvedUserService {
            service: us,
            effective_owner_id: org_user_id,
            has_server_credential: cred_info.has_server_credential,
        });
    }

    Ok(result)
}

struct CredentialClassification {
    /// Whether the service can be called (has credential or online node)
    is_executable: bool,
    /// Whether the backend holds a decrypt-able credential (false for node_managed)
    has_server_credential: bool,
}

/// Classify a UserService's credential availability.
///
/// `node_managed` keys do NOT provide a server-side credential (they decrypt to
/// None) so they require an online node. Regular active keys provide a server
/// credential. No-auth services are always executable.
fn classify_credential(
    us: &UserService,
    active_key_map: &HashMap<&str, &str>,
    node_ws_manager: &NodeWsManager,
    any_routing_node_online: bool,
) -> CredentialClassification {
    let node_online = us
        .node_id
        .as_deref()
        .is_some_and(|nid| node_ws_manager.is_connected(nid));

    // Node routing check runs BEFORE the `auth_method == "none"` fast
    // path: a no-auth service that was explicitly bound to a node is
    // still a "route via node" contract, and MCP must not advertise it
    // as directly executable when the node is offline (Codex P2 of the
    // sixth-round review). Routing is controlled by
    // `UserService.node_id`, independent of the backing
    // `UserApiKey.credential_type` — MCP treats node-routed services as
    // "node or nothing" so the user's network-isolation choice holds
    // even when the backend has the credential bytes stored
    // (NyxID#418 server-held model).
    //
    // `is_executable` uses `any_routing_node_online` instead of just
    // the primary `node_online` so failover via `NodeServiceBinding`
    // keeps tools visible when the pinned node is offline but a viable
    // fallback is online. `execute_tool` honors the same fallback
    // chain via `list_viable_binding_node_ids`, so advertising only
    // the primary's state would hide calls that would otherwise succeed
    // (tenth-round Codex review P2).
    //
    // Treat `node_id: Some("")` as unset: some legacy UserService rows
    // carry an empty string instead of `None`, and every other
    // routing-aware path already filters those out with `$ne: ""`. A
    // naive `is_some()` check would drop those services from MCP
    // discovery even though their credential is perfectly usable via
    // the direct-routing path (fourteenth-round Codex P2).
    let has_explicit_node = us.node_id.as_deref().is_some_and(|n| !n.is_empty());
    if has_explicit_node {
        let _ = node_online;
        return CredentialClassification {
            is_executable: any_routing_node_online,
            has_server_credential: false,
        };
    }

    if us.auth_method == "none" {
        return CredentialClassification {
            is_executable: true,
            has_server_credential: true,
        };
    }

    // Direct routing (no node) — fall back to credential_type semantics.
    if let Some(ak_id) = us.api_key_id.as_deref() {
        if let Some(&cred_type) = active_key_map.get(ak_id) {
            let is_node_managed = cred_type == "node_managed" || cred_type == "ssh_certificate";
            if is_node_managed {
                // node_managed keys require the node to be online — but
                // we're on the no-`node_id` branch here, so this state is
                // a stale reconcile artefact; treat it as unexecutable.
                return CredentialClassification {
                    is_executable: false,
                    has_server_credential: false,
                };
            }
            // Real key with server-side credential — direct injection works.
            return CredentialClassification {
                is_executable: true,
                has_server_credential: true,
            };
        }
        // Key exists in service but not in active set (inactive/revoked)
        return CredentialClassification {
            is_executable: false,
            has_server_credential: false,
        };
    }

    // No api_key_id and no node — the service is inert.
    CredentialClassification {
        is_executable: false,
        has_server_credential: false,
    }
}

// ---------------------------------------------------------------------------
// Generic proxy endpoint for custom user services
// ---------------------------------------------------------------------------

/// Build a single generic proxy endpoint for custom services that have no
/// predefined API endpoints. Lets the AI make arbitrary HTTP requests.
fn build_generic_proxy_endpoint(service_label: &str) -> McpToolEndpoint {
    McpToolEndpoint {
        endpoint_id: String::new(),
        name: "request".to_string(),
        description: Some(format!(
            "Make an HTTP request to {service_label}. Specify the method, path, and optional JSON body."
        )),
        method: "POST".to_string(),
        path: String::new(),
        parameters: None,
        request_body_schema: None,
        request_content_type: Some("application/json".to_string()),
        request_body_required: false,
        response_description: None,
    }
}

/// Build the JSON Schema input for a generic proxy tool. This is separate from
/// `build_input_schema` because generic tools have a different shape (method +
/// path + body come from arguments, not from endpoint metadata).
fn build_generic_proxy_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "method": {
                "type": "string",
                "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                "description": "HTTP method (defaults to GET)"
            },
            "path": {
                "type": "string",
                "description": "Request path (e.g., /v1/chat/completions)"
            },
            "body": {
                "description": "Request body (JSON object). Omit for GET/DELETE requests."
            }
        },
        "required": ["path"]
    })
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

    tools.push(McpToolDefinition {
        name: "nyx__ssh_exec".to_string(),
        description: "Execute a command on a remote SSH service. Returns stdout, stderr, \
            and exit code. The command runs on the remote machine authenticated via NyxID \
            SSH certificate."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "service": {
                    "type": "string",
                    "description": "Service slug or ID of the SSH service to execute on"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute on the remote machine"
                },
                "principal": {
                    "type": "string",
                    "description": "SSH principal (Unix username) to run the command as"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Maximum execution time in seconds (default: 30, max: 300)",
                    "default": 30
                }
            },
            "required": ["service", "command"]
        }),
    });

    tools.push(McpToolDefinition {
        name: "nyx__ssh_list_services".to_string(),
        description: "List available SSH services that can be used for remote command \
            execution."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {}
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
            let input_schema = if service.is_generic_proxy {
                build_generic_proxy_input_schema()
            } else {
                build_input_schema(endpoint)
            };
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
/// For user-managed services, resolves by exact UserService ID (not slug) and
/// routes through nodes when the service has a `node_id`, matching the same
/// node/failover behavior as `handlers/proxy.rs::execute_proxy_inner`.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool(
    http_client: &reqwest::Client,
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &std::sync::Arc<NodeWsManager>,
    user_id: &str,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
    arguments: &serde_json::Value,
    jwt_keys: &crate::crypto::jwt::JwtKeys,
    config: &crate::config::AppConfig,
    token_exchange_cache: &crate::services::provider_token_exchange_service::TokenExchangeCache,
    cloud_response_cache: &crate::services::cloud_response_cache::CloudResponseCache,
    exec_ctx: &McpExecContext<'_>,
) -> AppResult<(u16, String)> {
    use crate::models::user::{COLLECTION_NAME as USERS, User};
    use crate::services::node_ws_manager::{NodeProxyRequest, ProxyResponseType};
    use crate::services::{delegation_service, identity_service, node_service};

    // Build proxy arguments: generic proxy tools extract method/path from args
    let (method, path, query, parameter_headers, body) = if service.is_generic_proxy {
        build_generic_proxy_args(arguments)?
    } else {
        build_proxy_args(endpoint, arguments)?
    };

    // Resolve the proxy target and node routing from the fresh resolver result
    // (not cached loader flags -- credential state may have changed).
    let (target, node_route, has_server_credential) = match &service.source {
        McpToolSource::UserManaged {
            user_service_id, ..
        } => {
            let mut resolution = proxy_service::resolve_proxy_target_by_user_service_id(
                db,
                encryption_keys,
                user_id,
                user_service_id,
                Some(&service.service_slug),
                None,
            )
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("User service '{}' not found", service.service_slug))
            })?;
            let has_cred = resolution.has_server_credential;

            // Per-agent credential override: when acting as an API key with
            // an agent binding, swap in the override credential before execute.
            // Matches `execute_proxy_inner` in handlers/proxy.rs.
            if let Some(ak_id) = exec_ctx.api_key_id
                && let Some(override_cred) = proxy_service::resolve_agent_credential_override(
                    db,
                    encryption_keys,
                    user_id,
                    ak_id,
                    user_service_id,
                )
                .await?
            {
                resolution.target.credential = override_cred;
            }

            // Build the full NodeRoute (primary + fallbacks) from the resolution.
            let effective_owner = resolution
                .org_routing
                .as_ref()
                .map(|r| r.org_user_id.as_str())
                .unwrap_or(user_id);
            // Treat legacy `resolution.node_id == Some("")` as unset:
            // some `UserService` rows still carry the empty string
            // instead of `None`, and building a `NodeRoute` around an
            // empty node id would deterministically return `NodeOffline`
            // on every call and block the direct-credential fallback.
            // Matches the normalization applied in `classify_credential`
            // and in the PUT /keys handler. Fifteenth-round Codex P1.
            //
            // This normalization runs BEFORE the scope-check below so a
            // legacy row with `node_id: ""` is treated as "no node" by
            // both discovery and execution. Otherwise a scoped API key
            // would 403 on a legacy direct-routed service that
            // `classify_credential`/`load_callable_user_services`
            // already reported as directly executable (twenty-fifth-
            // round Codex P2).
            let effective_primary_node_id = resolution.node_id.as_deref().filter(|n| !n.is_empty());

            // Enforce node allow-list for scoped API keys on the primary
            // node, using the normalized value so legacy `""` rows
            // bypass the check entirely.
            if !exec_ctx.allow_all_nodes
                && let Some(nid) = effective_primary_node_id
                && !exec_ctx.allowed_node_ids.contains(&nid.to_string())
            {
                return Err(AppError::ApiKeyScopeForbidden(
                    "API key does not have access to this node".to_string(),
                ));
            }
            let nr = if let Some(primary_nid) = effective_primary_node_id {
                let mut fallback_ids: Vec<String> =
                    node_routing_service::list_viable_binding_node_ids(
                        db,
                        effective_owner,
                        &resolution.target.service.id,
                        node_ws_manager.as_ref(),
                    )
                    .await?
                    .into_iter()
                    .filter(|nid| nid != primary_nid)
                    .collect();
                // Trim failover candidates for scoped API keys.
                if !exec_ctx.allow_all_nodes {
                    fallback_ids.retain(|nid| exec_ctx.allowed_node_ids.contains(nid));
                }
                Some(node_routing_service::NodeRoute {
                    node_id: primary_nid.to_string(),
                    fallback_node_ids: fallback_ids,
                })
            } else {
                None
            };
            // A configured node route is a hard "route via node" contract,
            // regardless of whether the backend also happens to hold the
            // credential bytes server-side (NyxID#418 server-held model).
            // Forcing `has_server_credential = false` here disables the
            // "all nodes failed → try direct" fallback below, so MCP
            // never bypasses the node for user-managed node-routed tools.
            // (Sixth-round Codex review P1.)
            let has_cred_for_fallback = has_cred && nr.is_none();
            (resolution.target, nr, has_cred_for_fallback)
        }
        McpToolSource::Platform {
            downstream_service_id,
        } => {
            // For platform services, resolve node route first. When a node
            // route exists, use the lenient resolver (credential may be
            // absent if the node manages it). Otherwise, use the strict
            // resolver which requires a credential.
            let mut nr = node_routing_service::resolve_node_route(
                db,
                user_id,
                downstream_service_id,
                node_ws_manager.as_ref(),
            )
            .await?;

            // Enforce node allow-list for scoped API keys: reject the primary
            // node if it is out of scope, and prune out-of-scope failover
            // candidates. Mirrors the UserManaged branch and the REST proxy
            // check in `execute_proxy_inner`.
            if !exec_ctx.allow_all_nodes
                && let Some(route) = nr.as_mut()
            {
                if !exec_ctx.allowed_node_ids.contains(&route.node_id) {
                    return Err(AppError::ApiKeyScopeForbidden(
                        "API key does not have access to this node".to_string(),
                    ));
                }
                route
                    .fallback_node_ids
                    .retain(|nid| exec_ctx.allowed_node_ids.contains(nid));
            }

            let (t, has_cred) = if nr.is_some() {
                proxy_service::resolve_proxy_target_lenient(
                    db,
                    encryption_keys,
                    user_id,
                    downstream_service_id,
                )
                .await?
            } else {
                let t = proxy_service::resolve_proxy_target(
                    db,
                    encryption_keys,
                    user_id,
                    downstream_service_id,
                )
                .await?;
                (t, true)
            };
            // Platform services resolve their node route through
            // `NodeServiceBinding` rows, which are opt-in routing hints
            // rather than an explicit `UserService.node_id` contract.
            // The REST proxy mirrors this distinction via
            // `user_service_has_explicit_node()` — it only hard-requires
            // node routing for the UserManaged branch and allows direct
            // fallback for binding-based routes. Keep the same semantic
            // here: a transient node failure on a platform service can
            // fall back to direct HTTP if a server credential exists.
            (t, nr, has_cred)
        }
    };

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
                    db,
                )
                .await
                {
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

    // Resolve delegated credentials (only for platform services).
    // When a node route exists, swallow errors -- the node agent may inject
    // the credential locally, matching proxy.rs:891 behavior.
    let delegated = match &service.source {
        McpToolSource::UserManaged { .. } => Vec::new(),
        McpToolSource::Platform {
            downstream_service_id,
        } => {
            match delegation_service::resolve_delegated_credentials(
                db,
                encryption_keys,
                user_id,
                downstream_service_id,
            )
            .await
            {
                Ok(creds) => creds,
                Err(e) if node_route.is_some() => {
                    tracing::debug!(
                        service_id = %service.service_id,
                        error = %e,
                        "Server-side provider credentials unavailable; \
                         node agent will inject credentials"
                    );
                    vec![]
                }
                Err(e) => {
                    return Err(AppError::BadRequest(format!(
                        "Provider credentials not available: {e}"
                    )));
                }
            }
        }
    };

    // Content-Type header
    let req_headers = if service.is_generic_proxy {
        let mut h = reqwest::header::HeaderMap::new();
        if body.is_some() {
            h.insert(
                reqwest::header::CONTENT_TYPE,
                "application/json".parse().unwrap(),
            );
        }
        h
    } else {
        build_downstream_request_headers(endpoint, body.is_some())?
    };

    // -------------------------------------------------------------------
    // Route through node when a node route exists (primary + fallbacks).
    // Always attempt all nodes regardless of primary connection state --
    // send_proxy_request returns NodeOffline for disconnected nodes, then
    // we try fallbacks. Only fall through to direct forward_request when
    // all nodes fail AND the server holds a real credential.
    // -------------------------------------------------------------------
    if let Some(ref nr) = node_route {
        let method_str = method.to_string();

        let mut all_headers: Vec<(String, String)> = identity_headers.clone();
        for (name, value) in &req_headers {
            if let Ok(v) = value.to_str() {
                all_headers.push((name.to_string(), v.to_string()));
            }
        }

        // NyxID#356: service-level default headers must be injected on
        // node-routed MCP calls too, not just on the direct HTTP proxy
        // path. Without this, required defaults (e.g. `x-openclaw-scopes`)
        // would reach the downstream for regular proxy requests but go
        // missing for MCP tool invocations of the same service.
        all_headers = crate::models::default_request_header::merge_into_header_list(
            all_headers,
            &[
                target.catalog_default_headers.as_slice(),
                target.user_service_default_headers.as_slice(),
            ],
        );

        // Strip any default whose name collides with what the node
        // agent will append locally as the service credential. Matches
        // the trim on the node-routed HTTP / WS paths in
        // `handlers/proxy.rs`; without it, a default `x-api-key` (or
        // equivalent) would ride along in the frame and the node would
        // put the real credential on top of it, leaving two values on
        // the wire.
        if let Some(cred_name) = crate::services::proxy_service::credential_header_name(&target) {
            all_headers.retain(|(n, _)| !n.eq_ignore_ascii_case(&cred_name));
        }

        let node_request = NodeProxyRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            service_id: target.service.id.clone(),
            service_slug: target.service.slug.clone(),
            base_url: target.base_url.clone(),
            method: method_str,
            path: path.clone(),
            query: query.clone(),
            headers: all_headers,
            body: body.as_ref().map(|b| b.to_vec()),
        };

        let all_node_ids: Vec<&str> = std::iter::once(nr.node_id.as_str())
            .chain(nr.fallback_node_ids.iter().map(|s| s.as_str()))
            .collect();

        let mut last_error: Option<AppError> = None;
        for nid in &all_node_ids {
            let mut attempt = node_request.clone();
            attempt.request_id = uuid::Uuid::new_v4().to_string();

            let signing_secret = if config.node_hmac_signing_enabled {
                match node_service::get_node_signing_secret(db, encryption_keys, nid).await {
                    Ok(secret) => Some(secret),
                    Err(e @ AppError::NodeNotFound(_) | e @ AppError::NodeOffline(_)) => {
                        last_error = Some(e);
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            } else {
                None
            };

            match node_ws_manager
                .send_proxy_request(nid, attempt, signing_secret.as_ref().map(|s| s.as_slice()))
                .await
            {
                Ok(ProxyResponseType::Complete(resp)) => {
                    let body_text = String::from_utf8_lossy(&resp.body).to_string();
                    return Ok((resp.status, body_text));
                }
                Ok(ProxyResponseType::Streaming(mut rx)) => {
                    use crate::services::node_ws_manager::StreamChunk;
                    let mut status = 200u16;
                    let mut body_buf = Vec::new();
                    while let Some(chunk) = rx.recv().await {
                        match chunk {
                            StreamChunk::Start { status: s, .. } => {
                                status = s;
                            }
                            StreamChunk::Data(data) => {
                                body_buf.extend_from_slice(&data);
                            }
                            StreamChunk::End => break,
                            StreamChunk::Error(e) => {
                                return Ok((502, format!("Node streaming error: {e}")));
                            }
                            StreamChunk::Injected { .. } => {
                                // Metadata-only WS auth injection signal; MCP streaming
                                // responses must not include it in the response body.
                            }
                        }
                    }
                    return Ok((status, String::from_utf8_lossy(&body_buf).to_string()));
                }
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            }
        }

        // All nodes failed. Fall through to direct only when the server
        // holds a decrypt-able credential. node_managed keys and node-only
        // platform services have no server credential.
        if !has_server_credential {
            return Err(last_error
                .unwrap_or_else(|| AppError::NodeOffline("All node routes failed".to_string())));
        }
        // else: fall through to direct forward_request
    }

    // -------------------------------------------------------------------
    // Direct proxy (no node, or node offline with server credential fallback)
    // -------------------------------------------------------------------
    let response = proxy_service::forward_request(
        http_client,
        &target,
        method,
        &path,
        query.as_deref(),
        req_headers,
        proxy_service::ProxyBody::Buffered(body),
        identity_headers,
        delegated,
        None,
        token_exchange_cache,
        cloud_response_cache,
    )
    .await?;

    let status = response.status().as_u16();
    let body_text = response.text().await.map_err(|e| {
        tracing::error!("Failed to read downstream response: {e}");
        AppError::Internal("Failed to read downstream response".to_string())
    })?;

    Ok((status, body_text))
}

/// Build proxy arguments from a generic proxy tool call.
/// Extracts method, path, and body from the tool arguments directly.
fn build_generic_proxy_args(args: &serde_json::Value) -> AppResult<ProxyArgs> {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.trim_start_matches('/').to_string(),
        None => {
            return Err(AppError::BadRequest(
                "Missing required parameter: path".to_string(),
            ));
        }
    };

    let method = match args
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_uppercase()
        .as_str()
    {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
    };

    let body = args.get("body").and_then(|b| {
        if b.is_null() {
            return None;
        }
        let bytes = if let Some(s) = b.as_str() {
            s.as_bytes().to_vec()
        } else {
            serde_json::to_vec(b).ok()?
        };
        Some(bytes::Bytes::from(bytes))
    });

    Ok((method, path, None, Vec::new(), body))
}

// ---------------------------------------------------------------------------
// Meta-tool: nyx__search_tools
// ---------------------------------------------------------------------------

const MAX_SEARCH_RESULTS: usize = 25;

/// Result of searching all tools across all services.
pub struct SearchResult {
    pub matches: Vec<McpToolDefinition>,
    /// Service IDs that had matching tools.
    #[allow(dead_code)]
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
                let input_schema = if service.is_generic_proxy {
                    build_generic_proxy_input_schema()
                } else {
                    build_input_schema(endpoint)
                };
                matches.push(McpToolDefinition {
                    name,
                    description,
                    input_schema,
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
    // Load old-model connections
    let connections: Vec<UserServiceConnection> = db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find(doc! { "user_id": user_id, "is_active": true })
        .await?
        .try_collect()
        .await?;

    let connected_ids: HashSet<&str> = connections.iter().map(|c| c.service_id.as_str()).collect();

    // Load new-model AI Services -- exclude catalog services already provisioned
    let user_services: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! { "user_id": user_id, "is_active": true })
        .await?
        .try_collect()
        .await?;

    let user_service_catalog_ids: HashSet<&str> = user_services
        .iter()
        .filter_map(|us| us.catalog_service_id.as_deref())
        .collect();
    let user_service_slugs: HashSet<&str> =
        user_services.iter().map(|us| us.slug.as_str()).collect();

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
            // Already connected via old model
            if connected_ids.contains(svc.id.as_str()) {
                return false;
            }
            // Already provisioned as a UserService (by catalog ID or slug match)
            if user_service_catalog_ids.contains(svc.id.as_str()) {
                return false;
            }
            if user_service_slugs.contains(svc.slug.as_str()) {
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
            endpoint_id: String::new(),
            name: name.to_string(),
            description: Some(description.to_string()),
            method: "GET".to_string(),
            path: format!("/{name}"),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
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
            description: None,
            service_category: "connection".to_string(),
            endpoints,
            source: McpToolSource::Platform {
                downstream_service_id: id.to_string(),
            },
            is_generic_proxy: false,
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

        // Should only have the 6 meta-tools (4 core + 2 SSH)
        assert_eq!(tools.len(), 6);
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

        // 6 meta-tools + 1 weather tool (news excluded)
        assert_eq!(tools.len(), 7);
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

        // 6 meta-tools + 2 service tools
        assert_eq!(tools.len(), 8);
        assert!(tools.iter().any(|t| t.name == "weather__get_forecast"));
        assert!(tools.iter().any(|t| t.name == "news__headlines"));
    }

    #[test]
    fn build_input_schema_uses_base64_string_for_binary_bodies() {
        let endpoint = McpToolEndpoint {
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
            response_description: None,
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
            endpoint_id: String::new(),
            name: "upload_tarball".to_string(),
            description: Some("Upload a tarball".to_string()),
            method: "POST".to_string(),
            path: "/archives".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/x-tar".to_string()),
            request_body_required: true,
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
            name: "upload_tarball".to_string(),
            description: Some("Upload a tarball".to_string()),
            method: "POST".to_string(),
            path: "/archives".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/x-tar".to_string()),
            request_body_required: true,
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
        };

        let (_, _, _, _, body) = build_proxy_args(&endpoint, &serde_json::json!({}))
            .expect("optional wrapped JSON body should be allowed");

        assert!(body.is_none());
    }

    #[test]
    fn build_proxy_args_uses_alternate_body_field_when_body_param_exists() {
        use base64::Engine as _;

        let endpoint = McpToolEndpoint {
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: true,
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
            name: "submit_form".to_string(),
            description: Some("Submit a urlencoded form".to_string()),
            method: "POST".to_string(),
            path: "/forms".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/x-www-form-urlencoded".to_string()),
            request_body_required: true,
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
            name: "create_session".to_string(),
            description: Some("Create a session without a request body".to_string()),
            method: "POST".to_string(),
            path: "/sessions".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
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
            endpoint_id: String::new(),
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
            response_description: None,
        };

        assert_eq!(
            request_content_type_header_value(&endpoint, true),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn request_content_type_header_value_defaults_wildcard_binary_schema_to_octet_stream() {
        let endpoint = McpToolEndpoint {
            endpoint_id: String::new(),
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
            response_description: None,
        };

        assert_eq!(
            request_content_type_header_value(&endpoint, true),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn request_content_type_header_value_uses_endpoint_content_type() {
        let endpoint = McpToolEndpoint {
            endpoint_id: String::new(),
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
            response_description: None,
        };

        assert_eq!(
            request_content_type_header_value(&endpoint, true),
            Some("application/zip")
        );
    }

    #[test]
    fn request_content_type_header_value_omits_optional_body_without_payload() {
        let endpoint = McpToolEndpoint {
            endpoint_id: String::new(),
            name: "upload_skill".to_string(),
            description: Some("Upload a skill archive".to_string()),
            method: "POST".to_string(),
            path: "/skills".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: Some("application/zip".to_string()),
            request_body_required: false,
            response_description: None,
        };

        assert_eq!(request_content_type_header_value(&endpoint, false), None);
    }

    #[test]
    fn request_content_type_header_value_omits_default_json_without_payload() {
        let endpoint = McpToolEndpoint {
            endpoint_id: String::new(),
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
            response_description: None,
        };

        assert_eq!(request_content_type_header_value(&endpoint, false), None);
    }

    #[test]
    fn build_downstream_request_headers_sets_content_type_without_forcing_accept() {
        let endpoint = McpToolEndpoint {
            endpoint_id: String::new(),
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
            response_description: None,
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
