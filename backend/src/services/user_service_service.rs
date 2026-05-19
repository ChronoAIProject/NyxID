use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::org_membership::OrgRole;
use crate::models::ssh_auth_mode::SshAuthMode;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_api_key::COLLECTION_NAME as USER_API_KEYS;
use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
use crate::models::user_service::{COLLECTION_NAME, UserService};
use crate::models::ws_frame_injection::WsFrameInjection;
use crate::services::{
    agent_binding_service, audit_service, node_service, org_service, ws_frame_injector,
};

/// Valid auth methods for user services.
///
/// - `bearer`: `Authorization: Bearer <credential>` (standard OAuth bearer)
/// - `bot_bearer`: `Authorization: Bot <credential>` (Discord bot tokens)
/// - `header`: custom header named by `auth_key_name` set to `<credential>`
/// - `query`: URL query parameter `<auth_key_name>=<credential>`
/// - `basic`: HTTP Basic auth, credential is `username:password`
/// - `body`: merge `{<auth_key_name>: <credential>}` into the JSON request
///   body (POST/PUT/PATCH only) for providers that require credentials in
///   the payload rather than a header
/// - `token_exchange`: credential is a JSON blob; the proxy exchanges it
///   for a short-lived access token using the service's
///   `TokenExchangeConfig`, caches the result, and injects the token on
///   every outbound request (Lark/Feishu tenant tokens, OAuth 2.0
///   client_credentials, etc.)
/// - `path`: inject credential into URL path as a prefix segment
///   (`/<auth_key_name><credential>/...`), e.g. Telegram Bot API
///   (`/bot<token>/sendMessage`)
/// - `none`: no credential injection
const VALID_AUTH_METHODS: &[&str] = &[
    "bearer",
    "bot_bearer",
    "header",
    "query",
    "basic",
    "body",
    "token_exchange",
    "path",
    // AWS cloud-billing method. Credential is a JSON blob (see
    // `nyxid_cloud_auth::aws_sigv4::AwsCredentials`); signing happens
    // at the proxy boundary. `auth_key_name` is unused. NyxID#716.
    "aws_sigv4",
    "none",
];

/// Valid identity propagation modes.
const VALID_IDENTITY_MODES: &[&str] = &["none", "headers", "jwt", "both"];
const VALID_DELEGATION_SCOPES: &[&str] = &["llm:proxy", "proxy:*", "llm:status"];

/// Identity propagation and delegation token configuration.
#[derive(Clone, Debug)]
pub struct IdentityConfig {
    pub identity_propagation_mode: String,
    pub identity_include_user_id: bool,
    pub identity_include_email: bool,
    pub identity_include_name: bool,
    pub identity_jwt_audience: Option<String>,
    pub forward_access_token: bool,
    pub inject_delegation_token: bool,
    pub delegation_token_scope: String,
}

impl IdentityConfig {
    pub fn none() -> Self {
        Self {
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
        }
    }
}

fn validate_identity_config(config: &IdentityConfig) -> AppResult<()> {
    if !VALID_IDENTITY_MODES.contains(&config.identity_propagation_mode.as_str()) {
        return Err(AppError::ValidationError(format!(
            "Invalid identity_propagation_mode '{}'. Valid: {}",
            config.identity_propagation_mode,
            VALID_IDENTITY_MODES.join(", ")
        )));
    }

    if let Some(audience) = config.identity_jwt_audience.as_deref()
        && audience.len() > 2048
    {
        return Err(AppError::ValidationError(
            "identity_jwt_audience must not exceed 2048 characters".to_string(),
        ));
    }

    for scope in config.delegation_token_scope.split_whitespace() {
        if !VALID_DELEGATION_SCOPES.contains(&scope) {
            return Err(AppError::ValidationError(format!(
                "Invalid delegation_token_scope '{}'. Must be one of: {}",
                scope,
                VALID_DELEGATION_SCOPES.join(", ")
            )));
        }
    }

    Ok(())
}

fn normalize_identity_config(config: &IdentityConfig) -> AppResult<IdentityConfig> {
    validate_identity_config(config)?;

    let normalized_scope = {
        let scopes: Vec<&str> = config.delegation_token_scope.split_whitespace().collect();
        if scopes.is_empty() {
            "llm:proxy".to_string()
        } else {
            scopes.join(" ")
        }
    };

    Ok(IdentityConfig {
        identity_propagation_mode: config.identity_propagation_mode.clone(),
        identity_include_user_id: config.identity_include_user_id,
        identity_include_email: config.identity_include_email,
        identity_include_name: config.identity_include_name,
        identity_jwt_audience: config.identity_jwt_audience.clone(),
        forward_access_token: config.forward_access_token,
        inject_delegation_token: config.inject_delegation_token,
        delegation_token_scope: normalized_scope,
    })
}

/// Validate a slug: 1-80 chars, lowercase alphanumeric + hyphens, no
/// leading/trailing/consecutive hyphens.
pub(crate) fn validate_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty() || slug.len() > 80 {
        return Err(AppError::ValidationError(
            "Slug must be between 1 and 80 characters".to_string(),
        ));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::ValidationError(
            "Slug must contain only lowercase letters, digits, and hyphens".to_string(),
        ));
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        return Err(AppError::ValidationError(
            "Slug must not start or end with a hyphen".to_string(),
        ));
    }
    if slug.contains("--") {
        return Err(AppError::ValidationError(
            "Slug must not contain consecutive hyphens".to_string(),
        ));
    }
    Ok(())
}

fn validate_auth_method(method: &str) -> AppResult<()> {
    if !VALID_AUTH_METHODS.contains(&method) {
        return Err(AppError::ValidationError(format!(
            "Invalid auth_method '{}'. Valid: {}",
            method,
            VALID_AUTH_METHODS.join(", ")
        )));
    }
    Ok(())
}

pub(crate) fn auth_method_requires_key_name(auth_method: &str) -> bool {
    matches!(auth_method, "header" | "query" | "path" | "body")
}

pub(crate) fn auth_key_name_required_message(auth_method: &str) -> String {
    format!(
        "auth_key_name is required when auth_method is '{auth_method}' \
         (e.g. 'X-API-Key' for header, 'key' for query, 'app_secret' for body)"
    )
}

