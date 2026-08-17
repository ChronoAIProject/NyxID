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
    /// Representation validator over exactly `services`, using the same
    /// canonical projection as exact approval create and redemption.
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
    let exact_view_digest = mcp_service::exact_operation_view_digest(&view);
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
    use std::sync::{Arc, Mutex};

    use axum::{Extension, Json as AxumJson, Router, extract::Path, routing::get};
    use chrono::Utc;
    use mongodb::event::EventHandler;
    use mongodb::event::command::CommandEvent;
    use uuid::Uuid;

    use crate::crypto::{jwt, token::hash_token};
    use crate::handlers::exact_service_approvals;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::oauth_client::{
        COLLECTION_NAME as OAUTH_CLIENTS, OauthClient, ScopeProvenance,
    };
    use crate::models::service_approval_config::{ApprovalMode, ServiceApprovalConfig};
    use crate::models::service_endpoint::{
        COLLECTION_NAME as SERVICE_ENDPOINTS, OperationResponseContract, ServiceEndpoint,
    };
    use crate::models::user::UserType;
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::{approval_service, consent_service, token_exchange_service};

    use super::*;

    const TEST_USER_ID: &str = "00000000-0000-4000-8000-000000000001";
    const TEST_SERVICE_A: &str = "00000000-0000-4000-8000-000000000101";
    const TEST_SERVICE_B: &str = "00000000-0000-4000-8000-000000000102";
    const TEST_GENERIC_SERVICE: &str = "00000000-0000-4000-8000-000000000103";

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
                    endpoint_contract_digest: "sha256:contract".to_string(),
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
                endpoint_contract_digest: endpoint_contract_digest.to_string(),
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
    fn deterministic_fixture_binds_provider_identity_and_v2_digest_envelope() {
        let view = deterministic_exact_view_fixture();
        assert_eq!(
            view.services[0].catalog_service_id.as_deref(),
            Some("00000000-0000-4000-8000-000000000301")
        );
        let digest = mcp_service::exact_operation_view_digest(&view);
        assert_eq!(
            digest,
            "sha256:acfbddf689ec25828e01cb4c149f9fd5f3ab5c9f37fddc7d0891e088cf1030a5"
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
                                "endpoint_contract_digest": "sha256:44d63aba629068d100761faf33f1359e0837f554f691ea50fd30a69bb7b8ea5a"
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
                                "endpoint_contract_digest": "sha256:b33c78e543e5bd0db8ac01378ced3f1598ba15457c91b7526178fe91313e5021"
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
                                "endpoint_contract_digest": "sha256:3f488040e19cb9fdc9e9f2aefd9061058c16426a58a2ace0664373e5b9574d5f"
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
        let service = discovery
            .services
            .iter()
            .find(|service| service.user_service_id == TEST_SERVICE_A)
            .expect("catalog-backed service in discovery");
        let operation = service.operations.first().expect("typed operation");
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
                operation_generation: 1,
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
            AxumJson(generic_create(
                Some(discovery.exact_view_digest.clone()),
                "matrix-row-1-delegated-hidden",
            )),
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
            "matrix row 3 must not persist an unrelated exact-view fence"
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
        .expect("matrix row 3: non-delegated generic request redeems");
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

        let AxumJson(created) = exact_service_approvals::create_request(
            State(state.clone()),
            delegated_auth.clone(),
            AxumJson(
                crate::services::exact_service_approval_service::ExactServiceApprovalCreate {
                    user_service_id: TEST_SERVICE_A.to_string(),
                    endpoint_id: operation.endpoint_id.clone(),
                    catalog_digest: discovery.catalog_digest.clone(),
                    // Omit the additive field at create to prove the server
                    // persists its live exact-view fence and returns it for
                    // redeem, rather than trusting caller omission forever.
                    exact_view_digest: None,
                    endpoint_contract_digest: operation.endpoint_contract_digest.clone(),
                    operation_digest: operation_digest.clone(),
                    operation_id: operation.endpoint_id.clone(),
                    operation_generation: 1,
                    idempotency_key: "integration-idempotency-key".to_string(),
                    arguments: arguments.clone(),
                },
            ),
        )
        .await
        .expect("real exact create handler");
        assert_eq!(
            created.exact_view_digest,
            Some(discovery.exact_view_digest.clone())
        );
        assert_eq!(created.catalog_digest, discovery.catalog_digest);

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
                    operation_generation: 1,
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
