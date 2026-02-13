use axum::{extract::State, Json};

use crate::AppState;

/// GET /.well-known/openid-configuration
///
/// OpenID Connect Discovery endpoint. Returns the provider metadata
/// so relying parties can auto-configure themselves.
pub async fn openid_configuration(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let base = &state.config.base_url;

    Json(serde_json::json!({
        "issuer": state.config.jwt_issuer,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "userinfo_endpoint": format!("{base}/oauth/userinfo"),
        "jwks_uri": format!("{base}/.well-known/jwks.json"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "introspection_endpoint": format!("{base}/oauth/introspect"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "scopes_supported": ["openid", "profile", "email", "roles", "groups"],
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "email", "email_verified", "name", "picture", "nonce", "at_hash", "roles", "groups", "permissions", "acr", "amr", "auth_time", "sid"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_post", "none"],
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
        "registration_endpoint": format!("{base}/oauth/register"),
        "token_endpoint_auth_methods_supported": ["client_secret_post", "none"],
        "userinfo_endpoint": format!("{base}/oauth/userinfo"),
        "jwks_uri": format!("{base}/.well-known/jwks.json"),
        "introspection_endpoint": format!("{base}/oauth/introspect"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "scopes_supported": ["openid", "profile", "email", "roles", "groups"],
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "email", "email_verified", "name", "picture", "nonce", "at_hash", "roles", "groups", "permissions", "acr", "amr", "auth_time", "sid"],
    }))
}

/// GET /.well-known/oauth-protected-resource
///
/// RFC 9728 Protected Resource Metadata. MCP clients use this to discover
/// where to authenticate (NyxID's OAuth endpoints) before connecting.
pub async fn oauth_protected_resource(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let base = &state.config.base_url;

    Json(serde_json::json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "scopes_supported": ["openid", "profile", "email"],
        "bearer_methods_supported": ["header"],
    }))
}

/// GET /.well-known/jwks.json
///
/// JSON Web Key Set endpoint. Returns the public key(s) used to sign JWTs
/// so relying parties can verify token signatures.
pub async fn jwks(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "keys": [state.jwk_json]
    }))
}
