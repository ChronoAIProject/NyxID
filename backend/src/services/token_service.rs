use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use uuid::Uuid;

use super::{
    audit_outbox_service::{self, AuditEvent},
    audit_runtime::{AuditRuntime, policy::AuditTier, transaction},
};
use crate::config::AppConfig;
use crate::crypto::jwt::{self, JwtKeys};
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::audit_outbox::{AuditOwner, AuditPrincipal, AuditStage};
use crate::models::mcp_session::McpSessionStore;
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};

pub fn audit_principal(auth: &crate::mw::auth::AuthUser) -> AppResult<AuditPrincipal> {
    use crate::mw::auth::AuthMethod;
    Ok(match auth.auth_method {
        AuthMethod::ApiKey | AuthMethod::Relay => AuditPrincipal::ApiKey {
            id: auth
                .api_key_id
                .clone()
                .ok_or_else(|| AppError::Unauthorized("Missing key identity".into()))?,
            owner_id: auth.user_id.to_string(),
        },
        AuthMethod::ServiceAccount => AuditPrincipal::ServiceAccount {
            id: auth.user_id.to_string(),
        },
        _ => AuditPrincipal::Person {
            id: auth.user_id.to_string(),
        },
    })
}

/// Prepared once by the owning operation, before any transaction retry.
pub struct SessionAudit {
    pub event_id: Uuid,
    pub event: AuditEvent,
}
impl SessionAudit {
    pub fn new(principal: AuditPrincipal, owner: &str, action: &'static str, target: &str) -> Self {
        let id = Uuid::new_v4();
        Self {
            event_id: id,
            event: AuditEvent {
                operation_id: id.to_string(),
                stage: AuditStage::Committed,
                principal,
                owner: AuditOwner::Person { id: owner.into() },
                action,
                target_id: Some(target.into()),
            },
        }
    }
    pub(super) async fn enqueue(
        &self,
        db: &mongodb::Database,
        runtime: &AuditRuntime,
        session: &mut mongodb::ClientSession,
        tier: AuditTier,
    ) -> AppResult<()> {
        audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            runtime,
            tier,
            &self.event_id,
            &self.event,
        )
        .await?;
        Ok(())
    }
}

pub(super) async fn live_person(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    user_id: &str,
) -> AppResult<()> {
    use crate::models::user::{COLLECTION_NAME, User};
    let user = db
        .collection::<User>(COLLECTION_NAME)
        .find_one(doc! {"_id": user_id, "is_active": true})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Account is not active".into()))?;
    super::auth_service::ensure_person_user(&user)?;
    // Conflict with account disable/deletion rather than issuing from a stale snapshot.
    db.collection::<User>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": user_id, "is_active": true},
            doc! {"$set": {"updated_at": bson::DateTime::now()}},
        )
        .session(session)
        .await?;
    Ok(())
}

/// Grace period (in seconds) after refresh token rotation during which
/// reuse of the old token is treated as a legitimate retry rather than theft.
///
/// **Security trade-off**: A longer window gives clients more time to recover
/// from network failures during rotation (e.g., the response with the new
/// token was lost), but also gives an attacker who stole the old token a
/// window to use it before it is flagged as theft.
///
/// 120 seconds was chosen because:
/// - Network retries typically happen within seconds, not minutes.
/// - It covers slow mobile connections and client-side retry back-off.
/// - It is short enough that a stolen token has minimal usable window
///   (the attacker must also have the JWT, which has its own short TTL).
const REUSE_GRACE_PERIOD_SECS: i64 = 120;

/// Maximum depth when following a replacement chain.
/// Prevents infinite loops if the database has a cycle, and bounds the
/// number of DB round-trips for concurrent-rotation scenarios.
const MAX_REPLACEMENT_CHAIN_DEPTH: usize = 5;

/// Session lifetime for first-party sessions.
pub const SESSION_TTL_SECS: i64 = 30 * 24 * 3600;

/// Default bearer-token scopes for first-party NyxID clients.
///
/// Includes `proxy` so first-party token clients retain access to NyxID's
/// proxy, LLM gateway, and MCP surfaces after route-level scope enforcement.
pub const FIRST_PARTY_ACCESS_SCOPES: &str = "openid profile email proxy";

/// Tokens issued after successful authentication.
#[derive(Clone)]
pub struct IssuedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: String,
    pub access_expires_in: i64,
    pub resource_uris: Vec<String>,
}

/// Session issued for browser-based authentication flows.
#[derive(Clone)]
pub struct IssuedSession {
    pub session_token: String,
    pub session_id: String,
}

async fn follow_active_replacement_chain(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    first: &str,
    owner: &str,
) -> AppResult<Option<RefreshToken>> {
    let mut id = first.to_owned();
    for _ in 0..MAX_REPLACEMENT_CHAIN_DEPTH {
        let row = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"_id": &id, "user_id": owner})
            .session(&mut *session)
            .await?;
        match row {
            Some(row) if !row.revoked && row.expires_at > Utc::now() => return Ok(Some(row)),
            Some(RefreshToken {
                replaced_by: Some(next),
                ..
            }) => id = next,
            _ => return Ok(None),
        }
    }
    Ok(None)
}

fn build_reused_refresh_response(
    jwt_keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
    active_token: &RefreshToken,
    access_token: String,
    resource_uris: Vec<String>,
) -> AppResult<IssuedTokens> {
    let refresh_token = jwt::reissue_refresh_token(
        jwt_keys,
        config,
        user_id,
        &active_token.jti,
        active_token.created_at.timestamp(),
        active_token.expires_at.timestamp(),
    )?;

    Ok(IssuedTokens {
        access_token,
        refresh_token,
        session_id: active_token
            .session_id
            .clone()
            .unwrap_or_else(|| Uuid::nil().to_string()),
        access_expires_in: config.jwt_access_ttl_secs,
        resource_uris,
    })
}

/// A browser credential prepared outside retry callbacks. Never serialize it.
pub struct PreparedBrowserSession {
    pub issued: IssuedSession,
    session: Session,
    audit: SessionAudit,
}

impl PreparedBrowserSession {
    pub fn user_id(&self) -> &str {
        &self.session.user_id
    }
}

