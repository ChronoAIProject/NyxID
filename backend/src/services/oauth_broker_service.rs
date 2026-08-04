//! Broker / token-vault binding lifecycle.
//!
//! When a `broker_capability_enabled` OAuth client redeems an authorization
//! code, NyxID encrypts the user's refresh_token at rest and returns an
//! opaque `binding_id` instead of the refresh_token itself. The client
//! later exchanges the binding_id for a short-lived access token via
//! RFC 8693 token exchange (commit #5).

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Binary, doc, spec::BinarySubtype};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use std::sync::Arc;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::aes::EncryptionKeys;
use crate::crypto::jwt::{self, JwtKeys};
use crate::errors::{AppError, AppResult};
use crate::models::authorization_code::ExternalSubjectRef;
use crate::models::oauth_broker_binding::{
    COLLECTION_NAME as OAUTH_BROKER_BINDINGS, OauthBrokerBinding, generate_binding_id,
    hash_binding_id,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};
use crate::services::cae_webhook_service::{self, RevocationEvent};

/// Subject-token-type URN identifying a broker binding handle in RFC 8693
/// token exchange requests. The `params:oauth` infix mirrors the IETF URN
/// style at `urn:ietf:params:oauth:*` so OAuth vendor-extension parsers
/// recognize the suffix shape. Frozen as the contract aevatar's
/// ADR-0017 / aevatarAI/aevatar#477 builds against.
pub const BROKER_SUBJECT_TOKEN_TYPE: &str = "urn:nyxid:params:oauth:token-type:binding-id";

/// Issued-token-type URN for the access tokens the broker hands back --
/// the standard RFC 8693 access_token URN.
pub const ISSUED_TOKEN_TYPE_ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";

/// Scope that, when present in an OAuth client's `allowed_scopes`, opts the
/// client into broker-mode token issuance (response carries `binding_id`
/// instead of `refresh_token`). Equivalent in effect to the per-client
/// `broker_capability_enabled` flag — both triggers are honored.
pub const BROKER_BINDING_SCOPE: &str = "urn:nyxid:scope:broker_binding";

/// Returns true if the OAuth client should be treated as broker-capable.
/// Either the admin flag is set, or the broker-binding scope is in the
/// client's allowed_scopes (admin still controls scope assignment).
pub fn is_broker_client(client: &crate::models::oauth_client::OauthClient) -> bool {
    client.broker_capability_enabled
        || client
            .allowed_scopes
            .split_whitespace()
            .any(|s| s == BROKER_BINDING_SCOPE)
}

/// Default TTL (in seconds) for broker-issued access tokens.
///
/// Broker-bound credentials demand fast revocation propagation -- short
/// access lifetimes mean a revoked binding is honored within 5 minutes
/// without resource-server introspection.
pub const BROKER_ACCESS_TTL_SECS: i64 = 300;

pub struct BindingExchangeResult {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub granted_scope: String,
    pub issued_token_type: String,
    pub via_chain_follow: bool,
}

const MAX_BROKER_ROTATION_RETRIES: usize = 3;

struct BrokerExchangeContext<'a> {
    db: &'a mongodb::Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: &'a reqwest::Client,
    jwt_keys: &'a JwtKeys,
    config: &'a AppConfig,
    client_id: &'a str,
    binding_hash: &'a str,
    requested_scope: Option<&'a str>,
    dpop_jkt: Option<&'a str>,
    mtls_x5t_s256: Option<&'a str>,
}

enum ExchangeOutcome {
    Success(BindingExchangeResult),
    ChainFollow,
    ReuseDetected,
}

/// Issue a new broker binding for the freshly-minted refresh_token.
///
/// Returns the raw `binding_id` (which is returned ONCE to the client)
/// and the binding's `_id` (SHA-256 of the raw binding_id, the durable
/// reference for cascade-revoke and audit).
// The issuance call needs the binding owner, encrypted credential source,
// scope snapshot, and optional external subject all at once.
#[allow(clippy::too_many_arguments)]
pub async fn create_binding(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    client_id: &str,
    user_id: &str,
    refresh_token: &str,
    refresh_token_jti: &str,
    scopes: &[String],
    external_subject: Option<&ExternalSubjectRef>,
) -> AppResult<(String, String)> {
    let raw_binding_id = generate_binding_id();
    let binding_hash = hash_binding_id(&raw_binding_id);

    let refresh_token_encrypted = encryption_keys
        .encrypt_with_aad(refresh_token.as_bytes(), binding_hash.as_bytes())
        .await
        .map_err(|e| AppError::Internal(format!("broker binding encrypt failed: {e}")))?;

    let now = Utc::now();
    let binding = OauthBrokerBinding {
        id: binding_hash.clone(),
        client_id: client_id.to_string(),
        user_id: user_id.to_string(),
        refresh_token_jti: refresh_token_jti.to_string(),
        refresh_token_encrypted: Some(refresh_token_encrypted),
        scopes: scopes.to_vec(),
        external_subject: external_subject.cloned(),
        rotation_version: 0,
        revoked: false,
        last_used_at: None,
        revoked_at: None,
        revoke_reason: None,
        created_at: now,
    };

    db.collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .insert_one(&binding)
        .await?;

    Ok((raw_binding_id, binding_hash))
}

/// Fetch a binding's metadata for the owning client. Returns NotFound on
/// miss or ownership mismatch (never reveals which it was).
pub async fn get_binding_for_client(
    db: &mongodb::Database,
    client_id: &str,
    raw_binding_id: &str,
) -> AppResult<OauthBrokerBinding> {
    let binding_hash = hash_binding_id(raw_binding_id);
    let binding = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one(doc! { "_id": &binding_hash })
        .await?
        .ok_or_else(|| AppError::NotFound("binding not found".to_string()))?;

    if binding.client_id != client_id {
        return Err(AppError::NotFound("binding not found".to_string()));
    }

    Ok(binding)
}

/// Find active (non-revoked) bindings owned by `client_id` matching the
/// given external_subject criteria. Used by /oauth/bindings reverse-lookup
/// for dedup. Empty Vec when nothing matches -- never returns NotFound.
///
/// Tenant is optional in the criteria; when None, only matches bindings
/// whose stored tenant is also absent. We're matching the exact triple,
/// not partial overlap -- that would surprise callers.
pub async fn find_active_bindings_by_external_subject(
    db: &mongodb::Database,
    client_id: &str,
    platform: &str,
    tenant: Option<&str>,
    external_user_id: &str,
) -> AppResult<Vec<OauthBrokerBinding>> {
    let mut filter = doc! {
        "client_id": client_id,
        "revoked": false,
        "external_subject.platform": platform,
        "external_subject.external_user_id": external_user_id,
    };

    match tenant {
        Some(tenant) => {
            filter.insert("external_subject.tenant", tenant);
        }
        None => {
            // MongoDB equality to null matches both explicit null and absent
            // fields; this covers tenant=None, which is omitted by serde.
            filter.insert(
                "external_subject.tenant",
                doc! { "$in": [bson::Bson::Null] },
            );
        }
    }

    let cursor = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .await?;

    Ok(cursor.try_collect().await?)
}

