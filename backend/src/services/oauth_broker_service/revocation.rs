//! Revocation and notification evidence commit together; receiver I/O never
//! participates in the transaction.
use super::*;
use mongodb::{ClientSession, Database};

use crate::models::security_effect_intent::{SecurityEffect, SecurityEffectIntent};
use crate::services::{
    audit_service::{
        AuditActor,
        required::{self, TextField},
    },
    security_effect_service,
};

fn cae_intent(binding: &OauthBrokerBinding, source: &str) -> SecurityEffectIntent {
    security_effect_service::new_intent(
        Uuid::new_v4().to_string(),
        SecurityEffect::Cae {
            binding_id: binding.id.clone(),
            client_id: binding.client_id.clone(),
            revoke_source: source.into(),
        },
    )
}

fn revoke_source(reason: &str) -> &'static str {
    match reason {
        "client" | "client_revoked" => "client",
        "user" | "user_revoked" => "user",
        "reuse_detected" => "reuse_detected",
        _ => "admin",
    }
}

/// The caller batches these intents with its action audit in one insert.
pub(crate) async fn revoke_user_client_in_session(
    db: &Database,
    session: &mut ClientSession,
    client_id: &str,
    user_id: &str,
    reason: &str,
) -> AppResult<(u64, Vec<SecurityEffectIntent>)> {
    let filter = doc! {"client_id": client_id, "user_id": user_id, "revoked": false};
    let mut cursor = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find(filter.clone())
        .session(&mut *session)
        .await?;
    let bindings: Vec<OauthBrokerBinding> = cursor.stream(&mut *session).try_collect().await?;
    if bindings.is_empty() {
        return Ok((0, vec![]));
    }
    let now = bson::DateTime::now();
    let result = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .update_many(
            filter,
            doc! {"$set": {"revoked": true, "revoked_at": now, "revoke_reason": reason}},
        )
        .session(&mut *session)
        .await?;
    let jtis: Vec<_> = bindings
        .iter()
        .map(|b| b.refresh_token_jti.as_str())
        .collect();
    db.collection::<RefreshToken>(REFRESH_TOKENS).update_many(
        doc! {"jti": {"$in": jtis}, "client_id": client_id, "user_id": user_id, "revoked": false},
        doc! {"$set": {"revoked": true, "revoked_at": now}}).session(session).await?;
    Ok((
        result.modified_count,
        bindings
            .iter()
            .map(|b| cae_intent(b, revoke_source(reason)))
            .collect(),
    ))
}

pub(super) async fn revoke_reused_bindings(
    db: &Database,
    binding: &OauthBrokerBinding,
    binding_hash: &str,
) -> AppResult<()> {
    let context = (db, binding, binding_hash);
    crate::services::transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(revoke_reused_in_session(session, context))
    })
    .await
}

async fn revoke_reused_in_session(
    session: &mut ClientSession,
    (db, binding, binding_hash): &(&Database, &OauthBrokerBinding, &str),
) -> AppResult<()> {
    let (count, mut intents) = revoke_user_client_in_session(
        db,
        session,
        &binding.client_id,
        &binding.user_id,
        "reuse_detected",
    )
    .await?;
    let actor = AuditActor {
        user_id: binding.user_id.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    intents.push(required::intent(
        Some(&actor),
        "oauth_broker_binding_reuse_detected",
        Some(serde_json::json!({"client_id": required::text(TextField::Identifier, &binding.client_id),
            "binding_hash": binding_hash_prefix(binding_hash), "cascade_revoke_count": count})),
    )?);
    security_effect_service::enqueue_in_session(db, session, &intents).await
}

pub async fn revoke_binding_by_client_as(
    db: &Database,
    client_id: &str,
    raw_binding_id: &str,
    reason: &str,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<bool> {
    let context = RevokeOne {
        db,
        filter: doc! {"_id": hash_binding_id(raw_binding_id), "client_id": client_id, "revoked": false},
        reason,
        source: "client",
        actor: None,
        ip,
        user_agent,
    };
    crate::services::transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(revoke_one_in_session(session, context))
    })
    .await
}

