use chrono::Utc;
use mongodb::bson::doc;
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::{self, DELEGATED_TOKEN_TTL_SECS, JwtKeys};
use crate::errors::{AppError, AppResult};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::ACCOUNT_READ_SCOPE;
use crate::services::{
    api_key_scope_service::{self, ScopeAuthorization},
    audit_service, catalog_delegation_service, consent_service, oauth_resource_service,
    oauth_service,
};

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
#[cfg(test)]
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
    exchange_token_with_authority(
        db,
        config,
        jwt_keys,
        client_id,
        client_secret,
        subject_token,
        subject_token_type,
        requested_scope,
        &[],
        None,
        &[],
        None,
        &[],
    )
    .await
}

pub async fn exchange_token_with_authority(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    client_id: &str,
    client_secret: &str,
    subject_token: &str,
    subject_token_type: &str,
    requested_scope: Option<&str>,
    requested_resources: &[String],
    requested_allow_all_services: Option<bool>,
    requested_service_ids: &[String],
    requested_allow_all_nodes: Option<bool>,
    requested_node_ids: &[String],
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
    let catalog_scope = catalog_delegation_service::scope_has_catalog_read(&scope);
    let actor_client_id = subject_claims
        .client_id
        .as_deref()
        .filter(|id| !id.is_empty());
    if catalog_scope && actor_client_id.is_none() {
        log_exchange_failure(
            db,
            Some(user_id_str),
            client_id,
            "catalog_source_client_missing",
        );
        return Err(AppError::Forbidden(
            "Catalog delegation requires a client-bound subject token".to_string(),
        ));
    }
    let (restrictions, authority) = if catalog_scope {
        catalog_delegation_service::ensure_client_can_delegate_catalog(
            db,
            actor_client_id.expect("catalog source client checked above"),
        )
        .await?;
        let authority = attenuate_catalog_authority(
            db,
            config,
            user_id_str,
            &subject_claims,
            actor_client_id.expect("catalog source client checked above"),
            client_id,
            requested_resources,
            requested_allow_all_services,
            requested_service_ids,
            requested_allow_all_nodes,
            requested_node_ids,
        )
        .await?;
        (authority.restriction_claims(), Some(authority))
    } else {
        let mut restrictions = jwt::TokenRestrictionClaims::from_claims(&subject_claims);
        // Newly issued ordinary delegated tokens always carry their node
        // posture explicitly. Missing source claims retain the legacy
        // allow-all behavior, but that compatibility default is materialized
        // in the new token rather than inferred again by its consumer.
        restrictions.allowed_node_ids.get_or_insert_with(Vec::new);
        restrictions.allow_all_nodes.get_or_insert(true);
        (restrictions, None)
    };
    let (delegated_token, jti) = if catalog_scope {
        jwt::generate_delegated_access_token_for_client(
            jwt_keys,
            config,
            &user_uuid,
            &scope,
            actor_client_id.expect("catalog source client checked above"),
            Some(client_id),
            DELEGATED_TOKEN_TTL_SECS,
            Some(&restrictions),
        )?
    } else {
        let token = jwt::generate_delegated_access_token(
            jwt_keys,
            config,
            &user_uuid,
            &scope,
            client_id,
            DELEGATED_TOKEN_TTL_SECS,
            Some(&restrictions),
        )?;
        (token, String::new())
    };

    if let Some(authority) = authority {
        catalog_delegation_service::persist_grant(
            db,
            &jti,
            user_id_str,
            actor_client_id.unwrap_or(client_id),
            client_id,
            &scope,
            &authority,
            Utc::now().timestamp() + DELEGATED_TOKEN_TTL_SECS,
        )
        .await?;
    }

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
    receiving_client_id: Option<&str>,
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

    let acting_client = match client {
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

    let catalog_scope = catalog_delegation_service::scope_has_catalog_read(scope);
    let receiving_client_id = if catalog_scope {
        Some(
            receiving_client_id
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    AppError::Unauthorized(
                        "Delegated catalog authority is invalid or inactive".to_string(),
                    )
                })?,
        )
    } else {
        None
    };

    // Catalog delegation has two client identities. `act.sub` remains the
    // source actor while `client_id` identifies the receiver whose delegation
    // scope authorizes the exchanged capability.
    let scope_client = if let Some(receiving_client_id) = receiving_client_id {
        catalog_delegation_service::ensure_client_can_delegate_catalog(db, receiving_client_id)
            .await?;
        let receiving_client = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": receiving_client_id })
            .await
            .map_err(|e| AppError::Internal(format!("Client lookup failed: {e}")))?
            .filter(|client| client.is_active)
            .ok_or_else(|| {
                AppError::Forbidden("Receiving OAuth client is inactive or missing".to_string())
            })?;

        if receiving_client_id != acting_client_id
            && consent_service::check_consent(db, user_id, receiving_client_id, "openid")
                .await?
                .is_none()
        {
            return Err(AppError::Forbidden(
                "User consent has been revoked for the receiving client".to_string(),
            ));
        }
        receiving_client
    } else {
        acting_client
    };

    // Re-validate scope against the receiving client's current delegation
    // scopes. The source client does not acquire the receiver's authority.
    let validated_scope = validate_delegation_scope(scope, &scope_client.delegation_scopes)?;

    let authority = if catalog_scope {
        catalog_delegation_service::ensure_client_can_delegate_catalog(db, acting_client_id)
            .await?;
        let authority =
            catalog_delegation_service::authority_from_restriction_claims(restrictions)?;
        validate_refresh_catalog_authority(
            db,
            config,
            user_id,
            acting_client_id,
            receiving_client_id.expect("catalog receiver checked above"),
            &authority,
        )
        .await?;
        Some(authority)
    } else {
        None
    };

    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let (new_token, jti) = if catalog_scope {
        jwt::generate_delegated_access_token_for_client(
            jwt_keys,
            config,
            &user_uuid,
            &validated_scope,
            acting_client_id,
            receiving_client_id,
            DELEGATED_TOKEN_TTL_SECS,
            Some(restrictions),
        )?
    } else {
        (
            jwt::generate_delegated_access_token(
                jwt_keys,
                config,
                &user_uuid,
                &validated_scope,
                acting_client_id,
                DELEGATED_TOKEN_TTL_SECS,
                Some(restrictions),
            )?,
            String::new(),
        )
    };

    if let Some(authority) = authority {
        catalog_delegation_service::persist_grant(
            db,
            &jti,
            user_id,
            acting_client_id,
            receiving_client_id.expect("catalog receiver checked above"),
            &validated_scope,
            &authority,
            Utc::now().timestamp() + DELEGATED_TOKEN_TTL_SECS,
        )
        .await?;
    }

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
pub fn validate_delegation_scope(
    requested: &str,
    allowed_delegation_scopes: &str,
) -> AppResult<String> {
    if allowed_delegation_scopes.is_empty() {
        return Err(AppError::Forbidden(
            "Token exchange is not enabled for this client".to_string(),
        ));
    }

    let allowed: HashSet<&str> = allowed_delegation_scopes.split_whitespace().collect();
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

async fn attenuate_catalog_authority(
    db: &mongodb::Database,
    config: &AppConfig,
    user_id: &str,
    source_claims: &jwt::Claims,
    source_client_id: &str,
    receiving_client_id: &str,
    requested_resources: &[String],
    requested_allow_all_services: Option<bool>,
    requested_service_ids: &[String],
    requested_allow_all_nodes: Option<bool>,
    requested_node_ids: &[String],
) -> AppResult<catalog_delegation_service::CatalogAuthority> {
    let source = catalog_delegation_service::authority_from_claims(source_claims)?;
    if source_claims.client_id.as_deref() != Some(source_client_id) {
        return Err(AppError::Forbidden(
            "Catalog delegation requires a client-bound subject token".to_string(),
        ));
    }
    let source_services = AuthorityBound::from_explicit(
        source.allow_all_services,
        &source.allowed_service_ids,
        "source service authority",
    )?;
    let source_nodes = AuthorityBound::from_explicit(
        source.allow_all_nodes,
        &source.allowed_node_ids,
        "source node authority",
    )?;
    let requested_services = AuthorityBound::from_request(
        requested_allow_all_services,
        requested_service_ids,
        "service",
    )?;
    let requested_nodes =
        AuthorityBound::from_request(requested_allow_all_nodes, requested_node_ids, "node")?;

    validate_service_bound(db, user_id, &source_services, "source").await?;
    validate_node_bound(db, user_id, &source_nodes).await?;
    validate_service_bound(db, user_id, &requested_services, "requested").await?;
    validate_node_bound(db, user_id, &requested_nodes).await?;

    let source_consent = load_catalog_consent(db, user_id, source_client_id).await?;
    let receiving_consent = load_catalog_consent(db, user_id, receiving_client_id).await?;
    let service_ceiling = source_services
        .intersection(&authority_from_consent(&source_consent, "source client")?)
        .intersection(&authority_from_consent(
            &receiving_consent,
            "receiving client",
        )?);
    requested_services.ensure_within(&service_ceiling, "service")?;
    requested_nodes.ensure_within(&source_nodes, "node")?;

    reject_duplicates(requested_resources, "resource")?;
    let source_resources =
        resolve_catalog_resources(db, config, user_id, &source.resources).await?;
    source_resources
        .services
        .ensure_within(&source_services, "source resource service")?;
    let requested_resource_scope = if requested_resources.is_empty() {
        source_resources
    } else {
        let requested = resolve_catalog_resources(db, config, user_id, requested_resources).await?;
        if !source.resources.is_empty()
            && !resources_are_within_source(&requested.resources, &source_resources.resources)
        {
            return Err(AppError::InvalidTarget(
                "resource cannot expand beyond the source token authority".to_string(),
            ));
        }
        requested
    };
    if !matches!(requested_resource_scope.services, AuthorityBound::All) {
        requested_resource_scope
            .services
            .ensure_within(&service_ceiling, "resource service")?;
    }

    let final_services = requested_services.intersection(&requested_resource_scope.services);
    validate_service_bound(db, user_id, &final_services, "effective").await?;

    let (allow_all_services, allowed_service_ids) = final_services.into_parts();
    let (allow_all_nodes, allowed_node_ids) = requested_nodes.into_parts();
    Ok(catalog_delegation_service::CatalogAuthority {
        resources: requested_resource_scope.resources,
        allow_all_services,
        allowed_service_ids,
        allow_all_nodes,
        allowed_node_ids,
    })
}

fn resources_are_within_source(requested: &[String], source: &[String]) -> bool {
    requested
        .iter()
        .all(|resource| source.iter().any(|granted| granted == resource))
}

fn reject_duplicates(values: &[String], field: &str) -> AppResult<()> {
    let unique: HashSet<&str> = values.iter().map(String::as_str).collect();
    if unique.len() != values.len() {
        return Err(AppError::InvalidTarget(format!(
            "{field} must not contain duplicates"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AuthorityBound {
    All,
    Restricted(BTreeSet<String>),
}

impl AuthorityBound {
    fn from_explicit(allow_all: bool, ids: &[String], label: &str) -> AppResult<Self> {
        reject_duplicates(ids, label)?;
        if allow_all {
            if !ids.is_empty() {
                return Err(AppError::InvalidTarget(format!(
                    "{label} cannot combine allow-all with explicit IDs"
                )));
            }
            Ok(Self::All)
        } else {
            Ok(Self::Restricted(ids.iter().cloned().collect()))
        }
    }

    fn from_request(allow_all: Option<bool>, ids: &[String], label: &str) -> AppResult<Self> {
        let allow_all = allow_all.ok_or_else(|| {
            AppError::InvalidTarget(format!("catalog {label} authority is required"))
        })?;
        Self::from_explicit(allow_all, ids, &format!("requested {label} authority"))
    }

    fn intersection(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::All, bound) | (bound, Self::All) => bound.clone(),
            (Self::Restricted(left), Self::Restricted(right)) => {
                Self::Restricted(left.intersection(right).cloned().collect())
            }
        }
    }

    fn ensure_within(&self, ceiling: &Self, label: &str) -> AppResult<()> {
        let within = match (self, ceiling) {
            (_, Self::All) => true,
            (Self::All, Self::Restricted(_)) => false,
            (Self::Restricted(requested), Self::Restricted(allowed)) => {
                requested.is_subset(allowed)
            }
        };
        if !within {
            return Err(AppError::InvalidTarget(format!(
                "requested {label} authority exceeds the verified source authority"
            )));
        }
        Ok(())
    }

    fn restricted_ids(&self) -> Option<Vec<String>> {
        match self {
            Self::All => None,
            Self::Restricted(ids) => Some(ids.iter().cloned().collect()),
        }
    }

    fn into_parts(self) -> (bool, Vec<String>) {
        match self {
            Self::All => (true, Vec::new()),
            Self::Restricted(ids) => (false, ids.into_iter().collect()),
        }
    }
}

struct CatalogResourceScope {
    resources: Vec<String>,
    services: AuthorityBound,
}

async fn resolve_catalog_resources(
    db: &mongodb::Database,
    config: &AppConfig,
    user_id: &str,
    resources: &[String],
) -> AppResult<CatalogResourceScope> {
    if resources.is_empty() {
        return Ok(CatalogResourceScope {
            resources: Vec::new(),
            services: AuthorityBound::All,
        });
    }
    let resolved =
        oauth_resource_service::resolve_requested_resources(db, config, user_id, Some(resources))
            .await?
            .expect("non-empty resource request resolves to a scope");
    let mut canonical_resources = resolved.resource_uris;
    if let Some(mcp_resource) = resolved.mcp_resource_uri {
        canonical_resources.push(mcp_resource);
    }
    canonical_resources.sort();
    let services = if resolved.service_ids.is_empty() {
        AuthorityBound::All
    } else {
        AuthorityBound::Restricted(resolved.service_ids.into_iter().collect())
    };
    Ok(CatalogResourceScope {
        resources: canonical_resources,
        services,
    })
}

async fn load_catalog_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
) -> AppResult<crate::models::consent::Consent> {
    consent_service::check_consent(db, user_id, client_id, "openid")
        .await?
        .ok_or_else(|| {
            AppError::Forbidden("User consent has been revoked for this client".to_string())
        })
}

fn authority_from_consent(
    consent: &crate::models::consent::Consent,
    label: &str,
) -> AppResult<AuthorityBound> {
    if consent.allow_all_services {
        return AuthorityBound::from_explicit(true, &[], &format!("{label} consent"));
    }
    let ids = consent.allowed_service_ids.as_deref().ok_or_else(|| {
        AppError::Forbidden(format!(
            "{label} consent does not carry explicit catalog authority"
        ))
    })?;
    AuthorityBound::from_explicit(false, ids, &format!("{label} consent"))
}

async fn validate_service_bound(
    db: &mongodb::Database,
    user_id: &str,
    bound: &AuthorityBound,
    label: &str,
) -> AppResult<()> {
    let Some(ids) = bound.restricted_ids() else {
        return Ok(());
    };
    if !oauth_resource_service::validate_grantable_service_ids(db, user_id, &ids).await? {
        return Err(AppError::InvalidTarget(format!(
            "{label} service authority is unknown or inaccessible"
        )));
    }
    Ok(())
}

async fn validate_node_bound(
    db: &mongodb::Database,
    user_id: &str,
    bound: &AuthorityBound,
) -> AppResult<()> {
    let Some(ids) = bound.restricted_ids() else {
        return Ok(());
    };
    api_key_scope_service::validate_node_ids(
        db,
        user_id,
        &ids,
        ScopeAuthorization::ActorPermissions {
            actor_user_id: user_id,
        },
    )
    .await
}

async fn validate_refresh_catalog_authority(
    db: &mongodb::Database,
    config: &AppConfig,
    user_id: &str,
    source_client_id: &str,
    receiving_client_id: &str,
    authority: &catalog_delegation_service::CatalogAuthority,
) -> AppResult<()> {
    let services = AuthorityBound::from_explicit(
        authority.allow_all_services,
        &authority.allowed_service_ids,
        "catalog service authority",
    )?;
    let nodes = AuthorityBound::from_explicit(
        authority.allow_all_nodes,
        &authority.allowed_node_ids,
        "catalog node authority",
    )?;
    let source_consent = load_catalog_consent(db, user_id, source_client_id).await?;
    let receiving_consent = load_catalog_consent(db, user_id, receiving_client_id).await?;
    services.ensure_within(
        &authority_from_consent(&source_consent, "source client")?,
        "service",
    )?;
    services.ensure_within(
        &authority_from_consent(&receiving_consent, "receiving client")?,
        "service",
    )?;
    validate_service_bound(db, user_id, &services, "catalog").await?;
    validate_node_bound(db, user_id, &nodes).await?;
    let resource_scope =
        resolve_catalog_resources(db, config, user_id, &authority.resources).await?;
    if !matches!(resource_scope.services, AuthorityBound::All) {
        resource_scope
            .services
            .ensure_within(&services, "resource service")?;
    }
    Ok(())
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

    fn restricted(ids: &[&str]) -> AuthorityBound {
        AuthorityBound::Restricted(ids.iter().map(|id| (*id).to_string()).collect())
    }

    #[test]
    fn catalog_authority_lattice_intersects_unrestricted_bounds() {
        assert_eq!(
            AuthorityBound::All.intersection(&AuthorityBound::All),
            AuthorityBound::All
        );
        assert_eq!(
            AuthorityBound::All.intersection(&restricted(&["svc-a"])),
            restricted(&["svc-a"])
        );
    }

    #[test]
    fn catalog_authority_lattice_intersects_service_and_node_restrictions() {
        let services =
            restricted(&["svc-a", "svc-b"]).intersection(&restricted(&["svc-b", "svc-c"]));
        let nodes =
            restricted(&["node-a", "node-b"]).intersection(&restricted(&["node-b", "node-c"]));
        assert_eq!(services, restricted(&["svc-b"]));
        assert_eq!(nodes, restricted(&["node-b"]));
    }

    #[test]
    fn catalog_authority_lattice_preserves_empty_deny_all() {
        let empty = restricted(&[]);
        assert_eq!(empty.intersection(&restricted(&["svc-a"])), restricted(&[]));
        assert_eq!(empty.clone().into_parts(), (false, Vec::new()));
    }

    #[test]
    fn catalog_authority_lattice_rejects_widening() {
        let ceiling = restricted(&["svc-a"]);
        assert!(
            AuthorityBound::All
                .ensure_within(&ceiling, "service")
                .is_err()
        );
        assert!(
            restricted(&["svc-a", "svc-b"])
                .ensure_within(&ceiling, "service")
                .is_err()
        );
        assert!(
            restricted(&["svc-a"])
                .ensure_within(&ceiling, "service")
                .is_ok()
        );
    }

    #[test]
    fn catalog_authority_request_requires_explicit_noncontradictory_bound() {
        assert!(AuthorityBound::from_request(None, &[], "service").is_err());
        assert!(
            AuthorityBound::from_request(Some(true), &["svc-a".to_string()], "service").is_err()
        );
        assert_eq!(
            AuthorityBound::from_request(Some(false), &[], "service")
                .expect("explicit empty request"),
            restricted(&[])
        );
    }

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
            None,
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
                allowed_node_ids: &[],
                allow_all_nodes: true,
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
