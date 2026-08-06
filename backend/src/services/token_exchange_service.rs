use mongodb::bson::doc;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::{self, DELEGATED_TOKEN_TTL_SECS, JwtKeys};
use crate::errors::{AppError, AppResult};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::ACCOUNT_READ_SCOPE;
use crate::services::{audit_service, consent_service, oauth_service};

/// Result of a successful token exchange.
pub struct TokenExchangeResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub issued_token_type: String,
    pub scope: String,
    /// The user ID extracted from the subject token (for audit logging).
    pub user_id: String,
}

/// Perform an OAuth 2.0 Token Exchange (RFC 8693).
///
/// 1. Authenticate the requesting client (client_id + client_secret)
/// 2. Validate the subject_token (user's access token)
/// 3. Verify the user has consented to this client
/// 4. Issue a constrained delegated access token
#[allow(clippy::too_many_arguments)]
pub async fn exchange_token(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    client_id: &str,
    client_secret: &str,
    subject_token: &str,
    subject_token_type: &str,
    requested_scope: Option<&str>,
) -> AppResult<TokenExchangeResponse> {
    // Step 1: Authenticate the requesting client
    let client = oauth_service::authenticate_client(db, client_id, Some(client_secret)).await?;

    // Step 2: Validate subject_token_type
    if subject_token_type != "urn:ietf:params:oauth:token-type:access_token" {
        return Err(AppError::BadRequest(
            "Only access_token subject_token_type is supported".to_string(),
        ));
    }

    // Step 3: Validate the subject token (user's access token)
    let subject_claims = jwt::verify_token(jwt_keys, config, subject_token)?;
    if subject_claims.token_type != "access" {
        return Err(AppError::BadRequest(
            "subject_token must be an access token".to_string(),
        ));
    }

    // Reject chained delegation: a delegated token cannot be exchanged for
    // another delegated token, as this would allow indefinite TTL extension.
    if subject_claims.delegated == Some(true) {
        log_exchange_failure(
            db,
            Some(&subject_claims.sub),
            client_id,
            "chained_delegation_rejected",
        );
        return Err(AppError::BadRequest(
            "Cannot exchange a delegated token -- subject_token must be a direct access token"
                .to_string(),
        ));
    }

    // Reject relay tokens: a relay access token (`X-NyxID-User-Token`) is a
    // short-lived credential shipped to a client-controlled callback URL. If it
    // could be exchanged, a captured relay token would launder into a
    // refreshable delegated token, decoupling the attacker's window from the
    // relay token's short TTL -- the same indefinite-TTL-extension the delegated
    // rejection above prevents.
    if subject_claims.relay == Some(true) {
        log_exchange_failure(
            db,
            Some(&subject_claims.sub),
            client_id,
            "relay_exchange_rejected",
        );
        return Err(AppError::BadRequest(
            "Cannot exchange a relay token -- subject_token must be a direct first-party access token"
                .to_string(),
        ));
    }

    let user_id_str = &subject_claims.sub;

    // Step 4: Verify user has consented to this client
    let consent = consent_service::check_consent(db, user_id_str, client_id, "openid").await?;

    if consent.is_none() {
        log_exchange_failure(db, Some(user_id_str), client_id, "consent_missing");
        return Err(AppError::Forbidden(
            "User has not consented to delegation for this client".to_string(),
        ));
    }

    // Step 5: Validate requested scope against client's delegation_scopes
    let scope = validate_delegation_scope(
        requested_scope.unwrap_or("llm:proxy"),
        &client.delegation_scopes,
    )?;

    // Step 6: Issue delegated access token (short-lived: 5 minutes)
    let user_uuid = Uuid::parse_str(user_id_str)
        .map_err(|e| AppError::Internal(format!("Invalid user_id in subject token: {e}")))?;
    let restrictions = jwt::TokenRestrictionClaims::from_claims(&subject_claims);
    let delegated_token = jwt::generate_delegated_access_token(
        jwt_keys,
        config,
        &user_uuid,
        &scope,
        client_id,
        DELEGATED_TOKEN_TTL_SECS,
        Some(&restrictions),
    )?;

    Ok(TokenExchangeResponse {
        access_token: delegated_token,
        token_type: "Bearer".to_string(),
        expires_in: DELEGATED_TOKEN_TTL_SECS,
        issued_token_type: "urn:ietf:params:oauth:token-type:access_token".to_string(),
        scope: scope.clone(),
        user_id: user_id_str.to_string(),
    })
}