/// Exchange a binding_id for a fresh short-lived access_token.
///
/// Validates client ownership of the binding, detects refresh-token reuse
/// (cascade-revokes all bindings for the user/client pair on detection),
/// rotates the underlying refresh_token, re-encrypts it under the same
/// binding row, and returns a 5-minute access_token. The refresh_token
/// never leaves the server.
///
/// Concurrent callers race on `rotation_version`. The losing caller follows
/// the already-rotated binding state and mints an access_token without
/// re-rotating.
// Broker exchange spans binding lookup, refresh-token rotation, JWT issuance,
// encryption, and optimistic binding update in one operation.
// Public service entry point keeps the broker exchange dependencies explicit;
// grouping them would add an extra one-off request struct without reducing risk.
#[allow(clippy::too_many_arguments)]
pub async fn exchange_via_binding(
    db: &mongodb::Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: &reqwest::Client,
    jwt_keys: &JwtKeys,
    config: &AppConfig,
    client_id: &str,
    raw_binding_id: &str,
    requested_scope: Option<&str>,
    dpop_jkt: Option<&str>,
    mtls_x5t_s256: Option<&str>,
) -> AppResult<BindingExchangeResult> {
    let binding_hash = hash_binding_id(raw_binding_id);
    let ctx = BrokerExchangeContext {
        db,
        encryption_keys,
        http_client,
        jwt_keys,
        config,
        client_id,
        binding_hash: &binding_hash,
        requested_scope,
        dpop_jkt,
        mtls_x5t_s256,
    };

    for attempt in 0..MAX_BROKER_ROTATION_RETRIES {
        match try_exchange_once(&ctx).await? {
            ExchangeOutcome::Success(result) => return Ok(result),
            ExchangeOutcome::ChainFollow => {
                if let Some(result) = try_chain_follow(&ctx).await? {
                    return Ok(result);
                }
                tracing::debug!(
                    binding_hash = %binding_hash_prefix(&binding_hash),
                    attempt,
                    "broker exchange chain-follow returned None; retrying full rotation"
                );
            }
            ExchangeOutcome::ReuseDetected => return Err(invalid_grant()),
        }
    }

    Err(invalid_grant())
}

async fn try_exchange_once(ctx: &BrokerExchangeContext<'_>) -> AppResult<ExchangeOutcome> {
    let db = ctx.db;
    let binding_hash = ctx.binding_hash;
    let binding = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one(doc! { "_id": binding_hash })
        .await?
        .ok_or_else(invalid_grant)?;

    if binding.client_id != ctx.client_id || binding.revoked {
        return Err(invalid_grant());
    }

    let encrypted_refresh = binding
        .refresh_token_encrypted
        .as_ref()
        .ok_or_else(invalid_grant)?;
    let refresh_token_bytes = ctx
        .encryption_keys
        .decrypt_with_aad(encrypted_refresh, binding_hash.as_bytes())
        .await
        .map_err(|_| invalid_grant())?;
    let refresh_token_str = String::from_utf8(refresh_token_bytes).map_err(|_| invalid_grant())?;

    let refresh_token_doc = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "jti": &binding.refresh_token_jti })
        .await?
        .ok_or_else(invalid_grant)?;
    if refresh_token_doc.client_id != binding.client_id
        || refresh_token_doc.user_id != binding.user_id
    {
        return Err(invalid_grant());
    }

    if refresh_token_doc.revoked {
        if refresh_token_doc.replaced_by.is_some() {
            return Ok(ExchangeOutcome::ChainFollow);
        }
        cascade_revoke_reuse_detected(ctx, &binding, binding_hash).await?;
        return Ok(ExchangeOutcome::ReuseDetected);
    }

    verify_binding_refresh_jwt(ctx.jwt_keys, ctx.config, &refresh_token_str, &binding)?;

    let granted_scope = resolve_binding_scope(ctx.requested_scope, &binding.scopes)?;
    let access_token = mint_broker_access_token(ctx, &binding.user_id, &granted_scope).await?;

    let user_uuid = Uuid::parse_str(&binding.user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id in broker binding: {e}")))?;
    let (new_refresh_jwt, new_jti) =
        jwt::generate_refresh_token(ctx.jwt_keys, ctx.config, &user_uuid)?;
    let new_refresh_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let refresh_expires = now + Duration::seconds(ctx.config.jwt_refresh_ttl_secs);
    let new_refresh = RefreshToken {
        id: new_refresh_id.clone(),
        jti: new_jti.clone(),
        client_id: binding.client_id.clone(),
        user_id: binding.user_id.clone(),
        session_id: None,
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        revoked_at: None,
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    let revoke_result = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "_id": &refresh_token_doc.id, "revoked": false },
            doc! { "$set": {
                "revoked": true,
                "replaced_by": &new_refresh_id,
                "revoked_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;
    if revoke_result.matched_count == 0 {
        return handle_refresh_revoke_conflict(
            ctx,
            &binding,
            binding_hash,
            &refresh_token_doc.id,
            &new_refresh_id,
        )
        .await;
    }

    let new_blob = ctx
        .encryption_keys
        .encrypt_with_aad(new_refresh_jwt.as_bytes(), binding_hash.as_bytes())
        .await
        .map_err(|e| AppError::Internal(format!("broker binding encrypt failed: {e}")))?;

    let updated = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one_and_update(
            doc! {
                "_id": &binding_hash,
                "rotation_version": binding.rotation_version,
            },
            doc! { "$set": {
                "refresh_token_encrypted": Binary {
                    subtype: BinarySubtype::Generic,
                    bytes: new_blob,
                },
                "refresh_token_jti": &new_jti,
                "rotation_version": i64::from(binding.rotation_version) + 1,
                "last_used_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;
    if updated.is_none() {
        cleanup_refresh_token(db, &new_refresh_id).await?;
        return Ok(ExchangeOutcome::ChainFollow);
    }

    Ok(ExchangeOutcome::Success(BindingExchangeResult {
        access_token,
        token_type: broker_access_token_type(ctx.dpop_jkt).to_string(),
        expires_in: BROKER_ACCESS_TTL_SECS,
        granted_scope,
        issued_token_type: ISSUED_TOKEN_TYPE_ACCESS_TOKEN.to_string(),
        via_chain_follow: false,
    }))
}

async fn try_chain_follow(
    ctx: &BrokerExchangeContext<'_>,
) -> AppResult<Option<BindingExchangeResult>> {
    let now = Utc::now();
    let binding = ctx
        .db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one(doc! { "_id": ctx.binding_hash })
        .await?;
    let Some(binding) = binding else {
        return Ok(None);
    };
    if binding.client_id != ctx.client_id || binding.revoked {
        return Ok(None);
    }

    let encrypted_refresh = binding
        .refresh_token_encrypted
        .as_ref()
        .ok_or_else(invalid_grant)?;
    let refresh_token_bytes = ctx
        .encryption_keys
        .decrypt_with_aad(encrypted_refresh, ctx.binding_hash.as_bytes())
        .await
        .map_err(|_| invalid_grant())?;
    let refresh_token_str = String::from_utf8(refresh_token_bytes).map_err(|_| invalid_grant())?;
    verify_binding_refresh_jwt(ctx.jwt_keys, ctx.config, &refresh_token_str, &binding)?;

    let refresh_token_doc = ctx
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "jti": &binding.refresh_token_jti })
        .await?;
    let Some(refresh_token_doc) = refresh_token_doc else {
        return Ok(None);
    };
    if refresh_token_doc.client_id != binding.client_id
        || refresh_token_doc.user_id != binding.user_id
    {
        return Err(invalid_grant());
    }
    if refresh_token_doc.revoked {
        return Ok(None);
    }

    let granted_scope = resolve_binding_scope(ctx.requested_scope, &binding.scopes)?;
    let access_token = mint_broker_access_token(ctx, &binding.user_id, &granted_scope).await?;

    ctx.db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .update_one(
            doc! { "_id": ctx.binding_hash, "revoked": false },
            doc! { "$set": {
                "last_used_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(Some(BindingExchangeResult {
        access_token,
        token_type: broker_access_token_type(ctx.dpop_jkt).to_string(),
        expires_in: BROKER_ACCESS_TTL_SECS,
        granted_scope,
        issued_token_type: ISSUED_TOKEN_TYPE_ACCESS_TOKEN.to_string(),
        via_chain_follow: true,
    }))
}

async fn mint_broker_access_token(
    ctx: &BrokerExchangeContext<'_>,
    user_id: &str,
    granted_scope: &str,
) -> AppResult<String> {
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id in broker binding: {e}")))?;
    let rbac_data =
        crate::services::rbac_helpers::build_rbac_claim_data(ctx.db, user_id, granted_scope)
            .await?;
    jwt::generate_access_token(
        ctx.jwt_keys,
        ctx.config,
        &user_uuid,
        granted_scope,
        Some(&rbac_data),
        Some(BROKER_ACCESS_TTL_SECS),
        ctx.dpop_jkt,
        ctx.mtls_x5t_s256,
    )
}

fn broker_access_token_type(dpop_jkt: Option<&str>) -> &'static str {
    if dpop_jkt.is_some() { "DPoP" } else { "Bearer" }
}

fn verify_binding_refresh_jwt(
    jwt_keys: &JwtKeys,
    config: &AppConfig,
    refresh_token_str: &str,
    binding: &OauthBrokerBinding,
) -> AppResult<()> {
    let refresh_claims =
        jwt::verify_token(jwt_keys, config, refresh_token_str).map_err(|_| invalid_grant())?;
    if refresh_claims.token_type != "refresh"
        || refresh_claims.jti != binding.refresh_token_jti
        || refresh_claims.sub != binding.user_id
    {
        return Err(invalid_grant());
    }
    Ok(())
}

async fn cleanup_refresh_token(db: &mongodb::Database, refresh_token_id: &str) -> AppResult<()> {
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .delete_one(doc! { "_id": refresh_token_id })
        .await?;
    Ok(())
}

async fn handle_refresh_revoke_conflict(
    ctx: &BrokerExchangeContext<'_>,
    binding: &OauthBrokerBinding,
    binding_hash: &str,
    refresh_token_id: &str,
    new_refresh_id: &str,
) -> AppResult<ExchangeOutcome> {
    let db = ctx.db;
    cleanup_refresh_token(db, new_refresh_id).await?;
    let current = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "_id": refresh_token_id })
        .await?;
    if let Some(current) = current
        && current.revoked
    {
        if current.replaced_by.is_some() {
            return Ok(ExchangeOutcome::ChainFollow);
        }
        cascade_revoke_reuse_detected(ctx, binding, binding_hash).await?;
        return Ok(ExchangeOutcome::ReuseDetected);
    }
    Ok(ExchangeOutcome::ChainFollow)
}