pub fn prepare_browser_session(
    user_id: &str,
    ip: Option<&str>,
    ua: Option<&str>,
) -> AppResult<PreparedBrowserSession> {
    Uuid::parse_str(user_id)
        .map_err(|_| AppError::ValidationError("Invalid user identity".into()))?;
    let secret = generate_random_token();
    let now = Utc::now();
    let row = Session {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.into(),
        token_hash: hash_token(&secret),
        ip_address: ip.map(str::to_owned),
        user_agent: ua.map(str::to_owned),
        expires_at: now + Duration::seconds(SESSION_TTL_SECS),
        revoked: false,
        created_at: now,
        last_active_at: now,
    };
    let audit = SessionAudit::new(
        AuditPrincipal::Person { id: user_id.into() },
        user_id,
        "session_issued",
        &row.id,
    );
    Ok(PreparedBrowserSession {
        issued: IssuedSession {
            session_token: secret,
            session_id: row.id.clone(),
        },
        session: row,
        audit,
    })
}

/// The browser response layer has already prepared the cookie before consuming a
/// device grant; its hash is persisted without exposing it to the audit event.
pub(super) fn prepare_browser_session_with_hash(
    user_id: &str,
    hash: &str,
    ip: Option<&str>,
    ua: Option<&str>,
) -> AppResult<PreparedBrowserSession> {
    let mut prepared = prepare_browser_session(user_id, ip, ua)?;
    prepared.session.token_hash = hash.into();
    Ok(prepared)
}

async fn insert_session_row(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    row: &Session,
) -> AppResult<()> {
    live_person(db, session, &row.user_id).await?;
    if let Some(existing) = db
        .collection::<Session>(SESSIONS)
        .find_one(doc! {"_id": &row.id})
        .session(&mut *session)
        .await?
    {
        if existing.user_id != row.user_id || existing.token_hash != row.token_hash {
            return Err(AppError::AuditIntegrityConflict);
        }
        if existing.revoked || existing.expires_at <= Utc::now() {
            return Err(AppError::Unauthorized("Session expired or revoked".into()));
        }
    } else {
        db.collection::<Session>(SESSIONS)
            .insert_one(row)
            .session(session)
            .await?;
    }
    Ok(())
}

pub async fn insert_prepared_browser_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    prepared: &PreparedBrowserSession,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    insert_session_row(db, session, &prepared.session).await?;
    prepared
        .audit
        .enqueue(db, runtime, session, AuditTier::Availability)
        .await
}

pub async fn create_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    user_id: &str,
    ip: Option<&str>,
    ua: Option<&str>,
) -> AppResult<IssuedSession> {
    let prepared = prepare_browser_session(user_id, ip, ua)?;
    let context = (db, runtime, &prepared);
    transaction::run(db, &context, |session, (db, runtime, prepared)| {
        Box::pin(
            async move { insert_prepared_browser_session(db, runtime, prepared, session).await },
        )
    })
    .await?;
    Ok(prepared.issued)
}

pub async fn create_session_and_issue_tokens(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip: Option<&str>,
    ua: Option<&str>,
) -> AppResult<IssuedTokens> {
    let prepared = prepare_session_tokens(db, config, jwt_keys, user_id, ip, ua).await?;
    issue_prepared_session(db, runtime, prepared).await
}

pub async fn issue_prepared_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    prepared: PreparedSessionTokens,
) -> AppResult<IssuedTokens> {
    let context = (db, runtime, &prepared);
    transaction::run(db, &context, |session, (db, runtime, prepared)| {
        Box::pin(async move { insert_prepared_session(db, runtime, prepared, session).await })
    })
    .await?;
    Ok(prepared.tokens)
}

pub struct PreparedSessionTokens {
    pub tokens: IssuedTokens,
    session: Session,
    refresh: RefreshToken,
    audit: SessionAudit,
}

impl PreparedSessionTokens {
    pub fn user_id(&self) -> &str {
        &self.session.user_id
    }

    /// Preserve the authenticated caller when a credential creates a session
    /// for its owner. Password/social proof uses the default verified person.
    pub fn with_principal(mut self, principal: AuditPrincipal) -> Self {
        self.audit.event.principal = principal;
        self
    }
}

pub async fn insert_prepared_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    prepared: &PreparedSessionTokens,
    transaction: &mut mongodb::ClientSession,
) -> AppResult<()> {
    insert_session_row(db, transaction, &prepared.session).await?;
    if let Some(existing) = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! {"_id": &prepared.refresh.id})
        .session(&mut *transaction)
        .await?
    {
        if existing.jti != prepared.refresh.jti
            || existing.user_id != prepared.refresh.user_id
            || existing.session_id != prepared.refresh.session_id
        {
            return Err(AppError::AuditIntegrityConflict);
        }
        if existing.revoked {
            return Err(AppError::Unauthorized(
                "Refresh token has been revoked".into(),
            ));
        }
    } else {
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(&prepared.refresh)
            .session(&mut *transaction)
            .await?;
    }
    prepared
        .audit
        .enqueue(db, runtime, transaction, AuditTier::Availability)
        .await
}

pub async fn prepare_session_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<PreparedSessionTokens> {
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let now = Utc::now();
    let session = Session {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_owned(),
        token_hash: hash_token(&generate_random_token()),
        ip_address: ip_address.map(str::to_owned),
        user_agent: user_agent.map(str::to_owned),
        expires_at: now + Duration::seconds(SESSION_TTL_SECS),
        revoked: false,
        created_at: now,
        last_active_at: now,
    };

    // Resolve RBAC data and inject into the access token based on scope
    let scope = FIRST_PARTY_ACCESS_SCOPES;
    let mut rbac_data =
        crate::services::rbac_helpers::build_rbac_claim_data(db, user_id, scope).await?;
    rbac_data.sid = Some(session.id.clone());
    let access_token = jwt::generate_access_token(
        jwt_keys,
        config,
        &user_uuid,
        scope,
        Some(&rbac_data),
        None,
        None,
        None,
        None,
    )?;

    // Generate refresh token
    let (refresh_token_jwt, refresh_jti) =
        jwt::generate_refresh_token(jwt_keys, config, &user_uuid)?;

    let refresh_id = Uuid::new_v4().to_string();
    let refresh_expires = now + Duration::seconds(config.jwt_refresh_ttl_secs);

    // Persist refresh token metadata
    let new_refresh = RefreshToken {
        protected_consent: None,
        id: refresh_id,
        jti: refresh_jti,
        client_id: Uuid::nil().to_string(), // first-party client
        user_id: user_id.to_string(),
        session_id: Some(session.id.clone()),
        scope: None,
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        revoked_at: None,
        resource_uris: Vec::new(),
        allowed_service_ids: Vec::new(),
        allow_all_services: true,
        created_at: now,
    };

    let tokens = IssuedTokens {
        access_token,
        refresh_token: refresh_token_jwt,
        session_id: session.id.clone(),
        access_expires_in: config.jwt_access_ttl_secs,
        resource_uris: Vec::new(),
    };
    let audit = SessionAudit::new(
        AuditPrincipal::Person { id: user_id.into() },
        user_id,
        "session_issued",
        &session.id,
    );
    Ok(PreparedSessionTokens {
        tokens,
        session,
        refresh: new_refresh,
        audit,
    })
}