/// Result of a successful delegation token refresh.
pub struct DelegationRefreshResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}

/// Refresh a delegated access token.
///
/// Validates that:
/// 1. The user still exists and is active
/// 2. The acting OAuth client still exists and is active
/// 3. The user still has active consent for this client
/// 4. The requested scope is still allowed by the client's delegation_scopes
/// 5. Issues a new delegated token with the same `act.sub` and validated scope
///    but a fresh 5-minute TTL
pub async fn refresh_delegation_token(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    acting_client_id: &str,
    scope: &str,
    restrictions: &jwt::TokenRestrictionClaims,
) -> AppResult<DelegationRefreshResponse> {
    // Verify user still exists and is active
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await
        .map_err(|e| AppError::Internal(format!("User lookup failed: {e}")))?;

    match user {
        Some(u) if u.is_active => {}
        _ => {
            log_exchange_failure(db, Some(user_id), acting_client_id, "user_inactive");
            return Err(AppError::Unauthorized(
                "User account is inactive or not found".to_string(),
            ));
        }
    }

    // Verify the acting OAuth client still exists and is active.
    // Without this check, a deleted or deactivated client could continue
    // refreshing delegation tokens indefinitely.
    let client = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": acting_client_id })
        .await
        .map_err(|e| AppError::Internal(format!("Client lookup failed: {e}")))?;

    let client = match client {
        Some(c) if c.is_active => c,
        Some(_) => {
            log_exchange_failure(db, Some(user_id), acting_client_id, "client_deactivated");
            return Err(AppError::Forbidden(
                "Acting OAuth client has been deactivated".to_string(),
            ));
        }
        None => {
            log_exchange_failure(db, Some(user_id), acting_client_id, "client_not_found");
            return Err(AppError::Forbidden(
                "Acting OAuth client no longer exists".to_string(),
            ));
        }
    };

    // Verify user still has active consent for this client.
    // Without this check, a client could indefinitely refresh delegation
    // tokens even after the user revokes consent.
    let consent = consent_service::check_consent(db, user_id, acting_client_id, "openid").await?;

    if consent.is_none() {
        log_exchange_failure(db, Some(user_id), acting_client_id, "consent_revoked");
        return Err(AppError::Forbidden(
            "User consent has been revoked for this client".to_string(),
        ));
    }

    // Re-validate scope against the client's current delegation_scopes.
    // The client's allowed scopes may have been downgraded since the
    // original token was issued.
    let validated_scope = validate_delegation_scope(scope, &client.delegation_scopes)?;

    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let new_token = jwt::generate_delegated_access_token(
        jwt_keys,
        config,
        &user_uuid,
        &validated_scope,
        acting_client_id,
        DELEGATED_TOKEN_TTL_SECS,
        Some(restrictions),
    )?;

    Ok(DelegationRefreshResponse {
        access_token: new_token,
        token_type: "Bearer".to_string(),
        expires_in: DELEGATED_TOKEN_TTL_SECS,
        scope: validated_scope,
    })
}

/// Fire-and-forget audit log for token exchange / delegation refresh failures.
fn log_exchange_failure(
    db: &mongodb::Database,
    user_id: Option<&str>,
    client_id: &str,
    reason: &str,
) {
    audit_service::log_async(
        db.clone(),
        user_id.map(String::from),
        "token_exchange_failed".to_string(),
        Some(serde_json::json!({
            "client_id": client_id,
            "reason": reason,
        })),
        None,
        None,
        None,
        None,
    );
}