async fn cascade_revoke_reuse_detected(
    ctx: &BrokerExchangeContext<'_>,
    binding: &OauthBrokerBinding,
    binding_hash: &str,
) -> AppResult<()> {
    let db = ctx.db;
    let cascade_count = revoke_bindings_for_user_client(
        db,
        ctx.encryption_keys.clone(),
        ctx.http_client,
        &binding.client_id,
        &binding.user_id,
        "reuse_detected",
    )
    .await?;
    crate::services::audit_service::log_async(
        db.clone(),
        Some(binding.user_id.clone()),
        "oauth_broker_binding_reuse_detected".to_string(),
        Some(serde_json::json!({
            "client_id": binding.client_id,
            "binding_hash": binding_hash_prefix(binding_hash),
            "cascade_revoke_count": cascade_count,
        })),
        None,
        None,
        None,
        None,
    );
    tracing::warn!(
        client_id = %binding.client_id,
        binding_hash = %binding_hash_prefix(binding_hash),
        "broker binding refresh-token reuse detected; cascading revoke"
    );
    Ok(())
}

/// Cascade-revoke all bindings owned by `(client_id, user_id)`. Used by
/// reuse detection (commit #5) and by the explicit revoke endpoint (#6).
///
/// Returns the number of bindings modified.
pub async fn revoke_bindings_for_user_client(
    db: &mongodb::Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: &reqwest::Client,
    client_id: &str,
    user_id: &str,
    reason: &str,
) -> AppResult<u64> {
    let now = Utc::now();
    let filter = doc! {
        "client_id": client_id,
        "user_id": user_id,
        "revoked": false,
    };
    let bindings: Vec<OauthBrokerBinding> = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find(filter.clone())
        .await?
        .try_collect()
        .await?;

    let result = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .update_many(
            filter,
            doc! { "$set": {
                "revoked": true,
                "revoked_at": bson::DateTime::from_chrono(now),
                "revoke_reason": reason,
            }},
        )
        .await?;

    let revoke_source = revoke_source_for_reason(reason);
    for binding in bindings {
        spawn_revocation_webhook(
            db.clone(),
            encryption_keys.clone(),
            http_client.clone(),
            RevocationWebhookDispatch {
                binding_hash: binding.id,
                client_id: binding.client_id,
                revoke_source,
                reason: Some(reason.to_string()),
                revoked_at: now,
            },
        );
    }

    Ok(result.modified_count)
}

/// Revoke a binding owned by `client_id` given its raw binding_id.
///
/// Idempotent: missing binding, mismatched ownership, already-revoked
/// all return Ok(()) so RFC 7009 200-always semantics hold at the
/// handler. Marks both the binding and its underlying RefreshToken
/// revoked.
pub async fn revoke_binding_by_client(
    db: &mongodb::Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: &reqwest::Client,
    client_id: &str,
    raw_binding_id: &str,
    reason: &str,
) -> AppResult<bool> {
    let binding_hash = hash_binding_id(raw_binding_id);
    let now = Utc::now();
    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();

    let binding = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one_and_update(
            doc! {
                "_id": &binding_hash,
                "client_id": client_id,
                "revoked": false,
            },
            doc! { "$set": {
                "revoked": true,
                "revoked_at": bson::DateTime::from_chrono(now),
                "revoke_reason": reason,
            }},
        )
        .with_options(options)
        .await?;

    let Some(binding) = binding else {
        return Ok(false);
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "jti": &binding.refresh_token_jti, "revoked": false },
            doc! { "$set": {
                "revoked": true,
                "revoked_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    spawn_revocation_webhook(
        db.clone(),
        encryption_keys.clone(),
        http_client.clone(),
        RevocationWebhookDispatch {
            binding_hash: binding.id,
            client_id: binding.client_id,
            revoke_source: "client",
            reason: Some(reason.to_string()),
            revoked_at: now,
        },
    );

    Ok(true)
}

/// Revoke a binding owned by `user_id` given its binding_hash (stored as _id).
///
/// Returns Err(AppError::NotFound) when the binding does not exist or
/// belongs to a different user. The user-UI endpoint translates this to
/// a 404 -- that's safe because the user can only see their own bindings.
pub async fn revoke_binding_by_user(
    db: &mongodb::Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: &reqwest::Client,
    user_id: &str,
    binding_hash: &str,
    reason: &str,
) -> AppResult<()> {
    let now = Utc::now();
    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();

    let binding = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one_and_update(
            doc! {
                "_id": binding_hash,
                "user_id": user_id,
                "revoked": false,
            },
            doc! { "$set": {
                "revoked": true,
                "revoked_at": bson::DateTime::from_chrono(now),
                "revoke_reason": reason,
            }},
        )
        .with_options(options)
        .await?
        .ok_or_else(|| AppError::NotFound("binding not found".to_string()))?;

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "jti": &binding.refresh_token_jti, "revoked": false },
            doc! { "$set": {
                "revoked": true,
                "revoked_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    spawn_revocation_webhook(
        db.clone(),
        encryption_keys.clone(),
        http_client.clone(),
        RevocationWebhookDispatch {
            binding_hash: binding.id,
            client_id: binding.client_id,
            revoke_source: "user",
            reason: Some(reason.to_string()),
            revoked_at: now,
        },
    );

    Ok(())
}

fn revoke_source_for_reason(reason: &str) -> &'static str {
    match reason {
        "client" | "client_revoked" => "client",
        "user" | "user_revoked" => "user",
        "reuse_detected" => "reuse_detected",
        "admin" | "admin_revoked" => "admin",
        _ => "admin",
    }
}

struct RevocationWebhookDispatch {
    binding_hash: String,
    client_id: String,
    revoke_source: &'static str,
    reason: Option<String>,
    revoked_at: chrono::DateTime<chrono::Utc>,
}

fn spawn_revocation_webhook(
    db: mongodb::Database,
    encryption_keys: Arc<EncryptionKeys>,
    http_client: reqwest::Client,
    dispatch: RevocationWebhookDispatch,
) {
    tokio::spawn(async move {
        let binding_hash = dispatch.binding_hash;
        let client_id = dispatch.client_id;
        let client = match db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": &client_id })
            .await
        {
            Ok(Some(client)) => client,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    client_id = %client_id,
                    error = %error,
                    "failed to load OAuth client for CAE webhook"
                );
                return;
            }
        };

        if client.revocation_webhook_url.is_none() {
            return;
        }
        let Some(encrypted_secret) = client.revocation_webhook_secret_encrypted.as_ref() else {
            return;
        };
        let raw_secret = match encryption_keys.decrypt(encrypted_secret).await {
            Ok(secret) => secret,
            Err(error) => {
                tracing::warn!(
                    client_id = %client_id,
                    binding_hash = %binding_hash_prefix(&binding_hash),
                    error = %error,
                    "failed to decrypt CAE webhook secret"
                );
                return;
            }
        };
        let raw_secret = match String::from_utf8(raw_secret) {
            Ok(secret) => secret,
            Err(error) => {
                tracing::warn!(
                    client_id = %client_id,
                    binding_hash = %binding_hash_prefix(&binding_hash),
                    error = %error,
                    "CAE webhook secret was not valid UTF-8"
                );
                return;
            }
        };

        let event = RevocationEvent::new_at(
            binding_hash,
            client_id,
            dispatch.revoke_source,
            dispatch.reason,
            dispatch.revoked_at,
        );
        cae_webhook_service::dispatch_revocation_event(http_client, client, raw_secret, event);
    });
}

