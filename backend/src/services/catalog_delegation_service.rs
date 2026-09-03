use chrono::{DateTime, TimeZone, Utc};
use mongodb::bson::{self, doc};

use crate::crypto::jwt::{Claims, TokenRestrictionClaims};
use crate::errors::{AppError, AppResult};
use crate::models::catalog_delegation_grant::{
    COLLECTION_NAME as CATALOG_DELEGATION_GRANTS, CatalogDelegationGrant,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    api_key_scope_service::{self, ScopeAuthorization}, consent_service, mcp_service,
    oauth_resource_service, token_exchange_service,
};

pub const MCP_CATALOG_READ_SCOPE: &str = crate::mw::auth::MCP_CATALOG_READ_SCOPE;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogAuthority {
    pub resources: Vec<String>,
    pub allow_all_services: bool,
    pub allowed_service_ids: Vec<String>,
    pub allow_all_nodes: bool,
    pub allowed_node_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedCatalogGrant {
    grant_id: String,
    subject_user_id: String,
    actor_client_id: String,
    receiving_client_id: String,
    scope: String,
    authority: CatalogAuthority,
    expires_at: DateTime<Utc>,
}

impl VerifiedCatalogGrant {
    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }

    pub fn subject_user_id(&self) -> &str {
        &self.subject_user_id
    }

    pub fn actor_client_id(&self) -> &str {
        &self.actor_client_id
    }

    pub fn receiving_client_id(&self) -> &str {
        &self.receiving_client_id
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn authority(&self) -> &CatalogAuthority {
        &self.authority
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        subject_user_id: &str,
        actor_client_id: &str,
        receiving_client_id: &str,
        scope: &str,
        authority: CatalogAuthority,
    ) -> Self {
        Self {
            grant_id: uuid::Uuid::new_v4().to_string(),
            subject_user_id: subject_user_id.to_string(),
            actor_client_id: actor_client_id.to_string(),
            receiving_client_id: receiving_client_id.to_string(),
            scope: scope.to_string(),
            authority,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        }
    }
}

impl CatalogAuthority {
    pub fn restriction_claims(&self) -> TokenRestrictionClaims {
        TokenRestrictionClaims {
            resources: Some(self.resources.clone()),
            allowed_service_ids: Some(self.allowed_service_ids.clone()),
            allow_all_services: Some(self.allow_all_services),
            allowed_node_ids: Some(self.allowed_node_ids.clone()),
            allow_all_nodes: Some(self.allow_all_nodes),
        }
    }
}

pub fn scope_has_catalog_read(scope: &str) -> bool {
    scope
        .split_whitespace()
        .any(|value| value == MCP_CATALOG_READ_SCOPE)
}

#[allow(clippy::too_many_arguments)]
pub async fn persist_grant(
    db: &mongodb::Database,
    jti: &str,
    user_id: &str,
    actor_client_id: &str,
    receiving_client_id: &str,
    scope: &str,
    authority: &CatalogAuthority,
    expires_at: i64,
) -> AppResult<()> {
    let expires_at = Utc
        .timestamp_opt(expires_at, 0)
        .single()
        .ok_or_else(|| AppError::Internal("Invalid delegated token expiry".to_string()))?;
    let grant = CatalogDelegationGrant {
        id: jti.to_string(),
        user_id: user_id.to_string(),
        actor_client_id: actor_client_id.to_string(),
        receiving_client_id: receiving_client_id.to_string(),
        scope: scope.to_string(),
        resources: authority.resources.clone(),
        allow_all_services: authority.allow_all_services,
        allowed_service_ids: authority.allowed_service_ids.clone(),
        allow_all_nodes: authority.allow_all_nodes,
        allowed_node_ids: authority.allowed_node_ids.clone(),
        revoked: false,
        expires_at,
        created_at: Utc::now(),
    };

    db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
        .insert_one(grant)
        .await?;
    Ok(())
}

/// Validate the complete signed catalog authority against its online grant and
/// current OAuth policy. Every missing or mismatched field is deny-by-default.
pub async fn validate_live_grant(
    db: &mongodb::Database,
    config: &crate::config::AppConfig,
    claims: &Claims,
) -> AppResult<VerifiedCatalogGrant> {
    if !scope_has_catalog_read(&claims.scope) {
        return Err(invalid_catalog_authority());
    }

    let actor_client_id = claims
        .act
        .as_ref()
        .map(|actor| actor.sub.as_str())
        .ok_or_else(invalid_catalog_authority)?;
    let receiving_client_id = claims
        .client_id
        .as_deref()
        .ok_or_else(invalid_catalog_authority)?;
    let authority = authority_from_claims(claims)?;
    let now = bson::DateTime::from_chrono(Utc::now());
    let grant = db
        .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
        .find_one(doc! {
            "_id": &claims.jti,
            "revoked": false,
            "expires_at": { "$gt": now },
        })
        .await?
        .ok_or_else(invalid_catalog_authority)?;

    if !grant_matches_claims(
        &grant,
        claims,
        actor_client_id,
        receiving_client_id,
        &authority,
    ) {
        return Err(invalid_catalog_authority());
    }

    let actor_client = ensure_active_client(db, actor_client_id).await?;
    token_exchange_service::validate_delegation_scope(
        &claims.scope,
        &actor_client.delegation_scopes,
    )?;
    let receiving_client = ensure_active_client(db, receiving_client_id).await?;
    token_exchange_service::validate_delegation_scope(
        &claims.scope,
        &receiving_client.delegation_scopes,
    )?;
    ensure_consent_authority(db, &claims.sub, actor_client_id, &authority).await?;
    ensure_consent_authority(db, &claims.sub, receiving_client_id, &authority).await?;
    if !authority.allow_all_services
        && !oauth_resource_service::validate_grantable_service_ids(
            db,
            &claims.sub,
            &authority.allowed_service_ids,
        )
        .await?
    {
        return Err(invalid_catalog_authority());
    }
    if !authority.resources.is_empty() {
        let resolved = oauth_resource_service::resolve_requested_resources(
            db,
            config,
            &claims.sub,
            Some(&authority.resources),
        )
        .await?
        .ok_or_else(invalid_catalog_authority)?;
        let mut canonical_resources = resolved.resource_uris;
        if let Some(mcp_resource) = resolved.mcp_resource_uri {
            canonical_resources.push(mcp_resource);
        }
        canonical_resources.sort();
        if canonical_resources != authority.resources {
            return Err(invalid_catalog_authority());
        }
    }
    api_key_scope_service::validate_node_ids(
        db,
        &claims.sub,
        &authority.allowed_node_ids,
        ScopeAuthorization::ActorPermissions {
            actor_user_id: &claims.sub,
        },
    )
    .await?;
    Ok(VerifiedCatalogGrant {
        grant_id: grant.id,
        subject_user_id: grant.user_id,
        actor_client_id: grant.actor_client_id,
        receiving_client_id: grant.receiving_client_id,
        scope: grant.scope,
        authority,
        expires_at: grant.expires_at,
    })
}

pub fn service_scope_for_rest_request(
    auth: &AuthUser,
) -> AppResult<mcp_service::ServiceScope<'_>> {
    if auth.auth_method == AuthMethod::Delegated && scope_has_catalog_read(&auth.scope) {
        let authority = auth
            .verified_catalog_grant()
            .ok_or_else(invalid_catalog_authority)?
            .authority();
        return Ok(if authority.allow_all_services {
            mcp_service::ServiceScope::Unrestricted
        } else {
            mcp_service::ServiceScope::Allowed(authority.allowed_service_ids.as_slice())
        });
    }

    Ok(if auth.allow_all_services {
        mcp_service::ServiceScope::Unrestricted
    } else {
        mcp_service::ServiceScope::Allowed(auth.allowed_service_ids.as_slice())
    })
}