/// List all active user services for a user.
pub async fn list_user_services(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<UserService>> {
    let services: Vec<UserService> = db
        .collection::<UserService>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;
    Ok(services)
}

/// Provenance tag distinguishing personal credentials from org-shared ones.
///
/// Personal entries are owned directly by the actor; org entries are owned
/// by an org user the actor is a member of. Viewer-role memberships also
/// surface here with `allowed: false` so the UI can show "you can see this
/// but cannot use it".
#[derive(Debug, Clone)]
pub enum CredentialSource {
    Personal,
    Org {
        org_user_id: String,
        org_name: String,
        /// Org avatar (`User.avatar_url` on the org's user record). Surfaced
        /// to the frontend so shared org sources on `/keys` can render the
        /// same avatar as the Organizations page (#545); `None` when the org
        /// has no avatar configured.
        org_avatar_url: Option<String>,
        role: OrgRole,
        allowed: bool,
    },
}

/// A user service paired with the provenance of its credentials.
#[derive(Debug, Clone)]
pub struct UserServiceWithSource {
    pub service: UserService,
    pub source: CredentialSource,
}

/// List all user services visible to a person, including those inherited
/// from org memberships. Personal entries come first, then one section per
/// org. Viewer-role org services are returned with `allowed = false` so the
/// UI can render them as read-only.
///
/// **Dedup rule:** if the actor has both a personal and an org-inherited
/// service for the same slug, both are returned. The frontend groups by
/// `source` and the proxy resolution path picks personal first.
pub async fn list_user_services_with_sources(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<UserServiceWithSource>> {
    let mut out: Vec<UserServiceWithSource> = list_user_services(db, user_id)
        .await?
        .into_iter()
        .map(|s| UserServiceWithSource {
            service: s,
            source: CredentialSource::Personal,
        })
        .collect();

    let memberships = org_service::list_memberships_for_member(db, user_id, false).await?;

    // Cache org user lookups so we don't re-query the same org twice when
    // the user belongs to multiple memberships pointing at the same org
    // (shouldn't happen due to the unique index, but cheap to be safe).
    let mut org_meta_cache: std::collections::HashMap<String, (String, Option<String>)> =
        Default::default();

    for m in memberships {
        let effective_scope =
            crate::services::org_role_scope_service::effective_scope_for_membership(db, &m).await?;
        let (org_name, org_avatar_url) = if let Some(meta) = org_meta_cache.get(&m.org_user_id) {
            meta.clone()
        } else {
            let org = db
                .collection::<User>(USERS)
                .find_one(doc! { "_id": &m.org_user_id })
                .await?;
            let (name, avatar) = org
                .map(|u| (u.display_name, u.avatar_url))
                .unwrap_or((None, None));
            let name = name.unwrap_or_else(|| "Unnamed Org".to_string());
            let meta = (name, avatar);
            org_meta_cache.insert(m.org_user_id.clone(), meta.clone());
            meta
        };

        let org_services = list_user_services(db, &m.org_user_id).await?;
        for svc in org_services {
            // Scope filter: drop services outside the effective member scope
            // entirely. We do NOT return them with
            // `allowed: false` because the response payload still contains
            // endpoint_id, api_key_id, auth metadata, etc. -- a member
            // scoped to service A must not see metadata for service B.
            //
            // Role-based "can see but not proxy" (viewer) remains visible
            // with `allowed: false` because viewers are explicitly entitled
            // to see the listing of services their org has.
            if !crate::services::org_role_scope_service::scope_allows(&effective_scope, &svc.id) {
                continue;
            }
            // Viewer can see but not proxy. Member/Admin can use.
            let allowed = m.role.can_proxy();

            out.push(UserServiceWithSource {
                service: svc,
                source: CredentialSource::Org {
                    org_user_id: m.org_user_id.clone(),
                    org_name: org_name.clone(),
                    org_avatar_url: org_avatar_url.clone(),
                    role: m.role,
                    allowed,
                },
            });
        }
    }

    Ok(out)
}

/// Look up a `UserService` by id alone, WITHOUT ownership filtering.
///
/// Used by the `?_nyxid_via=` proxy path, which needs to load the row
/// first and then separately check access via `resolve_owner_access`.
/// Returns `None` if no active row exists with this id.
pub async fn find_user_service_by_id(
    db: &mongodb::Database,
    service_id: &str,
) -> AppResult<Option<UserService>> {
    Ok(db
        .collection::<UserService>(COLLECTION_NAME)
        .find_one(doc! { "_id": service_id, "is_active": true })
        .await?)
}

/// Get single user service by ID, verifying ownership.
pub async fn get_user_service(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<UserService> {
    db.collection::<UserService>(COLLECTION_NAME)
        .find_one(doc! { "_id": service_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User service not found".to_string()))
}

/// Find a user service by slug for a given user.
pub async fn find_by_slug(
    db: &mongodb::Database,
    user_id: &str,
    slug: &str,
) -> AppResult<Option<UserService>> {
    Ok(db
        .collection::<UserService>(COLLECTION_NAME)
        .find_one(doc! { "user_id": user_id, "slug": slug, "is_active": true })
        .await?)
}

/// Find a user service by catalog_service_id for a given user.
pub async fn find_by_catalog_service_id(
    db: &mongodb::Database,
    user_id: &str,
    catalog_service_id: &str,
) -> AppResult<Option<UserService>> {
    Ok(db
        .collection::<UserService>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "catalog_service_id": catalog_service_id,
            "is_active": true,
        })
        .await?)
}

/// Return the IDs of every active `UserService` for `user_id` that
/// references the given endpoint. Used by org-scope checks: an
/// `OrgMembership.allowed_service_ids` is a set of `UserService` ids,
/// so to gate write access on a `UserEndpoint` we have to translate
/// the endpoint id back to the services it backs.
pub async fn user_service_ids_for_endpoint(
    db: &mongodb::Database,
    user_id: &str,
    endpoint_id: &str,
) -> AppResult<Vec<String>> {
    let services: Vec<UserService> = db
        .collection::<UserService>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "endpoint_id": endpoint_id })
        .await?
        .try_collect()
        .await?;
    Ok(services.into_iter().map(|s| s.id).collect())
}

/// Return the IDs of every active `UserService` for `user_id` that
/// references the given external `UserApiKey`. See the endpoint helper
/// above for the rationale.
pub async fn user_service_ids_for_api_key(
    db: &mongodb::Database,
    user_id: &str,
    user_api_key_id: &str,
) -> AppResult<Vec<String>> {
    let services: Vec<UserService> = db
        .collection::<UserService>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "api_key_id": user_api_key_id })
        .await?
        .try_collect()
        .await?;
    Ok(services.into_iter().map(|s| s.id).collect())
}

/// Return the IDs of every `UserService` (active or inactive) for
/// `user_id` that points at the given catalog `DownstreamService.id`.
/// Used by the approval scope check, which needs the `UserService.id`
/// space because that is what `OrgMembership.allowed_service_ids`
/// stores. Inactive services are included so a member who deactivated
/// their UserService cannot dodge an outstanding approval.
pub async fn user_service_ids_for_catalog(
    db: &mongodb::Database,
    user_id: &str,
    catalog_service_id: &str,
) -> AppResult<Vec<String>> {
    let services: Vec<UserService> = db
        .collection::<UserService>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "catalog_service_id": catalog_service_id })
        .await?
        .try_collect()
        .await?;
    Ok(services.into_iter().map(|s| s.id).collect())
}

