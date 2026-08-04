use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, NaiveDate, Utc};
use futures::TryStreamExt;
use mongodb::bson::{DateTime as BsonDateTime, doc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::{
    AgentServiceBinding, COLLECTION_NAME as AGENT_SERVICE_BINDINGS,
};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::{key_service, org_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Request / Response types ---

fn default_true() -> bool {
    true
}

/// Resolve the effective owner for an ApiKey mutation. Returns the owner's
/// user_id so the caller passes it to `key_service::*` for downstream
/// filtering. Blocks non-admin org members (who get
/// `OrgRoleInsufficient`).
///
/// `OrgMembership.allowed_service_ids` is keyed by `UserService.id`, but
/// a NyxID `ApiKey` is an *agent identity*, not a service -- it has its
/// own `allowed_service_ids` scope that bounds which services its
/// bearer can call at runtime. The membership scope is therefore not
/// applied at the resource level here; org admins manage every
/// org-owned API key as a unit.
///
/// Used by update / delete / rotate / per-key read handlers.
async fn resolve_api_key_write_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<String> {
    let key = state
        .db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &key.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this API key".to_string(),
        ));
    }
    Ok(key.user_id)
}

/// Read variant: allows all active members (not just admins) to view an
/// org-owned ApiKey's metadata. See `resolve_api_key_write_owner` for
/// why the membership scope is not applied at the resource level.
async fn resolve_api_key_read_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<String> {
    let key = state
        .db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &key.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    Ok(key.user_id)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub scopes: Option<String>,
    /// Accepts RFC 3339 ("2026-04-01T00:00:00Z") or date-only ("2026-04-01").
    pub expires_at: Option<String>,
    pub description: Option<String>,
    /// UserService IDs this key can access via proxy.
    /// Only enforced when `allow_all_services` is `false`. Providing values
    /// without also setting `allow_all_services: false` will silently store
    /// the list but **not** scope the key.
    #[serde(default)]
    pub allowed_service_ids: Vec<String>,
    /// Node IDs this key can route through.
    /// Only enforced when `allow_all_nodes` is `false`. Providing values
    /// without also setting `allow_all_nodes: false` will silently store
    /// the list but **not** scope the key.
    #[serde(default)]
    pub allowed_node_ids: Vec<String>,
    /// If true, key can access ALL of the user's external services.
    /// Default: `true` (backward compatible -- existing keys have no
    /// restrictions). Set to `false` together with a non-empty
    /// `allowed_service_ids` to create a scoped key.
    #[serde(default = "default_true")]
    pub allow_all_services: bool,
    /// If true, key can route through ALL of the user's nodes.
    /// Default: `true` (backward compatible). Set to `false` together with a
    /// non-empty `allowed_node_ids` to create a node-scoped key.
    #[serde(default = "default_true")]
    pub allow_all_nodes: bool,
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    pub platform: Option<String>,
    pub callback_url: Option<String>,
    /// When set, create this NyxID agent API key under the given org. The
    /// resulting `ApiKey.user_id` is the org's user id, making the key
    /// visible to every org admin for management. Callers using the key
    /// (via `NYXID_ACCESS_TOKEN`) authenticate as the org -- proxy calls
    /// see org-owned services directly. The caller must be an admin of
    /// the target org.
    pub target_org_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub scopes: Option<String>,
    pub allowed_service_ids: Option<Vec<String>>,
    pub allowed_node_ids: Option<Vec<String>>,
    pub allow_all_services: Option<bool>,
    pub allow_all_nodes: Option<bool>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub rate_limit_per_second: Option<Option<u32>>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub rate_limit_burst: Option<Option<u32>>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub platform: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub callback_url: Option<Option<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub key_prefix: String,
    /// The full API key. Shown only once at creation time.
    pub full_key: String,
    pub scopes: String,
    pub created_at: String,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_second: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_burst: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AllowedServiceInfo {
    pub id: String,
    pub slug: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AllowedNodeInfo {
    pub id: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub key_prefix: String,
    pub scopes: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub allowed_services: Vec<AllowedServiceInfo>,
    pub allowed_nodes: Vec<AllowedNodeInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_second: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_burst: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bindings_count: u64,
    /// Provenance: whether this key is owned directly by the caller or
    /// inherited from an org the caller is a member of. Mirrors the
    /// `credential_source` field on `/user-services`. Used by the frontend
    /// to filter the binding/scope pickers to services owned by the same
    /// owner (personal agent keys bind to personal services, org agent
    /// keys bind to the same org's services).
    pub credential_source: crate::handlers::user_services_handler::CredentialSourceResponse,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyResponse>,
}

fn default_usage_days() -> u32 {
    7
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ApiKeyUsageQuery {
    #[serde(default = "default_usage_days")]
    pub days: u32,
}

#[derive(Debug, Deserialize, ToSchema, Default)]
pub struct ApiKeyUsageListQuery {
    #[serde(default = "default_usage_days")]
    pub days: u32,
    /// When set, return aggregate usage for keys owned by the given org
    /// instead of the caller's personal scope. The caller must be an admin
    /// of that org. Mirrors the gating on `ApiKeyListQuery::org_id` so the
    /// Usage Dashboard can fan out the same way the Agent Keys table does
    /// (see ChronoAIProject/NyxID#542).
    pub org_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, Default)]
pub struct ApiKeyListQuery {
    /// When set, list keys owned by the given org instead of the caller's
    /// personal scope. The caller must be an admin of that org.
    pub org_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyServiceUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    pub service_slug: String,
    pub service_label: String,
    pub request_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyUsageBucket {
    pub date: String,
    pub request_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyUsageResponse {
    pub api_key_id: String,
    pub api_key_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    pub request_count: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub error_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    pub top_services: Vec<ApiKeyServiceUsage>,
    pub daily_buckets: Vec<ApiKeyUsageBucket>,
    /// Provenance: Personal vs Org. Lets the dashboard render the same
    /// owner badge the Agent Keys table renders, so org admins see whose
    /// keys each card belongs to (ChronoAIProject/NyxID#542). Mirrors
    /// `ApiKeyResponse::credential_source`.
    pub credential_source: crate::handlers::user_services_handler::CredentialSourceResponse,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyUsageListResponse {
    pub usage: Vec<ApiKeyUsageResponse>,
    pub since: String,
    pub days: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteApiKeyResponse {
    pub message: String,
}

// --- Enrichment ---

/// Batch-enrich a list of API keys by loading all referenced UserServices and
/// Nodes in two `$in` queries instead of N+1 individual lookups.
async fn enrich_api_keys_batch(
    state: &AppState,
    actor_user_id: &str,
    keys: &[ApiKey],
) -> AppResult<Vec<ApiKeyResponse>> {
    use crate::handlers::user_services_handler::CredentialSourceResponse;

    // Compute credential_source per key. Most batches contain keys from a
    // single owner (personal OR a single org), so cache by owner id to
    // avoid quadratic resolve_owner_access calls.
    let unique_owners: Vec<String> = keys
        .iter()
        .map(|k| k.user_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut source_cache: HashMap<String, CredentialSourceResponse> = HashMap::new();
    for owner in &unique_owners {
        let source = resolve_credential_source(state, actor_user_id, owner).await?;
        source_cache.insert(owner.clone(), source);
    }

    let key_ids: Vec<&str> = keys.iter().map(|k| k.id.as_str()).collect();

    // Collect all referenced IDs across all keys
    let all_service_ids: Vec<&str> = keys
        .iter()
        .flat_map(|k| k.allowed_service_ids.iter().map(|s| s.as_str()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let all_node_ids: Vec<&str> = keys
        .iter()
        .flat_map(|k| k.allowed_node_ids.iter().map(|s| s.as_str()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // Batch-load UserServices
    let service_map: HashMap<String, UserService> = if all_service_ids.is_empty() {
        HashMap::new()
    } else {
        let services: Vec<UserService> = state
            .db
            .collection::<UserService>(USER_SERVICES)
            .find(doc! { "_id": { "$in": &all_service_ids } })
            .await?
            .try_collect()
            .await?;
        services.into_iter().map(|s| (s.id.clone(), s)).collect()
    };

    // Collect catalog_service_ids for name resolution
    let catalog_ids: Vec<&str> = service_map
        .values()
        .filter_map(|s| s.catalog_service_id.as_deref())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let catalog_name_map: HashMap<String, String> = if catalog_ids.is_empty() {
        HashMap::new()
    } else {
        let catalog_services: Vec<DownstreamService> = state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": &catalog_ids } })
            .await?
            .try_collect()
            .await?;
        catalog_services
            .into_iter()
            .map(|ds| (ds.id.clone(), ds.name))
            .collect()
    };

    // Collect endpoint_ids for label resolution
    let endpoint_ids: Vec<&str> = service_map
        .values()
        .map(|s| s.endpoint_id.as_str())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let endpoint_label_map: HashMap<String, String> = if endpoint_ids.is_empty() {
        HashMap::new()
    } else {
        let endpoints: Vec<UserEndpoint> = state
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": &endpoint_ids } })
            .await?
            .try_collect()
            .await?;
        endpoints
            .into_iter()
            .map(|ep| (ep.id.clone(), ep.label))
            .collect()
    };

    // Batch-load Nodes
    let node_map: HashMap<String, Node> = if all_node_ids.is_empty() {
        HashMap::new()
    } else {
        let nodes: Vec<Node> = state
            .db
            .collection::<Node>(NODES)
            .find(doc! { "_id": { "$in": &all_node_ids } })
            .await?
            .try_collect()
            .await?;
        nodes.into_iter().map(|n| (n.id.clone(), n)).collect()
    };

    let binding_counts: HashMap<String, u64> = if key_ids.is_empty() {
        HashMap::new()
    } else {
        let bindings: Vec<AgentServiceBinding> = state
            .db
            .collection::<AgentServiceBinding>(AGENT_SERVICE_BINDINGS)
            .find(doc! { "api_key_id": { "$in": &key_ids } })
            .await?
            .try_collect()
            .await?;

        let mut counts = HashMap::new();
        for binding in bindings {
            *counts.entry(binding.api_key_id).or_insert(0) += 1;
        }
        counts
    };

    // Build responses
    let items = keys
        .iter()
        .map(|key| {
            let allowed_services: Vec<AllowedServiceInfo> = key
                .allowed_service_ids
                .iter()
                .filter_map(|sid| {
                    service_map.get(sid).map(|svc| {
                        let label = endpoint_label_map
                            .get(&svc.endpoint_id)
                            .cloned()
                            .unwrap_or_else(|| svc.slug.clone());
                        let catalog_service_name = svc
                            .catalog_service_id
                            .as_ref()
                            .and_then(|cid| catalog_name_map.get(cid).cloned());
                        AllowedServiceInfo {
                            id: svc.id.clone(),
                            slug: svc.slug.clone(),
                            label,
                            catalog_service_name,
                        }
                    })
                })
                .collect();

            let allowed_nodes: Vec<AllowedNodeInfo> = key
                .allowed_node_ids
                .iter()
                .filter_map(|nid| {
                    node_map.get(nid).map(|node| AllowedNodeInfo {
                        id: node.id.clone(),
                        name: node.name.clone(),
                        status: node.status.as_str().to_string(),
                    })
                })
                .collect();

            ApiKeyResponse {
                id: key.id.clone(),
                name: key.name.clone(),
                description: key.description.clone(),
                key_prefix: key.key_prefix.clone(),
                scopes: key.scopes.clone(),
                last_used_at: key.last_used_at.map(|dt| dt.to_rfc3339()),
                expires_at: key.expires_at.map(|dt| dt.to_rfc3339()),
                is_active: key.is_active,
                created_at: key.created_at.to_rfc3339(),
                allowed_service_ids: key.allowed_service_ids.clone(),
                allowed_node_ids: key.allowed_node_ids.clone(),
                allow_all_services: key.allow_all_services,
                allow_all_nodes: key.allow_all_nodes,
                allowed_services,
                allowed_nodes,
                rate_limit_per_second: key.rate_limit_per_second,
                rate_limit_burst: key.rate_limit_burst,
                platform: key.platform.clone(),
                callback_url: key.callback_url.clone(),
                bindings_count: binding_counts.get(&key.id).copied().unwrap_or(0),
                credential_source: source_cache
                    .get(&key.user_id)
                    .cloned()
                    .unwrap_or(CredentialSourceResponse::Personal),
            }
        })
        .collect();

    Ok(items)
}

#[derive(Default)]
struct ServiceUsageAccumulator {
    service_id: Option<String>,
    service_slug: String,
    service_label: String,
    request_count: u64,
    error_count: u64,
}

struct ApiKeyUsageAccumulator {
    api_key_id: String,
    api_key_name: String,
    platform: Option<String>,
    request_count: u64,
    error_count: u64,
    last_used_at: Option<DateTime<Utc>>,
    top_services: HashMap<String, ServiceUsageAccumulator>,
    daily_buckets: BTreeMap<String, (u64, u64)>,
}

impl ApiKeyUsageAccumulator {
    fn new(key: &ApiKey) -> Self {
        Self {
            api_key_id: key.id.clone(),
            api_key_name: key.name.clone(),
            platform: key.platform.clone(),
            request_count: 0,
            error_count: 0,
            last_used_at: key.last_used_at,
            top_services: HashMap::new(),
            daily_buckets: BTreeMap::new(),
        }
    }
}

async fn load_user_service_info_map(
    state: &AppState,
    user_id: &str,
) -> AppResult<HashMap<String, (String, String)>> {
    let services: Vec<UserService> = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! { "user_id": user_id })
        .await?
        .try_collect()
        .await?;

    let endpoint_ids: Vec<&str> = services
        .iter()
        .map(|service| service.endpoint_id.as_str())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let endpoint_label_map: HashMap<String, String> = if endpoint_ids.is_empty() {
        HashMap::new()
    } else {
        let endpoints: Vec<UserEndpoint> = state
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": &endpoint_ids } })
            .await?
            .try_collect()
            .await?;
        endpoints
            .into_iter()
            .map(|endpoint| (endpoint.id, endpoint.label))
            .collect()
    };

    let mut map: HashMap<String, (String, String)> = services
        .into_iter()
        .map(|service| {
            let label = endpoint_label_map
                .get(&service.endpoint_id)
                .cloned()
                .unwrap_or_else(|| service.slug.clone());
            (service.id, (service.slug, label))
        })
        .collect();

    // Include DownstreamService (catalog) records as fallback for audit logs
    // that reference old-path service IDs not in the user's UserService collection.
    let catalog_services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! {})
        .await?
        .try_collect()
        .await?;
    for ds in catalog_services {
        map.entry(ds.id).or_insert_with(|| (ds.slug, ds.name));
    }

    Ok(map)
}

fn extract_response_status(event_data: Option<&serde_json::Value>) -> Option<u16> {
    event_data
        .and_then(|value| value.get("response_status"))
        .and_then(|value| value.as_u64())
        .and_then(|status| u16::try_from(status).ok())
}

/// Classify an audit event as an error for Usage aggregation.
///
/// `proxy_request_denied` events are always errors (emitted for pre-proxy
/// failures like 403 scope-forbidden and 429 rate-limited — see
/// ChronoAIProject/NyxID#341). For other event types (e.g. `proxy_request`),
/// we fall back to the downstream response status.
fn is_error_event(event_type: &str, event_data: Option<&serde_json::Value>) -> bool {
    matches!(event_type, "proxy_request_denied")
        || extract_response_status(event_data).is_some_and(|status| status >= 400)
}

fn extract_service_usage_info(
    event_data: Option<&serde_json::Value>,
    service_info_map: &HashMap<String, (String, String)>,
) -> (String, Option<String>, String, String) {
    if let Some(provider_slug) = event_data
        .and_then(|value| value.get("provider_slug"))
        .and_then(|value| value.as_str())
    {
        return (
            format!("provider:{provider_slug}"),
            None,
            provider_slug.to_string(),
            provider_slug.to_string(),
        );
    }

    if let Some(service_id) = event_data
        .and_then(|value| value.get("service_id"))
        .and_then(|value| value.as_str())
    {
        if let Some((slug, label)) = service_info_map.get(service_id) {
            return (
                format!("service:{service_id}"),
                Some(service_id.to_string()),
                slug.clone(),
                label.clone(),
            );
        }

        return (
            format!("service:{service_id}"),
            Some(service_id.to_string()),
            service_id.to_string(),
            service_id.to_string(),
        );
    }

    (
        "unknown".to_string(),
        None,
        "unknown".to_string(),
        "Unknown".to_string(),
    )
}

/// Build the contiguous UTC date range (oldest first) that the usage chart
/// should cover. Always returns exactly `days` entries, ending on `today`.
fn usage_date_range(today: NaiveDate, days: u32) -> Vec<String> {
    let count = days.max(1);
    (0..count)
        .rev()
        .map(|offset| {
            let date = today - chrono::Duration::days(i64::from(offset));
            date.format("%Y-%m-%d").to_string()
        })
        .collect()
}

/// Resolve the wire-format `CredentialSourceResponse` for a single owner
/// from the perspective of the actor. The actor's own owner_id resolves to
/// `Personal`; an org owner resolves to `Org { name, avatar, role }` after
/// looking up display_name + avatar_url from the users collection.
///
/// Mirrors the resolution loop inside `enrich_api_keys_batch` but for a
/// single owner. The handler is expected to have already authorized the
/// access; if `resolve_owner_access` returns `Forbidden`, this function
/// falls back to `Personal` rather than error so the response shape stays
/// stable (the keys list will be empty anyway in that case).
async fn resolve_credential_source(
    state: &AppState,
    actor_user_id: &str,
    owner_user_id: &str,
) -> AppResult<crate::handlers::user_services_handler::CredentialSourceResponse> {
    use crate::handlers::user_services_handler::CredentialSourceResponse;
    use crate::services::user_service_service::CredentialSource;

    if owner_user_id == actor_user_id {
        return Ok(CredentialSourceResponse::Personal);
    }

    let access = org_service::resolve_owner_access(&state.db, actor_user_id, owner_user_id).await?;
    let source: CredentialSource = match access {
        org_service::OwnerAccess::Direct => CredentialSource::Personal,
        org_service::OwnerAccess::AsOrgAdmin { org_user_id, .. } => {
            let org = state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! { "_id": &org_user_id })
                .await?;
            let (org_name, org_avatar_url) = org
                .map(|u| (u.display_name, u.avatar_url))
                .unwrap_or((None, None));
            let org_name = org_name.unwrap_or_else(|| "Unnamed Org".to_string());
            CredentialSource::Org {
                org_user_id,
                org_name,
                org_avatar_url,
                role: crate::models::org_membership::OrgRole::Admin,
                allowed: true,
            }
        }
        org_service::OwnerAccess::AsOrgMember {
            org_user_id, role, ..
        } => {
            let org = state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! { "_id": &org_user_id })
                .await?;
            let (org_name, org_avatar_url) = org
                .map(|u| (u.display_name, u.avatar_url))
                .unwrap_or((None, None));
            let org_name = org_name.unwrap_or_else(|| "Unnamed Org".to_string());
            let allowed = role.can_proxy();
            CredentialSource::Org {
                org_user_id,
                org_name,
                org_avatar_url,
                role,
                allowed,
            }
        }
        org_service::OwnerAccess::Forbidden => CredentialSource::Personal,
    };
    Ok(source.into())
}

async fn build_api_key_usage(
    state: &AppState,
    actor_user_id: &str,
    owner_user_id: &str,
    keys: &[ApiKey],
    days: u32,
) -> AppResult<Vec<ApiKeyUsageResponse>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    let clamped_days = days.clamp(1, 30);
    let today = Utc::now().date_naive();
    let bucket_dates = usage_date_range(today, clamped_days);
    let start_date = today - chrono::Duration::days(i64::from(clamped_days - 1));
    let since = start_date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always valid")
        .and_utc();
    let since_bson = BsonDateTime::from_millis(since.timestamp_millis());
    let key_ids: Vec<&str> = keys.iter().map(|key| key.id.as_str()).collect();

    let service_info_map = load_user_service_info_map(state, owner_user_id).await?;
    let credential_source = resolve_credential_source(state, actor_user_id, owner_user_id).await?;

    let entries: Vec<AuditLog> = state
        .db
        .collection::<AuditLog>(AUDIT_LOG)
        .find(doc! {
            "user_id": owner_user_id,
            "api_key_id": { "$in": &key_ids },
            "event_type": {
                "$in": [
                    "proxy_request",
                    "proxy_request_denied",
                    "llm_proxy_request",
                    "llm_gateway_request",
                ]
            },
            "created_at": { "$gte": since_bson },
        })
        .sort(doc! { "created_at": 1 })
        .await?
        .try_collect()
        .await?;

    let mut usage_map: HashMap<String, ApiKeyUsageAccumulator> = keys
        .iter()
        .map(|key| (key.id.clone(), ApiKeyUsageAccumulator::new(key)))
        .collect();

    for entry in entries {
        let Some(api_key_id) = entry.api_key_id.as_ref() else {
            continue;
        };
        let Some(accumulator) = usage_map.get_mut(api_key_id) else {
            continue;
        };

        let is_error = is_error_event(entry.event_type.as_str(), entry.event_data.as_ref());

        accumulator.request_count += 1;
        if is_error {
            accumulator.error_count += 1;
        }
        accumulator.last_used_at = accumulator
            .last_used_at
            .map(|current| current.max(entry.created_at))
            .or(Some(entry.created_at));

        let bucket_key = entry.created_at.format("%Y-%m-%d").to_string();
        let bucket = accumulator
            .daily_buckets
            .entry(bucket_key)
            .or_insert((0, 0));
        bucket.0 += 1;
        if is_error {
            bucket.1 += 1;
        }

        let (service_key, service_id, service_slug, service_label) =
            extract_service_usage_info(entry.event_data.as_ref(), &service_info_map);
        let service_usage = accumulator
            .top_services
            .entry(service_key)
            .or_insert_with(|| ServiceUsageAccumulator {
                service_id,
                service_slug,
                service_label,
                ..ServiceUsageAccumulator::default()
            });
        service_usage.request_count += 1;
        if is_error {
            service_usage.error_count += 1;
        }
    }

    let mut usage: Vec<ApiKeyUsageResponse> = usage_map
        .into_values()
        .map(|accumulator| {
            let mut top_services: Vec<ApiKeyServiceUsage> = accumulator
                .top_services
                .into_values()
                .map(|service| ApiKeyServiceUsage {
                    service_id: service.service_id,
                    service_slug: service.service_slug,
                    service_label: service.service_label,
                    request_count: service.request_count,
                    error_count: service.error_count,
                })
                .collect();
            top_services.sort_by(|left, right| {
                right
                    .request_count
                    .cmp(&left.request_count)
                    .then_with(|| left.service_slug.cmp(&right.service_slug))
            });
            top_services.truncate(5);

            let mut bucket_map = accumulator.daily_buckets;
            for date in &bucket_dates {
                bucket_map.entry(date.clone()).or_insert((0, 0));
            }
            let daily_buckets = bucket_map
                .into_iter()
                .map(|(date, (request_count, error_count))| ApiKeyUsageBucket {
                    date,
                    request_count,
                    error_count,
                })
                .collect::<Vec<_>>();

            let success_count = accumulator
                .request_count
                .saturating_sub(accumulator.error_count);
            let error_rate = if accumulator.request_count == 0 {
                0.0
            } else {
                accumulator.error_count as f64 / accumulator.request_count as f64
            };

            ApiKeyUsageResponse {
                api_key_id: accumulator.api_key_id,
                api_key_name: accumulator.api_key_name,
                platform: accumulator.platform,
                request_count: accumulator.request_count,
                success_count,
                error_count: accumulator.error_count,
                error_rate,
                last_used_at: accumulator.last_used_at.map(|dt| dt.to_rfc3339()),
                top_services,
                daily_buckets,
                credential_source: credential_source.clone(),
            }
        })
        .collect();

    usage.sort_by(|left, right| {
        right
            .request_count
            .cmp(&left.request_count)
            .then_with(|| left.api_key_name.cmp(&right.api_key_name))
    });

    Ok(usage)
}

// --- Handlers ---

#[utoipa::path(
    get,
    path = "/api/v1/api-keys",
    responses(
        (status = 200, description = "List of NyxID API keys", body = ApiKeyListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// GET /api/v1/api-keys
///
/// Defaults to listing the caller's personal API keys. Pass `?org_id=X`
/// to list keys owned by an org (the caller must be an admin of that org).
pub async fn list_keys(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ApiKeyListQuery>,
) -> AppResult<Json<ApiKeyListResponse>> {
    let actor = auth_user.user_id.to_string();
    let user_id_str = if let Some(target_org_id) = query.org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its API keys".to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor.clone()
    };
    let keys = key_service::list_api_keys(&state.db, &user_id_str).await?;
    let items = enrich_api_keys_batch(&state, &actor, &keys).await?;
    Ok(Json(ApiKeyListResponse { keys: items }))
}

#[utoipa::path(
    get,
    path = "/api/v1/api-keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "API key ID")
    ),
    responses(
        (status = 200, description = "API key details", body = ApiKeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "API key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// GET /api/v1/api-keys/{key_id}
pub async fn get_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<ApiKeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let user_id_str = resolve_api_key_read_owner(&state, &actor, &key_id).await?;
    let key = key_service::get_api_key(&state.db, &user_id_str, &key_id).await?;
    let enriched = enrich_api_keys_batch(&state, &actor, &[key]).await?;
    Ok(Json(enriched.into_iter().next().unwrap()))
}

#[utoipa::path(
    get,
    path = "/api/v1/api-keys/usage",
    params(
        ("days" = Option<u32>, Query, description = "Number of trailing days to aggregate (1-30)"),
        ("org_id" = Option<String>, Query, description = "Org owner scope. When set, returns usage for org-owned keys; caller must be admin of that org.")
    ),
    responses(
        (status = 200, description = "Usage summary for the user's API keys", body = ApiKeyUsageListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 403, description = "Caller is not an admin of the requested org", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// GET /api/v1/api-keys/usage
///
/// Defaults to aggregating usage for the caller's personal API keys. Pass
/// `?org_id=X` to aggregate usage for keys owned by an org (the caller must
/// be an admin of that org). The frontend Usage Dashboard fans out one
/// request per scope (personal + each admined org) and merges by
/// `api_key_id`, mirroring the Agent Keys table on the same page
/// (ChronoAIProject/NyxID#542).
pub async fn list_key_usage(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ApiKeyUsageListQuery>,
) -> AppResult<Json<ApiKeyUsageListResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = if let Some(target_org_id) = query.org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its API key usage".to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor.clone()
    };
    let days = query.days.clamp(1, 30);
    let keys = key_service::list_api_keys(&state.db, &owner_id).await?;
    let usage = build_api_key_usage(&state, &actor, &owner_id, &keys, days).await?;
    let since = (Utc::now() - chrono::Duration::days(i64::from(days))).to_rfc3339();

    Ok(Json(ApiKeyUsageListResponse { usage, since, days }))
}

#[utoipa::path(
    get,
    path = "/api/v1/api-keys/{key_id}/usage",
    params(
        ("key_id" = String, Path, description = "API key ID"),
        ("days" = Option<u32>, Query, description = "Number of trailing days to aggregate (1-30)")
    ),
    responses(
        (status = 200, description = "Usage summary for a specific API key", body = ApiKeyUsageResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "API key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// GET /api/v1/api-keys/{key_id}/usage
pub async fn get_key_usage(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
    Query(query): Query<ApiKeyUsageQuery>,
) -> AppResult<Json<ApiKeyUsageResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_api_key_read_owner(&state, &actor, &key_id).await?;
    let days = query.days.clamp(1, 30);
    let key = key_service::get_api_key(&state.db, &owner_id, &key_id).await?;
    let mut usage = build_api_key_usage(&state, &actor, &owner_id, &[key], days).await?;
    let response = usage
        .pop()
        .ok_or_else(|| AppError::NotFound("API key usage not found".to_string()))?;
    Ok(Json(response))
}

/// Parse an optional expiry date string. Accepts RFC 3339 datetime
/// (e.g. "2026-04-01T00:00:00Z") or date-only (e.g. "2026-04-01").
fn parse_expires_at(s: &str) -> AppResult<DateTime<Utc>> {
    // Try RFC 3339 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Try date-only (YYYY-MM-DD) -> end of day UTC
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        && let Some(dt) = date.and_hms_opt(23, 59, 59)
    {
        return Ok(dt.and_utc());
    }
    Err(AppError::ValidationError(
        "Invalid expires_at format. Use RFC 3339 (e.g. 2026-04-01T00:00:00Z) or date-only (e.g. 2026-04-01)".to_string(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/api-keys",
    request_body = CreateApiKeyRequest,
    responses(
        (status = 200, description = "Created NyxID API key (full key shown once)", body = CreateApiKeyResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// POST /api/v1/api-keys
pub async fn create_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateApiKeyRequest>,
) -> AppResult<Json<CreateApiKeyResponse>> {
    auth_user.ensure_write_scope()?;

    if body.name.is_empty() {
        return Err(AppError::ValidationError(
            "API key name is required".to_string(),
        ));
    }

    let scopes = body.scopes.as_deref().unwrap_or("read");

    let expires_at = body
        .expires_at
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(parse_expires_at)
        .transpose()?;

    if let Some(exp) = expires_at
        && exp <= Utc::now()
    {
        return Err(AppError::ValidationError(
            "expires_at must be in the future".to_string(),
        ));
    }

    let actor = auth_user.user_id.to_string();

    // If `target_org_id` is set, write the key under the org's user_id so
    // every admin of that org can manage it and every consumer of the key
    // authenticates as the org. The caller must be an admin of the target.
    // `allowed_service_ids`/`allowed_node_ids` scopes are then validated
    // against the org's owned resources, which is the intended behavior --
    // an org-owned API key can only scope to org-owned services.
    let user_id_str = if let Some(target_org_id) = body.target_org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create API keys under it".to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor
    };

    let created = key_service::create_api_key(
        &state.db,
        &user_id_str,
        &body.name,
        scopes,
        expires_at,
        body.description.as_deref(),
        Some(&body.allowed_service_ids),
        Some(&body.allowed_node_ids),
        Some(body.allow_all_services),
        Some(body.allow_all_nodes),
        body.rate_limit_per_second,
        body.rate_limit_burst,
        body.platform.as_deref(),
        body.callback_url.as_deref(),
    )
    .await?;

    // Telemetry: api_key.created. `scope_mode` collapses the two
    // allow-all flags into a single enum: "all" when both are unrestricted,
    // "scoped" otherwise (either services or nodes are pinned).
    let scope_mode = if created.allow_all_services && created.allow_all_nodes {
        "all"
    } else {
        "scoped"
    };
    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::ApiKeyCreated {
            platform: created.platform.clone(),
            scope_mode: scope_mode.to_string(),
            rate_limit_per_second: created.rate_limit_per_second,
        },
    );

    Ok(Json(CreateApiKeyResponse {
        id: created.id,
        name: created.name,
        description: created.description,
        key_prefix: created.key_prefix,
        full_key: created.full_key,
        scopes: created.scopes,
        created_at: created.created_at.to_rfc3339(),
        allowed_service_ids: created.allowed_service_ids,
        allowed_node_ids: created.allowed_node_ids,
        allow_all_services: created.allow_all_services,
        allow_all_nodes: created.allow_all_nodes,
        rate_limit_per_second: created.rate_limit_per_second,
        rate_limit_burst: created.rate_limit_burst,
        platform: created.platform,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/api-keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "API key ID")
    ),
    request_body = UpdateApiKeyRequest,
    responses(
        (status = 200, description = "Updated API key", body = ApiKeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "API key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// PUT /api/v1/api-keys/{key_id}
pub async fn update_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
    Json(body): Json<UpdateApiKeyRequest>,
) -> AppResult<Json<ApiKeyResponse>> {
    auth_user.ensure_write_scope()?;

    let actor = auth_user.user_id.to_string();
    let user_id_str = resolve_api_key_write_owner(&state, &actor, &key_id).await?;

    let updated = key_service::update_api_key_scope(
        &state.db,
        &user_id_str,
        &key_id,
        body.name.as_deref(),
        body.description.as_deref(),
        body.scopes.as_deref(),
        body.allowed_service_ids.as_deref(),
        body.allowed_node_ids.as_deref(),
        body.allow_all_services,
        body.allow_all_nodes,
        body.rate_limit_per_second,
        body.rate_limit_burst,
        body.platform.as_ref().map(|platform| platform.as_deref()),
        body.callback_url.as_ref().map(|url| url.as_deref()),
    )
    .await?;

    let enriched = enrich_api_keys_batch(&state, &actor, &[updated]).await?;
    Ok(Json(enriched.into_iter().next().unwrap()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/api-keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "API key ID")
    ),
    responses(
        (status = 200, description = "API key deleted", body = DeleteApiKeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "API key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// DELETE /api/v1/api-keys/{key_id}
pub async fn delete_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(key_id): Path<String>,
) -> AppResult<Json<DeleteApiKeyResponse>> {
    auth_user.ensure_write_scope()?;

    let actor = auth_user.user_id.to_string();
    let user_id_str = resolve_api_key_write_owner(&state, &actor, &key_id).await?;

    // Look up platform before delete so we can attribute the event. The
    // delete path only carries `key_id`, so the record must be fetched while
    // it still exists.
    let pre_delete_platform = key_service::get_api_key(&state.db, &user_id_str, &key_id)
        .await
        .ok()
        .and_then(|k| k.platform);

    key_service::delete_api_key(&state.db, &user_id_str, &key_id).await?;

    // Telemetry: api_key.deleted.
    emit_event(
        state.telemetry.as_deref(),
        &actor,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::ApiKeyDeleted {
            platform: pre_delete_platform,
        },
    );

    Ok(Json(DeleteApiKeyResponse {
        message: "API key deleted".to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/api-keys/{key_id}/rotate",
    params(
        ("key_id" = String, Path, description = "API key ID")
    ),
    responses(
        (status = 200, description = "Rotated API key (new full key shown once)", body = CreateApiKeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "API key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "API Keys"
)]
/// POST /api/v1/api-keys/{key_id}/rotate
pub async fn rotate_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(key_id): Path<String>,
) -> AppResult<Json<CreateApiKeyResponse>> {
    auth_user.ensure_write_scope()?;

    let actor = auth_user.user_id.to_string();
    let user_id_str = resolve_api_key_write_owner(&state, &actor, &key_id).await?;
    let created = key_service::rotate_api_key(&state.db, &user_id_str, &key_id).await?;

    // Telemetry: api_key.rotated.
    emit_event(
        state.telemetry.as_deref(),
        &actor,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::ApiKeyRotated {
            platform: created.platform.clone(),
        },
    );

    Ok(Json(CreateApiKeyResponse {
        id: created.id,
        name: created.name,
        description: created.description,
        key_prefix: created.key_prefix,
        full_key: created.full_key,
        scopes: created.scopes,
        created_at: created.created_at.to_rfc3339(),
        allowed_service_ids: created.allowed_service_ids,
        allowed_node_ids: created.allowed_node_ids,
        allow_all_services: created.allow_all_services,
        allow_all_nodes: created.allow_all_nodes,
        rate_limit_per_second: created.rate_limit_per_second,
        rate_limit_burst: created.rate_limit_burst,
        platform: created.platform,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        UpdateApiKeyRequest, extract_response_status, is_error_event, parse_expires_at,
        usage_date_range,
    };
    use chrono::{Duration, NaiveDate, Utc};
    use serde_json::json;

    #[test]
    fn parse_expires_at_accepts_future_rfc3339() {
        let future = (Utc::now() + Duration::days(7)).to_rfc3339();
        assert!(parse_expires_at(&future).is_ok());
    }

    #[test]
    fn parse_expires_at_accepts_past_dates_string_validation_is_handler_responsibility() {
        // parse_expires_at itself only parses; the handler enforces "must be in the future".
        assert!(parse_expires_at("2020-01-01").is_ok());
    }

    #[test]
    fn parse_expires_at_rejects_garbage() {
        assert!(parse_expires_at("not-a-date").is_err());
    }

    #[test]
    fn usage_date_range_returns_seven_contiguous_days() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        let dates = usage_date_range(today, 7);
        assert_eq!(
            dates,
            vec![
                "2026-04-09",
                "2026-04-10",
                "2026-04-11",
                "2026-04-12",
                "2026-04-13",
                "2026-04-14",
                "2026-04-15",
            ],
        );
    }

    #[test]
    fn usage_date_range_handles_single_day() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        assert_eq!(usage_date_range(today, 1), vec!["2026-04-15"]);
    }

    #[test]
    fn usage_date_range_clamps_zero_to_today_only() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        assert_eq!(usage_date_range(today, 0), vec!["2026-04-15"]);
    }

    #[test]
    fn usage_date_range_spans_month_boundary() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let dates = usage_date_range(today, 7);
        assert_eq!(
            dates,
            vec![
                "2026-04-26",
                "2026-04-27",
                "2026-04-28",
                "2026-04-29",
                "2026-04-30",
                "2026-05-01",
                "2026-05-02",
            ],
        );
    }

    #[test]
    fn platform_absent_means_no_change() {
        let req: UpdateApiKeyRequest = serde_json::from_str(r#"{"name": "k"}"#).unwrap();
        assert!(req.platform.is_none());
    }

    // Regression tests for ChronoAIProject/NyxID#542: the Usage Dashboard
    // omitted shared org-owned agent keys because `list_key_usage` ignored
    // the `org_id` query param. These tests pin the four scopes the
    // frontend now fans out across (personal, admin-of-org, member-of-org,
    // non-member-of-org) so the gate stays in lock-step with `list_keys`.
    mod list_key_usage_org_scope {
        use super::super::*;
        use crate::handlers::user_services_handler::CredentialSourceResponse;
        use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
        use crate::models::org_membership::{
            COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
        };
        use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
        use crate::test_utils::{
            connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        };
        use axum::Json;
        use axum::extract::{Query, State};
        use chrono::Utc;
        use uuid::Uuid;

        fn fixture_api_key(id: &str, owner_user_id: &str, name: &str) -> ApiKey {
            ApiKey {
                id: id.to_string(),
                user_id: owner_user_id.to_string(),
                name: name.to_string(),
                key_prefix: "nyxid_ag_test".to_string(),
                key_hash: "0123456789abcdef".to_string(),
                scopes: String::new(),
                callback_url: None,
                description: None,
                is_active: true,
                created_at: Utc::now(),
                last_used_at: None,
                expires_at: None,
                allow_all_services: true,
                allow_all_nodes: true,
                allowed_service_ids: Vec::new(),
                allowed_node_ids: Vec::new(),
                rate_limit_per_second: None,
                rate_limit_burst: None,
                platform: None,
            }
        }

        async fn seed_actor_and_org_with_keys(
            db: &mongodb::Database,
        ) -> (String, String, String, String) {
            let actor_id = Uuid::new_v4().to_string();
            let org_id = Uuid::new_v4().to_string();
            let personal_key_id = Uuid::new_v4().to_string();
            let org_key_id = Uuid::new_v4().to_string();

            db.collection::<User>(USERS)
                .insert_one(test_user(&actor_id, UserType::Person))
                .await
                .expect("insert actor");
            db.collection::<User>(USERS)
                .insert_one(test_user(&org_id, UserType::Org))
                .await
                .expect("insert org");
            db.collection::<ApiKey>(API_KEYS)
                .insert_many([
                    fixture_api_key(&personal_key_id, &actor_id, "personal-agent"),
                    fixture_api_key(&org_key_id, &org_id, "shared-org-agent"),
                ])
                .await
                .expect("insert api keys");

            (actor_id, org_id, personal_key_id, org_key_id)
        }

        #[tokio::test]
        async fn no_org_id_returns_only_personal_keys_with_personal_source() {
            let Some(db) = connect_test_database("usage_personal_scope").await else {
                eprintln!("Skipping MongoDB-backed test; no test database available");
                return;
            };

            let (actor_id, _org_id, personal_key_id, org_key_id) =
                seed_actor_and_org_with_keys(&db).await;
            let state = test_app_state(db);
            let auth = test_auth_user(&actor_id);

            let Json(resp) = list_key_usage(
                State(state),
                auth,
                Query(ApiKeyUsageListQuery {
                    days: 7,
                    org_id: None,
                }),
            )
            .await
            .expect("list usage");

            let ids: Vec<String> = resp.usage.iter().map(|u| u.api_key_id.clone()).collect();
            assert!(ids.contains(&personal_key_id), "personal key visible");
            assert!(
                !ids.contains(&org_key_id),
                "org key excluded from personal scope"
            );
            assert!(
                resp.usage
                    .iter()
                    .all(|u| matches!(u.credential_source, CredentialSourceResponse::Personal)),
                "personal scope tags every key as Personal"
            );
        }

        #[tokio::test]
        async fn org_id_as_admin_returns_org_keys_tagged_org() {
            let Some(db) = connect_test_database("usage_org_admin").await else {
                eprintln!("Skipping MongoDB-backed test; no test database available");
                return;
            };

            let (actor_id, org_id, personal_key_id, org_key_id) =
                seed_actor_and_org_with_keys(&db).await;
            db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
                .await
                .expect("insert admin membership");

            let state = test_app_state(db);
            let auth = test_auth_user(&actor_id);

            let Json(resp) = list_key_usage(
                State(state),
                auth,
                Query(ApiKeyUsageListQuery {
                    days: 7,
                    org_id: Some(org_id.clone()),
                }),
            )
            .await
            .expect("list usage as admin");

            let ids: Vec<String> = resp.usage.iter().map(|u| u.api_key_id.clone()).collect();
            assert!(ids.contains(&org_key_id), "org key visible under org scope");
            assert!(
                !ids.contains(&personal_key_id),
                "personal key excluded from org scope"
            );
            for entry in &resp.usage {
                match &entry.credential_source {
                    CredentialSourceResponse::Org {
                        org_id: tag_org_id, ..
                    } => {
                        assert_eq!(tag_org_id, &org_id);
                    }
                    CredentialSourceResponse::Personal => {
                        panic!("org-scoped usage must not tag entries as Personal");
                    }
                }
            }
        }

        #[tokio::test]
        async fn org_id_as_member_is_forbidden() {
            let Some(db) = connect_test_database("usage_org_member").await else {
                eprintln!("Skipping MongoDB-backed test; no test database available");
                return;
            };

            let (actor_id, org_id, _, _) = seed_actor_and_org_with_keys(&db).await;
            db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                .insert_one(test_membership(&org_id, &actor_id, OrgRole::Member, None))
                .await
                .expect("insert member membership");

            let state = test_app_state(db);
            let auth = test_auth_user(&actor_id);

            let err = list_key_usage(
                State(state),
                auth,
                Query(ApiKeyUsageListQuery {
                    days: 7,
                    org_id: Some(org_id),
                }),
            )
            .await
            .expect_err("members are not admins");
            assert!(
                matches!(err, AppError::OrgRoleInsufficient(_)),
                "members must hit the same gate as list_keys"
            );
        }

        #[tokio::test]
        async fn org_id_as_non_member_is_forbidden() {
            let Some(db) = connect_test_database("usage_org_nonmember").await else {
                eprintln!("Skipping MongoDB-backed test; no test database available");
                return;
            };

            let (actor_id, org_id, _, _) = seed_actor_and_org_with_keys(&db).await;
            // Deliberately do NOT insert a membership.
            let state = test_app_state(db);
            let auth = test_auth_user(&actor_id);

            let err = list_key_usage(
                State(state),
                auth,
                Query(ApiKeyUsageListQuery {
                    days: 7,
                    org_id: Some(org_id),
                }),
            )
            .await
            .expect_err("non-members must not list org usage");
            assert!(matches!(err, AppError::OrgRoleInsufficient(_)));
        }
    }

    #[test]
    fn platform_null_means_clear() {
        let req: UpdateApiKeyRequest = serde_json::from_str(r#"{"platform": null}"#).unwrap();
        assert_eq!(req.platform, Some(None));
    }

    #[test]
    fn platform_value_means_set() {
        let req: UpdateApiKeyRequest =
            serde_json::from_str(r#"{"platform": "claude-code"}"#).unwrap();
        assert_eq!(req.platform, Some(Some("claude-code".to_string())));
    }

    #[test]
    fn callback_url_null_means_clear() {
        let req: UpdateApiKeyRequest = serde_json::from_str(r#"{"callback_url": null}"#).unwrap();
        assert_eq!(req.callback_url, Some(None));
    }

    #[test]
    fn callback_url_empty_string_deserializes_as_present() {
        let req: UpdateApiKeyRequest = serde_json::from_str(r#"{"callback_url": ""}"#).unwrap();
        assert_eq!(req.callback_url, Some(Some(String::new())));
    }

    #[test]
    fn rate_limit_null_means_clear() {
        let req: UpdateApiKeyRequest =
            serde_json::from_str(r#"{"rate_limit_per_second": null}"#).unwrap();
        assert_eq!(req.rate_limit_per_second, Some(None));
    }

    // Regression tests for ChronoAIProject/NyxID#341: pre-proxy failures
    // (403 scope-forbidden and 429 rate-limited) emit `proxy_request_denied`
    // audit events and MUST be counted as errors in Usage aggregation.

    #[test]
    fn proxy_request_denied_counts_as_error() {
        assert!(is_error_event("proxy_request_denied", None));
    }

    #[test]
    fn rate_limited_denial_counts_as_error() {
        let data = json!({
            "service_id": "svc-1",
            "denial_reason": "rate_limited",
            "response_status": 429,
        });
        assert!(is_error_event("proxy_request_denied", Some(&data)));
    }

    #[test]
    fn scope_forbidden_service_denial_counts_as_error() {
        let data = json!({
            "service_id": "svc-1",
            "user_service_id": "us-1",
            "denial_reason": "api_key_scope_forbidden_service",
            "response_status": 403,
        });
        assert!(is_error_event("proxy_request_denied", Some(&data)));
    }

    #[test]
    fn scope_forbidden_node_denial_counts_as_error() {
        let data = json!({
            "service_id": "svc-1",
            "node_id": "node-1",
            "denial_reason": "api_key_scope_forbidden_node",
            "response_status": 403,
        });
        assert!(is_error_event("proxy_request_denied", Some(&data)));
    }

    #[test]
    fn scope_forbidden_legacy_denial_counts_as_error() {
        let data = json!({
            "service_id": "svc-1",
            "denial_reason": "api_key_scope_forbidden_legacy",
            "response_status": 403,
        });
        assert!(is_error_event("proxy_request_denied", Some(&data)));
    }

    #[test]
    fn successful_proxy_request_is_not_an_error() {
        let data = json!({
            "service_id": "svc-1",
            "response_status": 200,
        });
        assert!(!is_error_event("proxy_request", Some(&data)));
    }

    #[test]
    fn downstream_4xx_proxy_request_counts_as_error() {
        // Sanity check: the existing contract for 400/401/503 still holds.
        for status in [400u16, 401, 403, 429, 503] {
            let data = json!({
                "service_id": "svc-1",
                "response_status": status,
            });
            assert!(
                is_error_event("proxy_request", Some(&data)),
                "expected status {status} to count as error"
            );
        }
    }

    #[test]
    fn extract_response_status_returns_denial_status() {
        let data = json!({ "response_status": 403 });
        assert_eq!(extract_response_status(Some(&data)), Some(403));
    }

    #[test]
    fn extract_response_status_returns_none_for_missing_field() {
        assert_eq!(extract_response_status(Some(&json!({}))), None);
        assert_eq!(extract_response_status(None), None);
    }

    #[test]
    fn extract_response_status_ignores_non_numeric() {
        assert_eq!(
            extract_response_status(Some(&json!({"response_status": "ok"}))),
            None
        );
    }

    #[test]
    fn parse_expires_at_accepts_date_only_format() {
        let dt = parse_expires_at("2026-06-15").unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-06-15");
    }

    #[test]
    fn is_zero_returns_true_for_zero() {
        assert!(super::is_zero(&0));
        assert!(!super::is_zero(&1));
    }

    #[test]
    fn default_usage_days_is_seven() {
        assert_eq!(super::default_usage_days(), 7);
    }

    #[test]
    fn default_true_returns_true() {
        assert!(super::default_true());
    }

    #[test]
    fn proxy_request_no_status_is_not_error() {
        assert!(!is_error_event(
            "proxy_request",
            Some(&json!({"service_id": "s"}))
        ));
    }

    #[test]
    fn usage_date_range_thirty_days() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        let dates = usage_date_range(today, 30);
        assert_eq!(dates.len(), 30);
        assert_eq!(dates[0], "2026-04-26");
        assert_eq!(dates[29], "2026-05-25");
    }
}