pub async fn revoke_for_client(
    db: &mongodb::Database,
    jti: &str,
    client_id: &str,
) -> AppResult<()> {
    db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
        .update_one(
            doc! { "_id": jti, "receiving_client_id": client_id, "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .await?;
    Ok(())
}

pub async fn ensure_client_can_delegate_catalog(
    db: &mongodb::Database,
    client_id: &str,
) -> AppResult<()> {
    let client = ensure_active_client(db, client_id).await?;
    token_exchange_service::validate_delegation_scope(
        MCP_CATALOG_READ_SCOPE,
        &client.delegation_scopes,
    )?;
    Ok(())
}

pub fn authority_from_claims(claims: &Claims) -> AppResult<CatalogAuthority> {
    let allow_all_services = claims
        .allow_all_services
        .ok_or_else(invalid_catalog_authority)?;
    let allowed_service_ids = claims
        .allowed_service_ids
        .clone()
        .ok_or_else(invalid_catalog_authority)?;
    let allow_all_nodes = claims
        .allow_all_nodes
        .ok_or_else(invalid_catalog_authority)?;
    let allowed_node_ids = claims
        .allowed_node_ids
        .clone()
        .ok_or_else(invalid_catalog_authority)?;
    let resources = claims
        .resources
        .clone()
        .ok_or_else(invalid_catalog_authority)?;

    if (allow_all_services && !allowed_service_ids.is_empty())
        || (allow_all_nodes && !allowed_node_ids.is_empty())
        || has_duplicates(&allowed_service_ids)
        || has_duplicates(&allowed_node_ids)
        || has_duplicates(&resources)
    {
        return Err(invalid_catalog_authority());
    }

    Ok(CatalogAuthority {
        resources,
        allow_all_services,
        allowed_service_ids,
        allow_all_nodes,
        allowed_node_ids,
    })
}

pub fn authority_from_restriction_claims(
    restrictions: &TokenRestrictionClaims,
) -> AppResult<CatalogAuthority> {
    let claims = Claims {
        sub: String::new(),
        iss: String::new(),
        aud: String::new(),
        exp: 0,
        iat: 0,
        jti: String::new(),
        scope: MCP_CATALOG_READ_SCOPE.to_string(),
        token_type: "access".to_string(),
        client_id: None,
        roles: None,
        groups: None,
        permissions: None,
        sid: None,
        act: None,
        delegated: Some(true),
        sa: None,
        cnf: None,
        relay: None,
        relay_api_key_id: None,
        relay_api_key_name: None,
        relay_allowed_service_ids: None,
        relay_allowed_node_ids: None,
        relay_allow_all_services: None,
        relay_allow_all_nodes: None,
        resources: restrictions.resources.clone(),
        allowed_service_ids: restrictions.allowed_service_ids.clone(),
        allow_all_services: restrictions.allow_all_services,
        allowed_node_ids: restrictions.allowed_node_ids.clone(),
        allow_all_nodes: restrictions.allow_all_nodes,
        assistant_forward: None,
    };
    authority_from_claims(&claims)
}

async fn ensure_active_client(db: &mongodb::Database, client_id: &str) -> AppResult<OauthClient> {
    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id, "is_active": true })
        .await?
        .ok_or_else(invalid_catalog_authority)
}