/// Create a new user service.
///
/// `user_id` is the *effective owner* of the new service (the actor when
/// creating personal services, the org user_id when creating org-owned
/// services). `actor_user_id` is the human/API key actually making the
/// request -- it's used for the node ownership check, because nodes are
/// owned by individual people and an admin should be able to route an
/// org service through their personal node without re-registering it.
#[allow(clippy::too_many_arguments)]
pub async fn create_user_service(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    slug: &str,
    endpoint_id: &str,
    api_key_id: Option<&str>,
    auth_method: &str,
    auth_key_name: &str,
    catalog_service_id: Option<&str>,
    node_id: Option<&str>,
    node_priority: i32,
    service_type: &str,
    ssh_auth_mode: SshAuthMode,
    source: Option<&str>,
    source_id: Option<&str>,
    source_app_id: Option<&str>,
    identity: &IdentityConfig,
    ws_frame_injections: Option<&[WsFrameInjection]>,
) -> AppResult<UserService> {
    validate_slug(slug)?;
    validate_auth_method(auth_method)?;
    let identity = normalize_identity_config(identity)?;
    let node_id = node_id.filter(|nid| !nid.is_empty());
    if let Some(rules) = ws_frame_injections {
        ws_frame_injector::validate_rules(rules)?;
    }

    if source.is_some() != source_id.is_some() {
        return Err(AppError::ValidationError(
            "source and source_id must be provided together".to_string(),
        ));
    }

    if auth_key_name.len() > 200 || auth_key_name.contains('\r') || auth_key_name.contains('\n') {
        return Err(AppError::ValidationError(
            "Invalid auth_key_name".to_string(),
        ));
    }

    if auth_method_requires_key_name(auth_method) && auth_key_name.trim().is_empty() {
        return Err(AppError::ValidationError(auth_key_name_required_message(
            auth_method,
        )));
    }

    // `body` auth credential injection happens inside the backend proxy's
    // `forward_request()`. Node-routed requests bypass that path, so body
    // injection would silently not happen. Reject up front.
    if auth_method == "body" && node_id.is_some() {
        return Err(AppError::ValidationError(
            "auth_method 'body' is not supported for node-routed services. \
             Credential body injection only works for direct (non-node) routing."
                .to_string(),
        ));
    }

    // `token_exchange` performs server-side token exchange against the
    // configured endpoint directly from the backend process. Node-routed
    // requests would have to relay the exchange through the node agent,
    // which is not implemented. Reject at bind time.
    if auth_method == "token_exchange" && node_id.is_some() {
        return Err(AppError::ValidationError(
            "auth_method 'token_exchange' is not supported for node-routed services. \
             The token exchange runs server-side and does not flow through nodes."
                .to_string(),
        ));
    }

    if api_key_id.is_none() && auth_method != "none" {
        return Err(AppError::ValidationError(
            "Services without an API key must use auth_method 'none'".to_string(),
        ));
    }

    // Verify endpoint exists and belongs to user
    let ep_count = db
        .collection::<mongodb::bson::Document>(USER_ENDPOINTS)
        .count_documents(doc! { "_id": endpoint_id, "user_id": user_id })
        .await?;
    if ep_count == 0 {
        return Err(AppError::NotFound(
            "Endpoint not found or does not belong to user".to_string(),
        ));
    }

    // Verify api_key exists and belongs to user (skip for no-auth services)
    if let Some(ak_id) = api_key_id {
        let ak_count = db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .count_documents(doc! { "_id": ak_id, "user_id": user_id })
            .await?;
        if ak_count == 0 {
            return Err(AppError::NotFound(
                "API key not found or does not belong to user".to_string(),
            ));
        }
    }

    // Check slug uniqueness for active services
    let existing = find_by_slug(db, user_id, slug).await?;
    if existing.is_some() {
        return Err(AppError::Conflict(format!(
            "You already have an active service with slug '{slug}'"
        )));
    }

    if let Some(node_id) = node_id {
        // Actor-based check: the human (or API key) making the request must
        // have write access to the node. This lets an admin route an
        // org-owned service through their personal node, where they're the
        // direct owner. The service's effective owner (user_id) does not
        // need to match the node's owner.
        node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    }

    let now = Utc::now();
    let service = UserService {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        slug: slug.to_string(),
        endpoint_id: endpoint_id.to_string(),
        api_key_id: api_key_id.map(|s| s.to_string()),
        auth_method: auth_method.to_string(),
        auth_key_name: auth_key_name.to_string(),
        catalog_service_id: catalog_service_id.map(|s| s.to_string()),
        node_id: node_id.map(|s| s.to_string()),
        node_priority,
        service_type: service_type.to_string(),
        ssh_auth_mode,
        ssh_node_keys_stale: false,
        identity_propagation_mode: identity.identity_propagation_mode,
        identity_include_user_id: identity.identity_include_user_id,
        identity_include_email: identity.identity_include_email,
        identity_include_name: identity.identity_include_name,
        identity_jwt_audience: identity.identity_jwt_audience,
        forward_access_token: identity.forward_access_token,
        inject_delegation_token: identity.inject_delegation_token,
        delegation_token_scope: identity.delegation_token_scope,
        custom_user_agent: None,
        default_request_headers: None,
        ws_frame_injections: ws_frame_injections.unwrap_or_default().to_vec(),
        is_active: true,
        source: source.map(str::to_string),
        source_id: source_id.map(str::to_string),
        source_app_id: source_app_id.map(str::to_string),
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserService>(COLLECTION_NAME)
        .insert_one(&service)
        .await?;

    Ok(service)
}

/// Update service config (auth method, node routing, identity propagation, etc.).
///
/// `user_id` is the *effective owner* of the service (caller for personal,
/// org user_id for org-owned). `actor_user_id` is the human/API key making
/// the request -- used for the node ownership check (see
/// `create_user_service` for rationale).
#[allow(clippy::too_many_arguments)]
pub async fn update_user_service(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    service_id: &str,
    auth_method: Option<&str>,
    auth_key_name: Option<&str>,
    node_id: Option<&str>,
    node_priority: Option<i32>,
    is_active: Option<bool>,
    identity: Option<&IdentityConfig>,
    custom_user_agent: Option<&str>,
    default_request_headers: Option<
        &Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
    >,
    ws_frame_injections: Option<&[WsFrameInjection]>,
) -> AppResult<()> {
    let current = get_user_service(db, user_id, service_id).await?;
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(am) = auth_method {
        validate_auth_method(am)?;
        if am != "none" && current.api_key_id.is_none() {
            return Err(AppError::BadRequest(
                "This service has no stored credential. Add one before changing auth_method."
                    .to_string(),
            ));
        }
        set_doc.insert("auth_method", am);
    }
    if let Some(akn) = auth_key_name {
        set_doc.insert("auth_key_name", akn);
    }
    if let Some(nid) = node_id {
        if nid.is_empty() {
            // Empty string clears the node_id
            set_doc.insert("node_id", bson::Bson::Null);
        } else {
            // Actor-based check: see `create_user_service` for rationale.
            node_service::ensure_node_writable_by_actor(db, actor_user_id, nid).await?;
            set_doc.insert("node_id", nid);
        }
    }

    // Cross-field validation for credential injection methods. We check the
    // effective post-update state: incoming values override current values.
    let effective_auth_method = auth_method.unwrap_or(&current.auth_method);
    if auth_method_requires_key_name(effective_auth_method) {
        let effective_auth_key_name = auth_key_name.unwrap_or(&current.auth_key_name);
        if effective_auth_key_name.trim().is_empty() {
            return Err(AppError::ValidationError(auth_key_name_required_message(
                effective_auth_method,
            )));
        }
    }

    if effective_auth_method == "body" {
        // Normalize legacy `current.node_id == Some("")` to `None`.
        // Matches the normalization in `validate_update_inputs` (fifteenth-
        // round Codex P1) and in the `PUT /keys` handler so the
        // body/token_exchange guards don't incorrectly treat a legacy
        // direct-routed service as node-routed (nineteenth-round Codex P2).
        let effective_node_id: Option<&str> = match node_id {
            Some("") => None,
            Some(nid) => Some(nid),
            None => current.node_id.as_deref().filter(|n| !n.is_empty()),
        };
        if effective_node_id.is_some() {
            return Err(AppError::ValidationError(
                "auth_method 'body' is not supported for node-routed services. \
                 Credential body injection only works for direct (non-node) routing."
                    .to_string(),
            ));
        }
    }

    // Same node-routing reject for token_exchange post-update.
    if effective_auth_method == "token_exchange" {
        // Normalize legacy `current.node_id == Some("")` to `None`.
        // Matches the normalization in `validate_update_inputs` (fifteenth-
        // round Codex P1) and in the `PUT /keys` handler so the
        // body/token_exchange guards don't incorrectly treat a legacy
        // direct-routed service as node-routed (nineteenth-round Codex P2).
        let effective_node_id: Option<&str> = match node_id {
            Some("") => None,
            Some(nid) => Some(nid),
            None => current.node_id.as_deref().filter(|n| !n.is_empty()),
        };
        if effective_node_id.is_some() {
            return Err(AppError::ValidationError(
                "auth_method 'token_exchange' is not supported for node-routed services. \
                 The token exchange runs server-side and does not flow through nodes."
                    .to_string(),
            ));
        }
    }
    if let Some(np) = node_priority {
        set_doc.insert("node_priority", np);
    }
    if let Some(active) = is_active {
        set_doc.insert("is_active", active);
    }
    if let Some(id_config) = identity {
        let id_config = normalize_identity_config(id_config)?;
        set_doc.insert(
            "identity_propagation_mode",
            &id_config.identity_propagation_mode,
        );
        set_doc.insert(
            "identity_include_user_id",
            id_config.identity_include_user_id,
        );
        set_doc.insert("identity_include_email", id_config.identity_include_email);
        set_doc.insert("identity_include_name", id_config.identity_include_name);
        match &id_config.identity_jwt_audience {
            Some(aud) => {
                set_doc.insert("identity_jwt_audience", aud);
            }
            None => {
                set_doc.insert("identity_jwt_audience", bson::Bson::Null);
            }
        }
        set_doc.insert("forward_access_token", id_config.forward_access_token);
        set_doc.insert("inject_delegation_token", id_config.inject_delegation_token);
        set_doc.insert("delegation_token_scope", &id_config.delegation_token_scope);
    }
    if let Some(ua) = custom_user_agent {
        let trimmed = ua.trim();
        if trimmed.is_empty() {
            set_doc.insert("custom_user_agent", bson::Bson::Null);
        } else {
            if trimmed.len() > 256 {
                return Err(AppError::ValidationError(
                    "custom_user_agent must not exceed 256 characters".to_string(),
                ));
            }
            if trimmed.bytes().any(|b| b < 0x20 && b != b'\t') {
                return Err(AppError::ValidationError(
                    "custom_user_agent must not contain control characters".to_string(),
                ));
            }
            set_doc.insert("custom_user_agent", trimmed);
        }
    }

    // default_request_headers: None means "no change". Some(None) means
    // explicitly clear the field. Some(Some(list)) means replace with list.
    // Validation is delegated to the shared module so admin + user paths
    // enforce the same denylist / length caps. NyxID#356.
    let mut audit_default_header_names: Option<Vec<String>> = None;
    if let Some(drh) = default_request_headers {
        match drh {
            Some(list) => {
                // Restore stored values for entries whose `value` was
                // submitted as the redaction placeholder — otherwise a
                // GET → editor → PUT round trip clobbers every
                // sensitive value with the literal placeholder string.
                // `current` was fetched at the top of this function.
                let reconciled = crate::models::default_request_header::reconcile_with_stored(
                    list.clone(),
                    current.default_request_headers.as_deref(),
                );
                let normalized =
                    crate::models::default_request_header::validate_headers(reconciled)?;
                match normalized {
                    Some(norm) => {
                        audit_default_header_names =
                            Some(norm.iter().map(|h| h.name.clone()).collect());
                        let bson_val = bson::to_bson(&norm).map_err(|e| {
                            AppError::Internal(format!(
                                "Failed to serialize default_request_headers: {e}"
                            ))
                        })?;
                        set_doc.insert("default_request_headers", bson_val);
                    }
                    None => {
                        audit_default_header_names = Some(Vec::new());
                        set_doc.insert("default_request_headers", bson::Bson::Null);
                    }
                }
            }
            None => {
                audit_default_header_names = Some(Vec::new());
                set_doc.insert("default_request_headers", bson::Bson::Null);
            }
        }
    }

    if let Some(rules) = ws_frame_injections {
        ws_frame_injector::validate_rules(rules)?;
        set_doc.insert(
            "ws_frame_injections",
            bson::to_bson(rules)
                .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?,
        );
    }

    let result = db
        .collection::<UserService>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": service_id, "user_id": user_id },
            doc! { "$set": set_doc },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("User service not found".to_string()));
    }

    // Audit per-user default header mutations (NyxID#356). Names only —
    // values never reach the audit store, even when non-sensitive,
    // because clients sometimes mistake which entries hold secrets.
    // Mirrors the admin `service_default_headers_updated` event so org
    // observability covers both surfaces uniformly.
    if let Some(names) = audit_default_header_names {
        crate::services::audit_service::log_async(
            db.clone(),
            Some(actor_user_id.to_string()),
            "user_service_default_headers_updated".to_string(),
            Some(serde_json::json!({
                "user_service_id": service_id,
                "owner_user_id": user_id,
                "header_names": names,
            })),
            None,
            None,
            None,
            None,
        );
    }

    Ok(())
}

