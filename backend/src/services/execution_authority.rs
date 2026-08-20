//! Producer-owned execution-authority digest for exact-service approvals.
//!
//! The catalog/exact-view fences bind operation identity. This digest binds
//! the post-resolution execution inputs (`ProxyTarget` destination, auth,
//! credential identity+epoch, identity injection, default headers, proxy
//! operation policy, and the configured node-binding set) so an approved
//! effect cannot be silently retargeted while the approval is pending.

use serde::Serialize;

use crate::models::default_request_header::DefaultRequestHeader;
use crate::models::downstream_service::ProxyOperationPolicy;
use crate::services::mcp_service;
use crate::services::proxy_service::UserServiceResolution;

pub const CONTRACT_VERSION: &str = "nyxid-exact-execution-authority.v1";

#[derive(Clone, Debug, Serialize)]
pub struct CredentialProjection {
    pub api_key_id: Option<String>,
    pub credential_epoch: i64,
    pub master_credential: bool,
    pub override_api_key_id: Option<String>,
    pub override_epoch: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IdentityInjectionProjection {
    pub identity_propagation_mode: String,
    pub identity_include_user_id: bool,
    pub identity_include_email: bool,
    pub identity_include_name: bool,
    pub identity_jwt_audience: Option<String>,
    pub forward_access_token: bool,
    pub inject_delegation_token: bool,
    pub delegation_token_scope: String,
    pub custom_user_agent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HeaderProjection {
    pub name: String,
    pub value: String,
    pub overridable: bool,
    pub sensitive: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DefaultHeadersProjection {
    pub catalog: Vec<HeaderProjection>,
    pub user_service: Vec<HeaderProjection>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NodeRouteProjection {
    pub primary_node_id: Option<String>,
    pub configured_fallback_node_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutionAuthorityProjection {
    pub contract_version: &'static str,
    pub user_service_id: String,
    pub destination_base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub credential: CredentialProjection,
    pub identity_injection: IdentityInjectionProjection,
    pub default_headers: DefaultHeadersProjection,
    pub proxy_operation_policy: Option<ProxyOperationPolicy>,
    pub node_route: NodeRouteProjection,
}

#[derive(Clone, Debug)]
pub struct OverrideCredentialIdentity {
    pub api_key_id: String,
    pub credential_epoch: i64,
}

pub fn build_projection(
    resolution: &UserServiceResolution,
    override_credential: Option<&OverrideCredentialIdentity>,
    configured_fallback_node_ids: Vec<String>,
) -> ExecutionAuthorityProjection {
    let target = &resolution.target;
    let service = &target.service;
    let mut fallbacks = configured_fallback_node_ids;
    if let Some(primary) = resolution.node_id.as_deref().filter(|id| !id.is_empty()) {
        fallbacks.retain(|id| id != primary);
    }
    fallbacks.sort();
    fallbacks.dedup();

    ExecutionAuthorityProjection {
        contract_version: CONTRACT_VERSION,
        user_service_id: resolution.user_service_id.clone(),
        destination_base_url: target.base_url.clone(),
        auth_method: target.auth_method.clone(),
        auth_key_name: target.auth_key_name.clone(),
        credential: CredentialProjection {
            api_key_id: resolution.api_key_id.clone(),
            credential_epoch: resolution.credential_epoch,
            master_credential: resolution.master_credential,
            override_api_key_id: override_credential.map(|value| value.api_key_id.clone()),
            override_epoch: override_credential.map(|value| value.credential_epoch),
        },
        identity_injection: IdentityInjectionProjection {
            identity_propagation_mode: service.identity_propagation_mode.clone(),
            identity_include_user_id: service.identity_include_user_id,
            identity_include_email: service.identity_include_email,
            identity_include_name: service.identity_include_name,
            identity_jwt_audience: service.identity_jwt_audience.clone(),
            forward_access_token: service.forward_access_token,
            inject_delegation_token: service.inject_delegation_token,
            delegation_token_scope: service.delegation_token_scope.clone(),
            custom_user_agent: service.custom_user_agent.clone(),
        },
        default_headers: DefaultHeadersProjection {
            catalog: project_headers(&target.catalog_default_headers),
            user_service: project_headers(&target.user_service_default_headers),
        },
        proxy_operation_policy: service.proxy_operation_policy.clone(),
        node_route: NodeRouteProjection {
            primary_node_id: resolution
                .node_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(str::to_string),
            configured_fallback_node_ids: fallbacks,
        },
    }
}

pub fn digest(projection: &ExecutionAuthorityProjection) -> String {
    mcp_service::canonical_sha256(
        serde_json::to_value(projection).expect("execution authority projection is JSON"),
    )
}

fn project_headers(headers: &[DefaultRequestHeader]) -> Vec<HeaderProjection> {
    headers
        .iter()
        .map(|header| HeaderProjection {
            name: header.name.clone(),
            value: header.value.clone(),
            overridable: header.overridable,
            sensitive: header.sensitive,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_projection() -> ExecutionAuthorityProjection {
        ExecutionAuthorityProjection {
            contract_version: CONTRACT_VERSION,
            user_service_id: "us-alpha".to_string(),
            destination_base_url: "https://api.example.test".to_string(),
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential: CredentialProjection {
                api_key_id: Some("key-1".to_string()),
                credential_epoch: 1,
                master_credential: false,
                override_api_key_id: None,
                override_epoch: None,
            },
            identity_injection: IdentityInjectionProjection {
                identity_propagation_mode: "none".to_string(),
                identity_include_user_id: false,
                identity_include_email: false,
                identity_include_name: false,
                identity_jwt_audience: None,
                forward_access_token: false,
                inject_delegation_token: false,
                delegation_token_scope: "llm:proxy".to_string(),
                custom_user_agent: None,
            },
            default_headers: DefaultHeadersProjection {
                catalog: vec![HeaderProjection {
                    name: "X-Example".to_string(),
                    value: "one".to_string(),
                    overridable: false,
                    sensitive: false,
                }],
                user_service: Vec::new(),
            },
            proxy_operation_policy: None,
            node_route: NodeRouteProjection {
                primary_node_id: None,
                configured_fallback_node_ids: vec!["node-b".to_string(), "node-a".to_string()],
            },
        }
    }

    #[test]
    fn digest_is_deterministic_for_a_fixed_projection() {
        let first = digest(&sample_projection());
        let second = digest(&sample_projection());
        assert_eq!(first, second);
        assert!(first.starts_with("sha256:"));
        assert_eq!(first.len(), 7 + 64);
    }

    #[test]
    fn digest_is_insensitive_to_object_key_order() {
        let projection = sample_projection();
        let value = serde_json::to_value(&projection).unwrap();
        let canonical = mcp_service::canonical_json(value.clone());
        assert_ne!(
            serde_json::to_string(&value).unwrap(),
            serde_json::to_string(&canonical).unwrap()
        );
        assert_eq!(
            mcp_service::canonical_sha256(value),
            mcp_service::canonical_sha256(canonical)
        );
    }

    #[test]
    fn header_value_participates_in_the_digest() {
        let mut changed = sample_projection();
        changed.default_headers.catalog[0].value = "two".to_string();
        assert_ne!(digest(&sample_projection()), digest(&changed));
    }
}