async fn ensure_consent_authority(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    authority: &CatalogAuthority,
) -> AppResult<()> {
    let Some(consent) = consent_service::check_consent(db, user_id, client_id, "openid").await?
    else {
        return Err(invalid_catalog_authority());
    };
    if consent.allow_all_services {
        return Ok(());
    }
    let Some(consent_service_ids) = consent.allowed_service_ids else {
        return Err(invalid_catalog_authority());
    };
    if authority.allow_all_services
        || !authority.allowed_service_ids.iter().all(|service_id| {
            consent_service_ids
                .iter()
                .any(|allowed| allowed == service_id)
        })
    {
        return Err(invalid_catalog_authority());
    }
    Ok(())
}

fn grant_matches_claims(
    grant: &CatalogDelegationGrant,
    claims: &Claims,
    actor_client_id: &str,
    receiving_client_id: &str,
    authority: &CatalogAuthority,
) -> bool {
    grant.user_id == claims.sub
        && grant.actor_client_id == actor_client_id
        && grant.receiving_client_id == receiving_client_id
        && grant.scope == claims.scope
        && grant.resources == authority.resources
        && grant.allow_all_services == authority.allow_all_services
        && grant.allowed_service_ids == authority.allowed_service_ids
        && grant.allow_all_nodes == authority.allow_all_nodes
        && grant.allowed_node_ids == authority.allowed_node_ids
}