/// Pre-validate the field combination a `PUT /keys` request intends to
/// apply to a `UserService`, without touching state. Runs every rule that
/// `update_user_service` would enforce later — auth_method string,
/// node-write permission for the actor, body/token_exchange cross-field
/// constraints, identity config shape, custom_user_agent length/control
/// chars, and default_request_headers denylist/length caps — so callers
/// can provision side-effecting records (e.g., a new `UserApiKey` via
/// `unified_key_service::ensure_user_api_key_for_update`) only after
/// validation passes. This prevents orphaned credentials when the request
/// eventually fails in the service layer (NyxID#419 follow-up raised by
/// the Codex review of the fix).
///
/// Mirrors the exact checks performed inside `update_user_service` for the
/// same `(current, incoming)` pair. Keep them in sync.
#[allow(clippy::too_many_arguments)]
pub async fn validate_update_inputs(
    db: &mongodb::Database,
    actor_user_id: &str,
    current: &UserService,
    auth_method: Option<&str>,
    auth_key_name: Option<&str>,
    node_id: Option<&str>,
    identity: Option<&IdentityConfig>,
    custom_user_agent: Option<&str>,
    default_request_headers: Option<
        &Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
    >,
    credential: Option<&str>,
    new_endpoint_url: Option<&str>,
    new_openapi_spec_url: Option<&str>,
) -> AppResult<()> {
    if let Some(am) = auth_method {
        validate_auth_method(am)?;
    }

    let effective_auth_method = auth_method.unwrap_or(&current.auth_method);
    let effective_auth_key_name = auth_key_name.unwrap_or(&current.auth_key_name);
    // Treat legacy `current.node_id == Some("")` as unset. Some rows
    // in the wild still carry the empty string instead of `None`; every
    // node-routing code path filters those out with `$ne: ""`. Without
    // this normalization, `ensure_node_writable_by_actor("")` below
    // would return `NodeNotFound` and block otherwise valid PUTs on
    // legacy direct-routed services (fifteenth-round Codex P1).
    let effective_node_id: Option<&str> = match node_id {
        Some("") => None,
        Some(nid) => Some(nid),
        None => current.node_id.as_deref().filter(|n| !n.is_empty()),
    };

    if auth_method_requires_key_name(effective_auth_method)
        && effective_auth_key_name.trim().is_empty()
    {
        return Err(AppError::ValidationError(auth_key_name_required_message(
            effective_auth_method,
        )));
    }

    if effective_auth_method == "body" && effective_node_id.is_some() {
        return Err(AppError::ValidationError(
            "auth_method 'body' is not supported for node-routed services. \
             Credential body injection only works for direct (non-node) routing."
                .to_string(),
        ));
    }

    if effective_auth_method == "token_exchange" {
        if effective_node_id.is_some() {
            return Err(AppError::ValidationError(
                "auth_method 'token_exchange' is not supported for node-routed services. \
                 The token exchange runs server-side and does not flow through nodes."
                    .to_string(),
            ));
        }

        // Match the prerequisites `create_key` enforces: the service must
        // be catalog-backed AND the catalog entry must declare a
        // `token_exchange_config`. A PUT that leaves the service in a
        // token_exchange state without those would 200 the update and
        // then fail every subsequent proxy call inside
        // `load_token_exchange_config_for_user_service`.
        let Some(ref cat_id) = current.catalog_service_id else {
            return Err(AppError::ValidationError(
                "auth_method 'token_exchange' requires a catalog-backed \
                 service. Create the service from its catalog entry \
                 instead of promoting a custom endpoint."
                    .to_string(),
            ));
        };
        let catalog_entry = db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .find_one(doc! { "_id": cat_id })
            .await?
            .ok_or_else(|| {
                AppError::ValidationError(
                    "Catalog service for token_exchange no longer exists".to_string(),
                )
            })?;

        if catalog_entry.token_exchange_config.is_none() {
            return Err(AppError::ValidationError(format!(
                "Catalog service '{}' is not configured for token_exchange. \
                 Contact an admin to add a `token_exchange_config` or pick \
                 a different auth_method.",
                catalog_entry.slug
            )));
        }

        // If the caller is supplying a credential in the same PUT, run
        // the same JSON-object shape check `create_key` performs via
        // `validate_token_exchange_catalog_credential`. An empty/omitted
        // credential is fine — it just means the caller isn't rotating
        // the existing stored value.
        if let Some(cred) = credential
            && !cred.is_empty()
        {
            crate::services::unified_key_service::validate_token_exchange_catalog_credential(
                &catalog_entry,
                cred,
            )?;
        }
    }

    // Identity config: run the same normalize+validate pipeline so we
    // reject bad scope / oversized audience / etc. before provisioning.
    if let Some(cfg) = identity {
        normalize_identity_config(cfg)?;
    }

    // Custom User-Agent: mirror the trim/length/control-char rules applied
    // inside `update_user_service`. Empty (post-trim) is a "clear" and
    // always valid; non-empty has caps.
    if let Some(ua) = custom_user_agent {
        let trimmed = ua.trim();
        if !trimmed.is_empty() {
            if trimmed.len() > 256 {
                return Err(AppError::ValidationError(
                    "custom_user_agent must not exceed 256 characters".to_string(),
                ));
            }
            if trimmed.bytes().any(|b| b < 0x20 && b != b'\t') {
                return Err(AppError::ValidationError(
                    "custom_user_agent must not contain control characters".to_string(),
                ));
            }
        }
    }

    // default_request_headers: run the same reconcile + validate pipeline
    // against the currently-stored list. This catches denylisted names
    // and over-length values before provisioning a credential downstream
    // of a header-only update that would otherwise fail.
    if let Some(Some(list)) = default_request_headers {
        let reconciled = crate::models::default_request_header::reconcile_with_stored(
            list.clone(),
            current.default_request_headers.as_deref(),
        );
        crate::models::default_request_header::validate_headers(reconciled)?;
    }

    // Reject injection-param AND node_id changes on services whose
    // backing key is `node_managed` — the credential lives entirely
    // on the node agent, so the server can't re-push when the caller
    // changes `auth_method` / `auth_key_name` / `endpoint_url` (target
    // URL) or rebinds/clears `node_id`. Without this guard:
    //   * `auth_method`/`auth_key_name`/`endpoint_url` edits would
    //     leave the node serving the stale injection config with no
    //     path back to sync (twenty-eighth-round Codex P1).
    //   * `node_id` moves / clears skip the push path (no server-held
    //     credential to send) but still trigger the post-commit
    //     `credential_remove` on the old node — the only holder of the
    //     secret — leaving the service unusable until the user
    //     re-enters the credential on the new target
    //     (thirty-third-round Codex P1).
    // Users must instead run `nyxid node credentials add` on the node
    // directly, or promote the record first.
    //
    // Exception: when the caller supplies a non-empty `credential` in the
    // same request, `ensure_user_api_key_for_update` promotes the
    // node_managed record to a server-held credential type and the
    // subsequent push rewrites the new node's config with the fresh
    // secret + injection params. Blocking here would make the combined
    // migrate-and-update flow unusable (twenty-ninth-round Codex P2).
    let node_id_actually_changes = match node_id {
        None => false,
        Some(new_raw) => {
            let new_eff: Option<&str> = if new_raw.is_empty() {
                None
            } else {
                Some(new_raw)
            };
            let old_eff = current.node_id.as_deref().filter(|n| !n.is_empty());
            old_eff != new_eff
        }
    };
    let changes_injection_params = auth_method.is_some()
        || auth_key_name.is_some()
        || new_endpoint_url.is_some()
        || node_id_actually_changes;
    let caller_is_promoting = credential.is_some_and(|c| !c.is_empty());
    if changes_injection_params
        && !caller_is_promoting
        && let Some(ref ak_id) = current.api_key_id
    {
        let ak =
            crate::services::user_api_key_service::get_api_key(db, &current.user_id, ak_id).await?;
        if ak.credential_type == "node_managed" {
            return Err(AppError::BadRequest(
                "Cannot change auth_method/auth_key_name/endpoint_url/node_id on a \
                 node-managed service via `PUT /keys` without also supplying a \
                 new `credential` to promote the key to a server-held record. \
                 Either include `credential` in the same request, or run \
                 `nyxid node credentials add <slug> …` on the node directly."
                    .to_string(),
            ));
        }
    }

    // Endpoint URL format: mirror `user_endpoint_service::validate_endpoint_url`.
    // Skipping would let a strict node push forward an unvalidated
    // `target_url` (e.g., `ftp://…`) to the node and only fail the
    // backend commit afterwards — leaving server and node out of sync
    // (tenth-round Codex review P2). Empty + SSH URLs are still allowed.
    if let Some(url) = new_endpoint_url
        && !url.is_empty()
        && !url.starts_with("ssh://")
    {
        crate::services::url_validation::validate_base_url(url)?;
    }

    // OpenAPI spec URL format: same rationale. Without this, a PUT
    // that rotates a credential AND sets a malformed
    // `openapi_spec_url` would commit the credential (and push to the
    // node) before `update_endpoint` rejects the spec URL
    // (twenty-sixth-round Codex P2). Empty string is a valid clear.
    if let Some(url) = new_openapi_spec_url
        && !url.trim().is_empty()
    {
        crate::services::url_validation::validate_optional_spec_url(url)?;
    }

    // Node-write permission check #1: caller is explicitly setting a
    // non-empty node_id. Matches the gated branch inside
    // `update_user_service`; empty-string (clear) and None (keep) skip.
    if let Some(nid) = node_id
        && !nid.is_empty()
    {
        node_service::ensure_node_writable_by_actor(db, actor_user_id, nid).await?;
    }

    // Reject `PUT /keys` that binds a node onto a provider-backed
    // service whose `UserApiKey` is shared with another active
    // service. `create_api_key_from_provider_token` deliberately
    // reuses one `UserApiKey` per `UserProviderToken` (see that
    // function's doc comment), so flipping it to `node_managed` on
    // node bind would clear the access token for every direct-routed
    // service still using it (twenty-eighth-round Codex P1). The user
    // must explicitly recreate the service via the catalog path
    // (which provisions a fresh unshared `UserApiKey`) to move one
    // instance of a provider-backed service onto a node without
    // breaking the others.
    let node_id_changing = match node_id {
        Some("") => current.node_id.as_deref().is_some_and(|n| !n.is_empty()), // clearing
        Some(nid) => current.node_id.as_deref() != Some(nid),
        None => false,
    };
    if node_id_changing
        && effective_node_id.is_some()
        && let Some(ref ak_id) = current.api_key_id
    {
        let ak =
            crate::services::user_api_key_service::get_api_key(db, &current.user_id, ak_id).await?;
        if ak.provider_config_id.is_some() {
            let sibling_count = db
                .collection::<mongodb::bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {
                    "user_id": &current.user_id,
                    "api_key_id": ak_id,
                    "is_active": true,
                    "_id": { "$ne": &current.id },
                })
                .await?;
            if sibling_count > 0 {
                return Err(AppError::BadRequest(
                    "Cannot bind this service to a node: the underlying provider \
                     credential is shared with other services, and moving one onto \
                     a node would invalidate the others. Recreate this service \
                     from its catalog entry to get a dedicated credential, or \
                     unbind the shared provider token first."
                        .to_string(),
                ));
            }
        }
    }

    // Provider-backed node-routed credential guard. `create_key`
    // refuses `{node_id, provider_config_id, credential}` at creation
    // time; the PUT path must do the same *before* any key mutation so
    // a rejected request never leaves a rotated `UserApiKey` behind
    // (fourteenth-round Codex P1). Two provider-source checks cover
    // both "existing service has provider-linked key" and "upgrade
    // from auth_method=none on a provider-backed catalog entry":
    if credential.is_some_and(|c| !c.is_empty()) && effective_node_id.is_some() {
        // (a) An already-linked api_key carrying a provider_config_id.
        if let Some(ref ak_id) = current.api_key_id {
            let ak =
                crate::services::user_api_key_service::get_api_key(db, &current.user_id, ak_id)
                    .await?;
            if ak.provider_config_id.is_some() {
                return Err(AppError::BadRequest(
                    "Node-routed provider-backed services must be authorized on the node agent. \
                     Rotate the credential through the provider's OAuth/device-code/API-key flow, \
                     or clear `node_id` before storing a server-held credential."
                        .to_string(),
                ));
            }
        }
        // (b) First-time upgrade: no api_key yet, but the catalog entry
        //     is provider-backed.
        if current.api_key_id.is_none()
            && let Some(ref cat_id) = current.catalog_service_id
            && let Some(cat) = db
                .collection::<crate::models::downstream_service::DownstreamService>(
                    crate::models::downstream_service::COLLECTION_NAME,
                )
                .find_one(doc! { "_id": cat_id })
                .await?
            && cat.provider_config_id.is_some()
        {
            return Err(AppError::BadRequest(
                "Node-routed provider-backed services must be authorized on the node agent. \
                 Use the provider flow instead of sending a credential through this endpoint, \
                 or clear `node_id` first."
                    .to_string(),
            ));
        }
    }

    // Node-write permission check #2: caller may cause a credential
    // push to the effectively-bound node even when `node_id` itself
    // isn't in the body. The handler pushes on any of:
    //   (a) a new `credential` being stored server-side
    //   (b) a node-delivery-relevant field change (`auth_method`,
    //       `auth_key_name`, `endpoint_url`) on a service that already
    //       holds a server credential
    // In both cases the PUT ends up rewriting the node's local
    // credential config, so the actor must own the node regardless of
    // whether they touched `node_id`. Without this check, an org admin
    // with write access to the service but not the node could push
    // credentials into someone else's node config (tenth-round Codex
    // review P1). Skipped when `node_id` was already verified with the
    // same value above.
    let credential_supplied = credential.is_some_and(|c| !c.is_empty());
    let touches_node_delivery_field =
        auth_method.is_some() || auth_key_name.is_some() || new_endpoint_url.is_some();
    let may_push_to_node = credential_supplied || touches_node_delivery_field;
    if may_push_to_node
        && let Some(eff_node) = effective_node_id
        && node_id != Some(eff_node)
    {
        node_service::ensure_node_writable_by_actor(db, actor_user_id, eff_node).await?;
    }

    // Pre-commit check: when reassigning to a different node (or clearing
    // the binding), verify the actor can write to the *previous* node too.
    // The handler will later send `credential_remove` to that node; if we
    // discover the permission mismatch only after `update_user_service`
    // has persisted the new `node_id`, the PUT returns an error while the
    // routing change has already committed — the client sees a failure
    // and may retry, but the service is already moved
    // (twenty-ninth-round Codex P1). Doing the check here keeps the
    // commit/cleanup pair all-or-nothing from the caller's perspective.
    if let Some(new_nid_raw) = node_id {
        let new_effective: Option<&str> = if new_nid_raw.is_empty() {
            None
        } else {
            Some(new_nid_raw)
        };
        let old_effective: Option<&str> = current.node_id.as_deref().filter(|n| !n.is_empty());
        if let Some(old_nid) = old_effective
            && old_effective != new_effective
        {
            node_service::ensure_node_writable_by_actor(db, actor_user_id, old_nid).await?;
        }
    }

    Ok(())
}