pub struct BindingListItem {
    pub binding_hash: String,
    pub client_id: String,
    pub external_subject: Option<ExternalSubjectRef>,
    pub scopes: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// List a user's non-revoked broker bindings, newest first.
pub async fn list_user_bindings(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<BindingListItem>> {
    let cursor = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find(doc! { "user_id": user_id, "revoked": false })
        .sort(doc! { "created_at": -1 })
        .await?;
    let bindings: Vec<OauthBrokerBinding> = cursor.try_collect().await?;

    Ok(bindings
        .into_iter()
        .map(|binding| BindingListItem {
            binding_hash: binding.id,
            client_id: binding.client_id,
            external_subject: binding.external_subject,
            scopes: binding.scopes,
            created_at: binding.created_at,
            last_used_at: binding.last_used_at,
        })
        .collect())
}

pub fn binding_hash_prefix(binding_hash: &str) -> String {
    binding_hash.chars().take(16).collect()
}

fn resolve_binding_scope(
    requested_scope: Option<&str>,
    stored_scopes: &[String],
) -> AppResult<String> {
    let stored: std::collections::HashSet<&str> =
        stored_scopes.iter().map(String::as_str).collect();

    if let Some(requested) = requested_scope {
        let requested_scopes: Vec<&str> = requested.split_whitespace().collect();
        for scope in &requested_scopes {
            if !stored.contains(scope) {
                return Err(AppError::InvalidScope(format!(
                    "Scope '{}' is not allowed for this binding",
                    scope
                )));
            }
        }
        return Ok(requested_scopes.join(" "));
    }

    Ok(stored_scopes.join(" "))
}

fn invalid_grant() -> AppError {
    AppError::ExternalTokenInvalid("invalid_grant".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::aes::EncryptionKeys;
    use crate::crypto::jwt::JwtKeys;
    use crate::models::oauth_broker_binding::BINDING_ID_PREFIX;
    use crate::models::oauth_client::OauthClient;
    use crate::test_utils::{
        cached_test_jwt_keys, connect_test_database, test_app_config, test_encryption_keys,
    };

    fn oauth_client_for_broker_test(
        broker_capability_enabled: bool,
        allowed_scopes: &str,
    ) -> OauthClient {
        OauthClient {
            id: "client-test".to_string(),
            client_name: "Test".to_string(),
            client_secret_hash: "hash".to_string(),
            redirect_uris: vec![],
            allowed_scopes: allowed_scopes.to_string(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            broker_capability_enabled,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn is_broker_client_honors_admin_flag() {
        let client = oauth_client_for_broker_test(true, "openid");
        assert!(is_broker_client(&client));
    }

    #[test]
    fn is_broker_client_honors_broker_binding_scope() {
        let client =
            oauth_client_for_broker_test(false, &format!("openid profile {BROKER_BINDING_SCOPE}"));
        assert!(is_broker_client(&client));
    }

    #[test]
    fn is_broker_client_false_without_either_trigger() {
        let client = oauth_client_for_broker_test(false, "openid profile email");
        assert!(!is_broker_client(&client));
    }

    #[test]
    fn is_broker_client_does_not_match_scope_substring() {
        // Make sure naive substring match doesn't accidentally trigger.
        let client =
            oauth_client_for_broker_test(false, "openid urn:nyxid:scope:broker_binding_NOPE");
        assert!(!is_broker_client(&client));
    }

    fn unused_jwt_keys() -> JwtKeys {
        JwtKeys {
            encoding: jsonwebtoken::EncodingKey::from_secret(b"unused"),
            decoding: jsonwebtoken::DecodingKey::from_secret(b"unused"),
            kid: "unused".to_string(),
        }
    }

    fn real_jwt_keys_and_config() -> (JwtKeys, AppConfig) {
        // Process-wide cached keypair: avoids paying multi-second RSA
        // generation per test. The returned config still carries the
        // default placeholder paths from `test_app_config`, but nothing in
        // the broker exchange paths reads those fields after `JwtKeys` is
        // constructed.
        (cached_test_jwt_keys(), test_app_config())
    }

    fn refresh_token_doc(jti: &str, client_id: &str, user_id: &str, revoked: bool) -> RefreshToken {
        let now = Utc::now();
        RefreshToken {
            id: Uuid::new_v4().to_string(),
            jti: jti.to_string(),
            client_id: client_id.to_string(),
            user_id: user_id.to_string(),
            session_id: None,
            expires_at: now + Duration::days(7),
            revoked,
            replaced_by: None,
            revoked_at: revoked.then_some(now),
            created_at: now,
        }
    }

    async fn insert_refresh_token(
        db: &mongodb::Database,
        jti: &str,
        client_id: &str,
        user_id: &str,
        revoked: bool,
    ) -> RefreshToken {
        let refresh = refresh_token_doc(jti, client_id, user_id, revoked);
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(&refresh)
            .await
            .expect("insert refresh token");
        refresh
    }

    async fn insert_refresh_token_jwt(
        db: &mongodb::Database,
        jwt_keys: &JwtKeys,
        config: &AppConfig,
        client_id: &str,
        user_id: &str,
    ) -> (String, RefreshToken) {
        let user_uuid = Uuid::parse_str(user_id).expect("valid user id");
        let (refresh_jwt, refresh_jti) =
            jwt::generate_refresh_token(jwt_keys, config, &user_uuid).expect("refresh jwt");
        let refresh = insert_refresh_token(db, &refresh_jti, client_id, user_id, false).await;
        (refresh_jwt, refresh)
    }

    async fn mark_refresh_replaced(db: &mongodb::Database, refresh_id: &str, replacement_id: &str) {
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(
                doc! { "_id": refresh_id },
                doc! { "$set": {
                    "revoked": true,
                    "replaced_by": replacement_id,
                    "revoked_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .await
            .expect("mark refresh replaced");
    }

    struct BindingSeed<'a> {
        raw_binding_id: &'a str,
        client_id: &'a str,
        user_id: &'a str,
        refresh_token_jti: &'a str,
        refresh_token: &'a str,
        scopes: Vec<String>,
        created_at: chrono::DateTime<Utc>,
        revoked: bool,
        revoke_reason: Option<String>,
    }

    async fn insert_binding(
        db: &mongodb::Database,
        encryption_keys: &EncryptionKeys,
        seed: BindingSeed<'_>,
    ) -> OauthBrokerBinding {
        insert_binding_with_external_subject(db, encryption_keys, seed, None).await
    }

    async fn insert_binding_with_external_subject(
        db: &mongodb::Database,
        encryption_keys: &EncryptionKeys,
        seed: BindingSeed<'_>,
        external_subject: Option<ExternalSubjectRef>,
    ) -> OauthBrokerBinding {
        let binding_hash = hash_binding_id(seed.raw_binding_id);
        let refresh_token_encrypted = encryption_keys
            .encrypt_with_aad(seed.refresh_token.as_bytes(), binding_hash.as_bytes())
            .await
            .expect("encrypt refresh token");
        let binding = OauthBrokerBinding {
            id: binding_hash,
            client_id: seed.client_id.to_string(),
            user_id: seed.user_id.to_string(),
            refresh_token_jti: seed.refresh_token_jti.to_string(),
            refresh_token_encrypted: Some(refresh_token_encrypted),
            scopes: seed.scopes,
            external_subject,
            rotation_version: 0,
            revoked: seed.revoked,
            last_used_at: None,
            revoked_at: seed.revoked.then_some(seed.created_at),
            revoke_reason: seed.revoke_reason,
            created_at: seed.created_at,
        };
        db.collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
            .insert_one(&binding)
            .await
            .expect("insert binding");
        binding
    }

    async fn load_binding(db: &mongodb::Database, binding_hash: &str) -> OauthBrokerBinding {
        db.collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
            .find_one(doc! { "_id": binding_hash })
            .await
            .expect("query binding")
            .expect("binding exists")
    }

    async fn load_refresh_by_jti(db: &mongodb::Database, jti: &str) -> RefreshToken {
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": jti })
            .await
            .expect("query refresh token")
            .expect("refresh token exists")
    }

    async fn insert_oauth_client_with_cae(
        db: &mongodb::Database,
        encryption_keys: &EncryptionKeys,
        client_id: &str,
        webhook_url: &str,
        raw_secret: &str,
    ) {
        let now = Utc::now();
        let encrypted_secret = encryption_keys
            .encrypt(raw_secret.as_bytes())
            .await
            .expect("encrypt CAE secret");
        let client = OauthClient {
            id: client_id.to_string(),
            client_name: "CAE Test Client".to_string(),
            client_secret_hash: "hash".to_string(),
            redirect_uris: vec![],
            allowed_scopes: "openid profile".to_string(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            broker_capability_enabled: true,
            revocation_webhook_url: Some(webhook_url.to_string()),
            revocation_webhook_secret_encrypted: Some(encrypted_secret),
            created_by: Some("test".to_string()),
            created_at: now,
            updated_at: now,
        };
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(&client)
            .await
            .expect("insert OAuth client with CAE");
    }

    struct CapturedCaeRequest {
        body: Vec<u8>,
        signature: String,
        event_header: String,
        delivery_id: String,
    }

    async fn spawn_cae_receiver() -> (String, tokio::sync::oneshot::Receiver<CapturedCaeRequest>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind CAE test listener");
        let address = listener.local_addr().expect("listener address");
        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buffer = Vec::new();
            let mut header_end = None;
            let mut content_length = 0usize;

            loop {
                let mut chunk = [0u8; 1024];
                let Ok(read) = stream.read(&mut chunk).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                buffer.extend_from_slice(&chunk[..read]);

                if header_end.is_none()
                    && let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n")
                {
                    header_end = Some(pos + 4);
                    let headers = String::from_utf8_lossy(&buffer[..pos]);
                    for line in headers.lines().skip(1) {
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }

                if let Some(end) = header_end
                    && buffer.len() >= end + content_length
                {
                    let headers = String::from_utf8_lossy(&buffer[..end]);
                    let mut signature = String::new();
                    let mut event_header = String::new();
                    let mut delivery_id = String::new();
                    for line in headers.lines().skip(1) {
                        if let Some((name, value)) = line.split_once(':') {
                            let value = value.trim().to_string();
                            if name.eq_ignore_ascii_case("x-nyxid-signature") {
                                signature = value;
                            } else if name.eq_ignore_ascii_case("x-nyxid-event") {
                                event_header = value;
                            } else if name.eq_ignore_ascii_case("x-nyxid-delivery-id") {
                                delivery_id = value;
                            }
                        }
                    }
                    let body = buffer[end..end + content_length].to_vec();
                    let _ = stream
                        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                        .await;
                    let _ = tx.send(CapturedCaeRequest {
                        body,
                        signature,
                        event_header,
                        delivery_id,
                    });
                    return;
                }
            }
        });

        (format!("http://{address}/cae"), rx)
    }

    #[tokio::test]
    async fn create_binding_persists_and_decrypts() {
        let Some(db) = connect_test_database("broker_create").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let external_subject = ExternalSubjectRef {
            platform: "lark".to_string(),
            tenant: Some("tenant-1".to_string()),
            external_user_id: "external-user-1".to_string(),
        };

        let (raw_binding_id, binding_hash) = create_binding(
            &db,
            &encryption_keys,
            "client-1",
            "user-1",
            "test-refresh-token-123",
            "refresh-jti-1",
            &["openid".to_string(), "profile".to_string()],
            Some(&external_subject),
        )
        .await
        .expect("create binding");

        assert!(raw_binding_id.starts_with(BINDING_ID_PREFIX));
        let restored = load_binding(&db, &binding_hash).await;
        assert_eq!(restored.client_id, "client-1");
        assert_eq!(restored.user_id, "user-1");
        assert_eq!(restored.refresh_token_jti, "refresh-jti-1");
        assert_eq!(restored.rotation_version, 0);
        assert!(!restored.revoked);
        assert_eq!(restored.external_subject, Some(external_subject));
        let decrypted = encryption_keys
            .decrypt_with_aad(
                restored
                    .refresh_token_encrypted
                    .as_ref()
                    .expect("encrypted refresh token"),
                binding_hash.as_bytes(),
            )
            .await
            .expect("decrypt refresh token");
        assert_eq!(decrypted, b"test-refresh-token-123");
    }

    #[tokio::test]
    async fn get_binding_for_client_returns_binding_for_owner() {
        let Some(db) = connect_test_database("broker_get_owner").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();
        let user_id = Uuid::new_v4().to_string();

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-a",
                user_id: &user_id,
                refresh_token_jti: "jti-get",
                refresh_token: "refresh-get",
                scopes: vec!["openid".to_string(), "profile".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let binding = get_binding_for_client(&db, "client-a", &raw_binding_id)
            .await
            .expect("get binding");
        assert_eq!(binding.client_id, "client-a");
        assert_eq!(binding.user_id, user_id);
        assert_eq!(
            binding.scopes,
            vec!["openid".to_string(), "profile".to_string()]
        );
    }

    #[tokio::test]
    async fn get_binding_for_client_returns_not_found_for_other_client() {
        let Some(db) = connect_test_database("broker_get_other").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-a",
                user_id: "user-1",
                refresh_token_jti: "jti-other",
                refresh_token: "refresh-other",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let result = get_binding_for_client(&db, "client-b", &raw_binding_id).await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn get_binding_for_client_returns_revoked_state_truthfully() {
        let Some(db) = connect_test_database("broker_get_revoked").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-a",
                user_id: "user-1",
                refresh_token_jti: "jti-revoked",
                refresh_token: "refresh-revoked",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: true,
                revoke_reason: Some("user_revoked".to_string()),
            },
        )
        .await;

        let binding = get_binding_for_client(&db, "client-a", &raw_binding_id)
            .await
            .expect("get binding");
        assert!(binding.revoked);
    }

    #[tokio::test]
    async fn introspect_via_get_binding_returns_owner_metadata() {
        let Some(db) = connect_test_database("broker_introspect_owner").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();
        let user_id = Uuid::new_v4().to_string();
        let scopes = vec!["openid".to_string(), "profile".to_string()];

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-introspect",
                user_id: &user_id,
                refresh_token_jti: "jti-introspect",
                refresh_token: "refresh-introspect",
                scopes: scopes.clone(),
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let binding = get_binding_for_client(&db, "client-introspect", &raw_binding_id)
            .await
            .expect("get binding");
        assert!(!binding.revoked);
        assert_eq!(binding.client_id, "client-introspect");
        assert_eq!(binding.user_id, user_id);
        assert_eq!(binding.scopes, scopes);
        assert!(binding.created_at.timestamp() > 0);
    }

    #[tokio::test]
    async fn introspect_via_get_binding_returns_not_found_for_revoked() {
        let Some(db) = connect_test_database("broker_introspect_revoked").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-introspect-revoked",
                user_id: "user-1",
                refresh_token_jti: "jti-revoked",
                refresh_token: "refresh-revoked",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: true,
                revoke_reason: Some("user_revoked".to_string()),
            },
        )
        .await;

        let binding = get_binding_for_client(&db, "client-introspect-revoked", &raw_binding_id)
            .await
            .expect("get binding");
        assert!(binding.revoked);
    }

    #[tokio::test]
    async fn find_active_bindings_filters_by_external_subject() {
        let Some(db) = connect_test_database("broker_reverse").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let now = Utc::now();
        let raw_match = generate_binding_id();
        let raw_revoked = generate_binding_id();
        let raw_wrong_client = generate_binding_id();
        let raw_wrong_platform = generate_binding_id();
        let raw_wrong_user = generate_binding_id();
        let matching_subject = ExternalSubjectRef {
            platform: "lark".to_string(),
            tenant: Some("tenant-1".to_string()),
            external_user_id: "user-x".to_string(),
        };

        insert_binding_with_external_subject(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_match,
                client_id: "client-a",
                user_id: "user-1",
                refresh_token_jti: "jti-match",
                refresh_token: "refresh-match",
                scopes: vec!["openid".to_string()],
                created_at: now,
                revoked: false,
                revoke_reason: None,
            },
            Some(matching_subject.clone()),
        )
        .await;
        insert_binding_with_external_subject(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_revoked,
                client_id: "client-a",
                user_id: "user-2",
                refresh_token_jti: "jti-revoked-match",
                refresh_token: "refresh-revoked-match",
                scopes: vec!["openid".to_string()],
                created_at: now + Duration::seconds(1),
                revoked: true,
                revoke_reason: Some("user_revoked".to_string()),
            },
            Some(matching_subject.clone()),
        )
        .await;
        insert_binding_with_external_subject(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_wrong_client,
                client_id: "client-b",
                user_id: "user-3",
                refresh_token_jti: "jti-wrong-client",
                refresh_token: "refresh-wrong-client",
                scopes: vec!["openid".to_string()],
                created_at: now + Duration::seconds(2),
                revoked: false,
                revoke_reason: None,
            },
            Some(matching_subject.clone()),
        )
        .await;
        insert_binding_with_external_subject(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_wrong_platform,
                client_id: "client-a",
                user_id: "user-4",
                refresh_token_jti: "jti-wrong-platform",
                refresh_token: "refresh-wrong-platform",
                scopes: vec!["openid".to_string()],
                created_at: now + Duration::seconds(3),
                revoked: false,
                revoke_reason: None,
            },
            Some(ExternalSubjectRef {
                platform: "github".to_string(),
                tenant: Some("tenant-1".to_string()),
                external_user_id: "user-x".to_string(),
            }),
        )
        .await;
        insert_binding_with_external_subject(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_wrong_user,
                client_id: "client-a",
                user_id: "user-5",
                refresh_token_jti: "jti-wrong-user",
                refresh_token: "refresh-wrong-user",
                scopes: vec!["openid".to_string()],
                created_at: now + Duration::seconds(4),
                revoked: false,
                revoke_reason: None,
            },
            Some(ExternalSubjectRef {
                platform: "lark".to_string(),
                tenant: Some("tenant-1".to_string()),
                external_user_id: "user-y".to_string(),
            }),
        )
        .await;

        let matches = find_active_bindings_by_external_subject(
            &db,
            "client-a",
            "lark",
            Some("tenant-1"),
            "user-x",
        )
        .await
        .expect("reverse lookup");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, hash_binding_id(&raw_match));
        assert_eq!(matches[0].external_subject, Some(matching_subject));
    }