fn has_duplicates(values: &[String]) -> bool {
    let unique: std::collections::HashSet<&str> = values.iter().map(String::as_str).collect();
    unique.len() != values.len()
}

fn invalid_catalog_authority() -> AppError {
    AppError::Unauthorized("Delegated catalog authority is invalid or inactive".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    async fn live_grant_fixture() -> Option<(mongodb::Database, crate::config::AppConfig, Claims)> {
        let db = crate::test_utils::connect_test_database("catalog_delegation_live_grant").await?;
        let config = crate::test_utils::test_app_config();
        let actor_client_id = "catalog-live-actor";
        let receiving_client_id = "catalog-live-receiver";
        let claims = Claims {
            sub: uuid::Uuid::new_v4().to_string(),
            iss: config.jwt_issuer.clone(),
            aud: config.jwt_issuer.clone(),
            exp: Utc::now().timestamp() + 300,
            iat: Utc::now().timestamp(),
            jti: uuid::Uuid::new_v4().to_string(),
            scope: MCP_CATALOG_READ_SCOPE.to_string(),
            token_type: "access".to_string(),
            client_id: Some(receiving_client_id.to_string()),
            roles: None,
            groups: None,
            permissions: None,
            sid: None,
            act: Some(crate::crypto::jwt::ActorClaim {
                sub: actor_client_id.to_string(),
            }),
            delegated: Some(true),
            sa: None,
            cnf: None,
            relay: None,
            relay_api_key_id: None,
            relay_api_key_name: None,
            relay_allowed_service_ids: None,
            relay_allowed_node_ids: None,
            relay_allow_all_services: None,
            relay_allow_all_nodes: None,
            resources: Some(Vec::new()),
            allowed_service_ids: Some(Vec::new()),
            allow_all_services: Some(false),
            allowed_node_ids: Some(Vec::new()),
            allow_all_nodes: Some(false),
            assistant_forward: None,
        };

        let now = Utc::now();
        for client_id in [actor_client_id, receiving_client_id] {
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_one(OauthClient {
                    id: client_id.to_string(),
                    client_name: client_id.to_string(),
                    client_secret_hash: String::new(),
                    redirect_uris: vec!["https://example.com/callback".to_string()],
                    allowed_scopes: "openid".to_string(),
                    scope_provenance: Default::default(),
                    grant_types: "authorization_code".to_string(),
                    client_type: "confidential".to_string(),
                    is_active: true,
                    delegation_scopes: MCP_CATALOG_READ_SCOPE.to_string(),
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
                .expect("insert OAuth client");
            consent_service::grant_consent_with_services(
                &db,
                &claims.sub,
                client_id,
                "openid",
                Some(Vec::new()),
            )
            .await
            .expect("grant catalog consent");
        }

        let authority = authority_from_claims(&claims).expect("explicit catalog authority");
        persist_grant(
            &db,
            &claims.jti,
            &claims.sub,
            actor_client_id,
            receiving_client_id,
            &claims.scope,
            &authority,
            claims.exp,
        )
        .await
        .expect("persist catalog grant");
        let proof = validate_live_grant(&db, &config, &claims)
            .await
            .expect("active catalog grant");
        assert_eq!(proof.grant_id(), claims.jti);
        assert_eq!(proof.subject_user_id(), claims.sub);
        assert_eq!(proof.actor_client_id(), actor_client_id);
        assert_eq!(proof.receiving_client_id(), receiving_client_id);
        assert_eq!(proof.scope(), claims.scope);
        assert_eq!(proof.authority(), &authority);
        assert_eq!(proof.expires_at().timestamp(), claims.exp);

        Some((db, config, claims))
    }

    fn assert_invalid<T>(result: AppResult<T>) {
        assert!(matches!(
            result,
            Err(AppError::Unauthorized(message))
                if message == "Delegated catalog authority is invalid or inactive"
        ));
    }

    #[tokio::test]
    async fn validate_live_grant_rejects_expired_grant() {
        let Some((db, config, claims)) = live_grant_fixture().await else {
            return;
        };
        db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .update_one(
                doc! { "_id": &claims.jti },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } },
            )
            .await
            .expect("expire catalog grant");

        assert_invalid(validate_live_grant(&db, &config, &claims).await);
    }

    #[tokio::test]
    async fn validate_live_grant_rejects_revoked_grant() {
        let Some((db, config, claims)) = live_grant_fixture().await else {
            return;
        };
        db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .update_one(
                doc! { "_id": &claims.jti },
                doc! { "$set": { "revoked": true } },
            )
            .await
            .expect("revoke catalog grant");

        assert_invalid(validate_live_grant(&db, &config, &claims).await);
    }

    #[tokio::test]
    async fn validate_live_grant_rejects_disabled_client() {
        let Some((db, config, claims)) = live_grant_fixture().await else {
            return;
        };
        let receiving_client_id = claims.client_id.as_deref().expect("receiving client");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .update_one(
                doc! { "_id": receiving_client_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("disable OAuth client");

        assert_invalid(validate_live_grant(&db, &config, &claims).await);
    }

    #[tokio::test]
    async fn validate_live_grant_rejects_removed_consent() {
        let Some((db, config, claims)) = live_grant_fixture().await else {
            return;
        };
        let actor_client_id = claims
            .act
            .as_ref()
            .map(|actor| actor.sub.as_str())
            .expect("acting client");
        consent_service::revoke_consent(&db, &claims.sub, actor_client_id)
            .await
            .expect("remove actor consent");

        assert_invalid(validate_live_grant(&db, &config, &claims).await);
    }

    fn claims() -> Claims {
        Claims {
            sub: uuid::Uuid::new_v4().to_string(),
            iss: "https://nyx.example".to_string(),
            aud: "https://nyx.example".to_string(),
            exp: Utc::now().timestamp() + 300,
            iat: Utc::now().timestamp(),
            jti: uuid::Uuid::new_v4().to_string(),
            scope: MCP_CATALOG_READ_SCOPE.to_string(),
            token_type: "access".to_string(),
            client_id: Some("aevatar".to_string()),
            roles: None,
            groups: None,
            permissions: None,
            sid: None,
            act: Some(crate::crypto::jwt::ActorClaim {
                sub: "nyxid-assistant".to_string(),
            }),
            delegated: Some(true),
            sa: None,
            cnf: None,
            relay: None,
            relay_api_key_id: None,
            relay_api_key_name: None,
            relay_allowed_service_ids: None,
            relay_allowed_node_ids: None,
            relay_allow_all_services: None,
            relay_allow_all_nodes: None,
            resources: Some(Vec::new()),
            allowed_service_ids: Some(Vec::new()),
            allow_all_services: Some(false),
            allowed_node_ids: Some(Vec::new()),
            allow_all_nodes: Some(false),
            assistant_forward: None,
        }
    }

    #[test]
    fn explicit_empty_sets_remain_deny_all() {
        let authority = authority_from_claims(&claims()).expect("explicit authority");
        assert!(!authority.allow_all_services);
        assert!(authority.allowed_service_ids.is_empty());
        assert!(!authority.allow_all_nodes);
        assert!(authority.allowed_node_ids.is_empty());
    }

    fn verified_grant(authority: CatalogAuthority) -> VerifiedCatalogGrant {
        VerifiedCatalogGrant::for_test(
            "00000000-0000-4000-8000-000000000001",
            "assistant-client",
            "assistant-client",
            MCP_CATALOG_READ_SCOPE,
            authority,
        )
    }

    #[test]
    fn rest_service_scope_uses_verified_delegated_authority() {
        let mut auth = crate::test_utils::test_auth_user(
            "00000000-0000-4000-8000-000000000001",
        );
        auth.auth_method = AuthMethod::Delegated;
        auth.scope = format!("proxy:* {MCP_CATALOG_READ_SCOPE}");
        auth.allow_all_services = true;
        auth.verified_catalog_grant = Some(verified_grant(CatalogAuthority {
            resources: Vec::new(),
            allow_all_services: false,
            allowed_service_ids: vec!["service-from-proof".to_string()],
            allow_all_nodes: true,
            allowed_node_ids: Vec::new(),
        }));

        match service_scope_for_rest_request(&auth).expect("verified scope") {
            mcp_service::ServiceScope::Allowed(ids) => {
                assert_eq!(ids, ["service-from-proof"]);
            }
            mcp_service::ServiceScope::Unrestricted => panic!("proof restriction was ignored"),
        }
    }

    #[test]
    fn rest_service_scope_preserves_every_other_auth_method() {
        for method in [
            AuthMethod::Session,
            AuthMethod::AccessToken,
            AuthMethod::ApiKey,
            AuthMethod::ServiceAccount,
            AuthMethod::Relay,
            AuthMethod::Delegated,
        ] {
            let mut auth = crate::test_utils::test_auth_user(
                "00000000-0000-4000-8000-000000000001",
            );
            auth.auth_method = method.clone();
            auth.scope = "proxy:*".to_string();
            auth.allow_all_services = false;
            auth.allowed_service_ids = vec!["existing-service".to_string()];

            match service_scope_for_rest_request(&auth).expect("existing service scope") {
                mcp_service::ServiceScope::Allowed(ids) => {
                    assert_eq!(ids, ["existing-service"], "{method:?}");
                }
                mcp_service::ServiceScope::Unrestricted => {
                    panic!("{method:?} lost its existing service restriction")
                }
            }
        }
    }

    #[test]
    fn delegated_catalog_scope_without_verified_grant_fails_closed() {
        let mut auth = crate::test_utils::test_auth_user(
            "00000000-0000-4000-8000-000000000001",
        );
        auth.auth_method = AuthMethod::Delegated;
        auth.scope = format!("proxy:* {MCP_CATALOG_READ_SCOPE}");

        assert!(service_scope_for_rest_request(&auth).is_err());
    }

    #[test]
    fn unrestricted_verified_authority_remains_unrestricted() {
        let mut auth = crate::test_utils::test_auth_user(
            "00000000-0000-4000-8000-000000000001",
        );
        auth.auth_method = AuthMethod::Delegated;
        auth.scope = format!("proxy:* {MCP_CATALOG_READ_SCOPE}");
        auth.allow_all_services = false;
        auth.verified_catalog_grant = Some(verified_grant(CatalogAuthority {
            resources: Vec::new(),
            allow_all_services: true,
            allowed_service_ids: Vec::new(),
            allow_all_nodes: true,
            allowed_node_ids: Vec::new(),
        }));

        assert!(matches!(
            service_scope_for_rest_request(&auth),
            Ok(mcp_service::ServiceScope::Unrestricted)
        ));
    }

    #[test]
    fn missing_or_inconsistent_authority_fails_closed() {
        let mut missing = claims();
        missing.allow_all_nodes = None;
        assert!(authority_from_claims(&missing).is_err());

        let mut inconsistent = claims();
        inconsistent.allow_all_services = Some(true);
        inconsistent.allowed_service_ids = Some(vec!["svc-1".to_string()]);
        assert!(authority_from_claims(&inconsistent).is_err());
    }

    #[test]
    fn grant_snapshot_rejects_wrong_actor_or_receiver() {
        let claims = claims();
        let authority = authority_from_claims(&claims).expect("explicit authority");
        let grant = CatalogDelegationGrant {
            id: claims.jti.clone(),
            user_id: claims.sub.clone(),
            actor_client_id: "nyxid-assistant".to_string(),
            receiving_client_id: "aevatar".to_string(),
            scope: claims.scope.clone(),
            resources: authority.resources.clone(),
            allow_all_services: authority.allow_all_services,
            allowed_service_ids: authority.allowed_service_ids.clone(),
            allow_all_nodes: authority.allow_all_nodes,
            allowed_node_ids: authority.allowed_node_ids.clone(),
            revoked: false,
            expires_at: Utc::now(),
            created_at: Utc::now(),
        };

        assert!(grant_matches_claims(
            &grant,
            &claims,
            "nyxid-assistant",
            "aevatar",
            &authority,
        ));
        assert!(!grant_matches_claims(
            &grant,
            &claims,
            "wrong-actor",
            "aevatar",
            &authority,
        ));
        assert!(!grant_matches_claims(
            &grant,
            &claims,
            "nyxid-assistant",
            "wrong-receiver",
            &authority,
        ));
    }
}