/// Attach a newly-provisioned `UserApiKey` to an existing `UserService`.
///
/// Used by the PUT /keys upgrade path when a service that was created with
/// `auth_method: "none"` is switched to a credential-bearing method or
/// receives its first stored credential. This is the only writer that sets
/// `api_key_id` on an existing `UserService` row — `create_user_service`
/// already attaches the key at creation time. See `unified_key_service::
/// ensure_user_api_key_for_update` for the full upgrade protocol and
/// NyxID#419 for the bug this closes.
pub async fn link_api_key(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    api_key_id: &str,
) -> AppResult<()> {
    let ak_count = db
        .collection::<mongodb::bson::Document>(USER_API_KEYS)
        .count_documents(doc! { "_id": api_key_id, "user_id": user_id })
        .await?;
    if ak_count == 0 {
        return Err(AppError::NotFound(
            "API key not found or does not belong to user".to_string(),
        ));
    }

    // Compare-and-set on `api_key_id`. Two concurrent upgrade PUTs for
    // the same no-auth service both provision their own `UserApiKey`
    // and race here; without the `api_key_id: null` predicate the last
    // write wins and the earlier one is orphaned — a "leaked" credential
    // that still appears under external key management (twenty-ninth-
    // round Codex P2). We also accept a re-attach of the same
    // `api_key_id` so an idempotent retry of a single request doesn't
    // return Conflict.
    let result = db
        .collection::<UserService>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": service_id,
                "user_id": user_id,
                "$or": [
                    { "api_key_id": null },
                    { "api_key_id": { "$exists": false } },
                    { "api_key_id": api_key_id },
                ],
            },
            doc! {
                "$set": {
                    "api_key_id": api_key_id,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    if result.matched_count == 0 {
        // Distinguish "service missing" from "service already bound to a
        // different api_key" so the caller can reclaim the orphan
        // credential it just provisioned.
        let existing = db
            .collection::<UserService>(COLLECTION_NAME)
            .find_one(doc! { "_id": service_id, "user_id": user_id })
            .await?;
        return match existing {
            None => Err(AppError::NotFound("User service not found".to_string())),
            Some(_) => Err(AppError::Conflict(
                "User service already has an API key bound (concurrent upgrade); \
                 the duplicate credential has been discarded. Retry the PUT to \
                 see the current state."
                    .to_string(),
            )),
        };
    }

    Ok(())
}

pub(crate) fn ssh_node_keys_stale_after_transition(
    current_stale: bool,
    from: SshAuthMode,
    to: SshAuthMode,
) -> bool {
    current_stale || (from == SshAuthMode::NodeKey && to != SshAuthMode::NodeKey)
}

/// Update only the SSH auth mode on a user service. All v1 transitions are
/// accepted; switching away from NodeKey marks node-side keys stale so
/// `nyxid node ssh-credentials prune --stale` can clean orphaned entries.
pub async fn update_ssh_auth_mode(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    service_id: &str,
    mode: SshAuthMode,
) -> AppResult<UserService> {
    let current = get_user_service(db, user_id, service_id).await?;
    if current.service_type != "ssh" {
        return Err(AppError::ValidationError(
            "SSH auth mode can only be changed for SSH services".to_string(),
        ));
    }

    validate_ssh_auth_mode_transition(db, &current, mode).await?;

    let from = current.ssh_auth_mode;
    let ssh_node_keys_stale =
        ssh_node_keys_stale_after_transition(current.ssh_node_keys_stale, from, mode);
    let now = Utc::now();

    db.collection::<UserService>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": service_id, "user_id": user_id },
            doc! {
                "$set": {
                    "ssh_auth_mode": mode.as_str(),
                    "ssh_node_keys_stale": ssh_node_keys_stale,
                    "updated_at": bson::DateTime::from_chrono(now),
                }
            },
        )
        .await?;

    if from != mode {
        audit_service::log_async(
            db.clone(),
            Some(actor_user_id.to_string()),
            "service.ssh_auth_mode_changed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "owner_user_id": user_id,
                "from": from.as_str(),
                "to": mode.as_str(),
                "actor": actor_user_id,
            })),
            None,
            None,
            None,
            None,
        );
    }

    get_user_service(db, user_id, service_id).await
}

