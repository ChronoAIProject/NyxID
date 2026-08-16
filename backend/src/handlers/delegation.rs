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
        contract_version: "nyxid-delegated-operation-catalog.v1",
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

    use mongodb::event::EventHandler;
    use mongodb::event::command::CommandEvent;

    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::service_endpoint::{
        COLLECTION_NAME as SERVICE_ENDPOINTS, OperationResponseContract, ServiceEndpoint,
    };
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

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
            contract_version: "nyxid-delegated-operation-catalog.v1",
            catalog_digest: "sha256:opaque".to_string(),
            exact_view_digest: "sha256:exact-view".to_string(),
            resolved_at: "2026-08-14T00:00:00Z".to_string(),
            authority_expires_at: "2026-08-14T00:05:00Z".to_string(),
            services: vec![mcp_service::ExactOperationViewService {
                user_service_id: "service-1".to_string(),
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
            actual,
            serde_json::json!({
                "contract_version": "nyxid-delegated-operation-catalog.v1",
                "catalog_digest": "sha256:ea55205436c9a87a35ec9d21072cff07e77dc2ef23409aeb16760f0d08662a62",
                "exact_view_digest": "sha256:e1d168ef49f0b5955c39fa4a0767791fa9a75141ed4c4df68a6d1b53f87284fc",
                "resolved_at": "<resolved-at>",
                "authority_expires_at": grant.expires_at.to_rfc3339(),
                "services": [
                    {
                        "user_service_id": TEST_SERVICE_A,
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