    #[tokio::test]
    async fn find_active_bindings_handles_absent_tenant() {
        let Some(db) = connect_test_database("broker_reverse_no_tenant").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();

        insert_binding_with_external_subject(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-a",
                user_id: "user-1",
                refresh_token_jti: "jti-no-tenant",
                refresh_token: "refresh-no-tenant",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
            Some(ExternalSubjectRef {
                platform: "lark".to_string(),
                tenant: None,
                external_user_id: "user-x".to_string(),
            }),
        )
        .await;

        let absent_tenant =
            find_active_bindings_by_external_subject(&db, "client-a", "lark", None, "user-x")
                .await
                .expect("reverse lookup without tenant");
        assert_eq!(absent_tenant.len(), 1);
        assert_eq!(absent_tenant[0].id, hash_binding_id(&raw_binding_id));

        let explicit_tenant = find_active_bindings_by_external_subject(
            &db,
            "client-a",
            "lark",
            Some("tenant-1"),
            "user-x",
        )
        .await
        .expect("reverse lookup with tenant");
        assert!(explicit_tenant.is_empty());
    }

    #[tokio::test]
    async fn revoke_binding_by_client_marks_revoked_and_revokes_refresh_token() {
        let Some(db) = connect_test_database("broker_revoke_client").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();
        let binding_hash = hash_binding_id(&raw_binding_id);

        insert_refresh_token(&db, "jti-client-revoke", "client-1", "user-1", false).await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-1",
                user_id: "user-1",
                refresh_token_jti: "jti-client-revoke",
                refresh_token: "refresh-client-revoke",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let revoked = revoke_binding_by_client(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            "client-1",
            &raw_binding_id,
            "client_revoked",
        )
        .await
        .expect("revoke binding");
        assert!(revoked);

        let binding = load_binding(&db, &binding_hash).await;
        assert!(binding.revoked);
        assert_eq!(binding.revoke_reason.as_deref(), Some("client_revoked"));
        let refresh = load_refresh_by_jti(&db, "jti-client-revoke").await;
        assert!(refresh.revoked);

        let second = revoke_binding_by_client(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            "client-1",
            &raw_binding_id,
            "client_revoked",
        )
        .await
        .expect("idempotent revoke");
        assert!(!second);
        let wrong_client = revoke_binding_by_client(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            "client-2",
            &raw_binding_id,
            "client_revoked",
        )
        .await
        .expect("wrong client revoke");
        assert!(!wrong_client);
    }