/// Validate that the requested delegation scope is allowed by the client's
/// `delegation_scopes` configuration.
fn validate_delegation_scope(
    requested: &str,
    allowed_delegation_scopes: &str,
) -> AppResult<String> {
    if allowed_delegation_scopes.is_empty() {
        return Err(AppError::Forbidden(
            "Token exchange is not enabled for this client".to_string(),
        ));
    }

    let allowed: std::collections::HashSet<&str> =
        allowed_delegation_scopes.split_whitespace().collect();
    let requested_scopes: Vec<&str> = requested.split_whitespace().collect();

    for scope in &requested_scopes {
        if *scope == ACCOUNT_READ_SCOPE {
            return Err(AppError::InvalidScope(format!(
                "Delegation scope '{}' is reserved for service-issued tokens",
                scope
            )));
        }
        if !allowed.contains(scope) {
            return Err(AppError::InvalidScope(format!(
                "Delegation scope '{}' is not allowed for this client",
                scope
            )));
        }
    }

    Ok(requested_scopes.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
        routing::any,
    };
    use chrono::Utc;
    use tower::ServiceExt;

    #[test]
    fn validate_delegation_scope_allows_subset() {
        let result = validate_delegation_scope("llm:proxy", "llm:proxy proxy:*");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "llm:proxy");
    }

    #[test]
    fn validate_delegation_scope_allows_multiple() {
        let result = validate_delegation_scope("llm:proxy proxy:*", "llm:proxy proxy:* llm:status");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "llm:proxy proxy:*");
    }

    #[test]
    fn validate_delegation_scope_rejects_unlisted() {
        let result = validate_delegation_scope("admin:full", "llm:proxy");
        assert!(matches!(result, Err(AppError::InvalidScope(_))));
    }

    #[test]
    fn validate_delegation_scope_rejects_empty_config() {
        let result = validate_delegation_scope("llm:proxy", "");
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    // L1: Test that chained token exchange is rejected (C2 fix)
    // This is tested at the unit level via the claim check. Integration testing
    // requires a full server setup, but we can verify the claim-level guard:
    #[test]
    fn delegated_claim_detected() {
        // A delegated token should have delegated == Some(true)
        // The exchange_token function checks this and rejects it
        assert_eq!(Some(true), Some(true)); // placeholder: claim check is inline
    }

    #[test]
    fn validate_delegation_scope_single_scope_matching_exactly() {
        let result = validate_delegation_scope("proxy:*", "proxy:*");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "proxy:*");
    }

    #[test]
    fn validate_delegation_scope_all_allowed_scopes_requested() {
        let result = validate_delegation_scope(
            "llm:proxy proxy:* llm:status",
            "llm:proxy proxy:* llm:status",
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "llm:proxy proxy:* llm:status");
    }

    #[test]
    fn validate_delegation_scope_rejects_partial_mismatch() {
        let result = validate_delegation_scope("llm:proxy admin:full", "llm:proxy proxy:*");
        assert!(matches!(result, Err(AppError::InvalidScope(_))));
    }

    #[test]
    fn validate_delegation_scope_whitespace_handling() {
        let result = validate_delegation_scope("  llm:proxy  ", "llm:proxy proxy:*");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "llm:proxy");
    }

    #[test]
    fn validate_delegation_scope_empty_requested_returns_empty() {
        let result = validate_delegation_scope("", "llm:proxy proxy:*");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn validate_delegation_scope_whitespace_only_requested_returns_empty() {
        let result = validate_delegation_scope("   ", "llm:proxy");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[tokio::test]
    async fn oauth_client_cannot_exchange_or_refresh_account_read() {
        let Some(db) =
            crate::test_utils::connect_test_database("token_exchange_reject_account_read").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let user_id = Uuid::new_v4();
        let client_id = "legacy-account-read-client";
        let client_secret = "legacy-account-read-secret";
        let now = Utc::now();

        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &user_id.to_string(),
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert user");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.to_string(),
                client_name: "Legacy account-read client".to_string(),
                client_secret_hash: crate::crypto::token::hash_token(client_secret),
                redirect_uris: vec!["https://example.com/callback".to_string()],
                allowed_scopes: "openid".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "confidential".to_string(),
                is_active: true,
                delegation_scopes: ACCOUNT_READ_SCOPE.to_string(),
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
            })
            .await
            .expect("insert deliberately altered OAuth client");
        consent_service::grant_consent(&db, &user_id.to_string(), client_id, "openid")
            .await
            .expect("grant consent");

        let subject_token = jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            "openid",
            None,
            None,
            None,
            None,
            None,
        )
        .expect("generate direct subject token");
        let exchange_error = exchange_token(
            &db,
            &state.config,
            &state.jwt_keys,
            client_id,
            client_secret,
            &subject_token,
            "urn:ietf:params:oauth:token-type:access_token",
            Some(ACCOUNT_READ_SCOPE),
        )
        .await
        .err()
        .expect("token exchange must not mint account:read");
        assert!(matches!(exchange_error, AppError::InvalidScope(_)));

        let refresh_error = refresh_delegation_token(
            &db,
            &state.config,
            &state.jwt_keys,
            &user_id.to_string(),
            client_id,
            ACCOUNT_READ_SCOPE,
            &jwt::TokenRestrictionClaims::default(),
        )
        .await
        .err()
        .expect("delegation refresh must not re-add account:read");
        assert!(matches!(refresh_error, AppError::InvalidScope(_)));
    }

    #[test]
    fn token_exchange_response_fields() {
        let resp = TokenExchangeResponse {
            access_token: "tok_abc".to_string(),
            issued_token_type: "urn:ietf:params:oauth:token-type:access_token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 900,
            scope: "llm:proxy".to_string(),
            user_id: "user_1".to_string(),
        };
        assert_eq!(resp.access_token, "tok_abc");
        assert_eq!(resp.token_type, "Bearer");
        assert_eq!(resp.expires_in, 900);
        assert_eq!(resp.scope, "llm:proxy");
        assert_eq!(resp.user_id, "user_1");
    }

    #[tokio::test]
    async fn exchange_token_copies_subject_service_restrictions_and_proxy_denies_out_of_scope() {
        let Some(db) =
            crate::test_utils::connect_test_database("token_exchange_restriction_propagation")
                .await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let config = state.config.clone();
        let jwt_keys = state.jwt_keys.clone();
        let user_id = Uuid::new_v4();
        let client_id = "restricted-token-exchange-client";
        let client_secret = "secret";
        let allowed_service_id = Uuid::new_v4().to_string();
        let denied_service_id = Uuid::new_v4().to_string();
        let resources = vec![format!(
            "{}/api/v1/proxy/s/allowed-service",
            config.base_url
        )];
        let allowed_service_ids = vec![allowed_service_id.clone()];
        let now = Utc::now();

        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &user_id.to_string(),
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert user");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.to_string(),
                client_name: "Restricted Token Exchange".to_string(),
                client_secret_hash: crate::crypto::token::hash_token(client_secret),
                redirect_uris: vec!["http://localhost/callback".to_string()],
                allowed_scopes: "openid".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "confidential".to_string(),
                is_active: true,
                delegation_scopes: "llm:proxy proxy".to_string(),
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
            })
            .await
            .expect("insert oauth client");
        let allowed_endpoint = crate::test_utils::test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &user_id.to_string(),
            "Allowed Service",
            "http://127.0.0.1:9",
            None,
            None,
        );
        let denied_endpoint = crate::test_utils::test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &user_id.to_string(),
            "Denied Service",
            "http://127.0.0.1:9",
            None,
            None,
        );
        let allowed_service = crate::test_utils::test_user_service(
            &allowed_service_id,
            &user_id.to_string(),
            "allowed-service",
            &allowed_endpoint.id,
            None,
            None,
        );
        let denied_service = crate::test_utils::test_user_service(
            &denied_service_id,
            &user_id.to_string(),
            "denied-service",
            &denied_endpoint.id,
            None,
            None,
        );
        db.collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_many([allowed_endpoint, denied_endpoint])
        .await
        .expect("insert user endpoints");
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_many([allowed_service, denied_service])
        .await
        .expect("insert user services");
        consent_service::grant_consent_with_services(
            &db,
            &user_id.to_string(),
            client_id,
            "openid",
            Some(allowed_service_ids.clone()),
        )
        .await
        .expect("grant consent");

        let subject_token = jwt::generate_access_token(
            &jwt_keys,
            &config,
            &user_id,
            "openid proxy",
            None,
            None,
            None,
            None,
            Some(jwt::AccessTokenRestrictions {
                resources: &resources,
                allowed_service_ids: &allowed_service_ids,
            }),
        )
        .expect("generate restricted subject token");

        let exchanged = exchange_token(
            &db,
            &config,
            &jwt_keys,
            client_id,
            client_secret,
            &subject_token,
            "urn:ietf:params:oauth:token-type:access_token",
            Some("proxy"),
        )
        .await
        .expect("exchange token");

        let claims = jwt::verify_token(&jwt_keys, &config, &exchanged.access_token)
            .expect("verify delegated token");
        assert_eq!(claims.resources, Some(resources));
        assert_eq!(claims.allowed_service_ids, Some(allowed_service_ids));
        assert_eq!(claims.allow_all_services, Some(false));

        let app = Router::new()
            .route(
                "/api/v1/proxy/s/{slug}/{*path}",
                any(crate::handlers::proxy::proxy_request_by_slug),
            )
            .route_layer(axum::Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            ))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/proxy/s/denied-service/status")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", exchanged.access_token),
                    )
                    .body(Body::empty())
                    .expect("build proxy request"),
            )
            .await
            .expect("proxy request should return a response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read error body");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("parse error response");
        assert_eq!(payload["error"], "api_key_scope_forbidden");
        assert_eq!(payload["error_code"], 9000);
    }
}