pub async fn revoke_binding_by_user_as(
    db: &Database,
    actor: &AuditActor,
    binding_hash: &str,
    reason: &str,
) -> AppResult<()> {
    let context = RevokeOne {
        db,
        filter: doc! {"_id": binding_hash, "user_id": &actor.user_id, "revoked": false},
        reason,
        source: "user",
        actor: Some(actor),
        ip: None,
        user_agent: None,
    };
    if crate::services::transaction_service::run_db_only(db, &context, |session, context| {
        Box::pin(revoke_one_in_session(session, context))
    })
    .await?
    {
        Ok(())
    } else {
        Err(AppError::NotFound("binding not found".into()))
    }
}

struct RevokeOne<'a> {
    db: &'a Database,
    filter: bson::Document,
    reason: &'a str,
    source: &'a str,
    actor: Option<&'a AuditActor>,
    ip: Option<&'a str>,
    user_agent: Option<&'a str>,
}

async fn revoke_one_in_session(
    session: &mut ClientSession,
    context: &RevokeOne<'_>,
) -> AppResult<bool> {
    let RevokeOne {
        db,
        filter,
        reason,
        source,
        actor,
        ip,
        user_agent,
    } = context;
    let now = bson::DateTime::now();
    let Some(binding) = db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one_and_update(
            filter.clone(),
            doc! {"$set": {"revoked": true, "revoked_at": now, "revoke_reason": reason}},
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
    else {
        return Ok(false);
    };
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! {"jti": &binding.refresh_token_jti, "revoked": false},
            doc! {"$set": {"revoked": true, "revoked_at": now}},
        )
        .session(&mut *session)
        .await?;
    let metadata = Some(
        serde_json::json!({"revoke_source": source, "client_id": required::text(TextField::Identifier, &binding.client_id),
        "binding_hash": binding_hash_prefix(&binding.id), "reason": reason}),
    );
    let audit = if let Some(actor) = actor {
        required::intent(Some(actor), "oauth_broker_binding_revoked", metadata)?
    } else {
        required::client_intent("oauth_broker_binding_revoked", metadata, *ip, *user_agent)?
    };
    security_effect_service::enqueue_in_session(
        db,
        session,
        &[audit, cae_intent(&binding, source)],
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
pub async fn revoke_binding_by_client(
    db: &Database,
    _keys: Arc<EncryptionKeys>,
    _http: &reqwest::Client,
    client_id: &str,
    raw: &str,
    reason: &str,
) -> AppResult<bool> {
    revoke_binding_by_client_as(db, client_id, raw, reason, None, None).await
}

#[cfg(test)]
pub async fn revoke_binding_by_user(
    db: &Database,
    _keys: Arc<EncryptionKeys>,
    _http: &reqwest::Client,
    user_id: &str,
    binding: &str,
    reason: &str,
) -> AppResult<()> {
    revoke_binding_by_user_as(
        db,
        &AuditActor {
            user_id: user_id.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        binding,
        reason,
    )
    .await
}

#[cfg(test)]
pub async fn revoke_bindings_for_user_client(
    db: &Database,
    _keys: Arc<EncryptionKeys>,
    _http: &reqwest::Client,
    client_id: &str,
    user_id: &str,
    reason: &str,
) -> AppResult<u64> {
    let context = (db, client_id, user_id, reason);
    crate::services::transaction_service::run_db_only(
        db,
        &context,
        |session, &(db, client, user, reason)| {
            Box::pin(async move {
                let (count, intents) =
                    revoke_user_client_in_session(db, session, client, user, reason).await?;
                security_effect_service::enqueue_in_session(db, session, &intents).await?;
                Ok(count)
            })
        },
    )
    .await
}