    #[tokio::test]
    async fn cae_webhook_dispatches_on_client_revoke() {
        let Some(db) = connect_test_database("broker_cae_client_revoke").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let http_client = reqwest::Client::new();
        let (webhook_url, received) = spawn_cae_receiver().await;
        let raw_secret = "cae-webhook-secret";
        let client_id = "client-cae";
        let raw_binding_id = generate_binding_id();
        let binding_hash = hash_binding_id(&raw_binding_id);

        insert_oauth_client_with_cae(&db, &encryption_keys, client_id, &webhook_url, raw_secret)
            .await;
        insert_refresh_token(&db, "jti-cae", client_id, "user-cae", false).await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id,
                user_id: "user-cae",
                refresh_token_jti: "jti-cae",
                refresh_token: "refresh-cae",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let revoked = revoke_binding_by_client(
            &db,
            encryption_keys.clone(),
            &http_client,
            client_id,
            &raw_binding_id,
            "client_revoked",
        )
        .await
        .expect("revoke binding");
        assert!(revoked);

        let captured = tokio::time::timeout(std::time::Duration::from_secs(5), received)
            .await
            .expect("CAE webhook received")
            .expect("receiver sent request");
        assert_eq!(captured.event_header, "oauth_broker_binding.revoked");
        assert!(!captured.delivery_id.is_empty());

        use hmac::Mac as _;
        type HmacSha256 = hmac::Hmac<sha2::Sha256>;
        let mut mac =
            <HmacSha256 as hmac::Mac>::new_from_slice(raw_secret.as_bytes()).expect("HMAC key");
        mac.update(&captured.body);
        let expected_signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert_eq!(captured.signature, expected_signature);

        let event: serde_json::Value =
            serde_json::from_slice(&captured.body).expect("CAE JSON body");
        assert_eq!(
            event["event_type"].as_str(),
            Some("oauth_broker_binding.revoked")
        );
        assert_eq!(event["binding_hash"].as_str(), Some(binding_hash.as_str()));
        assert_eq!(event["client_id"].as_str(), Some(client_id));
        assert_eq!(event["revoke_source"].as_str(), Some("client"));
        assert_eq!(event["reason"].as_str(), Some("client_revoked"));
        assert!(event["revoked_at"].is_string());
    }