async fn validate_ssh_auth_mode_transition(
    db: &mongodb::Database,
    current: &UserService,
    mode: SshAuthMode,
) -> AppResult<()> {
    if mode == SshAuthMode::ProxyOnly {
        return Ok(());
    }

    let Some(ref catalog_service_id) = current.catalog_service_id else {
        return Err(AppError::ValidationError(
            "SSH auth mode cert or node_key requires a catalog-backed SSH service".to_string(),
        ));
    };
    let catalog = db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(doc! { "_id": catalog_service_id })
        .await?
        .ok_or_else(|| {
            AppError::ValidationError(
                "Catalog service for SSH auth mode no longer exists".to_string(),
            )
        })?;
    let ssh_config = catalog.ssh_config.as_ref().ok_or_else(|| {
        AppError::ValidationError(
            "Catalog service for SSH auth mode is missing ssh_config".to_string(),
        )
    })?;

    crate::services::ssh_service::validate_ssh_auth_mode_settings(
        mode,
        ssh_config.certificate_ttl_minutes,
        &ssh_config.allowed_principals,
    )
}

/// Deactivate a user service (soft delete).
///
/// `actor_user_id` is the human/API key making the request -- forwarded to
/// `update_user_service` for symmetry, but not actually used since
/// deactivation doesn't change the node_id.
pub async fn deactivate_user_service(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    update_user_service(
        db,
        user_id,
        actor_user_id,
        service_id,
        None,
        None,
        None,
        None,
        Some(false),
        None,
        None,
        None,
        None,
    )
    .await?;

    // Cascade-clean any agent service bindings that referenced this
    // service. Without this, the Agent Key detail page keeps showing
    // bindings pointing at a now-inactive service (issue #324).
    agent_binding_service::cleanup_bindings_for_user_service(db, user_id, service_id).await?;
    crate::services::org_role_scope_service::remove_service_from_all_scopes(
        db, user_id, service_id,
    )
    .await?;

    Ok(())
}

