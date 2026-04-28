use std::collections::HashMap;

use mongodb::bson::doc;
use serde::Serialize;
use utoipa::ToSchema;

use futures::TryStreamExt;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::{org_service, user_service_service};

use super::services::{ServiceResponse, SshServiceConfigResponse};

/// Per-viewer routing summary for a single catalog `DownstreamService`.
///
/// Resolved from the viewer's personal `UserService` rows (rows where
/// `user_id == viewer_user_id`). Org-shared bindings are intentionally
/// excluded -- editing those silently from a catalog page would leak
/// across org boundaries.
#[derive(Clone, Debug, Default)]
pub struct ViewerRouting {
    pub node_id: Option<String>,
    pub user_service_id: Option<String>,
    pub binding_count: u32,
}

impl ViewerRouting {
    /// `binding_count == 1` -- safe to populate `node_id` /
    /// `your_user_service_id` on the response. Zero or many bindings
    /// leave both flat fields unset; the FE renders the corresponding
    /// empty / disambiguation state from `binding_count` alone.
    fn single(node_id: Option<String>, user_service_id: String) -> Self {
        Self {
            node_id,
            user_service_id: Some(user_service_id),
            binding_count: 1,
        }
    }

    fn empty() -> Self {
        Self {
            node_id: None,
            user_service_id: None,
            binding_count: 0,
        }
    }

    fn ambiguous(count: u32) -> Self {
        Self {
            node_id: None,
            user_service_id: None,
            binding_count: count,
        }
    }
}

/// Batch-resolve the viewer's personal routing for a set of catalog
/// `DownstreamService` ids in a single MongoDB query.
///
/// Returns a `HashMap<catalog_service_id, ViewerRouting>` for every
/// catalog id passed in (missing entries are treated as
/// `ViewerRouting::empty()` by the caller via `.unwrap_or_default()`).
pub async fn compute_viewer_routing(
    db: &mongodb::Database,
    viewer_user_id: &str,
    catalog_service_ids: &[&str],
) -> AppResult<HashMap<String, ViewerRouting>> {
    if catalog_service_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let id_strings: Vec<String> = catalog_service_ids.iter().map(|s| s.to_string()).collect();
    let user_services: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "user_id": viewer_user_id,
            "catalog_service_id": { "$in": &id_strings },
        })
        .await?
        .try_collect()
        .await?;

    // Group by catalog_service_id so we can detect "multiple bindings".
    let mut grouped: HashMap<String, Vec<UserService>> = HashMap::new();
    for us in user_services {
        let Some(catalog_id) = us.catalog_service_id.clone() else {
            continue;
        };
        grouped.entry(catalog_id).or_default().push(us);
    }

    let mut out = HashMap::with_capacity(grouped.len());
    for (catalog_id, mut bindings) in grouped {
        let entry = match bindings.len() {
            0 => ViewerRouting::empty(),
            1 => {
                let us = bindings.pop().unwrap();
                ViewerRouting::single(us.node_id.clone(), us.id)
            }
            // Don't pick arbitrarily when the viewer has multiple
            // personal bindings -- the FE shows a disambiguation hint.
            n => ViewerRouting::ambiguous(u32::try_from(n).unwrap_or(u32::MAX)),
        };
        out.insert(catalog_id, entry);
    }

    Ok(out)
}

/// Verify that the authenticated user has admin privileges.
pub async fn require_admin(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    Ok(())
}

/// Verify admin or service creator.
pub async fn require_admin_or_creator(
    state: &AppState,
    auth_user: &AuthUser,
    service_created_by: &str,
) -> AppResult<()> {
    let user_id_str = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin && service_created_by != user_id_str {
        return Err(AppError::Forbidden(
            "Only admins or the service creator can perform this action".to_string(),
        ));
    }

    Ok(())
}

/// Fetch a service by ID or return NotFound.
pub async fn fetch_service(state: &AppState, service_id: &str) -> AppResult<DownstreamService> {
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))
}

#[derive(Debug)]
pub enum ResolvedService {
    Catalog(Box<DownstreamService>),
    Owned {
        user_service: Box<UserService>,
        user_endpoint: Box<UserEndpoint>,
        owner_id: String,
    },
}