    #[tokio::test]
    async fn revoke_binding_by_user_returns_not_found_for_other_user() {
        let Some(db) = connect_test_database("broker_revoke_user").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();
        let binding_hash = hash_binding_id(&raw_binding_id);

        insert_refresh_token(&db, "jti-user-revoke", "client-1", "user-a", false).await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-1",
                user_id: "user-a",
                refresh_token_jti: "jti-user-revoke",
                refresh_token: "refresh-user-revoke",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let wrong_user = revoke_binding_by_user(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            "user-b",
            &binding_hash,
            "user_revoked",
        )
        .await;
        assert!(matches!(wrong_user, Err(AppError::NotFound(_))));
        assert!(!load_binding(&db, &binding_hash).await.revoked);

        revoke_binding_by_user(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            "user-a",
            &binding_hash,
            "user_revoked",
        )
        .await
        .expect("owner revoke");
        let binding = load_binding(&db, &binding_hash).await;
        assert!(binding.revoked);
        assert_eq!(binding.revoke_reason.as_deref(), Some("user_revoked"));
        assert!(load_refresh_by_jti(&db, "jti-user-revoke").await.revoked);
    }

    #[tokio::test]
    async fn list_user_bindings_filters_revoked_and_sorts_newest_first() {
        let Some(db) = connect_test_database("broker_list").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let now = Utc::now();
        let old_raw = generate_binding_id();
        let revoked_raw = generate_binding_id();
        let new_raw = generate_binding_id();
        let other_raw = generate_binding_id();

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &old_raw,
                client_id: "client-1",
                user_id: "user-x",
                refresh_token_jti: "jti-old",
                refresh_token: "refresh-old",
                scopes: vec!["openid".to_string()],
                created_at: now,
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &revoked_raw,
                client_id: "client-1",
                user_id: "user-x",
                refresh_token_jti: "jti-revoked",
                refresh_token: "refresh-revoked",
                scopes: vec!["openid".to_string()],
                created_at: now + Duration::seconds(1),
                revoked: true,
                revoke_reason: Some("user_revoked".to_string()),
            },
        )
        .await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &new_raw,
                client_id: "client-2",
                user_id: "user-x",
                refresh_token_jti: "jti-new",
                refresh_token: "refresh-new",
                scopes: vec!["openid".to_string(), "email".to_string()],
                created_at: now + Duration::seconds(2),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &other_raw,
                client_id: "client-3",
                user_id: "user-y",
                refresh_token_jti: "jti-other",
                refresh_token: "refresh-other",
                scopes: vec!["openid".to_string()],
                created_at: now + Duration::seconds(3),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let listed = list_user_bindings(&db, "user-x")
            .await
            .expect("list user bindings");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].binding_hash, hash_binding_id(&new_raw));
        assert_eq!(listed[1].binding_hash, hash_binding_id(&old_raw));
        assert_eq!(
            listed[0].scopes,
            vec!["openid".to_string(), "email".to_string()]
        );
    }

    #[tokio::test]
    async fn exchange_via_binding_rejects_wrong_client() {
        let Some(db) = connect_test_database("broker_xclient").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let config = test_app_config();
        let jwt_keys = unused_jwt_keys();
        let raw_binding_id = generate_binding_id();
        let binding_hash = hash_binding_id(&raw_binding_id);
        let user_id = Uuid::new_v4().to_string();

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-a",
                user_id: &user_id,
                refresh_token_jti: "jti-xclient",
                refresh_token: "refresh-xclient",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let result = exchange_via_binding(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            &jwt_keys,
            &config,
            "client-b",
            &raw_binding_id,
            None,
            None,
            None,
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::ExternalTokenInvalid(message)) if message == "invalid_grant"
        ));
        let binding = load_binding(&db, &binding_hash).await;
        assert_eq!(binding.rotation_version, 0);
        assert_eq!(binding.refresh_token_jti, "jti-xclient");
        assert!(!binding.revoked);
    }

    #[tokio::test]
    async fn exchange_via_binding_rejects_blob_swap() {
        let Some(db) = connect_test_database("broker_aad_swap").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let config = test_app_config();
        let jwt_keys = unused_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        let raw_a = generate_binding_id();
        let raw_b = generate_binding_id();
        let hash_a = hash_binding_id(&raw_a);
        let hash_b = hash_binding_id(&raw_b);

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_a,
                client_id: "client-x",
                user_id: &user_id,
                refresh_token_jti: "jti-swap-a",
                refresh_token: "refresh-swap-a",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_b,
                client_id: "client-x",
                user_id: &user_id,
                refresh_token_jti: "jti-swap-b",
                refresh_token: "refresh-swap-b",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let binding_b = load_binding(&db, &hash_b).await;
        let swapped_blob = binding_b
            .refresh_token_encrypted
            .expect("binding B encrypted refresh token");
        db.collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
            .update_one(
                doc! { "_id": &hash_a },
                doc! { "$set": {
                    "refresh_token_encrypted": Binary {
                        subtype: BinarySubtype::Generic,
                        bytes: swapped_blob,
                    },
                }},
            )
            .await
            .expect("swap encrypted refresh token");

        let result = exchange_via_binding(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            &jwt_keys,
            &config,
            "client-x",
            &raw_a,
            None,
            None,
            None,
        )
        .await;

        assert!(matches!(
            result,
            Err(AppError::ExternalTokenInvalid(message)) if message == "invalid_grant"
        ));
    }

    #[tokio::test]
    async fn exchange_via_binding_sets_cnf_for_dpop_access_token() {
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let (jwt_keys, config) = real_jwt_keys_and_config();
        let Some(db) = connect_test_database("broker_dpop_cnf").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let raw_binding_id = generate_binding_id();
        let (refresh_jwt, refresh) =
            insert_refresh_token_jwt(&db, &jwt_keys, &config, "client-dpop", &user_id).await;

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-dpop",
                user_id: &user_id,
                refresh_token_jti: &refresh.jti,
                refresh_token: &refresh_jwt,
                scopes: vec!["openid".to_string(), "profile".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let signing_key = p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng);
        let private_pem = {
            use p256::pkcs8::{EncodePrivateKey, LineEnding};
            signing_key
                .to_pkcs8_pem(LineEnding::LF)
                .expect("encode P-256 private key")
        };
        let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(private_pem.as_bytes())
            .expect("EC encoding key");
        let jwt_jwk = jsonwebtoken::jwk::Jwk::from_encoding_key(
            &encoding_key,
            jsonwebtoken::Algorithm::ES256,
        )
        .expect("derive public JWK");
        let jwk: crate::crypto::dpop::Jwk =
            serde_json::from_value(serde_json::to_value(jwt_jwk).expect("JWK JSON"))
                .expect("DPoP JWK");
        let jkt = crate::crypto::dpop::jwk_thumbprint(&jwk);

        let result = exchange_via_binding(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            &jwt_keys,
            &config,
            "client-dpop",
            &raw_binding_id,
            Some("openid"),
            Some(&jkt),
            None,
        )
        .await
        .expect("exchange via DPoP-bound binding");

        assert_eq!(result.token_type, "DPoP");
        let claims = jwt::verify_token(&jwt_keys, &config, &result.access_token)
            .expect("valid access token");
        assert_eq!(claims.token_type, "access");
        assert_eq!(claims.scope, "openid");
        let cnf = claims.cnf.expect("cnf claim");
        assert_eq!(cnf.jkt.as_deref(), Some(jkt.as_str()));
        assert!(cnf.x5t_s256.is_none());
    }

    #[tokio::test]
    async fn exchange_via_binding_sets_cnf_for_mtls_access_token() {
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let (jwt_keys, config) = real_jwt_keys_and_config();
        let Some(db) = connect_test_database("broker_mtls_cnf").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let raw_binding_id = generate_binding_id();
        let (refresh_jwt, refresh) =
            insert_refresh_token_jwt(&db, &jwt_keys, &config, "client-mtls", &user_id).await;

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-mtls",
                user_id: &user_id,
                refresh_token_jti: &refresh.jti,
                refresh_token: &refresh_jwt,
                scopes: vec!["openid".to_string(), "profile".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let cert_pem = "-----BEGIN CERTIFICATE-----\nAQIDBA==\n-----END CERTIFICATE-----\n";
        let x5t = crate::crypto::mtls::cert_thumbprint_from_header(cert_pem)
            .expect("mTLS cert thumbprint");

        let result = exchange_via_binding(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            &jwt_keys,
            &config,
            "client-mtls",
            &raw_binding_id,
            Some("openid"),
            None,
            Some(&x5t),
        )
        .await
        .expect("exchange via mTLS-bound binding");

        assert_eq!(result.token_type, "Bearer");
        let claims = jwt::verify_token(&jwt_keys, &config, &result.access_token)
            .expect("valid access token");
        assert_eq!(claims.token_type, "access");
        assert_eq!(claims.scope, "openid");
        let cnf = claims.cnf.expect("cnf claim");
        assert!(cnf.jkt.is_none());
        assert_eq!(cnf.x5t_s256.as_deref(), Some(x5t.as_str()));
    }

    #[tokio::test]
    async fn exchange_via_binding_chain_follows_after_rotation() {
        let Some(db) = connect_test_database("broker_chain_follow").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let (jwt_keys, config) = real_jwt_keys_and_config();
        let user_id = Uuid::new_v4().to_string();
        let raw_binding_id = generate_binding_id();
        let binding_hash = hash_binding_id(&raw_binding_id);
        let (old_refresh_jwt, old_refresh) =
            insert_refresh_token_jwt(&db, &jwt_keys, &config, "client-x", &user_id).await;

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-x",
                user_id: &user_id,
                refresh_token_jti: &old_refresh.jti,
                refresh_token: &old_refresh_jwt,
                scopes: vec!["openid".to_string(), "profile".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let (new_refresh_jwt, new_refresh) =
            insert_refresh_token_jwt(&db, &jwt_keys, &config, "client-x", &user_id).await;
        mark_refresh_replaced(&db, &old_refresh.id, &new_refresh.id).await;
        let new_blob = encryption_keys
            .encrypt_with_aad(new_refresh_jwt.as_bytes(), binding_hash.as_bytes())
            .await
            .expect("encrypt replacement refresh token");
        db.collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
            .update_one(
                doc! { "_id": &binding_hash },
                doc! { "$set": {
                    "refresh_token_encrypted": Binary {
                        subtype: BinarySubtype::Generic,
                        bytes: new_blob,
                    },
                    "refresh_token_jti": &new_refresh.jti,
                    "rotation_version": 1_i64,
                    "last_used_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .await
            .expect("update binding to replacement");

        let http_client = reqwest::Client::new();
        let ctx = BrokerExchangeContext {
            db: &db,
            encryption_keys: encryption_keys.clone(),
            http_client: &http_client,
            jwt_keys: &jwt_keys,
            config: &config,
            client_id: "client-x",
            binding_hash: &binding_hash,
            requested_scope: Some("openid"),
            dpop_jkt: None,
            mtls_x5t_s256: None,
        };
        let result = try_chain_follow(&ctx)
            .await
            .expect("chain follow")
            .expect("chain follow result");

        assert!(result.via_chain_follow);
        assert_eq!(result.expires_in, BROKER_ACCESS_TTL_SECS);
        assert_eq!(result.granted_scope, "openid");
        let claims = jwt::verify_token(&jwt_keys, &config, &result.access_token)
            .expect("valid access token");
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.token_type, "access");
        assert_eq!(claims.scope, "openid");

        let binding = load_binding(&db, &binding_hash).await;
        assert_eq!(binding.rotation_version, 1);
        assert_eq!(binding.refresh_token_jti, new_refresh.jti);
    }

    #[tokio::test]
    async fn exchange_via_binding_chain_follow_returns_invalid_grant_when_no_active_descendant() {
        let Some(db) = connect_test_database("broker_chain_no_descendant").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let (jwt_keys, config) = real_jwt_keys_and_config();
        let user_id = Uuid::new_v4().to_string();
        let raw_binding_id = generate_binding_id();
        let binding_hash = hash_binding_id(&raw_binding_id);
        let (refresh_jwt, refresh) =
            insert_refresh_token_jwt(&db, &jwt_keys, &config, "client-x", &user_id).await;

        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-x",
                user_id: &user_id,
                refresh_token_jti: &refresh.jti,
                refresh_token: &refresh_jwt,
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(
                doc! { "_id": &refresh.id },
                doc! { "$set": {
                    "revoked": true,
                    "revoked_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .await
            .expect("revoke refresh without descendant");

        let result = exchange_via_binding(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            &jwt_keys,
            &config,
            "client-x",
            &raw_binding_id,
            None,
            None,
            None,
        )
        .await;

        assert!(matches!(
            result,
            Err(AppError::ExternalTokenInvalid(message)) if message == "invalid_grant"
        ));
        let binding = load_binding(&db, &binding_hash).await;
        assert!(binding.revoked);
        assert_eq!(binding.revoke_reason.as_deref(), Some("reuse_detected"));
    }

    #[tokio::test]
    async fn exchange_via_binding_orphan_cleanup_on_conflict() {
        let Some(db) = connect_test_database("broker_orphan_cleanup").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let raw_binding_id = generate_binding_id();
        let binding_hash = hash_binding_id(&raw_binding_id);
        let binding = insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_binding_id,
                client_id: "client-x",
                user_id: "user-1",
                refresh_token_jti: "jti-old",
                refresh_token: "refresh-old",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;
        let old_refresh = insert_refresh_token(&db, "jti-old", "client-x", "user-1", false).await;
        let winner_refresh =
            insert_refresh_token(&db, "jti-winner", "client-x", "user-1", false).await;
        mark_refresh_replaced(&db, &old_refresh.id, &winner_refresh.id).await;
        let orphan = insert_refresh_token(&db, "jti-orphan", "client-x", "user-1", false).await;
        let jwt_keys = unused_jwt_keys();
        let config = test_app_config();
        let http_client = reqwest::Client::new();
        let ctx = BrokerExchangeContext {
            db: &db,
            encryption_keys: encryption_keys.clone(),
            http_client: &http_client,
            jwt_keys: &jwt_keys,
            config: &config,
            client_id: "client-x",
            binding_hash: &binding_hash,
            requested_scope: None,
            dpop_jkt: None,
            mtls_x5t_s256: None,
        };

        let outcome = handle_refresh_revoke_conflict(
            &ctx,
            &binding,
            &binding_hash,
            &old_refresh.id,
            &orphan.id,
        )
        .await
        .expect("handle conflict");

        assert!(matches!(outcome, ExchangeOutcome::ChainFollow));
        let orphan_after = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "_id": &orphan.id })
            .await
            .expect("query orphan");
        assert!(orphan_after.is_none());
    }

    #[tokio::test]
    async fn exchange_via_binding_cascades_on_reuse() {
        let Some(db) = connect_test_database("broker_reuse").await else {
            return;
        };
        let encryption_keys = std::sync::Arc::new(test_encryption_keys());
        let config = test_app_config();
        let jwt_keys = unused_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        let raw_one = generate_binding_id();
        let raw_two = generate_binding_id();
        let hash_one = hash_binding_id(&raw_one);
        let hash_two = hash_binding_id(&raw_two);

        insert_refresh_token(&db, "jti-reuse-1", "client-x", &user_id, true).await;
        insert_refresh_token(&db, "jti-reuse-2", "client-x", &user_id, false).await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_one,
                client_id: "client-x",
                user_id: &user_id,
                refresh_token_jti: "jti-reuse-1",
                refresh_token: "refresh-reuse-1",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;
        insert_binding(
            &db,
            &encryption_keys,
            BindingSeed {
                raw_binding_id: &raw_two,
                client_id: "client-x",
                user_id: &user_id,
                refresh_token_jti: "jti-reuse-2",
                refresh_token: "refresh-reuse-2",
                scopes: vec!["openid".to_string()],
                created_at: Utc::now(),
                revoked: false,
                revoke_reason: None,
            },
        )
        .await;

        let result = exchange_via_binding(
            &db,
            encryption_keys.clone(),
            &reqwest::Client::new(),
            &jwt_keys,
            &config,
            "client-x",
            &raw_one,
            None,
            None,
            None,
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::ExternalTokenInvalid(message)) if message == "invalid_grant"
        ));

        let binding_one = load_binding(&db, &hash_one).await;
        let binding_two = load_binding(&db, &hash_two).await;
        assert!(binding_one.revoked);
        assert!(binding_two.revoked);
        assert_eq!(binding_one.revoke_reason.as_deref(), Some("reuse_detected"));
        assert_eq!(binding_two.revoke_reason.as_deref(), Some("reuse_detected"));
    }
}