/// Create a short-lived pending MFA session.
///
/// This binds a temporary token hash to the user_id so the MFA verification
/// step can validate that the user already passed password authentication.
/// The session expires in 5 minutes and is marked with a specific user_agent
/// to distinguish it from real sessions.
pub async fn create_mfa_pending_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    user_id: &str,
    temp_token_hash: &str,
) -> AppResult<String> {
    let mut prepared = prepare_browser_session(user_id, None, Some("mfa_pending"))?;
    prepared.session.token_hash = temp_token_hash.into();
    prepared.session.expires_at = prepared.session.created_at + Duration::minutes(5);
    prepared.audit.event.action = "mfa_pending_issued";
    let context = (db, runtime, &prepared);
    transaction::run(db, &context, |session, (db, runtime, prepared)| {
        Box::pin(
            async move { insert_prepared_browser_session(db, runtime, prepared, session).await },
        )
    })
    .await?;
    Ok(prepared.session.id)
}

/// Refresh an expired access token using a valid refresh token.
///
/// Implements refresh token rotation: the old token is revoked and
/// a new refresh token is issued alongside the new access token.
/// Does NOT generate a new session token (reuses the existing session).
///
/// When `mcp_sessions` is provided, session revocations also cascade
/// to the affected user's bearer-less (session-fallback) MCP sessions.
pub async fn refresh_tokens(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    refresh_token_str: &str,
    mcp_sessions: Option<&McpSessionStore>,
    requested_resources: Option<&[String]>,
) -> AppResult<IssuedTokens> {
    let claims = jwt::verify_token(jwt_keys, config, refresh_token_str)?;
    if claims.token_type != "refresh" {
        return Err(AppError::Unauthorized("Expected refresh token".into()));
    }
    let owner = claims.sub.to_string();
    let user_uuid = Uuid::parse_str(&owner)
        .map_err(|_| AppError::Unauthorized("Invalid refresh identity".into()))?;
    // Resolve and sign from prepared, immutable grant inputs before mutation.
    // The transaction compares them with the live row/replacement and fences
    // revocation; a rejected resource request must not consume a refresh token.
    let grant = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! {"jti": &claims.jti, "user_id": &owner})
        .await?
        .ok_or_else(|| AppError::Unauthorized("Refresh token not found".into()))?;
    let scope = grant.scope.as_deref().unwrap_or(FIRST_PARTY_ACCESS_SCOPES);
    let resources = super::oauth_resource_service::resolve_token_resource_scope(
        db,
        config,
        &owner,
        requested_resources,
        &grant.resource_uris,
        &grant.allowed_service_ids,
        grant.allow_all_services,
    )
    .await?;
    let mut rbac = super::rbac_helpers::build_rbac_claim_data(db, &owner, scope).await?;
    let (new_jwt, new_jti) = jwt::generate_refresh_token(jwt_keys, config, &user_uuid)?;
    let new_id = Uuid::new_v4().to_string();
    let audit = SessionAudit::new(
        AuditPrincipal::Person { id: owner.clone() },
        &owner,
        "session_refreshed",
        &new_id,
    );
    let context = (
        db,
        runtime,
        &owner,
        &claims.jti,
        &new_id,
        &new_jti,
        config,
        &audit,
        &grant,
    );
    let decision = transaction::run(
        db,
        &context,
        |session, (db, runtime, owner, jti, new_id, new_jti, config, audit, grant)| {
            Box::pin(async move {
                let stored = db
                    .collection::<RefreshToken>(REFRESH_TOKENS)
                    .find_one(doc! {"jti": *jti, "user_id": *owner})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::Unauthorized("Refresh token not found".into()))?;
                if stored.expires_at <= Utc::now() {
                    return Err(AppError::Unauthorized("Refresh token expired".into()));
                }
                live_person(db, session, owner).await?;
                if stored.client_id == Uuid::nil().to_string()
                    && let Some(sid) = &stored.session_id
                {
                    let changed = db
                        .collection::<Session>(SESSIONS)
                        .update_one(
                            doc! {"_id": sid, "user_id": *owner, "revoked": false,
                            "expires_at": {"$gt": bson::DateTime::now()}},
                            doc! {"$set": {"last_active_at": bson::DateTime::now()}},
                        )
                        .session(&mut *session)
                        .await?;
                    if changed.matched_count != 1 {
                        return Err(AppError::Unauthorized("Session expired or revoked".into()));
                    }
                }
                if stored.client_id != Uuid::nil().to_string() {
                    let active = db
                        .collection::<OauthClient>(OAUTH_CLIENTS)
                        .find_one(doc! {"_id": &stored.client_id, "is_active": true})
                        .session(&mut *session)
                        .await?;
                    if active.is_none() {
                        let changed = db
                            .collection::<RefreshToken>(REFRESH_TOKENS)
                            .update_one(
                                doc! {"_id": &stored.id, "revoked": false},
                                doc! {"$set": {"revoked": true, "revoked_at": bson::DateTime::now()}},
                            )
                            .session(&mut *session)
                            .await?;
                        if changed.modified_count != 0 {
                            let event = SessionAudit {
                                event_id: audit.event_id,
                                event: AuditEvent {
                                    action: "refresh_client_revoked",
                                    target_id: Some(stored.id.clone()),
                                    ..audit.event.clone()
                                },
                            };
                            event
                                .enqueue(db, runtime, session, AuditTier::Narrowing)
                                .await?;
                        }
                        return Ok(Err(AppError::AuthenticationFailed(
                            "Issuing OAuth client is no longer active".into(),
                        )));
                    }
                    // Share the client's write fence with deactivation/deletion.
                    db.collection::<OauthClient>(OAUTH_CLIENTS)
                        .update_one(
                            doc! {"_id": &stored.client_id, "is_active": true},
                            doc! {"$set": {"updated_at": bson::DateTime::now()}},
                        )
                        .session(&mut *session)
                        .await?;
                }
                let original_reference = stored.protected_consent.clone();
                let reused = stored.revoked;
                let active = if stored.revoked {
                    let within = stored
                        .revoked_at
                        .is_none_or(|at| (Utc::now() - at).num_seconds() <= REUSE_GRACE_PERIOD_SECS);
                    let next = if within && let Some(first) = &stored.replaced_by {
                        follow_active_replacement_chain(db, session, first, owner).await?
                    } else {
                        None
                    };
                    match next {
                        Some(next) => next,
                        None => {
                            if let Some(sid) = &stored.session_id {
                                revoke_session_in_session(
                                    db,
                                    runtime,
                                    session,
                                    sid,
                                    &audit.event.principal,
                                    audit,
                                )
                                .await?;
                            }
                            return Ok(Err(AppError::Unauthorized(
                                "Refresh token has been revoked".into(),
                            )));
                        }
                    }
                } else {
                    stored
                };
                if active.protected_consent != original_reference
                    || active.protected_consent != grant.protected_consent
                    || active.client_id != grant.client_id
                    || active.session_id != grant.session_id
                    || active.scope != grant.scope
                    || active.resource_uris != grant.resource_uris
                    || active.allowed_service_ids != grant.allowed_service_ids
                    || active.allow_all_services != grant.allow_all_services
                {
                    return Err(super::consent_service::grant_changed());
                }
                let scope = active.scope.as_deref().unwrap_or(FIRST_PARTY_ACCESS_SCOPES);
                super::consent_service::fence_protected_reference_in_session(
                    db,
                    session,
                    &active.user_id,
                    &active.client_id,
                    scope,
                    active.protected_consent.as_ref(),
                    &audit.event.operation_id,
                )
                .await?;
                if reused {
                    return Ok(Ok((active, true)));
                }
                let now = Utc::now();
                let changed = db.collection::<RefreshToken>(REFRESH_TOKENS).update_one(doc! {"_id": &active.id, "revoked": false},
                        doc! {"$set": {"revoked": true, "revoked_at": bson::DateTime::from_chrono(now), "replaced_by": *new_id}}).session(&mut *session).await?;
                if changed.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "Refresh token was concurrently rotated, please retry".into(),
                    ));
                }
                let next = RefreshToken {
                    id: (*new_id).clone(),
                    jti: (*new_jti).clone(),
                    expires_at: now + Duration::seconds(config.jwt_refresh_ttl_secs),
                    revoked: false,
                    revoked_at: None,
                    replaced_by: None,
                    created_at: now,
                    ..active
                };
                db.collection::<RefreshToken>(REFRESH_TOKENS)
                    .insert_one(&next)
                    .session(&mut *session)
                    .await?;
                audit
                    .enqueue(db, runtime, session, AuditTier::Availability)
                    .await?;
                Ok(Ok((next, false)))
            })
        },
    )
    .await?;
    let (active, reused) = match decision {
        Ok(value) => value,
        Err(error) => {
            if let Some(mcp) = mcp_sessions {
                mcp.remove_fallback_sessions_by_user_id(&owner).await?;
            }
            return Err(error);
        }
    };
    let scope = active.scope.as_deref().unwrap_or(FIRST_PARTY_ACCESS_SCOPES);
    let restrictions = (!resources.allow_all_services).then_some(jwt::AccessTokenRestrictions {
        resources: &resources.resource_uris,
        allowed_service_ids: &resources.allowed_service_ids,
        allowed_node_ids: &[],
        allow_all_nodes: true,
    });
    let access = if active.client_id == Uuid::nil().to_string() {
        rbac.sid = active.session_id.clone();
        jwt::generate_access_token(
            jwt_keys,
            config,
            &user_uuid,
            scope,
            Some(&rbac),
            None,
            None,
            None,
            restrictions,
        )?
    } else {
        jwt::generate_oauth_access_token_with_consent(
            jwt_keys,
            config,
            &user_uuid,
            scope,
            Some(&rbac),
            None,
            None,
            None,
            restrictions,
            &active.client_id,
            active.protected_consent.as_ref(),
        )?
    };
    if reused {
        return build_reused_refresh_response(
            jwt_keys,
            config,
            &user_uuid,
            &active,
            access,
            resources.resource_uris,
        );
    }
    Ok(IssuedTokens {
        access_token: access,
        refresh_token: new_jwt,
        session_id: active.session_id.unwrap_or_else(|| Uuid::nil().to_string()),
        access_expires_in: config.jwt_access_ttl_secs,
        resource_uris: resources.resource_uris,
    })
}

