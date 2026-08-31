use axum::Json;
use axum::extract::State;
use chrono::Utc;
use mongodb::bson::doc;
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::catalog_delegation_grant::{
    COLLECTION_NAME as CATALOG_DELEGATION_GRANTS, CatalogDelegationGrant,
};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    audit_service, catalog_delegation_service, mcp_service, token_exchange_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event, hash_short_id};

#[derive(Debug, Serialize)]
pub struct DelegatedOperationCatalogResponse {
    pub contract_version: &'static str,
    /// Opaque admission token. This intentionally uses the pre-existing
    /// #1424 approval digest over the complete scoped catalog, not a checksum
    /// of this filtered response body. Unrelated catalog metadata can therefore
    /// cause a drift rejection without changing the listed operations.
    pub catalog_digest: String,
    /// Opaque v2 representation validator. During the bounded rolling window
    /// it uses the pre-additive projection understood by old replicas.
    pub exact_view_digest: String,
    pub resolved_at: String,
    pub authority_expires_at: String,
    pub services: Vec<mcp_service::ExactOperationViewService>,
    pub total_services: usize,
    pub total_operations: usize,
}

/// GET /api/v1/delegation/operation-catalog
///
/// The catalog scope is discovery-only. Approval create/observe/redeem remain
/// governed by their existing proxy-scope checks; callers need both
/// `mcp:catalog:read` and `proxy:*` (or `proxy`) for the full workflow.
pub async fn get_operation_catalog(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<DelegatedOperationCatalogResponse>> {
    let jti = require_catalog_authority(&auth_user)?;
    let now = Utc::now();
    let grant = state
        .db
        .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
        .find_one(doc! {
            "_id": jti,
            "revoked": false,
            "expires_at": { "$gt": mongodb::bson::DateTime::from_chrono(now) },
        })
        .await?
        .ok_or_else(invalid_catalog_authority)?;

    if !grant_matches_token(&grant, &auth_user) {
        return Err(invalid_catalog_authority());
    }

    let node_scope = if grant.allow_all_nodes {
        mcp_service::NodeScope::Unrestricted
    } else {
        mcp_service::NodeScope::Allowed(grant.allowed_node_ids.as_slice())
    };
    let service_scope = if grant.allow_all_services {
        mcp_service::ServiceScope::Unrestricted
    } else {
        mcp_service::ServiceScope::Allowed(grant.allowed_service_ids.as_slice())
    };
    let resolution_user_id = auth_user.proxy_resolution_user_id();
    let catalog = mcp_service::load_operation_catalog(
        &state.db,
        state.node_ws_manager.as_ref(),
        &resolution_user_id,
        node_scope,
        service_scope,
    )
    .await?;
    // `catalog_digest` remains the pre-existing broad approval fence shared
    // with `/mcp/config`. `exact_view_digest` is the additive representation
    // validator over exactly the generic-free response below.
    let view = mcp_service::exact_operation_view(&catalog.services);
    let catalog_digest = mcp_service::operation_catalog_digest(&catalog.services);
    // Keep the v2 fence consumable by pre-deploy replicas during the bounded
    // mixed-version window. The additive operation fields remain in `services`.
    let exact_view_digest = mcp_service::legacy_exact_operation_view_digest(&view);
    let resolved_at = Utc::now();
    let services = view.services;
    let total_services = services.len();
    let total_operations = services
        .iter()
        .map(|service| service.operations.len())
        .sum();

    Ok(Json(DelegatedOperationCatalogResponse {
        contract_version: "nyxid-delegated-operation-catalog.v2",
        catalog_digest,
        exact_view_digest,
        resolved_at: resolved_at.to_rfc3339(),
        authority_expires_at: grant.expires_at.to_rfc3339(),
        services,
        total_services,
        total_operations,
    }))
}

/// Pre-database eligibility gate for the delegated operation catalog.
///
/// Returns the token's `jti` only when the caller may look up catalog
/// authority at all. Split out from the handler so the rejection matrix is
/// exercisable without a database.
fn require_catalog_authority(auth_user: &AuthUser) -> AppResult<&str> {
    if auth_user.auth_method != AuthMethod::Delegated {
        return Err(AppError::Forbidden(
            "Only delegated tokens can access the operation catalog".to_string(),
        ));
    }
    if !catalog_delegation_service::scope_has_catalog_read(&auth_user.scope) {
        return Err(AppError::Forbidden(
            "Delegated token lacks mcp:catalog:read".to_string(),
        ));
    }
    auth_user
        .token_jti
        .as_deref()
        .ok_or_else(invalid_catalog_authority)
}

/// Every authority field the delegated token asserts must still match the live
/// grant. Any drift fails closed: the grant is the online policy anchor, and a
/// token whose claims have diverged from it is treated as invalid rather than
/// reconciled. Missing actor/receiving client identity is also a rejection —
/// a delegated token without both is never a valid catalog caller.
fn grant_matches_token(grant: &CatalogDelegationGrant, auth_user: &AuthUser) -> bool {
    let (Some(actor_client_id), Some(receiving_client_id)) = (
        auth_user.acting_client_id.as_deref(),
        auth_user.oauth_client_id.as_deref(),
    ) else {
        return false;
    };

    grant.user_id == auth_user.user_id.to_string()
        && grant.actor_client_id == actor_client_id
        && grant.receiving_client_id == receiving_client_id
        && grant.scope == auth_user.scope
        && grant.resources == auth_user.resource_uris.clone().unwrap_or_default()
        && grant.allow_all_services == auth_user.allow_all_services
        && grant.allowed_service_ids == auth_user.allowed_service_ids
        && grant.allow_all_nodes == auth_user.allow_all_nodes
        && grant.allowed_node_ids == auth_user.allowed_node_ids
}

fn invalid_catalog_authority() -> AppError {
    AppError::Unauthorized("Delegated catalog authority is invalid or inactive".to_string())
}

#[derive(Serialize)]
pub struct DelegationRefreshResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}