/// Resolve either a catalog `DownstreamService` or a readable `UserService`
/// paired with its backing `UserEndpoint`.
pub async fn resolve_service_or_user_service(
    state: &AppState,
    service_id: &str,
    caller_user_id: &str,
) -> AppResult<ResolvedService> {
    if let Some(service) = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
    {
        return Ok(ResolvedService::Catalog(Box::new(service)));
    }

    let Some(user_service) =
        user_service_service::find_user_service_by_id(&state.db, service_id).await?
    else {
        return Err(AppError::NotFound("Service not found".to_string()));
    };

    let access =
        org_service::resolve_owner_access(&state.db, caller_user_id, &user_service.user_id).await?;
    if !access.can_read() || !access.allows_resource(&user_service.id) {
        return Err(AppError::NotFound("Service not found".to_string()));
    }

    let user_endpoint = state
        .db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! {
            "_id": &user_service.endpoint_id,
            "user_id": &user_service.user_id,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    Ok(ResolvedService::Owned {
        owner_id: user_service.user_id.clone(),
        user_service: Box::new(user_service),
        user_endpoint: Box::new(user_endpoint),
    })
}

/// Build a `ServiceResponse` from a `DownstreamService` model.
///
/// `viewer` carries the viewer's personal routing for this catalog row
/// (issue #416). Pass `None` for handlers that don't have viewer
/// context or know the result will always be "no binding" (e.g.
/// the create handler, where the catalog row didn't exist a moment
/// ago). See [`compute_viewer_routing`] for batched resolution and
/// [`ViewerRouting`] for the multi-binding semantics.
pub fn service_to_response_with_viewer(
    s: DownstreamService,
    viewer: Option<&ViewerRouting>,
) -> ServiceResponse {
    ServiceResponse {
        id: s.id,
        name: s.name,
        slug: s.slug,
        description: s.description,
        base_url: s.base_url,
        service_type: s.service_type,
        visibility: s.visibility,
        auth_method: s.auth_method,
        auth_type: s.auth_type,
        auth_key_name: s.auth_key_name,
        is_active: s.is_active,
        oauth_client_id: s.oauth_client_id,
        openapi_spec_url: s.openapi_spec_url.clone(),
        api_spec_url: s.openapi_spec_url,
        asyncapi_spec_url: s.asyncapi_spec_url,
        streaming_supported: s.streaming_supported,
        ssh_config: s.ssh_config.map(|ssh| SshServiceConfigResponse {
            host: ssh.host,
            port: ssh.port,
            certificate_auth_enabled: ssh.certificate_auth_enabled,
            certificate_ttl_minutes: ssh.certificate_ttl_minutes,
            allowed_principals: ssh.allowed_principals,
            ca_public_key: ssh.ca_public_key,
        }),
        service_category: s.service_category,
        requires_user_credential: s.requires_user_credential,
        identity_propagation_mode: s.identity_propagation_mode,
        identity_include_user_id: s.identity_include_user_id,
        identity_include_email: s.identity_include_email,
        identity_include_name: s.identity_include_name,
        identity_jwt_audience: s.identity_jwt_audience,
        forward_access_token: s.forward_access_token,
        inject_delegation_token: s.inject_delegation_token,
        delegation_token_scope: s.delegation_token_scope,
        homepage_url: s.homepage_url,
        repository_url: s.repository_url,
        issues_url: s.issues_url,
        capabilities: s.capabilities,
        auth_notes: s.auth_notes,
        known_limitations: s.known_limitations,
        required_permissions: s.required_permissions,
        examples_url: s.examples_url,
        recommended_skills: s.recommended_skills,
        custom_user_agent: s.custom_user_agent,
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            s.default_request_headers,
        ),
        ws_frame_injections: s.ws_frame_injections,
        developer_app_ids: s.developer_app_ids,
        created_by: s.created_by,
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
        node_id: viewer.and_then(|v| v.node_id.clone()),
        your_user_service_id: viewer.and_then(|v| v.user_service_id.clone()),
        your_binding_count: viewer.map_or(0, |v| v.binding_count),
    }
}

