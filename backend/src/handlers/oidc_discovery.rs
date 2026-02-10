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
        "scopes_supported": ["openid", "profile", "email"],
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "email", "email_verified", "name", "picture", "nonce"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_post", "none"],
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