/// POST /api/v1/delegation/refresh
///
/// Refresh a delegated access token. Only accepts delegated tokens
/// (tokens with `act.sub` / `acting_client_id`). Issues a new delegation
/// token with the same scope and acting client but a fresh 5-minute TTL.
pub async fn refresh_delegation_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
) -> AppResult<Json<DelegationRefreshResponse>> {
    // Only delegated tokens can use this endpoint
    let acting_client_id = auth_user.acting_client_id.as_deref().ok_or_else(|| {
        AppError::Forbidden("Only delegated tokens can be refreshed via this endpoint".to_string())
    })?;

    let user_id_str = auth_user.user_id.to_string();

    let result = token_exchange_service::refresh_delegation_token(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &user_id_str,
        acting_client_id,
        auth_user.oauth_client_id.as_deref(),
        &auth_user.scope,
        &crate::crypto::jwt::TokenRestrictionClaims::from_auth_user(&auth_user),
    )
    .await?;

    emit_event(
        state.telemetry.as_deref(),
        &user_id_str,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AuthDelegationRefreshed {
            // Hash: raw UUID would be scrubbed to `[UUID_REDACTED]`.
            client_id: hash_short_id(acting_client_id),
        },
    );

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "delegation_token_refreshed",
        Some(serde_json::json!({
            "acting_client_id": acting_client_id,
            "scope": &result.scope,
        })),
    );

    Ok(Json(DelegationRefreshResponse {
        access_token: result.access_token,
        token_type: result.token_type,
        expires_in: result.expires_in,
        scope: result.scope,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use axum::{
        Extension, Json as AxumJson, Router,
        extract::Path,
        routing::{any, get},
    };
    use chrono::Utc;
    use mongodb::event::EventHandler;
    use mongodb::event::command::CommandEvent;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::crypto::{jwt, token::hash_token};
    use crate::handlers::exact_service_approvals;
    use crate::models::default_request_header::DefaultRequestHeader;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, ProxyOperationPolicy,
        ProxyOperationRule,
    };
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_service_binding::{
        COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
    };
    use crate::models::oauth_client::{
        COLLECTION_NAME as OAUTH_CLIENTS, OauthClient, ScopeProvenance,
    };
    use crate::models::service_approval_config::{ApprovalMode, ServiceApprovalConfig};
    use crate::models::service_endpoint::{
        COLLECTION_NAME as SERVICE_ENDPOINTS, OperationResponseContract, ServiceEndpoint,
    };
    use crate::models::user::UserType;
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::{approval_service, consent_service, token_exchange_service};

    use super::*;

    const TEST_USER_ID: &str = "00000000-0000-4000-8000-000000000001";
    const TEST_SERVICE_A: &str = "00000000-0000-4000-8000-000000000101";
    const TEST_SERVICE_B: &str = "00000000-0000-4000-8000-000000000102";
    const TEST_GENERIC_SERVICE: &str = "00000000-0000-4000-8000-000000000103";
    const TEST_NODE_ID: &str = "00000000-0000-4000-8000-000000000600";
    const TEST_OUT_OF_SCOPE_NODE_ID: &str = "00000000-0000-4000-8000-000000000601";
    const TEST_CATALOG_SERVICE_ID: &str = "00000000-0000-4000-8000-000000000301";
    const TEST_USER_ENDPOINT_ID: &str = "00000000-0000-4000-8000-000000000201";

    /// Shape of the one operation the full-router fixture drives. Tests select
    /// the fixture's service by this operation rather than by its id: selecting
    /// on `user_service_id` and then sending that same id back would make every
    /// "response-derived" assertion circular.
    const FIXTURE_OPERATION_NAME: &str = "alpha_list";
    const FIXTURE_OPERATION_METHOD: &str = "GET";
    const FIXTURE_OPERATION_PATH: &str = "/items";

    const CREATE_PATH: &str = "/api/v1/approvals/exact-service/requests";

    struct FullRouterFixture {
        db: mongodb::Database,
        state: crate::AppState,
        app: Router,
        delegated_token: String,
        source_token: String,
        grant: CatalogDelegationGrant,
        delegated_scope: String,
        receiver_client_id: String,
        receiver_secret: String,
        requested_service_ids: Vec<String>,
        requested_node_ids: Vec<String>,
        catalog_digest: String,
        exact_view_digest: String,
        /// Taken from the discovery response, never from `TEST_SERVICE_A`, so
        /// create bodies carry a response-derived id.
        user_service_id: String,
        operation: mcp_service::ExactOperationViewOperation,
        operation_digest: String,
        provider_calls: Arc<AtomicUsize>,
        /// Every `Authorization` header the test router actually received, in
        /// order. Token-continuity assertions read this instead of re-reading
        /// the fixture field that produced the header.
        sent_authorizations: Arc<Mutex<Vec<String>>>,
        provider: tokio::task::JoinHandle<()>,
        _node_rx:
            tokio::sync::mpsc::Receiver<crate::services::node_ws_manager::NodeOutboundMessage>,
    }

    impl Drop for FullRouterFixture {
        fn drop(&mut self) {
            self.provider.abort();
        }
    }

    async fn setup_full_router_fixture(prefix: &str) -> FullRouterFixture {
        let db = crate::test_utils::connect_transaction_test_database(prefix).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let user_id = Uuid::parse_str(TEST_USER_ID).unwrap();
        let actor_client_id = format!("{prefix}-actor");
        let receiver_client_id = format!("{prefix}-receiver");
        let actor_secret = format!("{prefix}-actor-secret");
        let receiver_secret = format!("{prefix}-receiver-secret");
        let delegated_scope = format!(
            "{} proxy",
            catalog_delegation_service::MCP_CATALOG_READ_SCOPE
        );
        let requested_service_ids = vec![TEST_SERVICE_A.to_string()];

        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(TEST_USER_ID, UserType::Person))
            .await
            .expect("insert full-router user");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_many([
                oauth_client(&actor_client_id, &actor_secret, &delegated_scope),
                oauth_client(&receiver_client_id, &receiver_secret, &delegated_scope),
            ])
            .await
            .expect("insert full-router OAuth clients");
        consent_service::grant_consent_with_services(
            &db,
            TEST_USER_ID,
            &actor_client_id,
            "openid",
            Some(requested_service_ids.clone()),
        )
        .await
        .expect("grant full-router actor consent");
        consent_service::grant_consent_with_services(
            &db,
            TEST_USER_ID,
            &receiver_client_id,
            "openid",
            Some(requested_service_ids.clone()),
        )
        .await
        .expect("grant full-router receiver consent");

        let fixture_auth = fixture_catalog_auth();
        seed_operation_catalog_fixture(&db, &fixture_auth).await;
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000403" },
                mongodb::bson::doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("disable alternate provider endpoint");

        let provider_calls = Arc::new(AtomicUsize::new(0));
        let route_calls = provider_calls.clone();
        let fallback_calls = provider_calls.clone();
        let provider_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind full-router provider spy");
        let provider_addr = provider_listener.local_addr().unwrap();
        let provider = tokio::spawn(async move {
            axum::serve(
                provider_listener,
                Router::new()
                    .route(
                        "/items",
                        get(move || {
                            let route_calls = route_calls.clone();
                            async move {
                                route_calls.fetch_add(1, Ordering::SeqCst);
                                AxumJson(serde_json::json!({"ok": true}))
                            }
                        }),
                    )
                    .fallback(any(move || {
                        let fallback_calls = fallback_calls.clone();
                        async move {
                            fallback_calls.fetch_add(1, Ordering::SeqCst);
                            (
                                StatusCode::NOT_FOUND,
                                AxumJson(serde_json::json!({"unexpected": true})),
                            )
                        }
                    })),
            )
            .await
            .expect("serve full-router provider spy");
        });
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000201" },
                mongodb::bson::doc! { "$set": { "url": format!("http://{provider_addr}") } },
            )
            .await
            .expect("point full-router endpoint at provider spy");
        db.collection::<ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(ServiceApprovalConfig {
            id: Uuid::new_v4().to_string(),
            user_id: TEST_USER_ID.to_string(),
            service_id: "00000000-0000-4000-8000-000000000301".to_string(),
            service_name: "Alpha Service".to_string(),
            approval_required: true,
            approval_mode: ApprovalMode::PerRequest,
            rules: Vec::new(),
            default_effect: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("seed full-router approval config");

        let requested_node_ids = vec![TEST_NODE_ID.to_string()];
        db.collection::<Node>(NODES)
            .insert_one(Node {
                id: TEST_NODE_ID.to_string(),
                user_id: TEST_USER_ID.to_string(),
                name: "full-router-scope-node".to_string(),
                status: NodeStatus::Online,
                auth_token_hash: "scope-node-token-hash".to_string(),
                signing_secret_encrypted: None,
                signing_secret_hash: "scope-node-signing-hash".to_string(),
                last_heartbeat_at: Some(Utc::now()),
                connected_at: Some(Utc::now()),
                metadata: None,
                metrics: NodeMetrics::default(),
                connection_owner: None,
                is_active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("insert fixture node for explicit node-bound token");
        let (node_tx, node_rx) = tokio::sync::mpsc::channel(8);
        crate::test_utils::register_test_node_connection(&state, TEST_NODE_ID, node_tx).await;

        let source_token = jwt::generate_oauth_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            "openid",
            None,
            None,
            None,
            None,
            Some(jwt::AccessTokenRestrictions {
                resources: &[],
                allowed_service_ids: &requested_service_ids,
                allowed_node_ids: &requested_node_ids,
                allow_all_nodes: false,
            }),
            &actor_client_id,
        )
        .expect("mint full-router source token");
        let exchanged = token_exchange_service::exchange_token_with_authority(
            &db,
            &state.config,
            &state.jwt_keys,
            &receiver_client_id,
            &receiver_secret,
            &source_token,
            "urn:ietf:params:oauth:token-type:access_token",
            Some(&delegated_scope),
            &[],
            Some(false),
            &requested_service_ids,
            Some(false),
            &requested_node_ids,
        )
        .await
        .expect("exchange full-router delegated token");

        let (_, private) = crate::routes::build_router();
        let sent_authorizations: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let app = private
            .with_state(state.clone())
            .layer(axum::middleware::from_fn_with_state(
                sent_authorizations.clone(),
                record_transmitted_authorization,
            ));
        let (discovery_status, discovery) = full_router_json_request(
            &app,
            Method::GET,
            "/api/v1/delegation/operation-catalog",
            &exchanged.access_token,
            None,
        )
        .await;
        assert_eq!(
            discovery_status,
            StatusCode::OK,
            "discovery failed: {discovery}"
        );
        assert_eq!(
            discovery["contract_version"],
            "nyxid-delegated-operation-catalog.v2"
        );
        let services: Vec<mcp_service::ExactOperationViewService> =
            serde_json::from_value(discovery["services"].clone())
                .expect("decode discovery services from HTTP body");
        let service = unique_fixture_service(&services);
        // Identity is asserted, never used as the selector.
        assert_eq!(
            service.user_service_id, TEST_SERVICE_A,
            "discovery must surface the seeded fixture service"
        );
        assert!(
            service.node_id.is_none(),
            "fixture service must stay direct-routed so the HTTP spy remains the effect path"
        );
        let operation = fixture_operation(service).clone();
        let arguments = serde_json::json!({});
        let operation_digest = mcp_service::exact_operation_digest_from_parts(
            &service.user_service_id,
            &operation.endpoint_id,
            &operation.endpoint_contract_digest,
            &arguments,
        );

        let delegated_auth = delegated_auth_from_token(&state, &exchanged.access_token);
        let grant = db
            .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .find_one(mongodb::bson::doc! {
                "_id": delegated_auth.token_jti.as_deref().unwrap()
            })
            .await
            .expect("load exchanged grant")
            .expect("exchange persisted a grant");
        assert!(
            !grant.allow_all_nodes,
            "full-router token must be explicitly node-bound"
        );
        assert_eq!(grant.allowed_node_ids, requested_node_ids);

        FullRouterFixture {
            db,
            state,
            app,
            delegated_token: exchanged.access_token,
            source_token,
            grant,
            delegated_scope,
            receiver_client_id,
            receiver_secret,
            requested_service_ids,
            requested_node_ids,
            catalog_digest: discovery["catalog_digest"]
                .as_str()
                .expect("discovery catalog_digest")
                .to_string(),
            exact_view_digest: discovery["exact_view_digest"]
                .as_str()
                .expect("discovery exact_view_digest")
                .to_string(),
            user_service_id: service.user_service_id.clone(),
            operation,
            operation_digest,
            provider_calls,
            sent_authorizations,
            provider,
            _node_rx: node_rx,
        }
    }

    /// Records the `Authorization` header of every request the test router
    /// actually receives. Continuity assertions read this log so they observe
    /// the bearer as transmitted, rather than re-reading the fixture field the
    /// header was built from.
    async fn record_transmitted_authorization(
        axum::extract::State(log): axum::extract::State<Arc<Mutex<Vec<String>>>>,
        request: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        if let Some(value) = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            log.lock()
                .expect("transmitted authorization log")
                .push(value.to_string());
        }
        next.run(request).await
    }

    fn is_fixture_operation(operation: &mcp_service::ExactOperationViewOperation) -> bool {
        operation.name == FIXTURE_OPERATION_NAME
            && operation.method == FIXTURE_OPERATION_METHOD
            && operation.path == FIXTURE_OPERATION_PATH
    }

    /// Resolve the fixture's service from a discovery response by the operation
    /// it publishes. Uniqueness is asserted, so the selector cannot silently
    /// match a different service if the catalog fixture grows.
    fn unique_fixture_service(
        services: &[mcp_service::ExactOperationViewService],
    ) -> &mcp_service::ExactOperationViewService {
        let mut matching = services
            .iter()
            .filter(|service| service.operations.iter().any(is_fixture_operation));
        let service = matching
            .next()
            .expect("discovery must publish the fixture operation");
        assert!(
            matching.next().is_none(),
            "the fixture operation must identify exactly one discovered service"
        );
        service
    }

    fn fixture_operation(
        service: &mcp_service::ExactOperationViewService,
    ) -> &mcp_service::ExactOperationViewOperation {
        service
            .operations
            .iter()
            .find(|operation| is_fixture_operation(operation))
            .expect("fixture operation in discovery response")
    }

    fn full_router_create_body(
        fixture: &FullRouterFixture,
        idempotency_key: &str,
        exact_view_digest: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "user_service_id": fixture.user_service_id,
            "endpoint_id": fixture.operation.endpoint_id,
            "catalog_digest": fixture.catalog_digest,
            "exact_view_digest": exact_view_digest,
            "endpoint_contract_digest": fixture.operation.endpoint_contract_digest,
            "operation_digest": fixture.operation_digest,
            "operation_id": fixture.operation.endpoint_id,
            "operation_generation": fixture.operation.operation_generation,
            "idempotency_key": idempotency_key,
            "arguments": {},
        })
    }

    fn fence_body_from_created(
        created: &serde_json::Value,
        exact_view_digest_override: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "catalog_digest": created["catalog_digest"],
            "exact_view_digest": exact_view_digest_override,
            "operation_digest": created["operation_digest"],
            "operation_id": created["operation_id"],
            "operation_generation": created["operation_generation"],
            "idempotency_key": created["idempotency_key"],
        })
    }

    async fn full_router_json_request(
        app: &Router,
        method: Method,
        uri: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        let body = if let Some(body) = body {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&body).expect("serialize router request"))
        } else {
            Body::empty()
        };
        let response = app
            .clone()
            .oneshot(builder.body(body).expect("build router request"))
            .await
            .expect("full router response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read full router response");
        let json = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "full router returned non-JSON body ({error}): {}",
                String::from_utf8_lossy(&bytes)
            )
        });
        (status, json)
    }

    /// Re-mint the fixture's delegated token and re-resolve its live grant.
    ///
    /// `jwt::DELEGATED_TOKEN_TTL_SECS` is a compile-time 300s, while a full
    /// suite run takes longer than that and can starve a test thread, so a
    /// token minted at fixture-build time may already be expired by the time a
    /// later phase issues its request. The mutation matrices use this helper
    /// before slow DB mutation phases. The single-token AC4 journey deliberately
    /// avoids it and keeps its phases back-to-back.
    ///
    /// Ordering matters: always refresh BEFORE mutating/revoking the grant,
    /// never between the mutation and the request. Refreshing after the
    /// mutation would hand the request a brand new valid grant and the
    /// assertion would prove nothing.
    async fn refresh_full_router_delegation(fixture: &mut FullRouterFixture) {
        let exchanged = token_exchange_service::exchange_token_with_authority(
            &fixture.db,
            &fixture.state.config,
            &fixture.state.jwt_keys,
            &fixture.receiver_client_id,
            &fixture.receiver_secret,
            &fixture.source_token,
            "urn:ietf:params:oauth:token-type:access_token",
            Some(&fixture.delegated_scope),
            &[],
            Some(false),
            &fixture.requested_service_ids,
            Some(false),
            &fixture.requested_node_ids,
        )
        .await
        .expect("re-exchange full-router delegated token");
        let delegated_auth = delegated_auth_from_token(&fixture.state, &exchanged.access_token);
        let grant = fixture
            .db
            .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .find_one(mongodb::bson::doc! {
                "_id": delegated_auth.token_jti.as_deref().unwrap()
            })
            .await
            .expect("load refreshed full-router grant")
            .expect("refreshed exchange persisted a grant");
        fixture.delegated_token = exchanged.access_token;
        fixture.grant = grant;
    }

    /// Assert that every delegated bearer the router has actually received so
    /// far is byte-identical to the one the journey started with.
    ///
    /// This reads the transmitted `Authorization` headers, not
    /// `fixture.delegated_token`. Comparing the fixture field against a clone
    /// of itself would pass even if a phase sent a freshly exchanged token, so
    /// the continuity claim has to be made against the wire.
    ///
    /// The human decision phase legitimately uses the first-party source token,
    /// so that header is excluded; everything else must be the delegated one.
    fn assert_full_router_delegated_token_unchanged(
        fixture: &FullRouterFixture,
        expected_token: &str,
        expected_jti: &str,
        phase: &str,
    ) {
        let transmitted = fixture
            .sent_authorizations
            .lock()
            .expect("transmitted authorization log");
        let source_header = format!("Bearer {}", fixture.source_token);
        let delegated: Vec<&String> = transmitted
            .iter()
            .filter(|value| **value != source_header)
            .collect();
        assert!(
            !delegated.is_empty(),
            "no delegated request had been transmitted by {phase}"
        );
        let expected_header = format!("Bearer {expected_token}");
        for value in &delegated {
            assert_eq!(
                *value, &expected_header,
                "a delegated phase transmitted a different bearer by {phase}"
            );
        }
        // Parse the JTI out of the bytes that were actually sent.
        let sent_token = delegated
            .last()
            .expect("delegated bearer")
            .strip_prefix("Bearer ")
            .expect("bearer scheme on transmitted authorization");
        let auth = delegated_auth_from_token(&fixture.state, sent_token);
        assert_eq!(
            auth.token_jti.as_deref(),
            Some(expected_jti),
            "transmitted delegated token JTI changed during {phase}"
        );
    }

    async fn approve_full_router_request(fixture: &FullRouterFixture, request_id: &str) {
        let path = format!("/api/v1/approvals/requests/{request_id}/decide");
        let (status, body) = full_router_json_request(
            &fixture.app,
            Method::POST,
            &path,
            &fixture.source_token,
            Some(serde_json::json!({ "approved": true })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "decide failed: {body}");
        assert_eq!(body["status"], "approved", "decide body: {body}");
    }

    async fn reshape_full_router_request_as_main_v1(fixture: &FullRouterFixture, request_id: &str) {
        fixture
            .db
            .collection::<mongodb::bson::Document>(crate::models::approval_request::COLLECTION_NAME)
            .update_one(
                mongodb::bson::doc! { "_id": request_id },
                mongodb::bson::doc! { "$unset": {
                    "exact_service.execution_authority_binding": "",
                    "exact_service.producer_generation_bound": "",
                    "exact_service.exact_view_digest_binding": "",
                } },
            )
            .await
            .expect("reshape request as current-main v1 row");
    }

    async fn create_full_router_request(
        fixture: &FullRouterFixture,
        idempotency_key: &str,
    ) -> serde_json::Value {
        let (status, body) = full_router_json_request(
            &fixture.app,
            Method::POST,
            CREATE_PATH,
            &fixture.delegated_token,
            Some(full_router_create_body(
                fixture,
                idempotency_key,
                Some(&fixture.exact_view_digest),
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create failed: {body}");
        body
    }

    async fn redeem_full_router_request(
        fixture: &FullRouterFixture,
        created: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id");
        let path = format!("/api/v1/approvals/exact-service/requests/{request_id}/redeem");
        full_router_json_request(
            &fixture.app,
            Method::POST,
            &path,
            &fixture.delegated_token,
            Some(fence_body_from_created(
                created,
                created["exact_view_digest"].as_str(),
            )),
        )
        .await
    }

    async fn assert_no_full_router_redemption(fixture: &FullRouterFixture, request_id: &str) {
        let request = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "_id": request_id })
            .await
            .expect("load full-router request")
            .expect("full-router request exists");
        assert!(
            request
                .exact_service
                .and_then(|binding| binding.redemption)
                .is_none(),
            "rejected full-router redeem must not claim the row"
        );
    }

    async fn assert_full_router_terminal_redemption(
        fixture: &FullRouterFixture,
        request_id: &str,
        expected_status: crate::models::approval_request::ExactServiceRedemptionStatus,
        expected_failure_code: &str,
    ) {
        let request = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "_id": request_id })
            .await
            .expect("load full-router terminal request")
            .expect("full-router terminal request exists");
        let terminal = request
            .exact_service
            .and_then(|binding| binding.redemption)
            .expect("terminal revalidation must be durable");
        assert_eq!(terminal.status, expected_status);
        assert_eq!(
            terminal.failure_code.as_deref(),
            Some(expected_failure_code)
        );
        assert!(terminal.completed_at.is_some());
        assert!(terminal.receipt.is_none());
    }

    fn assert_error_response(
        status: StatusCode,
        body: &serde_json::Value,
        expected_status: StatusCode,
        expected_code: u64,
        expected_message: &str,
    ) {
        assert_eq!(status, expected_status, "unexpected error body: {body}");
        assert_eq!(body["error_code"], expected_code);
        assert_eq!(body["message"], expected_message);
    }

    #[test]
    fn delegation_refresh_response_serializes_all_fields() {
        let resp = DelegationRefreshResponse {
            access_token: "eyJhbGciOi...".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 300,
            scope: "llm:proxy".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["access_token"], "eyJhbGciOi...");
        assert_eq!(json["token_type"], "Bearer");
        assert_eq!(json["expires_in"], 300);
        assert_eq!(json["scope"], "llm:proxy");
    }

    #[test]
    fn delegation_refresh_response_field_names_match_oauth_convention() {
        let resp = DelegationRefreshResponse {
            access_token: "token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 900,
            scope: "openid".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        // Verify field names use snake_case as expected by OAuth specs
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("access_token"));
        assert!(obj.contains_key("token_type"));
        assert!(obj.contains_key("expires_in"));
        assert!(obj.contains_key("scope"));
        assert_eq!(obj.len(), 4);
    }

    #[test]
    fn operation_catalog_response_is_typed_and_secret_free() {
        let response = DelegatedOperationCatalogResponse {
            contract_version: "nyxid-delegated-operation-catalog.v2",
            catalog_digest: "sha256:opaque".to_string(),
            exact_view_digest: "sha256:exact-view".to_string(),
            resolved_at: "2026-08-14T00:00:00Z".to_string(),
            authority_expires_at: "2026-08-14T00:05:00Z".to_string(),
            services: vec![mcp_service::ExactOperationViewService {
                user_service_id: "service-1".to_string(),
                catalog_service_id: Some("catalog-1".to_string()),
                service_slug: "github".to_string(),
                service_name: "GitHub".to_string(),
                description: Some("typed service".to_string()),
                node_id: None,
                operations: vec![mcp_service::ExactOperationViewOperation {
                    endpoint_id: "op_1".to_string(),
                    name: "list_repositories".to_string(),
                    method: "GET".to_string(),
                    path: "/user/repos".to_string(),
                    parameters: None,
                    request_body_schema: None,
                    request_content_type: None,
                    request_body_required: false,
                    response: Default::default(),
                    risk: Some(crate::models::service_endpoint::EndpointRisk::Write),
                    supports_idempotency_key: true,
                    endpoint_contract_digest: "sha256:contract".to_string(),
                    operation_generation: Some(7),
                }],
            }],
            total_services: 1,
            total_operations: 1,
        };
        let value = serde_json::to_value(response).expect("serialize catalog response");
        let encoded = value.to_string().to_ascii_lowercase();
        for forbidden in ["credential", "token", "authorization", "secret", "api_key"] {
            assert!(!encoded.contains(forbidden), "response leaked {forbidden}");
        }
        assert_eq!(value["total_operations"], 1);
    }

    fn deterministic_exact_view_fixture() -> mcp_service::ExactOperationView {
        let response = || crate::models::service_endpoint::OperationResponseContract {
            content_types: vec!["application/json".to_string()],
            binary_artifact: Some(false),
        };
        let operation = |endpoint_id: &str,
                         name: &str,
                         method: &str,
                         path: &str,
                         endpoint_contract_digest: &str| {
            mcp_service::ExactOperationViewOperation {
                endpoint_id: endpoint_id.to_string(),
                name: name.to_string(),
                method: method.to_string(),
                path: path.to_string(),
                parameters: None,
                request_body_schema: None,
                request_content_type: None,
                request_body_required: false,
                response: response(),
                risk: Some(crate::models::service_endpoint::EndpointRisk::Write),
                supports_idempotency_key: true,
                endpoint_contract_digest: endpoint_contract_digest.to_string(),
                operation_generation: Some(7),
            }
        };
        mcp_service::ExactOperationView {
            services: vec![
                mcp_service::ExactOperationViewService {
                    user_service_id: TEST_SERVICE_A.to_string(),
                    catalog_service_id: Some("00000000-0000-4000-8000-000000000301".to_string()),
                    service_slug: "alpha".to_string(),
                    service_name: "Alpha Service".to_string(),
                    description: None,
                    node_id: None,
                    operations: vec![
                        operation(
                            "00000000-0000-4000-8000-000000000401",
                            "alpha_list",
                            "GET",
                            "/items",
                            "sha256:44d63aba629068d100761faf33f1359e0837f554f691ea50fd30a69bb7b8ea5a",
                        ),
                        operation(
                            "00000000-0000-4000-8000-000000000402",
                            "alpha_create",
                            "POST",
                            "/items",
                            "sha256:b33c78e543e5bd0db8ac01378ced3f1598ba15457c91b7526178fe91313e5021",
                        ),
                    ],
                },
                mcp_service::ExactOperationViewService {
                    user_service_id: TEST_SERVICE_B.to_string(),
                    catalog_service_id: Some("00000000-0000-4000-8000-000000000302".to_string()),
                    service_slug: "beta".to_string(),
                    service_name: "Beta Service".to_string(),
                    description: None,
                    node_id: None,
                    operations: vec![operation(
                        "00000000-0000-4000-8000-000000000403",
                        "beta_status",
                        "GET",
                        "/status",
                        "sha256:3f488040e19cb9fdc9e9f2aefd9061058c16426a58a2ace0664373e5b9574d5f",
                    )],
                },
            ],
        }
    }

    #[test]
    fn deterministic_fixture_binds_provider_identity_generation_and_v2_digest_envelope() {
        let view = deterministic_exact_view_fixture();
        assert_eq!(
            view.services[0].catalog_service_id.as_deref(),
            Some("00000000-0000-4000-8000-000000000301")
        );
        let digest = mcp_service::exact_operation_view_digest(&view);
        assert_eq!(
            digest,
            "sha256:bb033ad66809eca3f8225eadff97226de98311856a211ca80be91f2797730bb9"
        );
        assert_eq!(
            mcp_service::legacy_exact_operation_view_digest(&view),
            "sha256:acfbddf689ec25828e01cb4c149f9fd5f3ab5c9f37fddc7d0891e088cf1030a5",
            "the pre-additive v2 representation stays pinned during rolling compatibility"
        );
        let mut provider_changed = view.clone();
        provider_changed.services[0].catalog_service_id =
            Some("00000000-0000-4000-8000-000000000302".to_string());
        assert_ne!(
            digest,
            mcp_service::exact_operation_view_digest(&provider_changed),
            "catalog provider rebinding must move the exact-view fence"
        );
    }

    // ---- Catalog authority: fail-closed matrix -------------------------------
    //
    // `get_operation_catalog` needs a live database, so the authority decision
    // is split into two pure functions and the rejection matrix is asserted
    // against those directly. Each case below drives a distinct input to a
    // distinct decision; none of them re-assert the same call twice.

    const TEST_ACTOR: &str = "nyxid-assistant";
    const TEST_RECEIVER: &str = "aevatar";

    fn catalog_auth(method: AuthMethod) -> AuthUser {
        AuthUser {
            user_id: uuid::Uuid::new_v4(),
            session_id: None,
            scope: catalog_delegation_service::MCP_CATALOG_READ_SCOPE.to_string(),
            acting_client_id: Some(TEST_ACTOR.to_string()),
            oauth_client_id: Some(TEST_RECEIVER.to_string()),
            token_jti: Some(uuid::Uuid::new_v4().to_string()),
            approval_owner_user_id: None,
            auth_method: method,
            allow_all_services: false,
            allow_all_nodes: false,
            allowed_service_ids: vec!["svc-1".to_string()],
            resource_uris: Some(vec!["https://nyxid.test/api/v1/proxy/s/github".to_string()]),
            allowed_node_ids: vec!["node-1".to_string()],
            api_key_id: None,
            api_key_name: None,
            api_key_purpose: crate::models::api_key::ApiKeyPurpose::General,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    fn fixture_catalog_auth() -> AuthUser {
        let mut auth = catalog_auth(AuthMethod::Delegated);
        auth.user_id = uuid::Uuid::parse_str(TEST_USER_ID).unwrap();
        auth.token_jti = Some("00000000-0000-4000-8000-000000000501".to_string());
        // Deliberately reverse the service order. The response projection must
        // still sort by `user_service_id`.
        auth.allowed_service_ids = vec![
            TEST_GENERIC_SERVICE.to_string(),
            TEST_SERVICE_B.to_string(),
            TEST_SERVICE_A.to_string(),
        ];
        auth.allowed_node_ids.clear();
        auth.resource_uris = Some(Vec::new());
        auth
    }

    /// A grant agreeing with `auth` on every bound authority field.
    fn matching_grant(auth: &AuthUser) -> CatalogDelegationGrant {
        let now = mongodb::bson::DateTime::now().to_chrono();
        CatalogDelegationGrant {
            id: auth.token_jti.clone().expect("fixture carries a jti"),
            user_id: auth.user_id.to_string(),
            actor_client_id: TEST_ACTOR.to_string(),
            receiving_client_id: TEST_RECEIVER.to_string(),
            scope: auth.scope.clone(),
            resources: auth.resource_uris.clone().unwrap_or_default(),
            allow_all_services: auth.allow_all_services,
            allowed_service_ids: auth.allowed_service_ids.clone(),
            allow_all_nodes: auth.allow_all_nodes,
            allowed_node_ids: auth.allowed_node_ids.clone(),
            revoked: false,
            expires_at: now + chrono::Duration::minutes(5),
            created_at: now,
        }
    }

    fn template_service(id: &str, slug: &str) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = id.to_string();
        service.name = format!("{slug} template");
        service.slug = slug.to_string();
        service.requires_user_credential = true;
        service
    }

    fn template_endpoint(
        id: &str,
        service_id: &str,
        name: &str,
        method: &str,
        path: &str,
    ) -> ServiceEndpoint {
        let now = Utc::now();
        ServiceEndpoint {
            id: id.to_string(),
            service_id: service_id.to_string(),
            name: name.to_string(),
            description: Some(format!("{name} description omitted from exact view")),
            method: method.to_string(),
            path: path.to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: Some("response description omitted".to_string()),
            response: OperationResponseContract {
                content_types: vec!["application/json".to_string()],
                binary_artifact: Some(false),
            },
            risk: None,
            supports_idempotency_key: false,
            is_active: true,
            operation_generation: 1,
            created_at: now,
            updated_at: now,
        }
    }

    async fn seed_operation_catalog_fixture(
        db: &mongodb::Database,
        auth: &AuthUser,
    ) -> CatalogDelegationGrant {
        let grant = matching_grant(auth);
        db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .insert_one(&grant)
            .await
            .expect("insert catalog grant");

        let catalog_a = "00000000-0000-4000-8000-000000000301";
        let catalog_b = "00000000-0000-4000-8000-000000000302";
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_many([
                template_service(catalog_b, "beta-template"),
                template_service(catalog_a, "alpha-template"),
            ])
            .await
            .expect("insert catalog templates");
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .insert_many([
                template_endpoint(
                    "00000000-0000-4000-8000-000000000403",
                    catalog_b,
                    "beta_status",
                    "GET",
                    "/status",
                ),
                template_endpoint(
                    "00000000-0000-4000-8000-000000000402",
                    catalog_a,
                    "alpha_create",
                    "POST",
                    "/items",
                ),
                template_endpoint(
                    "00000000-0000-4000-8000-000000000401",
                    catalog_a,
                    "alpha_list",
                    "GET",
                    "/items",
                ),
            ])
            .await
            .expect("insert operation templates");

        let endpoints = [
            (
                "00000000-0000-4000-8000-000000000201",
                "Alpha Service",
                Some(catalog_a),
            ),
            (
                "00000000-0000-4000-8000-000000000202",
                "Beta Service",
                Some(catalog_b),
            ),
            (
                "00000000-0000-4000-8000-000000000203",
                "Hidden Generic",
                None,
            ),
        ];
        for (id, label, catalog_id) in endpoints {
            db.collection::<UserEndpoint>(USER_ENDPOINTS)
                .insert_one(crate::test_utils::test_user_endpoint(
                    id,
                    TEST_USER_ID,
                    label,
                    "https://provider.invalid",
                    None,
                    catalog_id,
                ))
                .await
                .expect("insert user endpoint");
        }
        for (id, slug, endpoint_id, catalog_id) in [
            (
                TEST_SERVICE_B,
                "beta",
                "00000000-0000-4000-8000-000000000202",
                Some(catalog_b),
            ),
            (
                TEST_GENERIC_SERVICE,
                "hidden-generic",
                "00000000-0000-4000-8000-000000000203",
                None,
            ),
            (
                TEST_SERVICE_A,
                "alpha",
                "00000000-0000-4000-8000-000000000201",
                Some(catalog_a),
            ),
        ] {
            db.collection::<UserService>(USER_SERVICES)
                .insert_one(crate::test_utils::test_user_service(
                    id,
                    TEST_USER_ID,
                    slug,
                    endpoint_id,
                    catalog_id,
                    None,
                ))
                .await
                .expect("insert user service");
        }
        grant
    }

    fn assert_inactive_authority(error: AppError) {
        assert!(matches!(
            error,
            AppError::Unauthorized(message)
                if message == "Delegated catalog authority is invalid or inactive"
        ));
    }

    #[tokio::test]
    async fn handler_rejects_absent_revoked_and_expired_live_grants() {
        let Some(db) = crate::test_utils::connect_test_database("delegation_handler_grants").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());

        let absent = fixture_catalog_auth();
        assert_inactive_authority(
            get_operation_catalog(State(state.clone()), absent)
                .await
                .expect_err("absent grant must fail at the handler boundary"),
        );

        let mut revoked_auth = fixture_catalog_auth();
        revoked_auth.token_jti = Some("00000000-0000-4000-8000-000000000502".to_string());
        let mut revoked = matching_grant(&revoked_auth);
        revoked.revoked = true;
        db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .insert_one(revoked)
            .await
            .unwrap();
        assert_inactive_authority(
            get_operation_catalog(State(state.clone()), revoked_auth)
                .await
                .expect_err("revoked grant must fail at the handler boundary"),
        );

        let mut expired_auth = fixture_catalog_auth();
        expired_auth.token_jti = Some("00000000-0000-4000-8000-000000000503".to_string());
        let mut expired = matching_grant(&expired_auth);
        expired.expires_at = Utc::now() - chrono::Duration::seconds(1);
        db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .insert_one(expired)
            .await
            .unwrap();
        assert_inactive_authority(
            get_operation_catalog(State(state), expired_auth)
                .await
                .expect_err("expired grant must fail at the handler boundary"),
        );
    }

    #[tokio::test]
    async fn handler_output_is_deterministic_and_performs_no_database_writes() {
        let commands = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed = commands.clone();
        let handler = EventHandler::<CommandEvent>::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                observed.lock().unwrap().push(event.command_name);
            }
        });
        let Some(db) = crate::test_utils::connect_test_database_with_command_handler(
            "delegation_handler_snapshot",
            handler,
        )
        .await
        else {
            return;
        };
        let auth = fixture_catalog_auth();
        let grant = seed_operation_catalog_fixture(&db, &auth).await;
        commands.lock().unwrap().clear();

        let Json(response) =
            get_operation_catalog(State(crate::test_utils::test_app_state(db)), auth)
                .await
                .expect("resolve delegated operation catalog");
        let mut actual = serde_json::to_value(response).unwrap();
        let returned_view = mcp_service::ExactOperationView {
            services: serde_json::from_value(actual["services"].clone())
                .expect("deserialize returned exact-operation view"),
        };
        let returned_digest = mcp_service::legacy_exact_operation_view_digest(&returned_view);
        assert_eq!(
            actual["exact_view_digest"].as_str(),
            Some(returned_digest.as_str()),
            "the digest must be recomputable from exactly the caller-visible services"
        );
        actual["resolved_at"] = serde_json::json!("<resolved-at>");
        assert_eq!(
            actual["authority_expires_at"],
            grant.expires_at.to_rfc3339()
        );
        assert_eq!(
            actual["contract_version"], "nyxid-delegated-operation-catalog.v2",
            "the response and exact-view digest envelope must move together"
        );
        let service_ids: Vec<&str> = actual["services"]
            .as_array()
            .expect("services array")
            .iter()
            .map(|service| service["user_service_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            service_ids,
            vec![TEST_SERVICE_A, TEST_SERVICE_B],
            "delegated discovery sorts services by user_service_id"
        );
        let operation_ids: Vec<&str> = actual["services"][0]["operations"]
            .as_array()
            .expect("operations array")
            .iter()
            .map(|operation| operation["endpoint_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            operation_ids,
            vec![
                "00000000-0000-4000-8000-000000000401",
                "00000000-0000-4000-8000-000000000402",
            ],
            "delegated discovery sorts operations by endpoint_id"
        );
        assert_eq!(
            actual,
            serde_json::json!({
                "contract_version": "nyxid-delegated-operation-catalog.v2",
                "catalog_digest": "sha256:ea55205436c9a87a35ec9d21072cff07e77dc2ef23409aeb16760f0d08662a62",
                "exact_view_digest": "sha256:acfbddf689ec25828e01cb4c149f9fd5f3ab5c9f37fddc7d0891e088cf1030a5",
                "resolved_at": "<resolved-at>",
                "authority_expires_at": grant.expires_at.to_rfc3339(),
                "services": [
                    {
                        "user_service_id": TEST_SERVICE_A,
                        "catalog_service_id": "00000000-0000-4000-8000-000000000301",
                        "service_slug": "alpha",
                        "service_name": "Alpha Service",
                        "description": null,
                        "node_id": null,
                        "operations": [
                            {
                                "endpoint_id": "00000000-0000-4000-8000-000000000401",
                                "name": "alpha_list",
                                "method": "GET",
                                "path": "/items",
                                "parameters": null,
                                "request_body_schema": null,
                                "request_content_type": null,
                                "request_body_required": false,
                                "response": {"content_types": ["application/json"], "binary_artifact": false},
                                "risk": null,
                                "supports_idempotency_key": false,
                                "endpoint_contract_digest": "sha256:44d63aba629068d100761faf33f1359e0837f554f691ea50fd30a69bb7b8ea5a",
                                "operation_generation": 1
                            },
                            {
                                "endpoint_id": "00000000-0000-4000-8000-000000000402",
                                "name": "alpha_create",
                                "method": "POST",
                                "path": "/items",
                                "parameters": null,
                                "request_body_schema": null,
                                "request_content_type": null,
                                "request_body_required": false,
                                "response": {"content_types": ["application/json"], "binary_artifact": false},
                                "risk": null,
                                "supports_idempotency_key": false,
                                "endpoint_contract_digest": "sha256:b33c78e543e5bd0db8ac01378ced3f1598ba15457c91b7526178fe91313e5021",
                                "operation_generation": 1
                            }
                        ]
                    },
                    {
                        "user_service_id": TEST_SERVICE_B,
                        "catalog_service_id": "00000000-0000-4000-8000-000000000302",
                        "service_slug": "beta",
                        "service_name": "Beta Service",
                        "description": null,
                        "node_id": null,
                        "operations": [
                            {
                                "endpoint_id": "00000000-0000-4000-8000-000000000403",
                                "name": "beta_status",
                                "method": "GET",
                                "path": "/status",
                                "parameters": null,
                                "request_body_schema": null,
                                "request_content_type": null,
                                "request_body_required": false,
                                "response": {"content_types": ["application/json"], "binary_artifact": false},
                                "risk": null,
                                "supports_idempotency_key": false,
                                "endpoint_contract_digest": "sha256:3f488040e19cb9fdc9e9f2aefd9061058c16426a58a2ace0664373e5b9574d5f",
                                "operation_generation": 1
                            }
                        ]
                    }
                ],
                "total_services": 2,
                "total_operations": 3
            })
        );

        let write_commands: Vec<_> = commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| {
                matches!(
                    command.as_str(),
                    "insert" | "update" | "delete" | "findAndModify" | "bulkWrite"
                )
            })
            .cloned()
            .collect();
        assert!(
            write_commands.is_empty(),
            "catalog handler issued database writes: {write_commands:?}"
        );
        // This is a read facade over the domain collections. OpenAPI loading
        // may still perform outbound reads and mutate the process-local
        // api_docs_service cache; this assertion deliberately checks only
        // durable domain writes and does not spy away that qualification.
    }

    fn oauth_client(id: &str, secret: &str, delegation_scopes: &str) -> OauthClient {
        let now = Utc::now();
        OauthClient {
            id: id.to_string(),
            client_name: id.to_string(),
            client_secret_hash: hash_token(secret),
            redirect_uris: vec!["https://example.invalid/callback".to_string()],
            allowed_scopes: "openid".to_string(),
            scope_provenance: ScopeProvenance::Explicit,
            grant_types: "authorization_code refresh_token".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: delegation_scopes.to_string(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn delegated_auth_from_token(state: &crate::AppState, token: &str) -> AuthUser {
        let claims = jwt::verify_token(&state.jwt_keys, &state.config, token)
            .expect("verify minted delegated token");
        AuthUser {
            user_id: Uuid::parse_str(&claims.sub).expect("delegated subject is a UUID"),
            session_id: None,
            scope: claims.scope,
            acting_client_id: claims.act.map(|actor| actor.sub),
            oauth_client_id: claims.client_id,
            token_jti: Some(claims.jti),
            approval_owner_user_id: None,
            auth_method: AuthMethod::Delegated,
            allow_all_services: claims.allow_all_services.unwrap_or(true),
            allow_all_nodes: claims.allow_all_nodes.unwrap_or(true),
            allowed_service_ids: claims.allowed_service_ids.unwrap_or_default(),
            resource_uris: claims.resources,
            allowed_node_ids: claims.allowed_node_ids.unwrap_or_default(),
            api_key_id: None,
            api_key_name: None,
            api_key_purpose: crate::models::api_key::ApiKeyPurpose::General,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    fn exact_fence(
        created: &crate::services::exact_service_approval_service::ExactServiceApprovalResult,
    ) -> crate::services::exact_service_approval_service::ExactServiceApprovalFence {
        crate::services::exact_service_approval_service::ExactServiceApprovalFence {
            catalog_digest: created.catalog_digest.clone(),
            exact_view_digest: created.exact_view_digest.clone(),
            operation_digest: created.operation_digest.clone(),
            operation_id: created.operation_id.clone(),
            operation_generation: created.operation_generation,
            idempotency_key: created.idempotency_key.clone(),
        }
    }

    async fn redeem_exact(
        state: &crate::AppState,
        auth: &AuthUser,
        created: &crate::services::exact_service_approval_service::ExactServiceApprovalResult,
        fence: crate::services::exact_service_approval_service::ExactServiceApprovalFence,
    ) -> AppResult<
        AxumJson<crate::services::exact_service_approval_service::ExactServiceApprovalResult>,
    > {
        exact_service_approvals::redeem_request(
            State(state.clone()),
            Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::route_inventory::BillingIngress::Mcp,
                ),
            ),
            auth.clone(),
            Path(created.request_id.clone()),
            AxumJson(fence),
        )
        .await
    }

    /// Crosses the production token-exchange grant path and the actual
    /// delegated discovery, exact-create, approval-decision, and exact-redeem
    /// handler boundaries. Every catalog/exact fence sent to create/redeem is
    /// copied from the discovery/create response; only the argument-bound
    /// operation digest is obtained from the canonical operation helper because
    /// discovery intentionally publishes contract metadata, not an
    /// argument-specific invocation digest.
    #[tokio::test]
    async fn delegated_discovery_create_redeem_uses_live_catalog_evidence() {
        let Some(db) =
            crate::test_utils::connect_test_database("delegated_discovery_exact_redeem").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let user_id = Uuid::parse_str(TEST_USER_ID).unwrap();
        let actor_client_id = "integration-actor-client";
        let receiver_client_id = "integration-receiver-client";
        let actor_secret = "integration-actor-secret";
        let receiver_secret = "integration-receiver-secret";
        let delegated_scope = format!(
            "{} proxy",
            catalog_delegation_service::MCP_CATALOG_READ_SCOPE
        );
        let requested_service_ids =
            vec![TEST_SERVICE_A.to_string(), TEST_GENERIC_SERVICE.to_string()];

        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(TEST_USER_ID, UserType::Person))
            .await
            .expect("insert integration user");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_many([
                oauth_client(actor_client_id, actor_secret, &delegated_scope),
                oauth_client(receiver_client_id, receiver_secret, &delegated_scope),
            ])
            .await
            .expect("insert integration OAuth clients");
        consent_service::grant_consent_with_services(
            &db,
            TEST_USER_ID,
            actor_client_id,
            "openid",
            Some(requested_service_ids.clone()),
        )
        .await
        .expect("grant actor consent");
        consent_service::grant_consent_with_services(
            &db,
            TEST_USER_ID,
            receiver_client_id,
            "openid",
            Some(requested_service_ids.clone()),
        )
        .await
        .expect("grant receiver consent");

        // The fixture seeds the catalog rows and a harmless fixture grant. The
        // live token exchange below mints its own JTI and is the only grant
        // used by the handler.
        let fixture_auth = fixture_catalog_auth();
        seed_operation_catalog_fixture(&db, &fixture_auth).await;
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000403" },
                mongodb::bson::doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("keep alternate provider operation set empty for identity-only drift");

        let provider_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local provider");
        let provider_addr = provider_listener.local_addr().unwrap();
        let provider = tokio::spawn(async move {
            axum::serve(
                provider_listener,
                Router::new().route(
                    "/items",
                    get(|| async { AxumJson(serde_json::json!({"ok": true})) }),
                ),
            )
            .await
            .expect("serve local provider");
        });
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_many(
                mongodb::bson::doc! { "_id": { "$in": [
                    "00000000-0000-4000-8000-000000000201",
                    "00000000-0000-4000-8000-000000000203",
                ] } },
                mongodb::bson::doc! { "$set": { "url": format!("http://{provider_addr}") } },
            )
            .await
            .expect("point typed and generic user endpoints at local provider");

        db.collection::<ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_many([
            ServiceApprovalConfig {
                id: Uuid::new_v4().to_string(),
                user_id: TEST_USER_ID.to_string(),
                service_id: "00000000-0000-4000-8000-000000000301".to_string(),
                service_name: "Alpha Service".to_string(),
                approval_required: true,
                approval_mode: ApprovalMode::PerRequest,
                rules: Vec::new(),
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            ServiceApprovalConfig {
                id: Uuid::new_v4().to_string(),
                user_id: TEST_USER_ID.to_string(),
                service_id: TEST_GENERIC_SERVICE.to_string(),
                service_name: "Hidden Generic".to_string(),
                approval_required: true,
                approval_mode: ApprovalMode::PerRequest,
                rules: Vec::new(),
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ])
        .await
        .expect("seed per-request approval config");

        let source_token = jwt::generate_oauth_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            "openid",
            None,
            None,
            None,
            None,
            Some(jwt::AccessTokenRestrictions {
                resources: &[],
                allowed_service_ids: &requested_service_ids,
                allowed_node_ids: &[],
                allow_all_nodes: true,
            }),
            actor_client_id,
        )
        .expect("mint direct source token");
        let exchanged = token_exchange_service::exchange_token_with_authority(
            &db,
            &state.config,
            &state.jwt_keys,
            receiver_client_id,
            receiver_secret,
            &source_token,
            "urn:ietf:params:oauth:token-type:access_token",
            Some(&delegated_scope),
            &[],
            Some(false),
            &requested_service_ids,
            Some(true),
            &[],
        )
        .await
        .expect("mint delegated token and persist live catalog grant");
        let delegated_auth = delegated_auth_from_token(&state, &exchanged.access_token);
        crate::services::catalog_delegation_service::validate_live_grant(
            &db,
            &state.config,
            &jwt::verify_token(&state.jwt_keys, &state.config, &exchanged.access_token).unwrap(),
        )
        .await
        .expect("delegated token has a live grant before discovery");

        let AxumJson(discovery) =
            get_operation_catalog(State(state.clone()), delegated_auth.clone())
                .await
                .expect("real delegated discovery handler");
        assert_eq!(
            discovery.contract_version,
            "nyxid-delegated-operation-catalog.v2"
        );
        let service = unique_fixture_service(&discovery.services);
        assert_eq!(service.user_service_id, TEST_SERVICE_A);
        let operation = fixture_operation(service);
        assert_eq!(
            service.catalog_service_id.as_deref(),
            Some("00000000-0000-4000-8000-000000000301")
        );
        assert!(
            discovery
                .services
                .iter()
                .all(|service| service.user_service_id != TEST_GENERIC_SERVICE),
            "delegated discovery must keep the authorized generic target out of the exact view"
        );
        assert!(
            delegated_auth
                .allowed_service_ids
                .iter()
                .any(|service_id| service_id == TEST_GENERIC_SERVICE),
            "the membership test must not be confounded by the service allowlist"
        );

        let arguments = serde_json::json!({});
        let allowed_service_ids = requested_service_ids.clone();
        let live_catalog = mcp_service::load_operation_catalog(
            &db,
            state.node_ws_manager.as_ref(),
            TEST_USER_ID,
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Allowed(&allowed_service_ids),
        )
        .await
        .expect("resolve canonical operation for argument-bound digest");
        let live_endpoint = live_catalog
            .services
            .iter()
            .find(|service| service.service_id == TEST_SERVICE_A)
            .and_then(|service| {
                service
                    .endpoints
                    .iter()
                    .find(|endpoint| endpoint.endpoint_id == operation.endpoint_id)
            })
            .expect("operation remains in canonical catalog");
        let operation_digest =
            mcp_service::exact_operation_digest(TEST_SERVICE_A, live_endpoint, &arguments);

        let generic_service = live_catalog
            .services
            .iter()
            .find(|service| service.service_id == TEST_GENERIC_SERVICE)
            .expect("authorized generic service remains in the unprojected catalog");
        let generic_endpoint = generic_service
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == mcp_service::GENERIC_PROXY_ENDPOINT_ID)
            .expect("generic proxy endpoint remains a valid unprojected selector");
        let generic_arguments = serde_json::json!({"method": "GET", "path": "/items"});
        let generic_operation_digest = mcp_service::exact_operation_digest(
            TEST_GENERIC_SERVICE,
            generic_endpoint,
            &generic_arguments,
        );
        let generic_create = |exact_view_digest: Option<String>, idempotency_key: &str| {
            crate::services::exact_service_approval_service::ExactServiceApprovalCreate {
                user_service_id: TEST_GENERIC_SERVICE.to_string(),
                endpoint_id: generic_endpoint.endpoint_id.clone(),
                catalog_digest: discovery.catalog_digest.clone(),
                exact_view_digest,
                endpoint_contract_digest: mcp_service::endpoint_contract_digest(generic_endpoint),
                operation_digest: generic_operation_digest.clone(),
                operation_id: generic_endpoint.endpoint_id.clone(),
                operation_generation: Some(mcp_service::GENERIC_PROXY_OPERATION_GENERATION),
                idempotency_key: idempotency_key.to_string(),
                arguments: generic_arguments.clone(),
            }
        };

        let approval_requests = db.collection::<crate::models::approval_request::ApprovalRequest>(
            crate::models::approval_request::COLLECTION_NAME,
        );
        let rows_before = approval_requests
            .count_documents(mongodb::bson::doc! {})
            .await
            .expect("count approvals before delegated hidden-target create");
        let row_1_error = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(generic_create(None, "matrix-row-1-delegated-hidden")),
        )
        .await
        .expect_err("delegated generic target must be outside the exact view");
        assert!(matches!(
            row_1_error,
            AppError::NotFound(message) if message == "exact_operation_not_in_exact_view"
        ));
        assert_eq!(
            approval_requests
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count approvals after delegated hidden-target create"),
            rows_before,
            "matrix row 1 must reject before creating an ApprovalRequest"
        );

        let mut access_token_auth = delegated_auth.clone();
        access_token_auth.auth_method = AuthMethod::AccessToken;
        access_token_auth.scope = "proxy".to_string();
        access_token_auth.acting_client_id = None;
        access_token_auth.token_jti = None;
        let AxumJson(row_3_created) = exact_service_approvals::create_request(
            State(state.clone()),
            access_token_auth.clone(),
            AxumJson(generic_create(None, "matrix-row-3-access-token-generic")),
        )
        .await
        .expect("non-delegated generic target remains approvable");
        assert_eq!(row_3_created.exact_view_digest, None);
        assert_eq!(
            row_3_created.operation_generation,
            mcp_service::GENERIC_PROXY_OPERATION_GENERATION
        );
        let persisted_row_3 = approval_requests
            .find_one(mongodb::bson::doc! { "_id": &row_3_created.request_id })
            .await
            .expect("load persisted generic approval")
            .expect("generic approval row was persisted");
        assert_eq!(
            persisted_row_3
                .exact_service
                .and_then(|binding| binding.exact_view_digest),
            None,
            "generic approval must not persist an unrelated exact-view fence"
        );
        approval_service::process_decision(
            &db,
            &state.config,
            &state.http_client,
            state.fcm_auth.clone(),
            state.apns_auth.clone(),
            &row_3_created.request_id,
            true,
            None,
            None,
            "integration",
        )
        .await
        .expect("approve non-delegated generic request");
        let AxumJson(row_3_redeemed) = redeem_exact(
            &state,
            &access_token_auth,
            &row_3_created,
            exact_fence(&row_3_created),
        )
        .await
        .expect("non-delegated generic request redeems");
        assert_eq!(
            row_3_redeemed.state,
            crate::services::exact_service_approval_service::ExactServiceApprovalState::Redeemed
        );
        assert_eq!(
            row_3_redeemed
                .receipt
                .as_ref()
                .map(|receipt| receipt.http_status),
            Some(200)
        );

        let row_4_error = exact_service_approvals::create_request(
            State(state.clone()),
            access_token_auth,
            AxumJson(generic_create(
                Some(discovery.exact_view_digest.clone()),
                "matrix-row-4-access-token-generic-with-view",
            )),
        )
        .await
        .expect_err("an out-of-view target cannot bind the exact-view digest");
        assert!(matches!(
            row_4_error,
            AppError::BadRequest(message) if message == "exact_view_digest_not_applicable"
        ));

        let rows_before_omitted_create = approval_requests
            .count_documents(mongodb::bson::doc! {})
            .await
            .expect("count approvals before delegated omission");
        let omitted_error = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(
                crate::services::exact_service_approval_service::ExactServiceApprovalCreate {
                    user_service_id: TEST_SERVICE_A.to_string(),
                    endpoint_id: operation.endpoint_id.clone(),
                    catalog_digest: discovery.catalog_digest.clone(),
                    exact_view_digest: None,
                    endpoint_contract_digest: operation.endpoint_contract_digest.clone(),
                    operation_digest: operation_digest.clone(),
                    operation_id: operation.endpoint_id.clone(),
                    operation_generation: operation.operation_generation,
                    idempotency_key: "integration-idempotency-key".to_string(),
                    arguments: arguments.clone(),
                },
            ),
        )
        .await
        .expect_err("delegated exact create requires the caller exact-view digest");
        assert!(matches!(
            omitted_error,
            AppError::BadRequest(message)
                if message == crate::services::exact_service_approval_service::EXACT_VIEW_DIGEST_REQUIRED
        ));
        assert_eq!(
            approval_requests
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count approvals after delegated omission"),
            rows_before_omitted_create,
            "delegated digest omission must be rejected before persistence"
        );

        let generation_input = |operation_generation: Option<i64>, idempotency_key: &str| {
            crate::services::exact_service_approval_service::ExactServiceApprovalCreate {
                user_service_id: TEST_SERVICE_A.to_string(),
                endpoint_id: operation.endpoint_id.clone(),
                catalog_digest: discovery.catalog_digest.clone(),
                exact_view_digest: Some(discovery.exact_view_digest.clone()),
                endpoint_contract_digest: operation.endpoint_contract_digest.clone(),
                operation_digest: operation_digest.clone(),
                operation_id: operation.endpoint_id.clone(),
                operation_generation,
                idempotency_key: idempotency_key.to_string(),
                arguments: arguments.clone(),
            }
        };
        let AxumJson(missing_generation) = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(generation_input(None, "integration-generation-missing")),
        )
        .await
        .expect("delegated create may omit the advisory generation");
        assert_eq!(
            Some(missing_generation.operation_generation),
            operation.operation_generation
        );
        let AxumJson(mismatched_generation) = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(generation_input(
                Some(operation.operation_generation.unwrap() + 1),
                "integration-generation-mismatch",
            )),
        )
        .await
        .expect("caller generation mismatch is advisory at create");
        assert_eq!(
            Some(mismatched_generation.operation_generation),
            operation.operation_generation,
            "NyxID persists the producer generation, never the caller value"
        );
        let current_exact_view_digest =
            mcp_service::exact_operation_view_digest(&mcp_service::ExactOperationView {
                services: discovery.services.clone(),
            });
        assert_ne!(current_exact_view_digest, discovery.exact_view_digest);
        let mut current_digest_input =
            generation_input(None, "integration-current-exact-view-digest");
        current_digest_input.exact_view_digest = Some(current_exact_view_digest);
        let AxumJson(current_digest_created) = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(current_digest_input),
        )
        .await
        .expect("create accepts the additive v2 digest during rolling compatibility");
        assert_eq!(
            current_digest_created.exact_view_digest.as_deref(),
            Some(discovery.exact_view_digest.as_str()),
            "new rows retain the pre-additive v2 slot for old replicas"
        );
        assert_eq!(
            approval_requests
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count approvals after advisory generation creates"),
            rows_before_omitted_create + 3,
            "advisory generations and both v2 digest projections create approvals"
        );

        let AxumJson(created) = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(
                crate::services::exact_service_approval_service::ExactServiceApprovalCreate {
                    user_service_id: TEST_SERVICE_A.to_string(),
                    endpoint_id: operation.endpoint_id.clone(),
                    catalog_digest: discovery.catalog_digest.clone(),
                    exact_view_digest: Some(discovery.exact_view_digest.clone()),
                    endpoint_contract_digest: operation.endpoint_contract_digest.clone(),
                    operation_digest: operation_digest.clone(),
                    operation_id: operation.endpoint_id.clone(),
                    operation_generation: operation.operation_generation,
                    idempotency_key: "integration-idempotency-key".to_string(),
                    arguments: arguments.clone(),
                },
            ),
        )
        .await
        .expect("real exact create handler with delegated digest");
        assert_eq!(
            created.exact_view_digest,
            Some(discovery.exact_view_digest.clone())
        );
        assert_eq!(created.catalog_digest, discovery.catalog_digest);
        assert_eq!(
            Some(created.operation_generation),
            operation.operation_generation,
            "create must persist the producer generation published by discovery"
        );

        approval_service::process_decision(
            &db,
            &state.config,
            &state.http_client,
            state.fcm_auth.clone(),
            state.apns_auth.clone(),
            &created.request_id,
            true,
            None,
            None,
            "integration",
        )
        .await
        .expect("approve per-request exact request");

        let AxumJson(redeemed) = exact_service_approvals::redeem_request(
            State(state.clone()),
            Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::route_inventory::BillingIngress::Mcp,
                ),
            ),
            delegated_auth.clone(),
            Path(created.request_id.clone()),
            AxumJson(
                crate::services::exact_service_approval_service::ExactServiceApprovalFence {
                    catalog_digest: created.catalog_digest.clone(),
                    exact_view_digest: created.exact_view_digest.clone(),
                    operation_digest: created.operation_digest.clone(),
                    operation_id: created.operation_id.clone(),
                    operation_generation: created.operation_generation,
                    idempotency_key: created.idempotency_key.clone(),
                },
            ),
        )
        .await
        .expect("real exact redeem handler");
        assert_eq!(
            redeemed.state,
            crate::services::exact_service_approval_service::ExactServiceApprovalState::Redeemed
        );
        assert_eq!(
            redeemed.receipt.as_ref().map(|receipt| receipt.http_status),
            Some(200)
        );

        let AxumJson(redeem_omission) = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(
                crate::services::exact_service_approval_service::ExactServiceApprovalCreate {
                    user_service_id: TEST_SERVICE_A.to_string(),
                    endpoint_id: operation.endpoint_id.clone(),
                    catalog_digest: discovery.catalog_digest.clone(),
                    exact_view_digest: Some(discovery.exact_view_digest.clone()),
                    endpoint_contract_digest: operation.endpoint_contract_digest.clone(),
                    operation_digest: operation_digest.clone(),
                    operation_id: operation.endpoint_id.clone(),
                    operation_generation: operation.operation_generation,
                    idempotency_key: "integration-redeem-omission-key".to_string(),
                    arguments: arguments.clone(),
                },
            ),
        )
        .await
        .expect("create delegated request for redeem omission");
        approval_service::process_decision(
            &db,
            &state.config,
            &state.http_client,
            state.fcm_auth.clone(),
            state.apns_auth.clone(),
            &redeem_omission.request_id,
            true,
            None,
            None,
            "integration",
        )
        .await
        .expect("approve delegated request for redeem omission");
        let mut omitted_fence = exact_fence(&redeem_omission);
        omitted_fence.exact_view_digest = None;
        let redeem_omission_error =
            redeem_exact(&state, &delegated_auth, &redeem_omission, omitted_fence)
                .await
                .expect_err("delegated redeem must require the exact-view fence");
        assert!(matches!(
            redeem_omission_error,
            AppError::BadRequest(message)
                if message == crate::services::exact_service_approval_service::EXACT_VIEW_DIGEST_REQUIRED
        ));
        let persisted_redeem_omission = approval_requests
            .find_one(mongodb::bson::doc! { "_id": &redeem_omission.request_id })
            .await
            .expect("load redeem omission row")
            .expect("redeem omission row exists");
        assert!(
            persisted_redeem_omission
                .exact_service
                .and_then(|binding| binding.redemption)
                .is_none(),
            "digest omission must not claim redemption"
        );

        let AxumJson(provider_bound) = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(
                crate::services::exact_service_approval_service::ExactServiceApprovalCreate {
                    user_service_id: TEST_SERVICE_A.to_string(),
                    endpoint_id: operation.endpoint_id.clone(),
                    catalog_digest: discovery.catalog_digest.clone(),
                    exact_view_digest: Some(discovery.exact_view_digest.clone()),
                    endpoint_contract_digest: operation.endpoint_contract_digest.clone(),
                    operation_digest,
                    operation_id: operation.endpoint_id.clone(),
                    operation_generation: operation.operation_generation,
                    idempotency_key: "provider-binding-drift-key".to_string(),
                    arguments,
                },
            ),
        )
        .await
        .expect("create provider-bound exact request");
        approval_service::process_decision(
            &db,
            &state.config,
            &state.http_client,
            state.fcm_auth.clone(),
            state.apns_auth.clone(),
            &provider_bound.request_id,
            true,
            None,
            None,
            "integration",
        )
        .await
        .expect("approve provider-bound request");

        // Move the same endpoint contracts to the second catalog provider and
        // repoint the UserService. Endpoint identities and operation arguments
        // remain stable; the newly projected catalog_service_id is the direct
        // reason the exact-view fence changes.
        db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .update_many(
                mongodb::bson::doc! {
                    "service_id": "00000000-0000-4000-8000-000000000301"
                },
                mongodb::bson::doc! { "$set": {
                    "service_id": "00000000-0000-4000-8000-000000000302"
                } },
            )
            .await
            .expect("move endpoint contracts to alternate provider");
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": {
                    "catalog_service_id": "00000000-0000-4000-8000-000000000302"
                } },
            )
            .await
            .expect("repoint catalog provider binding");
        let rebound_catalog = mcp_service::load_operation_catalog(
            &db,
            state.node_ws_manager.as_ref(),
            TEST_USER_ID,
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Allowed(&allowed_service_ids),
        )
        .await
        .expect("resolve rebound provider catalog");
        assert_eq!(
            mcp_service::operation_catalog_digest(&rebound_catalog.services),
            discovery.catalog_digest,
            "provider identity is intentionally outside the shared /mcp/config fence"
        );
        assert_ne!(
            mcp_service::exact_operation_view_digest(&mcp_service::exact_operation_view(
                &rebound_catalog.services,
            )),
            discovery.exact_view_digest,
            "provider identity must move the exact-view fence"
        );

        let AxumJson(provider_drifted) = exact_service_approvals::redeem_request(
            State(state),
            Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::route_inventory::BillingIngress::Mcp,
                ),
            ),
            delegated_auth,
            Path(provider_bound.request_id.clone()),
            AxumJson(
                crate::services::exact_service_approval_service::ExactServiceApprovalFence {
                    catalog_digest: provider_bound.catalog_digest.clone(),
                    exact_view_digest: provider_bound.exact_view_digest.clone(),
                    operation_digest: provider_bound.operation_digest.clone(),
                    operation_id: provider_bound.operation_id.clone(),
                    operation_generation: provider_bound.operation_generation,
                    idempotency_key: provider_bound.idempotency_key.clone(),
                },
            ),
        )
        .await
        .expect("provider binding drift returns a typed fail-closed result");
        assert_eq!(
            provider_drifted.state,
            crate::services::exact_service_approval_service::ExactServiceApprovalState::Drifted
        );
        assert_eq!(
            provider_drifted.failure_code.as_deref(),
            Some("catalog_drift")
        );
        provider.abort();
    }

    #[tokio::test]
    async fn full_router_delegated_success_exactly_one_provider_call_and_ac6_matrix() {
        let mut fixture = setup_full_router_fixture("exact_router_success_ac6").await;
        let approvals = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            );
        let rows_before = approvals
            .count_documents(mongodb::bson::doc! {})
            .await
            .expect("count approvals before HTTP create rejection matrix");
        let wrong_digest = format!("sha256:{}", "0".repeat(64));
        for (name, digest, expected_status, expected_code, expected_message) in [
            (
                "omitted",
                None,
                StatusCode::BAD_REQUEST,
                1000,
                "Bad request: exact_view_digest_required",
            ),
            (
                "empty",
                Some(""),
                StatusCode::BAD_REQUEST,
                1000,
                "Bad request: exact_view_digest must not be empty when provided",
            ),
            (
                "noncanonical",
                Some("sha256:not-hex"),
                StatusCode::CONFLICT,
                1004,
                "Conflict: exact_service_exact_view_digest_drift",
            ),
            (
                "mismatch",
                Some(wrong_digest.as_str()),
                StatusCode::CONFLICT,
                1004,
                "Conflict: exact_service_exact_view_digest_drift",
            ),
        ] {
            refresh_full_router_delegation(&mut fixture).await;
            let (status, body) = full_router_json_request(
                &fixture.app,
                Method::POST,
                CREATE_PATH,
                &fixture.delegated_token,
                Some(full_router_create_body(
                    &fixture,
                    &format!("create-{name}"),
                    digest,
                )),
            )
            .await;
            assert_error_response(
                status,
                &body,
                expected_status,
                expected_code,
                expected_message,
            );
        }
        assert_eq!(
            approvals
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count approvals after HTTP create rejection matrix"),
            rows_before,
            "all delegated create fence failures must precede persistence"
        );
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);

        let idempotency_key = "router-success";
        refresh_full_router_delegation(&mut fixture).await;
        let (create_status, created) = full_router_json_request(
            &fixture.app,
            Method::POST,
            CREATE_PATH,
            &fixture.delegated_token,
            Some(full_router_create_body(
                &fixture,
                idempotency_key,
                Some(&fixture.exact_view_digest),
            )),
        )
        .await;
        assert_eq!(create_status, StatusCode::OK, "create failed: {created}");
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;

        for (name, digest, expected_status, expected_code, expected_message) in [
            (
                "omitted",
                None,
                StatusCode::BAD_REQUEST,
                1000,
                "Bad request: exact_view_digest_required",
            ),
            (
                "empty",
                Some(""),
                StatusCode::CONFLICT,
                1004,
                "Conflict: exact_service_redemption_conflict",
            ),
            (
                "noncanonical",
                Some("sha256:not-hex"),
                StatusCode::CONFLICT,
                1004,
                "Conflict: exact_service_redemption_conflict",
            ),
            (
                "mismatch",
                Some(wrong_digest.as_str()),
                StatusCode::CONFLICT,
                1004,
                "Conflict: exact_service_redemption_conflict",
            ),
        ] {
            refresh_full_router_delegation(&mut fixture).await;
            let path = format!("/api/v1/approvals/exact-service/requests/{request_id}/redeem");
            let (status, body) = full_router_json_request(
                &fixture.app,
                Method::POST,
                &path,
                &fixture.delegated_token,
                Some(fence_body_from_created(&created, digest)),
            )
            .await;
            assert_error_response(
                status,
                &body,
                expected_status,
                expected_code,
                expected_message,
            );
            assert_no_full_router_redemption(&fixture, &request_id).await;
            assert_eq!(
                fixture.provider_calls.load(Ordering::SeqCst),
                0,
                "redeem fence row {name} reached the provider"
            );
        }

        let status_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/status");
        let (status, observed) = full_router_json_request(
            &fixture.app,
            Method::GET,
            &status_path,
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "status failed: {observed}");
        assert_eq!(observed["state"], "approved");

        let redeem_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/redeem");
        refresh_full_router_delegation(&mut fixture).await;
        let (status, redeemed) = full_router_json_request(
            &fixture.app,
            Method::POST,
            &redeem_path,
            &fixture.delegated_token,
            Some(fence_body_from_created(
                &created,
                created["exact_view_digest"].as_str(),
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "redeem failed: {redeemed}");
        assert_eq!(redeemed["state"], "redeemed");
        assert_eq!(redeemed["receipt"]["http_status"], 200);
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            1,
            "the real-router success path must dispatch exactly one effect"
        );
        let persisted = approvals
            .find_one(mongodb::bson::doc! { "_id": &request_id })
            .await
            .expect("load completed approval")
            .expect("completed approval exists");
        assert_eq!(
            persisted
                .exact_service
                .and_then(|binding| binding.redemption)
                .map(|redemption| redemption.status),
            Some(crate::models::approval_request::ExactServiceRedemptionStatus::Completed)
        );
    }

    #[tokio::test]
    async fn full_router_grant_absent_revoked_and_expired_block_create_and_redeem() {
        let mut fixture = setup_full_router_fixture("exact_router_grant_matrix").await;
        let grants = fixture
            .db
            .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS);
        let approvals = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            );

        for grant_state in ["absent", "revoked", "expired"] {
            // Refresh before mutating: the phase must run against a live token
            // whose grant is the one we are about to invalidate.
            refresh_full_router_delegation(&mut fixture).await;
            match grant_state {
                "absent" => {
                    grants
                        .delete_one(mongodb::bson::doc! { "_id": &fixture.grant.id })
                        .await
                        .expect("delete live grant before create");
                }
                "revoked" => {
                    grants
                        .update_one(
                            mongodb::bson::doc! { "_id": &fixture.grant.id },
                            mongodb::bson::doc! { "$set": { "revoked": true } },
                        )
                        .await
                        .expect("revoke live grant before create");
                }
                "expired" => {
                    grants
                        .update_one(
                            mongodb::bson::doc! { "_id": &fixture.grant.id },
                            mongodb::bson::doc! { "$set": {
                                "expires_at": mongodb::bson::DateTime::from_chrono(
                                    Utc::now() - chrono::Duration::seconds(1)
                                )
                            } },
                        )
                        .await
                        .expect("expire live grant before create");
                }
                _ => unreachable!(),
            }
            let rows_before = approvals
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count approvals before inactive-grant create");
            let (status, body) = full_router_json_request(
                &fixture.app,
                Method::POST,
                CREATE_PATH,
                &fixture.delegated_token,
                Some(full_router_create_body(
                    &fixture,
                    &format!("grant-{grant_state}-create"),
                    Some(&fixture.exact_view_digest),
                )),
            )
            .await;
            assert_error_response(
                status,
                &body,
                StatusCode::UNAUTHORIZED,
                1001,
                "Unauthorized: Delegated catalog authority is invalid or inactive",
            );
            assert_eq!(
                approvals
                    .count_documents(mongodb::bson::doc! {})
                    .await
                    .expect("count approvals after inactive-grant create"),
                rows_before,
                "{grant_state} grant must reject before ApprovalRequest persistence"
            );
            grants
                .replace_one(
                    mongodb::bson::doc! { "_id": &fixture.grant.id },
                    &fixture.grant,
                )
                .with_options(
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await
                .expect("restore live grant after create rejection");

            // Second phase needs a live token again, and its own grant to
            // invalidate after approval.
            refresh_full_router_delegation(&mut fixture).await;
            let idempotency_key = format!("grant-{grant_state}-redeem");
            let created = create_full_router_request(&fixture, &idempotency_key).await;
            let request_id = created["request_id"]
                .as_str()
                .expect("create response request_id")
                .to_string();
            approve_full_router_request(&fixture, &request_id).await;
            match grant_state {
                "absent" => {
                    grants
                        .delete_one(mongodb::bson::doc! { "_id": &fixture.grant.id })
                        .await
                        .expect("delete live grant before redeem");
                }
                "revoked" => {
                    grants
                        .update_one(
                            mongodb::bson::doc! { "_id": &fixture.grant.id },
                            mongodb::bson::doc! { "$set": { "revoked": true } },
                        )
                        .await
                        .expect("revoke live grant before redeem");
                }
                "expired" => {
                    grants
                        .update_one(
                            mongodb::bson::doc! { "_id": &fixture.grant.id },
                            mongodb::bson::doc! { "$set": {
                                "expires_at": mongodb::bson::DateTime::from_chrono(
                                    Utc::now() - chrono::Duration::seconds(1)
                                )
                            } },
                        )
                        .await
                        .expect("expire live grant before redeem");
                }
                _ => unreachable!(),
            }
            let redeem_path =
                format!("/api/v1/approvals/exact-service/requests/{request_id}/redeem");
            let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
            let (status, body) = full_router_json_request(
                &fixture.app,
                Method::POST,
                &redeem_path,
                &fixture.delegated_token,
                Some(fence_body_from_created(
                    &created,
                    created["exact_view_digest"].as_str(),
                )),
            )
            .await;
            assert_error_response(
                status,
                &body,
                StatusCode::UNAUTHORIZED,
                1001,
                "Unauthorized: Delegated catalog authority is invalid or inactive",
            );
            assert_no_full_router_redemption(&fixture, &request_id).await;
            assert_eq!(
                fixture.provider_calls.load(Ordering::SeqCst),
                calls_before,
                "{grant_state} grant redeem must not reach the provider"
            );
            grants
                .replace_one(
                    mongodb::bson::doc! { "_id": &fixture.grant.id },
                    &fixture.grant,
                )
                .with_options(
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await
                .expect("restore live grant after redeem rejection");
        }
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            0,
            "every inactive-grant row must fail before downstream dispatch"
        );
    }

    #[tokio::test]
    async fn full_router_service_node_provider_operation_and_credential_mutations_fail_closed() {
        let mut fixture = setup_full_router_fixture("exact_router_drift_matrix").await;
        let user_services = fixture.db.collection::<UserService>(USER_SERVICES);
        let service_endpoints = fixture.db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS);

        let service_key = "service-deactivated";
        refresh_full_router_delegation(&mut fixture).await;
        let service_created = create_full_router_request(&fixture, service_key).await;
        let service_request = service_created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &service_request).await;
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("deactivate service between approve and redeem");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        let (status, body) = redeem_full_router_request(&fixture, &service_created).await;
        assert_error_response(
            status,
            &body,
            StatusCode::UNAUTHORIZED,
            1001,
            "Unauthorized: Delegated catalog authority is invalid or inactive",
        );
        assert_no_full_router_redemption(&fixture, &service_request).await;
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            calls_before,
            "service deactivation redeem must not reach the provider"
        );
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": { "is_active": true } },
            )
            .await
            .expect("restore service after deactivation row");

        let node_key = "node-binding-mutated";
        refresh_full_router_delegation(&mut fixture).await;
        let node_created = create_full_router_request(&fixture, node_key).await;
        let node_request = node_created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &node_request).await;
        // Bind onto the fixture-scoped node (the token's allowlist), not a
        // second node. An explicitly node-bound token excludes a service
        // whose `node_id` is outside the allowlist from the scoped catalog,
        // which fail-closes as `revoked` rather than `catalog_drift`. Binding
        // to TEST_NODE_ID keeps the service in scope so this row still pins
        // the exact-view digest change (`node_id: null` → the fixture node).
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": { "node_id": TEST_NODE_ID } },
            )
            .await
            .expect("mutate primary node binding");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        let (status, body) = redeem_full_router_request(&fixture, &node_created).await;
        assert_eq!(status, StatusCode::OK, "node mutation failed: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "catalog_drift");
        assert_full_router_terminal_redemption(
            &fixture,
            &node_request,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "catalog_drift",
        )
        .await;
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            calls_before,
            "node-binding mutation redeem must not reach the provider"
        );
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$unset": { "node_id": "" } },
            )
            .await
            .expect("restore direct service routing");

        let out_of_scope_node_key = "node-binding-out-of-scope";
        refresh_full_router_delegation(&mut fixture).await;
        let out_of_scope_node_created =
            create_full_router_request(&fixture, out_of_scope_node_key).await;
        let out_of_scope_node_request = out_of_scope_node_created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &out_of_scope_node_request).await;
        fixture
            .db
            .collection::<Node>(NODES)
            .insert_one(Node {
                id: TEST_OUT_OF_SCOPE_NODE_ID.to_string(),
                user_id: TEST_USER_ID.to_string(),
                name: "full-router-out-of-scope-node".to_string(),
                status: NodeStatus::Online,
                auth_token_hash: "out-of-scope-node-token-hash".to_string(),
                signing_secret_encrypted: None,
                signing_secret_hash: "out-of-scope-node-signing-hash".to_string(),
                last_heartbeat_at: Some(Utc::now()),
                connected_at: Some(Utc::now()),
                metadata: None,
                metrics: NodeMetrics::default(),
                connection_owner: None,
                is_active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("insert out-of-scope node for binding mutation");
        let (out_of_scope_node_tx, out_of_scope_node_rx) = tokio::sync::mpsc::channel(8);
        crate::test_utils::register_test_node_connection(
            &fixture.state,
            TEST_OUT_OF_SCOPE_NODE_ID,
            out_of_scope_node_tx,
        )
        .await;
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": { "node_id": TEST_OUT_OF_SCOPE_NODE_ID } },
            )
            .await
            .expect("bind service to node outside delegated allowlist");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        let (status, body) = redeem_full_router_request(&fixture, &out_of_scope_node_created).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "out-of-scope node mutation failed: {body}"
        );
        assert_eq!(body["state"], "revoked");
        assert_eq!(body["failure_code"], "selector_revoked");
        assert_full_router_terminal_redemption(
            &fixture,
            &out_of_scope_node_request,
            crate::models::approval_request::ExactServiceRedemptionStatus::Revoked,
            "selector_revoked",
        )
        .await;
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            calls_before,
            "out-of-scope node mutation redeem must not reach the provider"
        );
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$unset": { "node_id": "" } },
            )
            .await
            .expect("restore direct service routing after out-of-scope node row");
        fixture
            .state
            .node_ws_manager
            .unregister_connection(TEST_OUT_OF_SCOPE_NODE_ID);
        drop(out_of_scope_node_rx);

        let operation_key = "endpoint-contract-mutated";
        refresh_full_router_delegation(&mut fixture).await;
        let operation_created = create_full_router_request(&fixture, operation_key).await;
        let operation_request = operation_created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &operation_request).await;
        service_endpoints
            .update_one(
                mongodb::bson::doc! { "_id": &fixture.operation.endpoint_id },
                mongodb::bson::doc! { "$set": { "path": "/items-mutated" } },
            )
            .await
            .expect("mutate endpoint contract");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        let (status, body) = redeem_full_router_request(&fixture, &operation_created).await;
        assert_eq!(status, StatusCode::OK, "operation mutation failed: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "catalog_drift");
        assert_full_router_terminal_redemption(
            &fixture,
            &operation_request,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "catalog_drift",
        )
        .await;
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            calls_before,
            "endpoint-contract mutation redeem must not reach the provider"
        );
        service_endpoints
            .update_one(
                mongodb::bson::doc! { "_id": &fixture.operation.endpoint_id },
                mongodb::bson::doc! { "$set": { "path": "/items" } },
            )
            .await
            .expect("restore endpoint contract");

        let endpoint_key = "endpoint-deactivated";
        refresh_full_router_delegation(&mut fixture).await;
        let endpoint_created = create_full_router_request(&fixture, endpoint_key).await;
        let endpoint_request = endpoint_created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &endpoint_request).await;
        service_endpoints
            .update_one(
                mongodb::bson::doc! { "_id": &fixture.operation.endpoint_id },
                mongodb::bson::doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("deactivate selected endpoint");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        let (status, body) = redeem_full_router_request(&fixture, &endpoint_created).await;
        assert_eq!(status, StatusCode::OK, "endpoint mutation failed: {body}");
        assert_eq!(body["state"], "revoked");
        assert_eq!(body["failure_code"], "selector_revoked");
        assert_full_router_terminal_redemption(
            &fixture,
            &endpoint_request,
            crate::models::approval_request::ExactServiceRedemptionStatus::Revoked,
            "selector_revoked",
        )
        .await;
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            calls_before,
            "endpoint deactivation redeem must not reach the provider"
        );
        service_endpoints
            .update_one(
                mongodb::bson::doc! { "_id": &fixture.operation.endpoint_id },
                mongodb::bson::doc! { "$set": { "is_active": true } },
            )
            .await
            .expect("restore selected endpoint");

        let credential_id = "00000000-0000-4000-8000-000000000701";
        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(UserApiKey {
                id: credential_id.to_string(),
                user_id: TEST_USER_ID.to_string(),
                label: "full-router credential".to_string(),
                credential_type: "bearer".to_string(),
                credential_encrypted: Some(
                    fixture
                        .state
                        .encryption_keys
                        .encrypt(b"full-router-secret")
                        .await
                        .expect("encrypt full-router credential"),
                ),
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_attempt_nonce: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                credential_source: None,
                status: "active".to_string(),
                last_used_at: None,
                last_authorized_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                credential_epoch: 1,
            })
            .await
            .expect("insert active credential");
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": {
                    "api_key_id": credential_id,
                    "auth_method": "bearer",
                    "auth_key_name": "Authorization"
                } },
            )
            .await
            .expect("bind active credential");
        let credential_key = "credential-revoked";
        refresh_full_router_delegation(&mut fixture).await;
        let credential_created = create_full_router_request(&fixture, credential_key).await;
        let credential_request = credential_created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &credential_request).await;
        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                mongodb::bson::doc! { "_id": credential_id },
                mongodb::bson::doc! { "$set": { "status": "revoked" } },
            )
            .await
            .expect("revoke bound credential");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        let (status, body) = redeem_full_router_request(&fixture, &credential_created).await;
        assert_eq!(status, StatusCode::OK, "credential mutation failed: {body}");
        assert_eq!(body["state"], "revoked");
        assert_eq!(body["failure_code"], "selector_revoked");
        assert_full_router_terminal_redemption(
            &fixture,
            &credential_request,
            crate::models::approval_request::ExactServiceRedemptionStatus::Revoked,
            "selector_revoked",
        )
        .await;
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            calls_before,
            "credential revocation redeem must not reach the provider"
        );

        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                mongodb::bson::doc! { "_id": credential_id },
                mongodb::bson::doc! { "$set": { "status": "active" } },
            )
            .await
            .expect("restore credential before provider row");
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! {
                    "$unset": { "api_key_id": "" },
                    "$set": { "auth_method": "none", "auth_key_name": "" }
                },
            )
            .await
            .expect("restore no-auth service before provider row");

        let provider_key = "provider-rebound";
        refresh_full_router_delegation(&mut fixture).await;
        let provider_created = create_full_router_request(&fixture, provider_key).await;
        let provider_request = provider_created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &provider_request).await;
        service_endpoints
            .update_many(
                mongodb::bson::doc! {
                    "service_id": "00000000-0000-4000-8000-000000000301"
                },
                mongodb::bson::doc! { "$set": {
                    "service_id": "00000000-0000-4000-8000-000000000302"
                } },
            )
            .await
            .expect("move endpoint contracts to alternate provider");
        user_services
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": {
                    "catalog_service_id": "00000000-0000-4000-8000-000000000302"
                } },
            )
            .await
            .expect("rebind service to alternate provider");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        let (status, body) = redeem_full_router_request(&fixture, &provider_created).await;
        assert_eq!(status, StatusCode::OK, "provider mutation failed: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "catalog_drift");
        assert_full_router_terminal_redemption(
            &fixture,
            &provider_request,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "catalog_drift",
        )
        .await;
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            calls_before,
            "provider rebind redeem must not reach the provider"
        );

        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            0,
            "every full-router mutation row must fail before downstream dispatch"
        );
    }

    #[tokio::test]
    async fn full_router_producer_generation_only_drift_is_terminal_before_effect() {
        let mut fixture = setup_full_router_fixture("exact_router_generation_only_drift").await;
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "generation-only-drift").await;
        let request_id = created["request_id"].as_str().unwrap().to_string();
        approve_full_router_request(&fixture, &request_id).await;

        fixture
            .db
            .collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": &fixture.operation.endpoint_id },
                mongodb::bson::doc! { "$inc": { "operation_generation": 1 } },
            )
            .await
            .expect("advance only the producer generation");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "generation drift redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "catalog_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "catalog_drift",
        )
        .await;
    }

    #[tokio::test]
    async fn full_router_old_server_policy_update_drifts_without_generation_bump() {
        let mut fixture = setup_full_router_fixture("exact_router_rolling_policy_drift").await;
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "risk-generation-drift").await;
        let request_id = created["request_id"].as_str().unwrap().to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let before = fixture
            .db
            .collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .find_one(mongodb::bson::doc! { "_id": &fixture.operation.endpoint_id })
            .await
            .expect("load producer endpoint")
            .expect("producer endpoint exists");
        // Simulate the pre-generation endpoint writer that remains live during
        // a rolling deployment: it updates producer policy with `$set` only.
        fixture
            .db
            .collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": &before.id },
                mongodb::bson::doc! { "$set": {
                    "risk": "write",
                    "supports_idempotency_key": true,
                } },
            )
            .await
            .expect("publish old-server producer-policy update");
        let after = fixture
            .db
            .collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
            .find_one(mongodb::bson::doc! { "_id": &fixture.operation.endpoint_id })
            .await
            .expect("reload producer endpoint")
            .expect("producer endpoint still exists");
        assert_eq!(after.path, before.path);
        assert_ne!(after.risk, before.risk);
        assert_ne!(
            after.supports_idempotency_key,
            before.supports_idempotency_key
        );
        assert_eq!(
            after.operation_generation, before.operation_generation,
            "the regression must model an old writer that cannot bump generation"
        );

        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let status_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/status");
        let (status, observed) = full_router_json_request(
            &fixture.app,
            Method::GET,
            &status_path,
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "policy drift observe: {observed}");
        assert_eq!(observed["state"], "drifted");
        assert_eq!(observed["failure_code"], "catalog_drift");
        assert_no_full_router_redemption(&fixture, &request_id).await;
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);

        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "risk drift redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "catalog_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "catalog_drift",
        )
        .await;
    }

    #[tokio::test]
    async fn full_router_catalog_owned_execution_fields_are_live_authority() {
        for (name, update, expected_failure) in [
            (
                "category",
                mongodb::bson::doc! { "$set": { "service_category": "internal" } },
                "execution_authority_drift",
            ),
            (
                "credential-requirement",
                mongodb::bson::doc! { "$set": { "requires_user_credential": false } },
                "execution_authority_drift",
            ),
        ] {
            let mut fixture =
                setup_full_router_fixture(&format!("exact_router_catalog_authority_{name}")).await;
            refresh_full_router_delegation(&mut fixture).await;
            let created = create_full_router_request(&fixture, &format!("catalog-{name}")).await;
            let request_id = created["request_id"].as_str().unwrap().to_string();
            approve_full_router_request(&fixture, &request_id).await;

            fixture
                .db
                .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .update_one(
                    mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000301" },
                    update,
                )
                .await
                .unwrap_or_else(|error| panic!("mutate catalog-owned {name}: {error}"));
            let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
            refresh_full_router_delegation(&mut fixture).await;
            let (status, body) = redeem_full_router_request(&fixture, &created).await;
            assert_eq!(status, StatusCode::OK, "catalog {name} redeem: {body}");
            assert_eq!(body["state"], "drifted", "{name}");
            assert_eq!(body["failure_code"], expected_failure, "{name}");
            assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
            assert_full_router_terminal_redemption(
                &fixture,
                &request_id,
                crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
                expected_failure,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn full_router_discovery_response_alone_is_sufficient_client_evidence() {
        let mut fixture = setup_full_router_fixture("exact_router_ac4_client_evidence").await;
        refresh_full_router_delegation(&mut fixture).await;

        let (status, discovery) = full_router_json_request(
            &fixture.app,
            Method::GET,
            "/api/v1/delegation/operation-catalog",
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "discovery failed: {discovery}");
        assert_eq!(
            discovery["contract_version"],
            "nyxid-delegated-operation-catalog.v2"
        );
        let services: Vec<mcp_service::ExactOperationViewService> =
            serde_json::from_value(discovery["services"].clone())
                .expect("decode discovery services from HTTP body");
        let service = unique_fixture_service(&services);
        assert_eq!(service.user_service_id, TEST_SERVICE_A);
        assert!(service.node_id.is_none());
        let operation = fixture_operation(service);
        let arguments = serde_json::json!({});
        let operation_digest = mcp_service::exact_operation_digest_from_parts(
            &service.user_service_id,
            &operation.endpoint_id,
            &operation.endpoint_contract_digest,
            &arguments,
        );

        let idempotency_key = "ac4-client-evidence";
        let (create_status, created) = full_router_json_request(
            &fixture.app,
            Method::POST,
            CREATE_PATH,
            &fixture.delegated_token,
            Some(serde_json::json!({
                "user_service_id": &service.user_service_id,
                "endpoint_id": operation.endpoint_id,
                "catalog_digest": discovery["catalog_digest"],
                "exact_view_digest": discovery["exact_view_digest"],
                "endpoint_contract_digest": operation.endpoint_contract_digest,
                "operation_digest": operation_digest,
                "operation_id": operation.endpoint_id,
                "operation_generation": operation.operation_generation,
                "idempotency_key": idempotency_key,
                "arguments": arguments,
            })),
        )
        .await;
        assert_eq!(create_status, StatusCode::OK, "create failed: {created}");
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;

        refresh_full_router_delegation(&mut fixture).await;
        let (status, redeemed) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "redeem failed: {redeemed}");
        assert_eq!(redeemed["state"], "redeemed");
        assert_eq!(redeemed["receipt"]["http_status"], 200);
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            1,
            "response-derived discovery → create → redeem must dispatch exactly one effect"
        );
    }

    #[tokio::test]
    async fn full_router_endpoint_url_mutation_fails_closed_with_zero_provider_effect() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_url_mutation").await;
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-url-mutation").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let diverted_calls = Arc::new(AtomicUsize::new(0));
        let diverted_route = diverted_calls.clone();
        let diverted_fallback = diverted_calls.clone();
        let diverted_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind diverted provider spy");
        let diverted_addr = diverted_listener.local_addr().unwrap();
        let diverted_provider = tokio::spawn(async move {
            axum::serve(
                diverted_listener,
                Router::new()
                    .route(
                        "/items",
                        get(move || {
                            let diverted_route = diverted_route.clone();
                            async move {
                                diverted_route.fetch_add(1, Ordering::SeqCst);
                                AxumJson(serde_json::json!({"ok": true}))
                            }
                        }),
                    )
                    .fallback(any(move || {
                        let diverted_fallback = diverted_fallback.clone();
                        async move {
                            diverted_fallback.fetch_add(1, Ordering::SeqCst);
                            (
                                StatusCode::NOT_FOUND,
                                AxumJson(serde_json::json!({"unexpected": true})),
                            )
                        }
                    })),
            )
            .await
            .expect("serve diverted provider spy");
        });
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000201" },
                mongodb::bson::doc! { "$set": { "url": format!("http://{diverted_addr}") } },
            )
            .await
            .expect("repoint endpoint at diverted spy");

        let original_before = fixture.provider_calls.load(Ordering::SeqCst);
        let diverted_before = diverted_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        diverted_provider.abort();
        assert_eq!(status, StatusCode::OK, "redeem failed: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "execution_authority_drift");
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            original_before,
            "original spy must not receive the diverted effect"
        );
        assert_eq!(
            diverted_calls.load(Ordering::SeqCst),
            diverted_before,
            "diverted spy must not receive the unapproved effect"
        );
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        )
        .await;
    }

    async fn insert_bearer_credential(
        fixture: &FullRouterFixture,
        credential_id: &str,
        secret: &[u8],
        epoch: i64,
    ) {
        let encrypted = fixture
            .state
            .encryption_keys
            .encrypt(secret)
            .await
            .expect("encrypt test credential");
        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(UserApiKey {
                id: credential_id.to_string(),
                user_id: TEST_USER_ID.to_string(),
                label: "ac5 credential".to_string(),
                credential_type: "bearer".to_string(),
                credential_encrypted: Some(encrypted),
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_attempt_nonce: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                credential_source: None,
                status: "active".to_string(),
                last_used_at: None,
                last_authorized_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                credential_epoch: epoch,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("insert test credential");
    }

    async fn bind_service_credential(fixture: &FullRouterFixture, credential_id: &str) {
        fixture
            .db
            .collection::<UserService>(USER_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": {
                    "api_key_id": credential_id,
                    "auth_method": "bearer",
                    "auth_key_name": "Authorization"
                } },
            )
            .await
            .expect("bind test credential");
    }

    #[tokio::test]
    async fn full_router_credential_rebind_and_inplace_rotation_fail_closed() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_cred").await;
        insert_bearer_credential(
            &fixture,
            "00000000-0000-4000-8000-000000000711",
            b"secret-a",
            1,
        )
        .await;
        insert_bearer_credential(
            &fixture,
            "00000000-0000-4000-8000-000000000712",
            b"secret-b",
            1,
        )
        .await;
        bind_service_credential(&fixture, "00000000-0000-4000-8000-000000000711").await;

        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-cred-rebind").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;

        bind_service_credential(&fixture, "00000000-0000-4000-8000-000000000712").await;
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "rebind redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        )
        .await;

        bind_service_credential(&fixture, "00000000-0000-4000-8000-000000000711").await;
        let rotated = fixture
            .state
            .encryption_keys
            .encrypt(b"secret-a-rotated")
            .await
            .expect("encrypt rotated credential");
        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000711" },
                mongodb::bson::doc! {
                    "$set": { "credential_encrypted": mongodb::bson::Binary {
                        subtype: mongodb::bson::spec::BinarySubtype::Generic,
                        bytes: rotated,
                    } },
                    "$inc": { "credential_epoch": 1 }
                },
            )
            .await
            .expect("rotate credential in place");
        let created = create_full_router_request(&fixture, "ac5-cred-rotate").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;
        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000711" },
                mongodb::bson::doc! {
                    "$set": { "credential_encrypted": mongodb::bson::Binary {
                        subtype: mongodb::bson::spec::BinarySubtype::Generic,
                        bytes: fixture
                            .state
                            .encryption_keys
                            .encrypt(b"secret-a-rotated-again")
                            .await
                            .expect("encrypt second rotation"),
                    } },
                    "$inc": { "credential_epoch": 1 }
                },
            )
            .await
            .expect("second in-place rotation");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "rotation redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        )
        .await;
    }

    #[tokio::test]
    async fn full_router_background_token_refresh_does_not_drift() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_refresh").await;
        let access = fixture
            .state
            .encryption_keys
            .encrypt(b"oauth-access")
            .await
            .expect("encrypt access token");
        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(UserApiKey {
                id: "00000000-0000-4000-8000-000000000713".to_string(),
                user_id: TEST_USER_ID.to_string(),
                label: "oauth refresh canary".to_string(),
                credential_type: "oauth2".to_string(),
                credential_encrypted: None,
                access_token_encrypted: Some(access),
                refresh_token_encrypted: None,
                token_scopes: Some("openid".to_string()),
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_attempt_nonce: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                credential_source: None,
                status: "active".to_string(),
                last_used_at: None,
                last_authorized_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                credential_epoch: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("insert oauth key");
        bind_service_credential(&fixture, "00000000-0000-4000-8000-000000000713").await;

        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-refresh-canary").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let refreshed = fixture
            .state
            .encryption_keys
            .encrypt(b"oauth-access-refreshed")
            .await
            .expect("encrypt refreshed access token");
        fixture
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                mongodb::bson::doc! { "_id": "00000000-0000-4000-8000-000000000713" },
                mongodb::bson::doc! { "$set": {
                    "access_token_encrypted": mongodb::bson::Binary {
                        subtype: mongodb::bson::spec::BinarySubtype::Generic,
                        bytes: refreshed,
                    },
                    "updated_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
                } },
            )
            .await
            .expect("simulate background token refresh");

        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "refresh redeem: {body}");
        assert_eq!(body["state"], "redeemed");
        assert_eq!(body["receipt"]["http_status"], 200);
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
        let persisted = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "_id": &request_id })
            .await
            .expect("load redeemed request")
            .expect("redeemed request exists");
        assert_eq!(
            persisted
                .exact_service
                .and_then(|binding| binding.redemption)
                .map(|redemption| redemption.status),
            Some(crate::models::approval_request::ExactServiceRedemptionStatus::Completed)
        );
    }

    #[tokio::test]
    async fn full_router_auth_and_injection_config_mutations_fail_closed() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_injection").await;
        insert_bearer_credential(
            &fixture,
            "00000000-0000-4000-8000-000000000714",
            b"injection-matrix-secret",
            1,
        )
        .await;
        bind_service_credential(&fixture, "00000000-0000-4000-8000-000000000714").await;
        let mutations: [(&str, mongodb::bson::Document, mongodb::bson::Document); 7] = [
            (
                "auth-method",
                mongodb::bson::doc! { "$set": { "auth_method": "none" } },
                mongodb::bson::doc! { "$set": { "auth_method": "bearer" } },
            ),
            (
                "auth-key-name",
                mongodb::bson::doc! { "$set": { "auth_key_name": "X-Api-Key" } },
                mongodb::bson::doc! { "$set": { "auth_key_name": "" } },
            ),
            (
                "inject-delegation",
                mongodb::bson::doc! { "$set": { "inject_delegation_token": true } },
                mongodb::bson::doc! { "$set": { "inject_delegation_token": false } },
            ),
            (
                "identity-mode",
                mongodb::bson::doc! { "$set": { "identity_propagation_mode": "headers" } },
                mongodb::bson::doc! { "$set": { "identity_propagation_mode": "none" } },
            ),
            (
                "delegation-scope",
                mongodb::bson::doc! { "$set": { "delegation_token_scope": "proxy" } },
                mongodb::bson::doc! { "$set": { "delegation_token_scope": "llm:proxy" } },
            ),
            (
                "forward-access-token",
                mongodb::bson::doc! { "$set": { "forward_access_token": true } },
                mongodb::bson::doc! { "$set": { "forward_access_token": false } },
            ),
            (
                "custom-user-agent",
                mongodb::bson::doc! { "$set": { "custom_user_agent": "NyxID-Test/1.0" } },
                mongodb::bson::doc! { "$unset": { "custom_user_agent": "" } },
            ),
        ];
        for (name, mutate, restore) in mutations {
            refresh_full_router_delegation(&mut fixture).await;
            let created = create_full_router_request(&fixture, &format!("ac5-inj-{name}")).await;
            let request_id = created["request_id"]
                .as_str()
                .expect("create response request_id")
                .to_string();
            approve_full_router_request(&fixture, &request_id).await;
            fixture
                .db
                .collection::<UserService>(USER_SERVICES)
                .update_one(mongodb::bson::doc! { "_id": TEST_SERVICE_A }, mutate)
                .await
                .unwrap_or_else(|_| panic!("mutate {name}"));
            let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
            refresh_full_router_delegation(&mut fixture).await;
            let (status, body) = redeem_full_router_request(&fixture, &created).await;
            assert_eq!(status, StatusCode::OK, "{name} redeem: {body}");
            assert_eq!(body["state"], "drifted", "{name}");
            assert_eq!(body["failure_code"], "execution_authority_drift", "{name}");
            assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
            assert_full_router_terminal_redemption(
                &fixture,
                &request_id,
                crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
                "execution_authority_drift",
            )
            .await;
            fixture
                .db
                .collection::<UserService>(USER_SERVICES)
                .update_one(mongodb::bson::doc! { "_id": TEST_SERVICE_A }, restore)
                .await
                .unwrap_or_else(|_| panic!("restore {name}"));
        }
    }

    #[tokio::test]
    async fn full_router_default_header_mutations_fail_closed() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_headers").await;
        let header = DefaultRequestHeader {
            name: "X-Test-Header".to_string(),
            value: "alpha".to_string(),
            overridable: false,
            sensitive: false,
        };

        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-user-header").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;
        fixture
            .db
            .collection::<UserService>(USER_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$set": {
                    "default_request_headers": [mongodb::bson::to_bson(&header).unwrap()]
                } },
            )
            .await
            .expect("set user-layer headers");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "user header redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        )
        .await;
        fixture
            .db
            .collection::<UserService>(USER_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_SERVICE_A },
                mongodb::bson::doc! { "$unset": { "default_request_headers": "" } },
            )
            .await
            .expect("clear user-layer headers");

        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-catalog-header").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;
        fixture
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_CATALOG_SERVICE_ID },
                mongodb::bson::doc! { "$set": {
                    "default_request_headers": [mongodb::bson::to_bson(&header).unwrap()]
                } },
            )
            .await
            .expect("set catalog-layer headers");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "catalog header redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        )
        .await;
    }

    #[tokio::test]
    async fn full_router_proxy_operation_policy_mutation_fails_closed() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_policy").await;
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-policy").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;
        let policy = ProxyOperationPolicy {
            rules: vec![ProxyOperationRule {
                method: "GET".to_string(),
                path_template: "/items".to_string(),
            }],
        };
        fixture
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_CATALOG_SERVICE_ID },
                mongodb::bson::doc! { "$set": {
                    "proxy_operation_policy": mongodb::bson::to_bson(&policy).unwrap()
                } },
            )
            .await
            .expect("set proxy operation policy");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "policy redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        )
        .await;
    }

    #[tokio::test]
    async fn full_router_configured_fallback_binding_mutation_fails_closed() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_fallback").await;
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-fallback").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;
        fixture
            .db
            .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(NodeServiceBinding {
                id: uuid::Uuid::new_v4().to_string(),
                node_id: TEST_NODE_ID.to_string(),
                user_id: TEST_USER_ID.to_string(),
                service_id: TEST_CATALOG_SERVICE_ID.to_string(),
                is_active: true,
                priority: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("insert configured fallback binding");
        let calls_before = fixture.provider_calls.load(Ordering::SeqCst);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "fallback redeem: {body}");
        assert_eq!(body["state"], "drifted");
        assert_eq!(body["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), calls_before);
        assert_full_router_terminal_redemption(
            &fixture,
            &request_id,
            crate::models::approval_request::ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        )
        .await;

        fixture
            .db
            .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .delete_many(mongodb::bson::doc! {
                "service_id": TEST_CATALOG_SERVICE_ID,
            })
            .await
            .expect("remove fallback binding");

        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-disconnect").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;
        fixture
            .state
            .node_ws_manager
            .unregister_connection(TEST_NODE_ID);
        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "disconnect redeem: {body}");
        assert_eq!(body["state"], "redeemed");
        assert_eq!(body["receipt"]["http_status"], 200);
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn full_router_aba_url_revert_redeems_exactly_once() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_aba").await;
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-aba").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let original_url = fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID })
            .await
            .expect("load endpoint")
            .expect("endpoint exists")
            .url;
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": "http://127.0.0.1:9" } },
            )
            .await
            .expect("move url away");
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": &original_url } },
            )
            .await
            .expect("revert url");

        refresh_full_router_delegation(&mut fixture).await;
        let (status, body) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "aba redeem: {body}");
        assert_eq!(body["state"], "redeemed");
        assert_eq!(body["receipt"]["http_status"], 200);
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn full_router_observe_reports_execution_authority_drift_before_redeem() {
        let mut fixture = setup_full_router_fixture("exact_router_ac5_observe").await;
        let credential_id = "00000000-0000-4000-8000-000000000719";
        insert_bearer_credential(&fixture, credential_id, b"observe-secret", 1).await;
        bind_service_credential(&fixture, credential_id).await;
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, "ac5-observe").await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;
        let original_url = fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID })
            .await
            .expect("load endpoint before observe drift")
            .expect("endpoint exists before observe drift")
            .url;
        let credential_before = fixture
            .db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .find_one(mongodb::bson::doc! { "_id": credential_id })
            .await
            .expect("load credential before observe")
            .expect("credential exists before observe");
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": "http://127.0.0.1:9" } },
            )
            .await
            .expect("mutate url before observe");
        refresh_full_router_delegation(&mut fixture).await;
        let status_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/status");
        let (status, observed) = full_router_json_request(
            &fixture.app,
            Method::GET,
            &status_path,
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "observe failed: {observed}");
        assert_eq!(observed["state"], "drifted");
        assert_eq!(observed["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
        assert_no_full_router_redemption(&fixture, &request_id).await;
        let credential_after_drift = fixture
            .db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .find_one(mongodb::bson::doc! { "_id": credential_id })
            .await
            .expect("load credential after drift observe")
            .expect("credential exists after drift observe");
        assert_eq!(
            credential_after_drift, credential_before,
            "observe must not decrypt/touch/refresh credential state"
        );

        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": original_url } },
            )
            .await
            .expect("restore endpoint after transient observe drift");
        let (status, recovered) = full_router_json_request(
            &fixture.app,
            Method::GET,
            &status_path,
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "recovered observe failed: {recovered}"
        );
        assert_eq!(recovered["state"], "approved");
        assert!(recovered.get("failure_code").is_none());
        assert_no_full_router_redemption(&fixture, &request_id).await;
        let credential_after_recovery = fixture
            .db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .find_one(mongodb::bson::doc! { "_id": credential_id })
            .await
            .expect("load credential after recovered observe")
            .expect("credential exists after recovered observe");
        assert_eq!(credential_after_recovery, credential_before);
    }

    #[tokio::test]
    async fn full_router_main_v1_authority_rows_roll_forward_and_detect_v1_drift() {
        let fixture = setup_full_router_fixture("exact_router_rolling_v1_authority").await;
        let created = create_full_router_request(&fixture, "rolling-v1-unchanged").await;
        let request_id = created["request_id"].as_str().unwrap().to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let resolution =
            crate::services::proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                &fixture.db,
                &fixture.state.encryption_keys,
                TEST_USER_ID,
                &fixture.user_service_id,
                Some("alpha"),
            )
            .await
            .expect("resolve live authority snapshot")
            .expect("fixture authority snapshot");
        let configured_fallbacks =
            crate::services::node_routing_service::list_configured_binding_node_ids(
                &fixture.db,
                TEST_USER_ID,
                &resolution.target.service.id,
            )
            .await
            .expect("resolve configured fallback set");
        let projection = crate::services::execution_authority::build_projection(
            &resolution,
            None,
            configured_fallbacks,
        );
        let expected_v1 = crate::services::execution_authority::legacy_digest(&projection);
        let persisted = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "_id": &request_id })
            .await
            .expect("load new v2 request")
            .expect("new v2 request exists");
        let binding = persisted.exact_service.expect("exact binding");
        assert_eq!(
            binding.execution_authority_digest.as_deref(),
            Some(expected_v1.as_str())
        );
        assert_eq!(
            binding
                .execution_authority_binding
                .as_ref()
                .map(|value| value.projection_version.as_str()),
            Some(crate::services::execution_authority::CONTRACT_VERSION),
        );

        reshape_full_router_request_as_main_v1(&fixture, &request_id).await;
        let status_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/status");
        let (status, observed) = full_router_json_request(
            &fixture.app,
            Method::GET,
            &status_path,
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "v1 observe failed: {observed}");
        assert_eq!(observed["state"], "approved");
        let (status, redeemed) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "v1 redeem failed: {redeemed}");
        assert_eq!(redeemed["state"], "redeemed");

        let drift_created = create_full_router_request(&fixture, "rolling-v1-drift").await;
        let drift_request_id = drift_created["request_id"].as_str().unwrap().to_string();
        approve_full_router_request(&fixture, &drift_request_id).await;
        reshape_full_router_request_as_main_v1(&fixture, &drift_request_id).await;
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": "http://127.0.0.1:9" } },
            )
            .await
            .expect("mutate v1-visible destination");
        let drift_status_path =
            format!("/api/v1/approvals/exact-service/requests/{drift_request_id}/status");
        let (status, observed) = full_router_json_request(
            &fixture.app,
            Method::GET,
            &drift_status_path,
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "v1 drift observe failed: {observed}"
        );
        assert_eq!(observed["state"], "drifted");
        assert_eq!(observed["failure_code"], "execution_authority_drift");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn full_router_closed_port_is_failed_provider_unreachable() {
        let fixture = setup_full_router_fixture("exact_router_provider_unreachable").await;
        let closed_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind closed-port fixture");
        let closed_address = closed_listener.local_addr().expect("closed-port address");
        drop(closed_listener);
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": format!("http://{closed_address}") } },
            )
            .await
            .expect("point fixture at closed port");
        let created = create_full_router_request(&fixture, "provider-unreachable").await;
        let request_id = created["request_id"].as_str().unwrap().to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let (status, result) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "closed-port redeem failed: {result}"
        );
        assert_eq!(result["state"], "failed");
        assert_eq!(result["failure_code"], "provider_unreachable");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn full_router_reset_after_request_sent_is_outcome_unknown() {
        let fixture = setup_full_router_fixture("exact_router_request_reset").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind reset provider");
        let address = listener.local_addr().expect("reset provider address");
        let provider = tokio::spawn(async move {
            use tokio::io::AsyncReadExt as _;

            let (mut stream, _) = listener.accept().await.expect("accept provider request");
            let mut received = Vec::new();
            loop {
                let mut buffer = [0_u8; 1024];
                let read = stream
                    .read(&mut buffer)
                    .await
                    .expect("read complete provider request");
                if read == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..read]);
                if received.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(received.starts_with(b"GET /items HTTP/1.1\r\n"));
            drop(stream);
        });
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": format!("http://{address}") } },
            )
            .await
            .expect("point fixture at reset provider");
        let created = create_full_router_request(&fixture, "request-reset").await;
        let request_id = created["request_id"].as_str().unwrap().to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let (status, result) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "reset redeem failed: {result}");
        assert_eq!(result["state"], "failed");
        assert_eq!(result["failure_code"], "provider_outcome_unknown");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
        provider.await.expect("reset provider task");
    }

    #[tokio::test]
    async fn full_router_truncated_provider_body_is_outcome_unknown() {
        let fixture = setup_full_router_fixture("exact_router_truncated_body").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind truncated-body provider");
        let address = listener.local_addr().expect("truncated-body address");
        let provider = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;

            let (mut stream, _) = listener.accept().await.expect("accept provider request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 64\r\nConnection: close\r\n\r\n{\"partial\":true}",
                )
                .await
                .expect("write truncated provider response");
            stream.shutdown().await.expect("close truncated response");
        });
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "url": format!("http://{address}") } },
            )
            .await
            .expect("point fixture at truncated-body provider");
        let created = create_full_router_request(&fixture, "truncated-body").await;
        let request_id = created["request_id"].as_str().unwrap().to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let (status, result) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "truncated-body redeem failed: {result}"
        );
        assert_eq!(result["state"], "failed");
        assert_eq!(result["failure_code"], "provider_outcome_unknown");
        provider.await.expect("truncated-body provider task");
    }

    #[tokio::test]
    async fn full_router_instance_spec_operation_is_discoverable_and_redeemable() {
        let fixture = setup_full_router_fixture("exact_router_instance_spec").await;
        let _cache_guard = crate::services::api_docs_service::SpecCacheTestGuard::acquire();
        const SPEC_URL: &str = "https://example.com/exact-instance-spec.json";
        crate::services::api_docs_service::cache_test_spec(
            SPEC_URL,
            Some(TEST_USER_ID),
            serde_json::json!({
                "openapi": "3.1.0",
                "info": { "title": "Exact instance", "version": "1.0.0" },
                "paths": {
                    "/items": {
                        "get": {
                            "operationId": FIXTURE_OPERATION_NAME,
                            "responses": {
                                "200": {
                                    "description": "ok",
                                    "content": { "application/json": {
                                        "schema": { "type": "object" }
                                    } }
                                }
                            }
                        }
                    }
                }
            }),
        );
        fixture
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .update_one(
                mongodb::bson::doc! { "_id": TEST_USER_ENDPOINT_ID },
                mongodb::bson::doc! { "$set": { "openapi_spec_url": SPEC_URL } },
            )
            .await
            .expect("mount instance spec");

        let (status, discovery) = full_router_json_request(
            &fixture.app,
            Method::GET,
            "/api/v1/delegation/operation-catalog",
            &fixture.delegated_token,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "instance discovery failed: {discovery}"
        );
        let services: Vec<mcp_service::ExactOperationViewService> =
            serde_json::from_value(discovery["services"].clone()).unwrap();
        let service = unique_fixture_service(&services);
        let operation = fixture_operation(service);
        assert_eq!(operation.operation_generation, None);
        let arguments = serde_json::json!({});
        let operation_digest = mcp_service::exact_operation_digest_from_parts(
            &service.user_service_id,
            &operation.endpoint_id,
            &operation.endpoint_contract_digest,
            &arguments,
        );
        let (status, created) = full_router_json_request(
            &fixture.app,
            Method::POST,
            CREATE_PATH,
            &fixture.delegated_token,
            Some(serde_json::json!({
                "user_service_id": service.user_service_id,
                "endpoint_id": operation.endpoint_id,
                "catalog_digest": discovery["catalog_digest"],
                "exact_view_digest": discovery["exact_view_digest"],
                "endpoint_contract_digest": operation.endpoint_contract_digest,
                "operation_digest": operation_digest,
                "operation_id": operation.endpoint_id,
                "idempotency_key": "instance-spec-operation",
                "arguments": arguments,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "instance create failed: {created}");
        assert_eq!(created["operation_generation"], 0);
        let request_id = created["request_id"].as_str().unwrap().to_string();
        let persisted = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "_id": &request_id })
            .await
            .expect("load instance approval")
            .expect("instance approval exists");
        assert!(!persisted.exact_service.unwrap().producer_generation_bound);
        approve_full_router_request(&fixture, &request_id).await;
        let (status, redeemed) = redeem_full_router_request(&fixture, &created).await;
        assert_eq!(status, StatusCode::OK, "instance redeem failed: {redeemed}");
        assert_eq!(redeemed["state"], "redeemed");
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn full_router_single_token_discovery_create_decide_status_redeem() {
        let fixture = setup_full_router_fixture("exact_router_ac4_single_token").await;
        let delegated_token = fixture.delegated_token.clone();
        let delegated_jti = delegated_auth_from_token(&fixture.state, &delegated_token)
            .token_jti
            .expect("single-token journey delegated JTI");
        assert_full_router_delegated_token_unchanged(
            &fixture,
            &delegated_token,
            &delegated_jti,
            "journey start",
        );

        let (status, discovery) = full_router_json_request(
            &fixture.app,
            Method::GET,
            "/api/v1/delegation/operation-catalog",
            &delegated_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "discovery failed: {discovery}");
        assert_eq!(
            discovery["contract_version"],
            "nyxid-delegated-operation-catalog.v2"
        );
        assert_full_router_delegated_token_unchanged(
            &fixture,
            &delegated_token,
            &delegated_jti,
            "discovery",
        );

        let services: Vec<mcp_service::ExactOperationViewService> =
            serde_json::from_value(discovery["services"].clone())
                .expect("decode discovery services from HTTP body");
        let service = unique_fixture_service(&services);
        assert_eq!(service.user_service_id, TEST_SERVICE_A);
        assert!(service.node_id.is_none());
        let operation = fixture_operation(service);
        let arguments = serde_json::json!({});
        let operation_digest = mcp_service::exact_operation_digest_from_parts(
            &service.user_service_id,
            &operation.endpoint_id,
            &operation.endpoint_contract_digest,
            &arguments,
        );
        let (status, created) = full_router_json_request(
            &fixture.app,
            Method::POST,
            CREATE_PATH,
            &delegated_token,
            Some(serde_json::json!({
                "user_service_id": &service.user_service_id,
                "endpoint_id": operation.endpoint_id,
                "catalog_digest": discovery["catalog_digest"],
                "exact_view_digest": discovery["exact_view_digest"],
                "endpoint_contract_digest": operation.endpoint_contract_digest,
                "operation_digest": operation_digest,
                "operation_id": operation.endpoint_id,
                "operation_generation": operation.operation_generation,
                "idempotency_key": "ac4-single-token",
                "arguments": arguments,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create failed: {created}");
        assert_full_router_delegated_token_unchanged(
            &fixture,
            &delegated_token,
            &delegated_jti,
            "create",
        );

        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        // The human decision uses the first-party source token. The delegated
        // bearer retained for all delegated phases must remain unchanged.
        approve_full_router_request(&fixture, &request_id).await;
        assert_full_router_delegated_token_unchanged(
            &fixture,
            &delegated_token,
            &delegated_jti,
            "decision",
        );

        let status_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/status");
        let (status, observed) = full_router_json_request(
            &fixture.app,
            Method::GET,
            &status_path,
            &delegated_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "status failed: {observed}");
        assert_eq!(observed["state"], "approved");
        assert_full_router_delegated_token_unchanged(
            &fixture,
            &delegated_token,
            &delegated_jti,
            "status",
        );

        let redeem_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/redeem");
        let (status, redeemed) = full_router_json_request(
            &fixture.app,
            Method::POST,
            &redeem_path,
            &delegated_token,
            Some(fence_body_from_created(
                &created,
                created["exact_view_digest"].as_str(),
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "redeem failed: {redeemed}");
        assert_eq!(redeemed["state"], "redeemed");
        assert_eq!(redeemed["receipt"]["http_status"], 200);
        assert_full_router_delegated_token_unchanged(
            &fixture,
            &delegated_token,
            &delegated_jti,
            "redeem",
        );
        assert_eq!(
            fixture.provider_calls.load(Ordering::SeqCst),
            1,
            "the single-token journey must dispatch exactly one effect"
        );
    }

    #[tokio::test]
    async fn delegated_caller_without_catalog_scope_rejected_on_all_exact_routes() {
        let mut fixture = setup_full_router_fixture("exact_router_proxy_only").await;
        let request_key = "proxy-only-status-redeem";
        refresh_full_router_delegation(&mut fixture).await;
        let created = create_full_router_request(&fixture, request_key).await;
        let request_id = created["request_id"]
            .as_str()
            .expect("create response request_id")
            .to_string();
        approve_full_router_request(&fixture, &request_id).await;

        let proxy_only = token_exchange_service::exchange_token_with_authority(
            &fixture.db,
            &fixture.state.config,
            &fixture.state.jwt_keys,
            &fixture.receiver_client_id,
            &fixture.receiver_secret,
            &fixture.source_token,
            "urn:ietf:params:oauth:token-type:access_token",
            Some("proxy"),
            &[],
            Some(false),
            &fixture.requested_service_ids,
            Some(false),
            &fixture.requested_node_ids,
        )
        .await
        .expect("mint proxy-only delegated token");
        let proxy_claims = jwt::verify_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &proxy_only.access_token,
        )
        .expect("verify proxy-only delegated token");
        assert!(proxy_claims.act.is_some(), "token must be delegated");
        assert!(!catalog_delegation_service::scope_has_catalog_read(
            &proxy_claims.scope
        ));
        assert!(
            fixture
                .db
                .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
                .find_one(mongodb::bson::doc! { "_id": &proxy_claims.jti })
                .await
                .expect("query proxy-only grant")
                .is_none(),
            "proxy-only exchange must not mint catalog authority"
        );

        let approvals = fixture
            .db
            .collection::<crate::models::approval_request::ApprovalRequest>(
                crate::models::approval_request::COLLECTION_NAME,
            );
        let rows_before = approvals
            .count_documents(mongodb::bson::doc! {})
            .await
            .expect("count before proxy-only create");
        let status_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/status");
        let redeem_path = format!("/api/v1/approvals/exact-service/requests/{request_id}/redeem");
        let requests = [
            (
                "create",
                Method::POST,
                CREATE_PATH.to_string(),
                Some(full_router_create_body(&fixture, "proxy-only-create", None)),
            ),
            ("status", Method::GET, status_path, None),
            (
                "redeem",
                Method::POST,
                redeem_path,
                Some(fence_body_from_created(&created, None)),
            ),
        ];
        for (route, method, path, body) in requests {
            let (status, body) = full_router_json_request(
                &fixture.app,
                method,
                &path,
                &proxy_only.access_token,
                body,
            )
            .await;
            assert_error_response(
                status,
                &body,
                StatusCode::FORBIDDEN,
                1002,
                "Forbidden: delegated_catalog_scope_required",
            );
            assert_eq!(
                fixture.provider_calls.load(Ordering::SeqCst),
                0,
                "proxy-only {route} reached the provider"
            );
        }
        assert_eq!(
            approvals
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count after proxy-only exact routes"),
            rows_before,
            "proxy-only create must not persist a request"
        );
        assert_no_full_router_redemption(&fixture, &request_id).await;
    }

    #[test]
    fn every_non_delegated_auth_method_is_rejected_before_any_grant_lookup() {
        for method in [
            AuthMethod::Session,
            AuthMethod::AccessToken,
            AuthMethod::Relay,
            AuthMethod::ApiKey,
            AuthMethod::ServiceAccount,
        ] {
            let auth = catalog_auth(method);
            let err = require_catalog_authority(&auth)
                .expect_err("non-delegated auth must not reach catalog authority");
            assert!(
                matches!(err, AppError::Forbidden(_)),
                "{:?} produced {err:?}",
                auth.auth_method
            );
        }

        // The one method that may proceed.
        assert!(require_catalog_authority(&catalog_auth(AuthMethod::Delegated)).is_ok());
    }

    #[test]
    fn delegated_token_without_catalog_scope_is_rejected() {
        let mut auth = catalog_auth(AuthMethod::Delegated);
        auth.scope = crate::mw::auth::WIDE_PROXY_SCOPE.to_string();
        assert!(matches!(
            require_catalog_authority(&auth),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn delegated_catalog_token_without_jti_cannot_bind_to_a_grant() {
        let mut auth = catalog_auth(AuthMethod::Delegated);
        auth.token_jti = None;
        assert!(matches!(
            require_catalog_authority(&auth),
            Err(AppError::Unauthorized(_))
        ));
    }

    #[test]
    fn eligible_delegated_token_yields_its_own_jti() {
        let auth = catalog_auth(AuthMethod::Delegated);
        let jti = require_catalog_authority(&auth).expect("eligible token");
        assert_eq!(Some(jti), auth.token_jti.as_deref());
    }

    #[test]
    fn grant_agreeing_on_every_bound_field_is_accepted() {
        let auth = catalog_auth(AuthMethod::Delegated);
        assert!(grant_matches_token(&matching_grant(&auth), &auth));
    }

    #[test]
    fn drift_in_any_bound_authority_field_fails_closed() {
        let auth = catalog_auth(AuthMethod::Delegated);

        #[allow(clippy::type_complexity)]
        let mutations: Vec<(&str, Box<dyn Fn(&mut CatalogDelegationGrant)>)> = vec![
            (
                "user_id",
                Box::new(|g| g.user_id = uuid::Uuid::new_v4().to_string()),
            ),
            (
                "actor_client_id",
                Box::new(|g| g.actor_client_id = "other-actor".to_string()),
            ),
            (
                "receiving_client_id",
                Box::new(|g| g.receiving_client_id = "other-receiver".to_string()),
            ),
            (
                "scope",
                Box::new(|g| g.scope = format!("{} proxy:*", g.scope)),
            ),
            (
                "resources",
                Box::new(|g| {
                    g.resources
                        .push("https://nyxid.test/api/v1/proxy/s/extra".to_string())
                }),
            ),
            (
                "allow_all_services",
                Box::new(|g| g.allow_all_services = !g.allow_all_services),
            ),
            (
                "allowed_service_ids",
                Box::new(|g| g.allowed_service_ids.push("svc-2".to_string())),
            ),
            (
                "allow_all_nodes",
                Box::new(|g| g.allow_all_nodes = !g.allow_all_nodes),
            ),
            (
                "allowed_node_ids",
                Box::new(|g| g.allowed_node_ids.push("node-2".to_string())),
            ),
        ];

        for (field, mutate) in mutations {
            let mut grant = matching_grant(&auth);
            mutate(&mut grant);
            assert!(
                !grant_matches_token(&grant, &auth),
                "drift in `{field}` was accepted; catalog authority must fail closed"
            );
        }
    }

    #[test]
    fn delegated_token_missing_client_identity_never_matches_a_grant() {
        for drop_actor in [true, false] {
            let mut auth = catalog_auth(AuthMethod::Delegated);
            let grant = matching_grant(&auth);
            let dropped = if drop_actor {
                auth.acting_client_id = None;
                "acting_client_id"
            } else {
                auth.oauth_client_id = None;
                "oauth_client_id"
            };
            assert!(
                !grant_matches_token(&grant, &auth),
                "missing `{dropped}` must fail closed"
            );
        }
    }
}