/// Validate that `developer_app_ids` reference active OAuth clients.
///
/// Any mutation of `developer_app_ids` is admin-only because it affects
/// cross-user auto-provisioning -- both setting and clearing the field.
/// Each referenced OAuth client must exist and be active; deleted or
/// unknown IDs are rejected. An empty list is a valid admin-authorized
/// clear operation.
pub async fn validate_developer_app_ids(
    state: &AppState,
    auth_user: &AuthUser,
    app_ids: &[String],
) -> AppResult<()> {
    // Any mutation (set or clear) requires admin -- clearing also has
    // cross-user impact (stops auto-provisioning for consented users).
    require_admin(state, auth_user).await?;

    if app_ids.is_empty() {
        return Ok(());
    }

    if app_ids.len() > 50 {
        return Err(AppError::ValidationError(
            "developer_app_ids must not exceed 50 entries".to_string(),
        ));
    }

    let id_refs: Vec<&str> = app_ids.iter().map(|s| s.as_str()).collect();
    let active_clients: Vec<OauthClient> = state
        .db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find(doc! {
            "_id": { "$in": &id_refs },
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;

    let found_ids: std::collections::HashSet<&str> =
        active_clients.iter().map(|c| c.id.as_str()).collect();

    for id in app_ids {
        if !found_ids.contains(id.as_str()) {
            return Err(AppError::ValidationError(format!(
                "developer_app_ids references unknown or inactive OAuth client: {id}"
            )));
        }
    }

    Ok(())
}

pub fn require_http_service(service: &DownstreamService) -> AppResult<()> {
    if service.service_type != "http" {
        return Err(AppError::BadRequest(
            "This operation is only supported for HTTP services".to_string(),
        ));
    }

    Ok(())
}

/// Typed response for delete operations (CR-16).
#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteServiceResponse {
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::{ResolvedService, resolve_service_or_user_service};
    use crate::errors::AppError;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::user::COLLECTION_NAME as USERS;
    use crate::models::user::UserType;
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_user, test_user_endpoint, test_user_service,
    };
    use uuid::Uuid;

    fn custom_catalog_service(service_id: &str) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = service_id.to_string();
        service.slug = "catalog-service".to_string();
        service.name = "Catalog Service".to_string();
        service.base_url = "https://api.example.com".to_string();
        service
    }

    #[tokio::test]
    async fn resolver_returns_catalog_service_by_id() {
        let Some(db) = connect_test_database("resolve_service_catalog").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();

        let catalog = custom_catalog_service("catalog-1");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog.clone())
            .await
            .unwrap();

        let state = test_app_state(db);
        let resolved = resolve_service_or_user_service(&state, &catalog.id, &caller_id)
            .await
            .unwrap();

        match resolved {
            ResolvedService::Catalog(service) => assert_eq!(service.id, catalog.id),
            other => panic!("expected catalog service, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolver_returns_owned_user_service_with_endpoint() {
        let Some(db) = connect_test_database("resolve_service_owned").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "Custom API",
            "https://custom.example.com",
            Some("https://example.com/openapi.json"),
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "custom-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint.clone())
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service.clone())
            .await
            .unwrap();

        let state = test_app_state(db);
        let resolved = resolve_service_or_user_service(&state, &user_service.id, &caller_id)
            .await
            .unwrap();

        match resolved {
            ResolvedService::Owned {
                user_service: resolved_service,
                user_endpoint,
                owner_id,
            } => {
                assert_eq!(resolved_service.id, user_service.id);
                assert_eq!(user_endpoint.id, endpoint.id);
                assert_eq!(owner_id, caller_id);
            }
            other => panic!("expected owned service, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolver_hides_cross_user_service_ids() {
        let Some(db) = connect_test_database("resolve_service_cross_user").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        let owner_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &owner_id,
            "Other API",
            "https://other.example.com",
            Some("https://example.com/openapi.json"),
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &owner_id,
            "other-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(&caller_id, UserType::Person),
                test_user(&owner_id, UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service.clone())
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = resolve_service_or_user_service(&state, &user_service.id, &caller_id)
            .await
            .expect_err("cross-user service should be hidden");

        assert!(matches!(err, AppError::NotFound(message) if message == "Service not found"));
    }

    #[tokio::test]
    async fn resolver_returns_not_found_for_missing_service_id() {
        let Some(db) = connect_test_database("resolve_service_missing").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = resolve_service_or_user_service(&state, "missing-service", &caller_id)
            .await
            .expect_err("missing service should 404");

        assert!(matches!(err, AppError::NotFound(message) if message == "Service not found"));
    }

    // ----------------------------------------------------------------
    // Issue #416: viewer-scoped routing lookup for /services responses
    // ----------------------------------------------------------------

    use super::compute_viewer_routing;

    /// Empty input -> empty result; no DB hit needed.
    #[tokio::test]
    async fn viewer_routing_short_circuits_on_empty_input() {
        let Some(db) = connect_test_database("viewer_routing_empty").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };
        let viewer_id = Uuid::new_v4().to_string();
        let routing = compute_viewer_routing(&db, &viewer_id, &[]).await.unwrap();
        assert!(routing.is_empty());
    }

    /// No personal binding for the viewer -> map omits the catalog id
    /// (the caller's `.unwrap_or_default()` then renders count=0).
    #[tokio::test]
    async fn viewer_routing_no_binding() {
        let Some(db) = connect_test_database("viewer_routing_zero").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };
        let viewer_id = Uuid::new_v4().to_string();
        let catalog_id = Uuid::new_v4().to_string();

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&viewer_id, UserType::Person))
            .await
            .unwrap();

        let routing = compute_viewer_routing(&db, &viewer_id, &[catalog_id.as_str()])
            .await
            .unwrap();
        assert!(!routing.contains_key(&catalog_id));
    }

    /// Single personal binding routed via a node -> all three fields
    /// populated (`node_id`, `your_user_service_id`, count=1).
    #[tokio::test]
    async fn viewer_routing_single_binding_via_node() {
        let Some(db) = connect_test_database("viewer_routing_single_via").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };
        let viewer_id = Uuid::new_v4().to_string();
        let catalog_id = Uuid::new_v4().to_string();
        let node_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &viewer_id,
            "Bound",
            "https://bound.example.com",
            None,
            None,
        );
        let us = test_user_service(
            &Uuid::new_v4().to_string(),
            &viewer_id,
            "bound",
            &endpoint.id,
            Some(&catalog_id),
            Some(&node_id),
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&viewer_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(us.clone())
            .await
            .unwrap();

        let routing = compute_viewer_routing(&db, &viewer_id, &[catalog_id.as_str()])
            .await
            .unwrap();
        let entry = routing.get(&catalog_id).expect("entry present");
        assert_eq!(entry.node_id.as_deref(), Some(node_id.as_str()));
        assert_eq!(entry.user_service_id.as_deref(), Some(us.id.as_str()));
        assert_eq!(entry.binding_count, 1);
    }

    /// Single personal binding with direct routing -> `node_id` is
    /// `None` but `your_user_service_id` is populated (admin can still
    /// edit via the RoutingSection).
    #[tokio::test]
    async fn viewer_routing_single_binding_direct() {
        let Some(db) = connect_test_database("viewer_routing_single_direct").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };
        let viewer_id = Uuid::new_v4().to_string();
        let catalog_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &viewer_id,
            "Direct",
            "https://direct.example.com",
            None,
            None,
        );
        let us = test_user_service(
            &Uuid::new_v4().to_string(),
            &viewer_id,
            "direct",
            &endpoint.id,
            Some(&catalog_id),
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&viewer_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(us.clone())
            .await
            .unwrap();

        let routing = compute_viewer_routing(&db, &viewer_id, &[catalog_id.as_str()])
            .await
            .unwrap();
        let entry = routing.get(&catalog_id).expect("entry present");
        assert!(entry.node_id.is_none());
        assert_eq!(entry.user_service_id.as_deref(), Some(us.id.as_str()));
        assert_eq!(entry.binding_count, 1);
    }

    /// Multiple personal bindings -> count >= 2, neither flat field
    /// populated (FE shows a "manage in AI Services" link instead of
    /// silently picking one).
    #[tokio::test]
    async fn viewer_routing_multiple_bindings_dont_pick() {
        let Some(db) = connect_test_database("viewer_routing_multi").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };
        let viewer_id = Uuid::new_v4().to_string();
        let catalog_id = Uuid::new_v4().to_string();

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&viewer_id, UserType::Person))
            .await
            .unwrap();

        for slug in ["binding-a", "binding-b"] {
            let endpoint = test_user_endpoint(
                &Uuid::new_v4().to_string(),
                &viewer_id,
                slug,
                "https://multi.example.com",
                None,
                None,
            );
            let us = test_user_service(
                &Uuid::new_v4().to_string(),
                &viewer_id,
                slug,
                &endpoint.id,
                Some(&catalog_id),
                None,
            );
            db.collection::<UserEndpoint>(USER_ENDPOINTS)
                .insert_one(endpoint)
                .await
                .unwrap();
            db.collection::<UserService>(USER_SERVICES)
                .insert_one(us)
                .await
                .unwrap();
        }

        let routing = compute_viewer_routing(&db, &viewer_id, &[catalog_id.as_str()])
            .await
            .unwrap();
        let entry = routing.get(&catalog_id).expect("entry present");
        assert!(entry.node_id.is_none());
        assert!(entry.user_service_id.is_none());
        assert_eq!(entry.binding_count, 2);
    }

    /// Org-shared binding (different `user_id`) doesn't surface for the
    /// admin viewing the catalog -- editing org state from the catalog
    /// page would leak across org boundaries.
    #[tokio::test]
    async fn viewer_routing_excludes_org_shared_bindings() {
        let Some(db) = connect_test_database("viewer_routing_org_only").await else {
            eprintln!("skipping services_helpers integration test: no local MongoDB available");
            return;
        };
        let viewer_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let catalog_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &org_id,
            "Org",
            "https://org.example.com",
            None,
            None,
        );
        let us = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "org",
            &endpoint.id,
            Some(&catalog_id),
            Some(&Uuid::new_v4().to_string()),
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(&viewer_id, UserType::Person),
                test_user(&org_id, UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(us)
            .await
            .unwrap();

        let routing = compute_viewer_routing(&db, &viewer_id, &[catalog_id.as_str()])
            .await
            .unwrap();
        assert!(!routing.contains_key(&catalog_id));
    }
}