/// In-session hook for the caller's complete revocation transaction. The acting
/// identity is explicit; background expiry must pass System(Expiry).
pub async fn revoke_session_in_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    session: &mut mongodb::ClientSession,
    session_id: &str,
    principal: &AuditPrincipal,
    audit: &SessionAudit,
) -> AppResult<Option<String>> {
    let row = db
        .collection::<Session>(SESSIONS)
        .find_one(doc! {"_id": session_id})
        .session(&mut *session)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let wrong_owner = match principal {
        AuditPrincipal::Person { id } | AuditPrincipal::ServiceAccount { id } => id != &row.user_id,
        AuditPrincipal::ApiKey { owner_id, .. } => owner_id != &row.user_id,
        AuditPrincipal::System { .. } => false,
        AuditPrincipal::Node { .. } | AuditPrincipal::OracleWorker { .. } => true,
    };
    if wrong_owner {
        return Err(AppError::Forbidden(
            "Cannot revoke another user's session".into(),
        ));
    }
    let sessions = db
        .collection::<Session>(SESSIONS)
        .update_one(
            doc! {"_id": session_id, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .session(&mut *session)
        .await?;
    let tokens = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .update_many(
            doc! {"session_id": session_id, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .session(&mut *session)
        .await?;
    if sessions.modified_count != 0 || tokens.modified_count != 0 {
        let event = SessionAudit {
            event_id: audit.event_id,
            event: AuditEvent {
                principal: principal.clone(),
                owner: AuditOwner::Person {
                    id: row.user_id.clone(),
                },
                action: "session_revoked",
                target_id: Some(row.id),
                ..audit.event.clone()
            },
        };
        event
            .enqueue(db, runtime, session, AuditTier::Narrowing)
            .await?;
    }
    Ok(Some(row.user_id))
}

pub async fn revoke_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    session_id: &str,
    principal: AuditPrincipal,
    mcp_sessions: Option<&McpSessionStore>,
) -> AppResult<()> {
    let audit = SessionAudit::new(principal.clone(), "", "session_revoked", session_id);
    let context = (db, runtime, session_id, &principal, &audit);
    let owner = transaction::run(
        db,
        &context,
        |session, (db, runtime, id, principal, audit)| {
            Box::pin(async move {
                revoke_session_in_session(db, runtime, session, id, principal, audit).await
            })
        },
    )
    .await?;
    if let (Some(mcp), Some(owner)) = (mcp_sessions, owner) {
        mcp.remove_fallback_sessions_by_user_id(&owner).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    #[test]
    fn w8_id_verified_principal_keeps_key_owner_and_service_account_identity() {
        use crate::mw::auth::AuthMethod;
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        let key = Uuid::new_v4().to_string();
        let child = Uuid::new_v4().to_string();
        for owner in [&person, &org] {
            for method in [AuthMethod::ApiKey, AuthMethod::Relay] {
                let mut auth = test_auth_user(owner);
                auth.auth_method = method;
                auth.api_key_id = Some(key.clone());
                auth.api_key_credential_id = Some(child.clone());
                assert_eq!(
                    audit_principal(&auth).unwrap(),
                    AuditPrincipal::ApiKey {
                        id: key.clone(),
                        owner_id: owner.clone(),
                    }
                );
                auth.api_key_id = None;
                assert!(matches!(
                    audit_principal(&auth),
                    Err(AppError::Unauthorized(_))
                ));
            }
        }
        let sa = Uuid::new_v4().to_string();
        let mut auth = test_auth_user(&sa);
        auth.auth_method = AuthMethod::ServiceAccount;
        auth.approval_owner_user_id = Some(org);
        assert_eq!(
            audit_principal(&auth).unwrap(),
            AuditPrincipal::ServiceAccount { id: sa }
        );
        for method in [
            AuthMethod::Session,
            AuthMethod::AccessToken,
            AuthMethod::Delegated,
        ] {
            let mut auth = test_auth_user(&person);
            auth.auth_method = method;
            assert_eq!(
                audit_principal(&auth).unwrap(),
                AuditPrincipal::Person { id: person.clone() }
            );
        }
    }

    #[tokio::test]
    async fn w8_id_session_abort_commit_replay_and_safe_records() {
        use crate::models::audit_outbox::PENDING_COLLECTION_NAME as PENDING;
        use futures::TryStreamExt;
        let db = connect_transaction_test_database("w8_id_session_atomic").await;
        let state = test_app_state(db.clone());
        let actor = Uuid::new_v4().to_string();
        seed_user(&db, &actor).await;
        let prepared = prepare_session_tokens(
            &db,
            &state.config,
            &state.jwt_keys,
            &actor,
            Some("192.0.2.137"),
            Some("w8-id-private-agent"),
        )
        .await
        .unwrap();
        let context = (&db, &state.audit_runtime, &prepared);
        let aborted: AppResult<()> =
            transaction::run(&db, &context, |session, (db, runtime, prepared)| {
                Box::pin(async move {
                    insert_prepared_session(db, runtime, prepared, session).await?;
                    Err(AppError::Conflict("abort whole session issue".into()))
                })
            })
            .await;
        assert!(matches!(aborted, Err(AppError::Conflict(_))));
        for collection in [SESSIONS, REFRESH_TOKENS, PENDING] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        for _ in 0..2 {
            transaction::run(&db, &context, |session, (db, runtime, prepared)| {
                Box::pin(
                    async move { insert_prepared_session(db, runtime, prepared, session).await },
                )
            })
            .await
            .unwrap();
        }
        for collection in [SESSIONS, REFRESH_TOKENS, PENDING] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1
            );
        }
        assert_eq!(
            super::super::audit_state_service::load(&db)
                .await
                .unwrap()
                .pending_count,
            1
        );
        super::super::audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap();
        let forbidden = [
            prepared.tokens.access_token.clone(),
            prepared.tokens.refresh_token.clone(),
            prepared.session.token_hash.clone(),
            "192.0.2.137".into(),
            "w8-id-private-agent".into(),
        ];
        for collection in [
            PENDING,
            crate::models::audit_log::COLLECTION_NAME,
            crate::models::audit_outbox::COLLECTION_NAME,
        ] {
            let rows: Vec<bson::Document> = db
                .collection(collection)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert!(!rows.is_empty());
            for row in rows {
                let encoded = serde_json::to_string(&row).unwrap();
                for secret in &forbidden {
                    assert!(!encoded.contains(secret));
                }
            }
        }
        let pending: crate::models::audit_outbox::PendingAuditEvent = db
            .collection(PENDING)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            pending.event.principal,
            AuditPrincipal::Person { id: actor.clone() }
        );
        assert_eq!(pending.event.owner, AuditOwner::Person { id: actor });
        assert_eq!(pending.event.action, "session_issued");
    }

    #[tokio::test]
    async fn w8_id_session_unknown_commit_keeps_transaction_and_event_identity() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        let _guard = super::super::audit_state_service::tests::FAIL_COMMAND
            .lock()
            .await;
        let base = connect_transaction_test_database("w8_id_session_unknown").await;
        let app = format!("w8-id-{}", Uuid::new_v4());
        let commands = Arc::new(Mutex::new(Vec::new()));
        let captured = commands.clone();
        let mut options = mongodb::options::ClientOptions::parse(
            std::env::var("NYXID_TEST_DATABASE_URL").unwrap(),
        )
        .await
        .unwrap();
        options.app_name = Some(app.clone());
        options.command_event_handler = Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command_name == "commitTransaction"
            {
                captured.lock().unwrap().push(event.command);
            }
        }));
        let db = mongodb::Client::with_options(options)
            .unwrap()
            .database(base.name());
        let state = test_app_state(db.clone());
        let actor = Uuid::new_v4().to_string();
        seed_user(&db, &actor).await;
        let prepared =
            prepare_session_tokens(&db, &state.config, &state.jwt_keys, &actor, None, None)
                .await
                .unwrap();
        base.client()
            .database("admin")
            .run_command(doc! {
                "configureFailPoint":"failCommand","mode":{"times":1},"data":{
                    "failCommands":["commitTransaction"],"appName":&app,
                    "writeConcernError":{"code":64,"errmsg":"injected uncertain session commit"},
                    "errorLabels":["UnknownTransactionCommitResult"]
                }
            })
            .await
            .unwrap();
        let attempts = AtomicUsize::new(0);
        let context = (&db, &state.audit_runtime, &prepared, &attempts);
        transaction::run(
            &db,
            &context,
            |session, (db, runtime, prepared, attempts)| {
                Box::pin(async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    insert_prepared_session(db, runtime, prepared, session).await
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        let commits = commands.lock().unwrap().clone();
        assert!(commits.len() >= 2);
        for commit in &commits[1..] {
            assert_eq!(commit.get("lsid"), commits[0].get("lsid"));
            assert_eq!(commit.get("txnNumber"), commits[0].get("txnNumber"));
        }
        let context = (&db, &state.audit_runtime, &prepared);
        transaction::run(&db, &context, |session, (db, runtime, prepared)| {
            Box::pin(async move { insert_prepared_session(db, runtime, prepared, session).await })
        })
        .await
        .unwrap();
        for collection in [
            SESSIONS,
            REFRESH_TOKENS,
            crate::models::audit_outbox::PENDING_COLLECTION_NAME,
        ] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1
            );
        }
    }

    #[tokio::test]
    async fn w8_id_session_person_owner_isolation_and_system_revocation() {
        use crate::models::{
            audit_outbox::{
                PENDING_COLLECTION_NAME as PENDING, PendingAuditEvent, SystemAuditActor,
            },
            user::{COLLECTION_NAME as USERS, User, UserType},
        };
        let db = connect_transaction_test_database("w8_id_session_owners").await;
        let state = test_app_state(db.clone());
        let people = [Uuid::new_v4().to_string(), Uuid::new_v4().to_string()];
        let orgs = [Uuid::new_v4().to_string(), Uuid::new_v4().to_string()];
        for person in &people {
            seed_user(&db, person).await;
        }
        for org in &orgs {
            db.collection::<User>(USERS)
                .insert_one(test_user(org, UserType::Org))
                .await
                .unwrap();
            assert!(
                create_session(&db, &state.audit_runtime, org, None, None)
                    .await
                    .is_err()
            );
        }
        let first = create_session(&db, &state.audit_runtime, &people[0], None, None)
            .await
            .unwrap();
        let second = create_session(&db, &state.audit_runtime, &people[1], None, None)
            .await
            .unwrap();
        assert!(matches!(
            revoke_session(
                &db,
                &state.audit_runtime,
                &first.session_id,
                AuditPrincipal::Person {
                    id: people[1].clone()
                },
                None
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        let key = Uuid::new_v4().to_string();
        for owner in [&people[1], &orgs[0], &orgs[1]] {
            assert!(matches!(
                revoke_session(
                    &db,
                    &state.audit_runtime,
                    &first.session_id,
                    AuditPrincipal::ApiKey {
                        id: key.clone(),
                        owner_id: owner.clone()
                    },
                    None,
                )
                .await,
                Err(AppError::Forbidden(_))
            ));
        }
        revoke_session(
            &db,
            &state.audit_runtime,
            &first.session_id,
            AuditPrincipal::System {
                actor: SystemAuditActor::Expiry,
            },
            None,
        )
        .await
        .unwrap();
        assert!(
            !db.collection::<Session>(SESSIONS)
                .find_one(doc! {"_id":&second.session_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
        let event = db
            .collection::<PendingAuditEvent>(PENDING)
            .find_one(doc! {"event.action":"session_revoked"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            event.event.principal,
            AuditPrincipal::System {
                actor: SystemAuditActor::Expiry
            }
        );
        assert_eq!(
            event.event.owner,
            AuditOwner::Person {
                id: people[0].clone()
            }
        );
        assert_eq!(event.event.target_id, Some(first.session_id));
        let principal = AuditPrincipal::ApiKey {
            id: key,
            owner_id: people[1].clone(),
        };
        revoke_session(
            &db,
            &state.audit_runtime,
            &second.session_id,
            principal.clone(),
            None,
        )
        .await
        .unwrap();
        let event = db
            .collection::<PendingAuditEvent>(PENDING)
            .find_one(
                doc! {"event.action": "session_revoked", "event.target_id": &second.session_id},
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.event.principal, principal);
        assert_eq!(
            event.event.owner,
            AuditOwner::Person {
                id: people[1].clone()
            }
        );
    }

    #[tokio::test]
    async fn w8_id_session_and_mfa_c2_tier_cells() {
        use super::super::mfa_service::{self, MfaActor};
        use crate::models::{
            audit_outbox::SystemAuditActor,
            user::{COLLECTION_NAME as USERS, User},
        };
        let hash = crate::crypto::password::hash_password("w8-id-test-password").unwrap();
        // Persisted enrollment without authenticated contact is stale, never ready.
        // Each tuple covers age/count independently; Narrowing must always record.
        for (label, age, count, diverged, availability, standard) in [
            ("stale", 0, 1, false, true, true),
            ("standard_age", 7200, 1, false, true, false),
            ("availability_age", 90000, 1, false, false, false),
            ("standard_count", 0, 100000, false, true, false),
            ("availability_count", 0, 1000000, false, false, false),
            ("diverged", 0, 1, true, true, false),
        ] {
            let db = connect_transaction_test_database(&format!("w8_id_c2_{label}")).await;
            let state = test_app_state(db.clone());
            let actor = Uuid::new_v4().to_string();
            let mut user = test_user(&actor, crate::models::user::UserType::Person);
            user.password_hash = Some(hash.clone());
            user.mfa_enabled = true;
            db.collection::<User>(USERS).insert_one(user).await.unwrap();
            let initial = create_session(&db, &state.audit_runtime, &actor, None, None)
                .await
                .unwrap();
            // Model persisted backlog capacity without constructing a million
            // events. Keep its allocation high-water mark and timestamp valid.
            db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME).update_one(doc! {"singleton":true},doc! {"$set":{
                "witness_enrolled":true,"pending_count":count as i64,"next_order":count as i64,
                "oldest_pending_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(age))
            }}).await.unwrap();
            if diverged {
                let chain = super::super::audit_state_service::load(&db)
                    .await
                    .unwrap()
                    .id;
                db.collection::<bson::Document>(crate::models::audit_witness_delivery::COLLECTION_NAME).insert_one(doc! {
                    "_id":&chain,"divergence":{"seq":1_i64,"local_hash":"a".repeat(64),"witnessed_hash":"b".repeat(64)}
                }).await.unwrap();
                state
                    .audit_runtime
                    .restore_incident(&db, &chain)
                    .await
                    .unwrap();
            }
            let issued = create_session(&db, &state.audit_runtime, &actor, None, None).await;
            assert_eq!(
                issued.is_ok(),
                availability,
                "{label}: Availability: {:?}",
                issued.as_ref().err()
            );
            let disabled = mfa_service::disable_totp(
                &db,
                &state.audit_runtime,
                &MfaActor {
                    user_id: actor.clone(),
                    principal: AuditPrincipal::Person { id: actor.clone() },
                },
                "w8-id-test-password",
            )
            .await;
            assert_eq!(
                disabled.is_ok(),
                standard,
                "{label}: Standard: {:?}",
                disabled.as_ref().err()
            );
            let user = db
                .collection::<User>(USERS)
                .find_one(doc! {"_id":&actor})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                user.mfa_enabled, !standard,
                "{label}: rejected mutation rolls back"
            );
            revoke_session(
                &db,
                &state.audit_runtime,
                &initial.session_id,
                AuditPrincipal::System {
                    actor: SystemAuditActor::Expiry,
                },
                None,
            )
            .await
            .unwrap();
            assert!(
                db.collection::<Session>(SESSIONS)
                    .find_one(doc! {"_id":&initial.session_id})
                    .await
                    .unwrap()
                    .unwrap()
                    .revoked
            );
        }
    }

    async fn seed_user(db: &mongodb::Database, user_id: &str) {
        let user = test_user(user_id, crate::models::user::UserType::Person);
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(&user)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_session_inserts_record() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let result = create_session(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &user_id,
            Some("127.0.0.1"),
            Some("test-agent"),
        )
        .await;
        assert!(result.is_ok());

        let issued = result.unwrap();
        assert!(!issued.session_token.is_empty());
        assert!(!issued.session_id.is_empty());

        let stored = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &issued.session_id })
            .await
            .unwrap();
        assert!(stored.is_some());
        let stored = stored.unwrap();
        assert_eq!(stored.user_id, user_id);
        assert!(!stored.revoked);
        assert_eq!(stored.ip_address.as_deref(), Some("127.0.0.1"));
        assert_eq!(stored.user_agent.as_deref(), Some("test-agent"));
    }

    #[tokio::test]
    async fn test_create_session_rejects_invalid_uuid() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let result = create_session(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            "not-a-uuid",
            None,
            None,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_session_with_none_optional_fields() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &user_id,
            None,
            None,
        )
        .await
        .unwrap();
        let stored = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &issued.session_id })
            .await
            .unwrap()
            .unwrap();
        assert!(stored.ip_address.is_none());
        assert!(stored.user_agent.is_none());
    }

    #[tokio::test]
    async fn test_create_session_and_issue_tokens_returns_valid_tokens() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let result = create_session_and_issue_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &user_id,
            Some("10.0.0.1"),
            Some("browser/1.0"),
        )
        .await;
        assert!(result.is_ok());

        let issued = result.unwrap();
        assert!(!issued.access_token.is_empty());
        assert!(!issued.refresh_token.is_empty());
        assert!(!issued.session_id.is_empty());
        assert_eq!(issued.access_expires_in, config.jwt_access_ttl_secs);

        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.access_token).unwrap();
        assert_eq!(access_claims.sub, user_id);
        assert_eq!(access_claims.token_type, "access");

        let refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        assert_eq!(refresh_claims.sub, user_id);
        assert_eq!(refresh_claims.token_type, "refresh");
    }

    #[tokio::test]
    async fn test_create_session_and_issue_tokens_persists_refresh_token() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &user_id,
            None,
            None,
        )
        .await
        .unwrap();

        let refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        let stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &refresh_claims.jti })
            .await
            .unwrap();
        assert!(stored.is_some());
        let stored = stored.unwrap();
        assert_eq!(stored.user_id, user_id);
        assert!(!stored.revoked);
        assert_eq!(
            stored.session_id.as_deref(),
            Some(issued.session_id.as_str())
        );
    }

    #[tokio::test]
    async fn test_refresh_tokens_rotates_refresh_token() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &user_id,
            None,
            None,
        )
        .await
        .unwrap();

        let refreshed = refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &issued.refresh_token,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(!refreshed.access_token.is_empty());
        assert!(!refreshed.refresh_token.is_empty());
        assert_ne!(refreshed.refresh_token, issued.refresh_token);

        let old_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        let old_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &old_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(old_stored.revoked);
        assert!(old_stored.replaced_by.is_some());
    }

    #[tokio::test]
    async fn test_refresh_tokens_preserves_oauth_service_scope() {
        let Some(db) = connect_test_database("token_svc_oauth_scope").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let client_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.clone(),
                client_name: "Scoped OAuth Client".to_string(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["https://app.example/callback".to_string()],
                allowed_scopes: "openid proxy".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                default_service_catalog_slugs: Vec::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: Some(user_id.clone()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let user_uuid = Uuid::parse_str(&user_id).unwrap();
        let (refresh_jwt, refresh_jti) =
            crate::crypto::jwt::generate_refresh_token(&jwt_keys, &config, &user_uuid).unwrap();
        let allowed_service_ids = vec!["svc-1".to_string()];
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(RefreshToken {
                protected_consent: None,
                id: Uuid::new_v4().to_string(),
                jti: refresh_jti,
                client_id: client_id.clone(),
                user_id: user_id.clone(),
                session_id: None,
                scope: Some("openid proxy".to_string()),
                expires_at: now + Duration::days(7),
                revoked: false,
                replaced_by: None,
                revoked_at: None,
                resource_uris: Vec::new(),
                allowed_service_ids: allowed_service_ids.clone(),
                allow_all_services: false,
                created_at: now,
            })
            .await
            .unwrap();

        let refreshed = refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &refresh_jwt,
            None,
            None,
        )
        .await
        .unwrap();
        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.access_token).unwrap();
        assert_eq!(access_claims.scope, "openid proxy");
        assert_eq!(access_claims.client_id.as_deref(), Some(client_id.as_str()));
        assert_eq!(access_claims.allow_all_services, Some(false));
        assert_eq!(access_claims.allowed_service_ids, Some(allowed_service_ids));

        let new_refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.refresh_token).unwrap();
        let new_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": new_refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new_stored.scope.as_deref(), Some("openid proxy"));
        assert!(!new_stored.allow_all_services);
        assert_eq!(new_stored.allowed_service_ids, vec!["svc-1".to_string()]);
    }

    #[tokio::test]
    async fn test_refresh_resource_narrowing_is_per_request() {
        let Some(db) = connect_test_database("token_svc_resource_per_request").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let service_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(test_user_service(
            &service_id,
            &user_id,
            "svc-a",
            "endpoint-a",
            None,
            None,
        ))
        .await
        .unwrap();
        let resource_a =
            crate::services::oauth_resource_service::user_service_resource_uri(&config, "svc-a");

        let issued = create_session_and_issue_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &user_id,
            None,
            None,
        )
        .await
        .unwrap();

        let requested = vec![resource_a.clone()];
        let narrowed = refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &issued.refresh_token,
            None,
            Some(&requested),
        )
        .await
        .unwrap();

        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &narrowed.access_token).unwrap();
        assert_eq!(access_claims.resources, Some(vec![resource_a.clone()]));
        assert_eq!(access_claims.allowed_service_ids, Some(vec![service_id]));
        assert_eq!(access_claims.allow_all_services, Some(false));

        let narrowed_refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &narrowed.refresh_token).unwrap();
        let narrowed_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": narrowed_refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(narrowed_stored.allow_all_services);
        assert!(narrowed_stored.resource_uris.is_empty());
        assert!(narrowed_stored.allowed_service_ids.is_empty());

        let unrestricted = refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &narrowed.refresh_token,
            None,
            None,
        )
        .await
        .unwrap();
        let unrestricted_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &unrestricted.access_token)
                .unwrap();
        assert!(unrestricted_claims.resources.is_none());
        assert!(unrestricted_claims.allowed_service_ids.is_none());
        assert!(unrestricted_claims.allow_all_services.is_none());
    }

    #[tokio::test]
    async fn test_refresh_accepts_resource_within_consent_only_service_grant() {
        let Some(db) = connect_test_database("token_svc_consent_only_resource").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let client_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.clone(),
                client_name: "Consent Only OAuth Client".to_string(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["https://app.example/callback".to_string()],
                allowed_scopes: "openid proxy".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: Some(user_id.clone()),
                default_service_catalog_slugs: Vec::new(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let service_a_id = Uuid::new_v4().to_string();
        let service_b_id = Uuid::new_v4().to_string();
        let services = db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        );
        services
            .insert_many([
                test_user_service(&service_a_id, &user_id, "svc-a", "endpoint-a", None, None),
                test_user_service(&service_b_id, &user_id, "svc-b", "endpoint-b", None, None),
            ])
            .await
            .unwrap();
        let resource_a =
            crate::services::oauth_resource_service::user_service_resource_uri(&config, "svc-a");
        let resource_b =
            crate::services::oauth_resource_service::user_service_resource_uri(&config, "svc-b");

        let user_uuid = Uuid::parse_str(&user_id).unwrap();
        let (refresh_jwt, refresh_jti) =
            crate::crypto::jwt::generate_refresh_token(&jwt_keys, &config, &user_uuid).unwrap();
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(RefreshToken {
                protected_consent: None,
                id: Uuid::new_v4().to_string(),
                jti: refresh_jti,
                client_id: client_id.clone(),
                user_id: user_id.clone(),
                session_id: None,
                scope: Some("openid proxy".to_string()),
                expires_at: now + Duration::days(7),
                revoked: false,
                replaced_by: None,
                revoked_at: None,
                resource_uris: Vec::new(),
                allowed_service_ids: vec![service_a_id.clone()],
                allow_all_services: false,
                created_at: now,
            })
            .await
            .unwrap();

        let requested_a = vec![resource_a.clone()];
        let refreshed = refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &refresh_jwt,
            None,
            Some(&requested_a),
        )
        .await
        .unwrap();
        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.access_token).unwrap();
        assert_eq!(access_claims.resources, Some(vec![resource_a]));
        assert_eq!(
            access_claims.allowed_service_ids,
            Some(vec![service_a_id.clone()])
        );
        assert_eq!(access_claims.allow_all_services, Some(false));

        let new_refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.refresh_token).unwrap();
        let new_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": new_refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(!new_stored.allow_all_services);
        assert!(new_stored.resource_uris.is_empty());
        assert_eq!(new_stored.allowed_service_ids, vec![service_a_id]);

        let requested_b = vec![resource_b];
        let err = match refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &refreshed.refresh_token,
            None,
            Some(&requested_b),
        )
        .await
        {
            Ok(_) => panic!("resource outside consented services is rejected"),
            Err(err) => err,
        };
        assert!(matches!(err, AppError::InvalidTarget(_)));
    }

    #[tokio::test]
    async fn test_refresh_tokens_rejects_non_refresh_token() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4();

        let access_token = crate::crypto::jwt::generate_access_token(
            &jwt_keys, &config, &user_id, "openid", None, None, None, None, None,
        )
        .unwrap();

        let result = refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &access_token,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_refresh_tokens_rejects_revoked_token_no_replacement() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &user_id,
            None,
            None,
        )
        .await
        .unwrap();

        let claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(
                doc! { "jti": &claims.jti },
                doc! { "$set": { "revoked": true } },
            )
            .await
            .unwrap();

        let result = refresh_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &issued.refresh_token,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_revoke_session_marks_session_and_tokens() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &config,
            &jwt_keys,
            &user_id,
            None,
            None,
        )
        .await
        .unwrap();

        revoke_session(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &issued.session_id,
            AuditPrincipal::Person {
                id: user_id.clone(),
            },
            None,
        )
        .await
        .unwrap();

        let session = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &issued.session_id })
            .await
            .unwrap()
            .unwrap();
        assert!(session.revoked);

        let refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        let refresh = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(refresh.revoked);
    }

    #[tokio::test]
    async fn test_create_mfa_pending_session() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let temp_hash = "abc123hash";
        let session_id = create_mfa_pending_session(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &user_id,
            temp_hash,
        )
        .await
        .unwrap();

        let session = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &session_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.user_id, user_id);
        assert_eq!(session.token_hash, temp_hash);
        assert_eq!(session.user_agent.as_deref(), Some("mfa_pending"));
        assert!(!session.revoked);
    }
}
