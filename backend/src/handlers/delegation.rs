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
    pub resolved_at: String,
    pub authority_expires_at: String,
    pub services: Vec<DelegatedCatalogService>,
    pub total_services: usize,
    pub total_operations: usize,
}

#[derive(Debug, Serialize)]
pub struct DelegatedCatalogService {
    pub user_service_id: String,
    pub service_slug: String,
    pub service_name: String,
    pub description: Option<String>,
    pub node_id: Option<String>,
    pub operations: Vec<DelegatedCatalogOperation>,
}

#[derive(Debug, Serialize)]
pub struct DelegatedCatalogOperation {
    pub endpoint_id: String,
    pub name: String,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub request_content_type: Option<String>,
    pub request_body_required: bool,
    pub response: crate::models::service_endpoint::OperationResponseContract,
    pub endpoint_contract_digest: String,
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
    let catalog_digest = mcp_service::operation_catalog_digest(&catalog.services);
    let resolved_at = Utc::now();

    let services: Vec<_> = catalog
        .services
        .iter()
        .filter_map(|service| {
            let mcp_service::McpToolSource::UserManaged {
                user_service_id,
                node_id,
                ..
            } = &service.source
            else {
                return None;
            };
            if service.is_generic_proxy {
                return None;
            }
            let operations = service
                .endpoints
                .iter()
                .map(|endpoint| DelegatedCatalogOperation {
                    endpoint_id: endpoint.endpoint_id.clone(),
                    name: endpoint.name.clone(),
                    method: endpoint.method.clone(),
                    path: endpoint.path.clone(),
                    parameters: endpoint.parameters.clone(),
                    request_body_schema: endpoint.request_body_schema.clone(),
                    request_content_type: endpoint.request_content_type.clone(),
                    request_body_required: endpoint.request_body_required,
                    response: endpoint.response.clone(),
                    endpoint_contract_digest: mcp_service::endpoint_contract_digest(endpoint),
                })
                .collect::<Vec<_>>();
            Some(DelegatedCatalogService {
                user_service_id: user_service_id.clone(),
                service_slug: service.service_slug.clone(),
                service_name: service.service_name.clone(),
                description: service.description.clone(),
                node_id: node_id.clone(),
                operations,
            })
        })
        .collect();
    let total_services = services.len();
    let total_operations = services
        .iter()
        .map(|service| service.operations.len())
        .sum();

    Ok(Json(DelegatedOperationCatalogResponse {
        contract_version: "nyxid-delegated-operation-catalog.v1",
        catalog_digest,
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
    use super::*;

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
            resolved_at: "2026-08-14T00:00:00Z".to_string(),
            authority_expires_at: "2026-08-14T00:05:00Z".to_string(),
            services: vec![DelegatedCatalogService {
                user_service_id: "service-1".to_string(),
                service_slug: "github".to_string(),
                service_name: "GitHub".to_string(),
                description: Some("typed service".to_string()),
                node_id: None,
                operations: vec![DelegatedCatalogOperation {
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

    /// A grant agreeing with `auth` on every bound authority field.
    fn matching_grant(auth: &AuthUser) -> CatalogDelegationGrant {
        let now = Utc::now();
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
            ("scope", Box::new(|g| g.scope = format!("{} proxy:*", g.scope))),
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
