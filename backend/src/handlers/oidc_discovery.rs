use axum::{Json, extract::State};

use crate::AppState;
use crate::services::oauth_broker_service;

pub(crate) const OPENID_CONFIGURATION_SCOPES_SUPPORTED: &[&str] =
    &["openid", "profile", "email", "roles", "groups"];
pub(crate) const OAUTH_AUTHORIZATION_SERVER_SCOPES_SUPPORTED: &[&str] =
    &["openid", "profile", "email", "roles", "groups", "proxy"];

/// GET /.well-known/openid-configuration
///
/// OpenID Connect Discovery endpoint. Returns the provider metadata
/// so relying parties can auto-configure themselves.
pub async fn openid_configuration(State(state): State<AppState>) -> Json<serde_json::Value> {
    let base = &state.config.base_url;

    Json(serde_json::json!({
        "issuer": state.config.jwt_issuer,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "pushed_authorization_request_endpoint": format!("{base}/oauth/par"),
        "require_pushed_authorization_requests": false,
        "request_uri_parameter_supported": true,
        "userinfo_endpoint": format!("{base}/oauth/userinfo"),
        "jwks_uri": format!("{base}/.well-known/jwks.json"),
        "response_types_supported": ["code"],
        "grant_types_supported": [
            "authorization_code",
            "refresh_token",
            "client_credentials",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ],
        "subject_token_types_supported": [
            "urn:ietf:params:oauth:token-type:access_token",
            oauth_broker_service::BROKER_SUBJECT_TOKEN_TYPE,
        ],
        "nyxid_broker_binding_supported": true,
        "oauth_broker_binding_revocation_webhook_supported": true,
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "dpop_signing_alg_values_supported": ["ES256"],
        "tls_client_certificate_bound_access_tokens": true,
        "introspection_endpoint": format!("{base}/oauth/introspect"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "scopes_supported": OPENID_CONFIGURATION_SCOPES_SUPPORTED,
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "email", "email_verified", "name", "picture", "nonce", "at_hash", "roles", "groups", "permissions", "acr", "amr", "auth_time", "sid"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
        "userinfo_signing_alg_values_supported": ["none"],
    }))
}

/// GET /.well-known/oauth-authorization-server
///
/// RFC 8414 OAuth Authorization Server Metadata. MCP clients check this
/// endpoint before falling back to `/.well-known/openid-configuration`.
pub async fn oauth_authorization_server_metadata(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let base = &state.config.base_url;

    Json(serde_json::json!({
        "issuer": state.config.jwt_issuer,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "pushed_authorization_request_endpoint": format!("{base}/oauth/par"),
        "require_pushed_authorization_requests": false,
        "request_uri_parameter_supported": true,
        "registration_endpoint": format!("{base}/oauth/register"),
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
        "userinfo_endpoint": format!("{base}/oauth/userinfo"),
        "jwks_uri": format!("{base}/.well-known/jwks.json"),
        "introspection_endpoint": format!("{base}/oauth/introspect"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "scopes_supported": OAUTH_AUTHORIZATION_SERVER_SCOPES_SUPPORTED,
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": [
            "authorization_code",
            "refresh_token",
            "client_credentials",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ],
        "subject_token_types_supported": [
            "urn:ietf:params:oauth:token-type:access_token",
            oauth_broker_service::BROKER_SUBJECT_TOKEN_TYPE,
        ],
        "nyxid_broker_binding_supported": true,
        "oauth_broker_binding_revocation_webhook_supported": true,
        "code_challenge_methods_supported": ["S256"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "dpop_signing_alg_values_supported": ["ES256"],
        "tls_client_certificate_bound_access_tokens": true,
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "email", "email_verified", "name", "picture", "nonce", "at_hash", "roles", "groups", "permissions", "acr", "amr", "auth_time", "sid"],
    }))
}

/// GET /.well-known/oauth-protected-resource
///
/// RFC 9728 Protected Resource Metadata. MCP clients use this to discover
/// where to authenticate (NyxID's OAuth endpoints) before connecting.
pub async fn oauth_protected_resource(State(state): State<AppState>) -> Json<serde_json::Value> {
    let base = &state.config.base_url;

    Json(serde_json::json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "scopes_supported": ["openid", "profile", "email", "proxy"],
        "bearer_methods_supported": ["header"],
    }))
}

/// GET /.well-known/jwks.json
///
/// JSON Web Key Set endpoint. Returns the public key(s) used to sign JWTs
/// so relying parties can verify token signatures.
pub async fn jwks(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "keys": [state.jwk_json]
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        OAUTH_AUTHORIZATION_SERVER_SCOPES_SUPPORTED, OPENID_CONFIGURATION_SCOPES_SUPPORTED,
    };
    use crate::services::oauth_broker_service::BROKER_BINDING_SCOPE;

    #[test]
    fn public_discovery_scopes_do_not_include_broker_binding_scope() {
        assert!(!OPENID_CONFIGURATION_SCOPES_SUPPORTED.contains(&BROKER_BINDING_SCOPE));
        assert!(!OAUTH_AUTHORIZATION_SERVER_SCOPES_SUPPORTED.contains(&BROKER_BINDING_SCOPE));
    }
}