/// Migrate stale `UserService.auth_method = "none"` snapshots taken from
/// provider-delegated catalog entries (Anthropic, OpenAI, Gemini, ...)
/// before the provisioning path derived the effective injection config
/// from the SPR.
///
/// Background: `DownstreamService` rows for those services intentionally
/// store `auth_method = "none"` on the catalog row and carry the real
/// injection config on the `ServiceProviderRequirement`. Earlier versions
/// of `unified_key_service::create_key` copied the raw
/// `svc.auth_method` / `svc.auth_key_name` onto the UserService, so the
/// proxy (which reads `auth_method` straight off the UserService) never
/// injected the caller's credential and upstream returned
/// `"x-api-key header is required"`.
///
/// Scope:
/// - Match by `catalog_service_id` (so we never touch custom-endpoint
///   UserServices that carry no catalog link).
/// - Require `auth_method = "none"` AND `auth_key_name = ""` so an
///   admin's deliberate `"none"` customization with a named key is left
///   alone.
/// - Require `api_key_id` to be set -- auto-provisioned no-auth rows
///   (which have no api_key_id) are handled separately by
///   `reconcile_stale_auto_provisions` and must not be mutated here.
///
/// Idempotent: once the snapshot matches the SPR, `auth_method` is no
/// longer `"none"` and the filter no longer matches the row.
pub async fn backfill_stale_catalog_auth_snapshots(db: &mongodb::Database) -> AppResult<()> {
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as SPR_COLLECTION, ServiceProviderRequirement,
    };

    let stale_catalog_services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! { "auth_method": "none", "is_active": true })
        .await?
        .try_collect()
        .await?;

    let mut updated: u64 = 0;
    for svc in stale_catalog_services {
        let Some(spr) = db
            .collection::<ServiceProviderRequirement>(SPR_COLLECTION)
            .find_one(doc! { "service_id": &svc.id })
            .await?
        else {
            continue;
        };

        let injection_key = spr
            .injection_key
            .clone()
            .unwrap_or_else(|| "Authorization".to_string());

        let result = db
            .collection::<UserService>(COLLECTION_NAME)
            .update_many(
                doc! {
                    "catalog_service_id": &svc.id,
                    "auth_method": "none",
                    "auth_key_name": "",
                    "api_key_id": { "$ne": null },
                },
                doc! {
                    "$set": {
                        "auth_method": &spr.injection_method,
                        "auth_key_name": &injection_key,
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await?;

        if result.modified_count > 0 {
            tracing::info!(
                catalog_slug = %svc.slug,
                injection_method = %spr.injection_method,
                injection_key = %injection_key,
                matched = result.matched_count,
                modified = result.modified_count,
                "Migrated stale UserService auth_method snapshots"
            );
            updated += result.modified_count;
        }
    }

    if updated > 0 {
        tracing::info!(
            count = updated,
            "UserService auth_method snapshot migration complete"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, SshServiceConfig,
    };
    use crate::models::ws_frame_injection::{
        WsFrameDirection, WsFrameInjection, WsFrameKind, WsFrameTrigger,
    };
    use crate::test_utils::{connect_test_database, test_user_service};
    use mongodb::bson::doc;

    fn sample_identity_config() -> IdentityConfig {
        IdentityConfig {
            identity_propagation_mode: "headers".to_string(),
            identity_include_user_id: true,
            identity_include_email: true,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: true,
            delegation_token_scope: "llm:proxy".to_string(),
        }
    }

    fn test_downstream_ssh_service(
        service_id: &str,
        slug: &str,
        allowed_principals: Vec<String>,
    ) -> DownstreamService {
        let now = Utc::now();
        DownstreamService {
            id: service_id.to_string(),
            name: slug.to_string(),
            slug: slug.to_string(),
            description: None,
            base_url: "ssh://10.0.0.1:22".to_string(),
            service_type: "ssh".to_string(),
            visibility: "private".to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential_encrypted: Vec::new(),
            auth_type: Some("ssh".to_string()),
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: Some(SshServiceConfig {
                host: "10.0.0.1".to_string(),
                port: 22,
                ssh_auth_mode: SshAuthMode::ProxyOnly,
                certificate_auth_enabled: false,
                certificate_ttl_minutes: 30,
                allowed_principals,
                ca_private_key_encrypted: None,
                ca_public_key: None,
            }),
            oauth_client_id: None,
            service_category: "connection".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: "test".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn normalize_identity_config_defaults_blank_scope() {
        let mut config = sample_identity_config();
        config.delegation_token_scope = "   ".to_string();

        let normalized = normalize_identity_config(&config).expect("scope should normalize");
        assert_eq!(normalized.delegation_token_scope, "llm:proxy");
    }

    #[test]
    fn normalize_identity_config_rejects_invalid_scope() {
        let mut config = sample_identity_config();
        config.delegation_token_scope = "admin:full".to_string();

        let error = normalize_identity_config(&config).expect_err("scope should be rejected");
        assert!(matches!(
            error,
            AppError::ValidationError(message)
                if message.contains("Invalid delegation_token_scope")
        ));
    }

    #[test]
    fn normalize_identity_config_rejects_overlong_audience() {
        let mut config = sample_identity_config();
        config.identity_jwt_audience = Some("a".repeat(2049));

        let error =
            normalize_identity_config(&config).expect_err("audience length should be enforced");
        assert!(matches!(
            error,
            AppError::ValidationError(message)
                if message.contains("identity_jwt_audience must not exceed 2048 characters")
        ));
    }

    #[test]
    fn normalize_identity_config_preserves_valid_multiple_scopes() {
        let mut config = sample_identity_config();
        config.delegation_token_scope = "proxy:*   llm:status".to_string();

        let normalized = normalize_identity_config(&config).expect("scopes should validate");
        assert_eq!(normalized.delegation_token_scope, "proxy:* llm:status");
    }

    #[test]
    fn ssh_auth_mode_state_machine_marks_orphans_only_when_leaving_node_key() {
        let modes = [
            SshAuthMode::Cert,
            SshAuthMode::NodeKey,
            SshAuthMode::ProxyOnly,
        ];

        for from in modes {
            for to in modes {
                let expected = from == SshAuthMode::NodeKey && to != SshAuthMode::NodeKey;
                assert_eq!(
                    ssh_node_keys_stale_after_transition(false, from, to),
                    expected,
                    "unexpected stale transition from {from} to {to}",
                );
                assert!(
                    ssh_node_keys_stale_after_transition(true, from, to),
                    "already-stale services must stay stale from {from} to {to}",
                );
            }
        }
    }

    #[tokio::test]
    async fn update_ssh_auth_mode_revalidates_catalog_principals() {
        let Some(db) = connect_test_database("user_service_ssh_auth_mode_validation").await else {
            eprintln!("skipping user_service_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let catalog_id = uuid::Uuid::new_v4().to_string();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(test_downstream_ssh_service(
                &catalog_id,
                "router-empty-principals",
                Vec::new(),
            ))
            .await
            .unwrap();

        let service_id = uuid::Uuid::new_v4().to_string();
        let mut service = test_user_service(
            &service_id,
            &user_id,
            "router-empty-principals",
            "endpoint-1",
            Some(&catalog_id),
            Some("node-1"),
        );
        service.service_type = "ssh".to_string();
        service.ssh_auth_mode = SshAuthMode::ProxyOnly;
        db.collection::<UserService>(COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();

        let err = update_ssh_auth_mode(&db, &user_id, &user_id, &service_id, SshAuthMode::NodeKey)
            .await
            .expect_err("node_key should require catalog principals");
        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message.contains("allowed_principals is required")
        ));
        let unchanged = get_user_service(&db, &user_id, &service_id).await.unwrap();
        assert_eq!(unchanged.ssh_auth_mode, SshAuthMode::ProxyOnly);

        let proxy_service_id = uuid::Uuid::new_v4().to_string();
        let mut proxy_service = test_user_service(
            &proxy_service_id,
            &user_id,
            "router-proxy-only",
            "endpoint-1",
            Some(&catalog_id),
            Some("node-1"),
        );
        proxy_service.service_type = "ssh".to_string();
        proxy_service.ssh_auth_mode = SshAuthMode::NodeKey;
        db.collection::<UserService>(COLLECTION_NAME)
            .insert_one(&proxy_service)
            .await
            .unwrap();

        let updated = update_ssh_auth_mode(
            &db,
            &user_id,
            &user_id,
            &proxy_service_id,
            SshAuthMode::ProxyOnly,
        )
        .await
        .expect("proxy_only should not require catalog principals");
        assert_eq!(updated.ssh_auth_mode, SshAuthMode::ProxyOnly);
        assert!(updated.ssh_node_keys_stale);
    }

    #[test]
    fn validate_auth_method_accepts_token_exchange() {
        // Regression: token_exchange was missing from VALID_AUTH_METHODS
        // which made every api-lark-bot / api-feishu-bot key creation
        // fail with "Invalid auth_method 'token_exchange'" at the
        // user_service_service validation boundary.
        validate_auth_method("token_exchange").expect("token_exchange must be accepted");
    }

    #[test]
    fn validate_auth_method_accepts_all_known_methods() {
        for method in [
            "bearer",
            "bot_bearer",
            "header",
            "query",
            "basic",
            "body",
            "token_exchange",
            "path",
            "none",
        ] {
            validate_auth_method(method)
                .unwrap_or_else(|e| panic!("method {method} must be valid: {e}"));
        }
    }

    #[test]
    fn validate_auth_method_rejects_unknown() {
        assert!(validate_auth_method("lark_token_exchange").is_err());
        assert!(validate_auth_method("oauth2").is_err());
        assert!(validate_auth_method("").is_err());
    }

    #[test]
    fn validate_slug_accepts_eighty_characters_and_rejects_double_hyphens() {
        validate_slug(&"a".repeat(80)).expect("80-char slug should validate");
        assert!(matches!(
            validate_slug("bad--slug"),
            Err(AppError::ValidationError(message))
                if message == "Slug must not contain consecutive hyphens"
        ));
    }

    fn home_assistant_ws_rule() -> WsFrameInjection {
        WsFrameInjection {
            trigger: WsFrameTrigger::JsonFieldEquals {
                path: "$.type".to_string(),
                value: serde_json::json!("auth_required"),
            },
            template: r#"{"type":"auth","access_token":"${credential}"}"#.to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        }
    }

    async fn assert_create_user_service_rejects_empty_auth_key_name(method: &str) {
        let Some(db) = connect_test_database(&format!("user_service_empty_{method}")).await else {
            eprintln!("skipping user_service_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let err = create_user_service(
            &db,
            &user_id,
            &user_id,
            &format!("svc-{method}"),
            "endpoint-1",
            Some("api-key-1"),
            method,
            "",
            None,
            None,
            0,
            "http",
            SshAuthMode::ProxyOnly,
            None,
            None,
            None,
            &IdentityConfig::none(),
            None,
        )
        .await
        .expect_err("empty auth_key_name should be rejected");

        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message.contains(&format!("auth_method is '{method}'"))
        ));
    }

    #[tokio::test]
    async fn create_user_service_rejects_header_with_empty_auth_key_name() {
        assert_create_user_service_rejects_empty_auth_key_name("header").await;
    }

    #[tokio::test]
    async fn create_user_service_rejects_query_with_empty_auth_key_name() {
        assert_create_user_service_rejects_empty_auth_key_name("query").await;
    }

    #[tokio::test]
    async fn create_user_service_rejects_path_with_empty_auth_key_name() {
        assert_create_user_service_rejects_empty_auth_key_name("path").await;
    }

    #[tokio::test]
    async fn create_user_service_allows_bearer_with_empty_auth_key_name() {
        let Some(db) = connect_test_database("user_service_bearer_empty_auth_key_name").await
        else {
            eprintln!("skipping user_service_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        db.collection::<mongodb::bson::Document>(USER_ENDPOINTS)
            .insert_one(doc! { "_id": &endpoint_id, "user_id": &user_id })
            .await
            .unwrap();
        db.collection::<mongodb::bson::Document>(USER_API_KEYS)
            .insert_one(doc! { "_id": &api_key_id, "user_id": &user_id })
            .await
            .unwrap();

        let service = create_user_service(
            &db,
            &user_id,
            &user_id,
            "bearer-empty-key-name",
            &endpoint_id,
            Some(&api_key_id),
            "bearer",
            "",
            None,
            None,
            0,
            "http",
            SshAuthMode::ProxyOnly,
            None,
            None,
            None,
            &IdentityConfig::none(),
            None,
        )
        .await
        .expect("bearer auth should not require auth_key_name");

        assert_eq!(service.auth_method, "bearer");
        assert_eq!(service.auth_key_name, "");
    }

    #[tokio::test]
    async fn update_user_service_rejects_switch_to_header_without_auth_key_name() {
        let Some(db) =
            connect_test_database("user_service_update_header_empty_auth_key_name").await
        else {
            eprintln!("skipping user_service_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let mut service = test_user_service(
            &service_id,
            &user_id,
            "header-update",
            "endpoint-1",
            None,
            None,
        );
        service.api_key_id = Some("api-key-1".to_string());
        db.collection::<UserService>(COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();

        let err = update_user_service(
            &db,
            &user_id,
            &user_id,
            &service_id,
            Some("header"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("switching to header without auth_key_name should fail");

        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message.contains("auth_method is 'header'")
        ));
    }

    #[tokio::test]
    async fn update_user_service_round_trips_ws_frame_injections() {
        let Some(db) = connect_test_database("user_service_ws_frames").await else {
            eprintln!("skipping user_service_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let service = test_user_service(
            &service_id,
            &user_id,
            "home-assistant",
            "endpoint-1",
            None,
            None,
        );
        db.collection::<UserService>(COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();

        let rules = vec![home_assistant_ws_rule()];
        update_user_service(
            &db,
            &user_id,
            &user_id,
            &service_id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&rules),
        )
        .await
        .unwrap();

        let updated = get_user_service(&db, &user_id, &service_id).await.unwrap();
        assert_eq!(updated.ws_frame_injections.len(), 1);
        let rule = &updated.ws_frame_injections[0];
        assert_eq!(
            rule.template,
            r#"{"type":"auth","access_token":"${credential}"}"#
        );
        assert_eq!(rule.frame_kind, WsFrameKind::Text);
        assert_eq!(rule.direction, WsFrameDirection::Downstream);
        assert!(rule.consume_trigger);
        match &rule.trigger {
            WsFrameTrigger::JsonFieldEquals { path, value } => {
                assert_eq!(path, "$.type");
                assert_eq!(value, &serde_json::json!("auth_required"));
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_user_service_rejects_invalid_ws_frame_injections() {
        let Some(db) = connect_test_database("user_service_ws_frames_invalid").await else {
            eprintln!("skipping user_service_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let service = test_user_service(
            &service_id,
            &user_id,
            "home-assistant",
            "endpoint-1",
            None,
            None,
        );
        db.collection::<UserService>(COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();

        let too_many = vec![home_assistant_ws_rule(); 5];
        let err = update_user_service(
            &db,
            &user_id,
            &user_id,
            &service_id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&too_many),
        )
        .await
        .expect_err("more than four rules should be rejected");
        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message.contains("ws_frame_injections must not exceed 4 entries")
        ));

        let mut overlong = home_assistant_ws_rule();
        overlong.template = "a".repeat(4097);
        let overlong_rules = vec![overlong];
        let err = update_user_service(
            &db,
            &user_id,
            &user_id,
            &service_id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&overlong_rules),
        )
        .await
        .expect_err("overlong templates should be rejected");
        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message.contains("template must not exceed 4096 bytes")
        ));
    }
}
