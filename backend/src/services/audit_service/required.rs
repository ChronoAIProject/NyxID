use mongodb::Database;

use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_log::AuditLog,
        security_effect_intent::{SecurityEffect, SecurityEffectIntent},
    },
    services::{audit_chain_service, security_effect_service},
};

use super::AuditActor;

// These are forensic identifiers and bounded facts, never source documents.
const MAX_INTENT_METADATA_BYTES: usize = 16 * 1024;
const MAX_ACTOR_FIELD_BYTES: usize = 1024;

fn bounded(value: Option<&String>) -> Option<String> {
    value.map(|value| value.chars().take(MAX_ACTOR_FIELD_BYTES / 4).collect())
}

/// Callers select metadata explicitly; serializing a request/model/provider
/// response into an intent is forbidden. Validation must succeed before the source commits.
pub fn intent(
    actor: Option<&AuditActor>,
    event_type: &str,
    metadata: Option<serde_json::Value>,
) -> AppResult<SecurityEffectIntent> {
    let event = super::build_audit_entry(
        actor.map(|actor| actor.user_id.clone()),
        event_type.to_owned(),
        metadata,
        bounded(actor.and_then(|actor| actor.ip_address.as_ref())),
        bounded(actor.and_then(|actor| actor.user_agent.as_ref())),
        actor.and_then(|actor| actor.api_key_id.clone()),
        bounded(actor.and_then(|actor| actor.api_key_name.as_ref())),
    );
    checked_intent(event)
}

/// OAuth client requests have network context but no human acting user.
pub fn client_intent(
    event_type: &str,
    metadata: Option<serde_json::Value>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<SecurityEffectIntent> {
    checked_intent(super::build_audit_entry(
        None,
        event_type.into(),
        metadata,
        ip_address.map(|v| v.chars().take(MAX_ACTOR_FIELD_BYTES / 4).collect()),
        user_agent.map(|v| v.chars().take(MAX_ACTOR_FIELD_BYTES / 4).collect()),
        None,
        None,
    ))
}

fn checked_intent(event: AuditLog) -> AppResult<SecurityEffectIntent> {
    if event.event_type.len() > 128
        || bson::to_vec(&event)
            .map_err(|_| AppError::Internal("Required audit metadata could not be encoded".into()))?
            .len()
            > MAX_INTENT_METADATA_BYTES
    {
        return Err(AppError::Internal(
            "Required audit metadata exceeds its bound".into(),
        ));
    }
    Ok(security_effect_service::new_intent(
        event.id.clone(),
        SecurityEffect::Audit {
            event: Box::new(event),
        },
    ))
}

/// Replaying an immutable intent preserves its UUID and timestamp. The chain
/// append validates an existing UUID's immutable content before acknowledging.
pub(crate) async fn append_intent(db: &Database, event: &AuditLog) -> AppResult<()> {
    let key = super::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain is not initialized".into()))?;
    let inserted = audit_chain_service::append_chained_entry(db, event.clone(), key).await?;
    #[cfg(test)]
    super::notify_test_audit_write(&inserted);
    #[cfg(not(test))]
    let _ = inserted;
    Ok(())
}
