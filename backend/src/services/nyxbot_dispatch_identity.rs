//! Provider retry aliases do not replace the verified source-event identity.
use crate::{
    crypto::token::hash_token,
    errors::{AppError, AppResult},
    models::nyxbot_channel::EVENTS_COLLECTION_NAME,
};
use bson::doc;
use mongodb::{Database, options::ReturnDocument};

pub const READINESS: &str = "nyxbot:gateway-reply-arbitration-v1";
pub const MAX_ALIASES: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AliasOutcome {
    Added,
    Limit,
    Missing,
}

pub fn admission(namespace: &str, key: &str) -> crate::models::nyxbot_channel::DispatchIdentity {
    crate::models::nyxbot_channel::DispatchIdentity {
        version: 1,
        keys: vec![dispatch_key(namespace, key)],
    }
}
pub fn dispatch_key(namespace: &str, key: &str) -> String {
    hash_token(&format!("{namespace}\0{key}"))
}
pub fn relay_response(channel: &str, key: &str) -> String {
    format!("resp_{}", &hash_token(&format!("{channel}:{key}"))[..32])
}
pub fn native_response(channel: &str, binding: &str, key: &str) -> String {
    format!(
        "resp_{}",
        &hash_token(&format!("{channel}:{binding}:{key}"))[..32]
    )
}
pub fn native_event(response: &str) -> String {
    hash_token(&format!("native-response:{response}"))
}
pub fn custom_event(binding: &str, event: &str) -> String {
    hash_token(&format!("custom-event:v1\0{binding}\0{event}"))
}
pub fn custom_response(event_key: &str) -> String {
    format!("resp_{}", &event_key[..32])
}
pub fn followed_event(bot: &str, event: &str) -> String {
    hash_token(&format!("thread-follow:v1\0{bot}\0{event}"))
}

/// Duplicate admission writes only; never evict a previously acknowledged alias.
/// The returned document distinguishes an absent event from a full alias list in
/// the same write command, so the duplicate path cannot replay another event.
pub async fn alias_result(
    db: &Database,
    event: &str,
    channel: &str,
    key: &str,
) -> AppResult<AliasOutcome> {
    alias_response(db, event, channel, key, None).await
}

pub async fn alias_response(
    db: &Database,
    event: &str,
    channel: &str,
    key: &str,
    response: Option<&crate::models::nyxbot_response_read::Command>,
) -> AppResult<AliasOutcome> {
    let mut stages = vec![doc! {"$set":{"dispatch_identity":{"$mergeObjects":[
        {"$ifNull":["$dispatch_identity",{}]},
        {"keys":{"$let":{"vars":{"keys":{"$ifNull":["$dispatch_identity.keys",[]]}},"in":{
            "$cond":[
                {"$or":[{"$in":[key,"$$keys"]},{"$lt":[{"$size":"$$keys"},MAX_ALIASES]}]},
                {"$setUnion":["$$keys",[key]]},"$$keys"
            ]
        }}}}
    ]}}}];
    if let Some(command) = response {
        let value = bson::to_bson(command)
            .map_err(|_| AppError::Internal("Response alias unavailable".into()))?;
        for field in ["response_read", "response_read_pending"] {
            let reference = format!("${field}");
            let commands = format!("{reference}.commands");
            stages.push(doc! {"$set":{field:{"$cond":[
                {"$and":[{"$eq":[format!("{reference}.version"),1]},
                    {"$gt":[format!("{reference}.expires_at"),"$$NOW"]},
                    {"$in":[key,"$dispatch_identity.keys"]},
                    {"$not":[{"$in":[&command.response_id,{"$ifNull":[format!("{commands}.response_id"),[]]}]}]},
                    {"$lt":[{"$size":{"$ifNull":[&commands,[]]}},MAX_ALIASES]}]},
                {"$mergeObjects":[&reference,{"commands":{"$concatArrays":[&commands,{"$literal":[value.clone()]}]}}]},
                {"$ifNull":[&reference,"$$REMOVE"]}
            ]}}});
        }
    }
    let row = db
        .collection::<bson::Document>(EVENTS_COLLECTION_NAME)
        .find_one_and_update(doc! {"_id":event,"channel_id":channel}, stages)
        .projection(doc! {"dispatch_identity.keys":1,"response_read.commands":1,"response_read_pending.commands":1})
        .return_document(ReturnDocument::After)
        .await
        .map_err(|error| {
            let duplicate = match error.kind.as_ref() {
                mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(
                    write,
                )) => write.code == 11000,
                mongodb::error::ErrorKind::Command(command) => command.code == 11000,
                _ => false,
            };
            if duplicate {
                AppError::Conflict("dispatch_identity_conflict".into())
            } else {
                error.into()
            }
        })?;
    let Some(row) = row else {
        return Ok(AliasOutcome::Missing);
    };
    if let Some(command) = response {
        for field in ["response_read", "response_read_pending"] {
            if let Ok(mapping) = row.get_document(field)
                && let Ok(commands) = mapping.get_array("commands")
                && commands
                    .iter()
                    .filter_map(|v| v.as_document())
                    .any(|saved| {
                        saved.get_str("response_id").ok() == Some(command.response_id.as_str())
                            && saved.get_str("request_hash").ok()
                                != Some(command.request_hash.as_str())
                    })
            {
                return Err(AppError::Conflict("response_command_changed".into()));
            }
        }
    }
    let keys = row
        .get_document("dispatch_identity")
        .ok()
        .and_then(|identity| identity.get_array("keys").ok())
        .cloned()
        .unwrap_or_default();
    if keys.iter().any(|value| value.as_str() == Some(key)) {
        // Repeated aliases and newly appended aliases have identical replay
        // semantics, so one successful outcome is sufficient to the caller.
        Ok(AliasOutcome::Added)
    } else {
        Ok(AliasOutcome::Limit)
    }
}

#[cfg(test)]
pub async fn alias(db: &Database, event: &str, channel: &str, key: &str) -> AppResult<bool> {
    Ok(matches!(
        alias_result(db, event, channel, key).await?,
        AliasOutcome::Added
    ))
}

#[derive(serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DispatchState {
    NotReceived,
    Expired,
    InProgress {
        response_id: String,
    },
    Finished {
        response_id: String,
        outcome: &'static str,
        stored_response: bool,
        provider_delivery: &'static str,
    },
}
pub enum Lookup {
    Found(DispatchState),
    IdentityUnavailable,
}

#[allow(clippy::too_many_arguments)]
pub async fn lookup(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    channel: &crate::models::nyxbot_channel::NyxbotChannel,
    key: &str,
    custom_binding: Option<&str>,
    dispatched_at: Option<chrono::DateTime<chrono::Utc>>,
    active_since: Option<chrono::DateTime<chrono::Utc>>,
) -> AppResult<Lookup> {
    use crate::{
        models::{
            nyxbot_channel::NyxbotEvent,
            nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
        },
        services::nyxbot_response_recovery as recovery,
    };
    use chrono::{Duration, Utc};
    let now = Utc::now();
    let digest = dispatch_key(custom_binding.unwrap_or(&channel.id), key);
    let response_id = if channel.gateway_source.is_some() {
        native_response(
            &channel.id,
            channel.binding_id.as_deref().unwrap_or_default(),
            key,
        )
    } else {
        relay_response(&channel.id, key)
    };
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let events = db.collection::<bson::Document>(EVENTS_COLLECTION_NAME);
    let scope = doc! {"channel_id":&channel.id,"user_id":&channel.user_id,"custom_binding_id":custom_binding};
    let mut by_alias = scope.clone();
    by_alias.insert("dispatch_identity.keys", &digest);
    let mut event = events
        .find_one(by_alias)
        .projection(doc! {"event_context_ciphertext":0,"media":0})
        .session(&mut session)
        .await?;
    let mut stored = if custom_binding.is_none() {
        db.collection::<GatewayResponse>(RESPONSES).find_one(doc! {
            "response_id":&response_id,"channel_id":&channel.id,"user_id":&channel.user_id,
            "binding_id":&channel.binding_id,"expires_at":{"$gt":bson::DateTime::from_chrono(now)},
        }).session(&mut session).await?
    } else {
        None
    };
    if event.is_none() && custom_binding.is_none() {
        let fallback = stored
            .as_ref()
            .map(|r| r.event_key.clone())
            .unwrap_or_else(|| {
                if channel.gateway_source.is_some() {
                    native_event(&response_id)
                } else {
                    digest.clone()
                }
            });
        let mut filter = scope.clone();
        filter.insert("_id", fallback);
        event = events
            .find_one(filter)
            .projection(doc! {"event_context_ciphertext":0,"media":0})
            .session(&mut session)
            .await?;
    }
    let Some(raw) = event else {
        if stored.is_some() {
            return Err(crate::errors::AppError::OAuthAuthorityUnavailable);
        }
        // Historical body-keyed admissions on these platforms can outlive the
        // current enabled/supported flags. The source registry describes live
        // capabilities, not pre-release admission keys (retained up to 7 days).
        const LEGACY_BODY_KEYED_PLATFORMS: &[&str] = &["telegram", "lark", "feishu"];
        let ambiguous = custom_binding.is_some()
            || (channel.gateway_source.is_none()
                && (channel.gateway_bot_mentions.enabled
                    || channel.gateway_bot_mentions.supported
                    || LEGACY_BODY_KEYED_PLATFORMS.contains(&channel.platform.as_str())));
        let new_dispatch = dispatched_at
            .zip(active_since)
            .is_some_and(|(at, cutover)| at >= cutover);
        // Activation is immutable across rollbacks. A retained admission from
        // any old writer can lack the original alias, even after a new writer
        // has appended another alias for a duplicate.
        let legacy = if ambiguous && new_dispatch {
            let mut filter = scope;
            filter.insert("dispatch_identity.version", doc! {"$ne":1});
            filter.insert("expires_at", doc! {"$gt":bson::DateTime::from_chrono(now)});
            events
                .find_one(filter)
                .projection(doc! {"_id":1})
                .session(&mut session)
                .await?
                .is_some()
        } else {
            false
        };
        session.abort_transaction().await?;
        if ambiguous && (!new_dispatch || legacy) {
            return Ok(
                if dispatched_at.is_some_and(|at| at + Duration::days(7) <= now) {
                    Lookup::Found(DispatchState::Expired)
                } else {
                    Lookup::IdentityUnavailable
                },
            );
        }
        return Ok(Lookup::Found(
            if dispatched_at.is_some_and(|at| at + Duration::hours(24) <= now) {
                DispatchState::Expired
            } else {
                DispatchState::NotReceived
            },
        ));
    };
    if stored.is_none() && custom_binding.is_none() {
        stored = db.collection::<GatewayResponse>(RESPONSES).find_one(doc! {
            "event_key":raw.get_str("_id").map_err(|_|crate::errors::AppError::OAuthAuthorityUnavailable)?,
            "channel_id":&channel.id,"user_id":&channel.user_id,"binding_id":&channel.binding_id,
            "expires_at":{"$gt":bson::DateTime::from_chrono(now)},
        }).session(&mut session).await?;
    }
    let event: NyxbotEvent = bson::from_document(raw.clone())
        .map_err(|_| crate::errors::AppError::OAuthAuthorityUnavailable)?;
    if event.expires_at <= now {
        session.abort_transaction().await?;
        return Ok(Lookup::Found(DispatchState::Expired));
    }
    if let Some(execution) = event
        .delivery
        .as_ref()
        .and_then(|d| d.origin.org_execution.as_ref())
    {
        super::org_channel_service::validate(
            db,
            &event.delivery.as_ref().expect("delivery").origin,
            &execution.actor_id,
            &execution.agent_id,
        )
        .await?;
    }
    let message = if let (Some(turn), Some(conversation)) = (&event.turn_id, &event.conversation_id)
    {
        db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME).find_one(doc! {
            "user_id":super::org_channel_service::event_actor(&event),"conversation_id":conversation,
            "turn_id":turn,"role":"assistant",
        }).projection(doc! {"status":1}).session(&mut session).await?
    } else {
        None
    };
    session.abort_transaction().await?;
    let response_id = if custom_binding.is_some() {
        custom_response(&event.id)
    } else {
        stored
            .as_ref()
            .map_or(response_id, |r| r.response_id.clone())
    };
    let payload = match &stored {
        Some(stored) if stored.terminal => Some(recovery::payload(keys, stored).await?),
        _ => None,
    };
    let result_status = message
        .as_ref()
        .and_then(|m| m.get_str("status").ok())
        .filter(|s| matches!(*s, "completed" | "failed" | "cancelled" | "interrupted"))
        .or_else(|| payload.as_ref().and_then(|p| p.snapshot["status"].as_str()))
        .unwrap_or(&event.status);
    let outcome = match result_status {
        "running" | "resuming" | "queued" | "retryable" | "in_progress" => {
            return Ok(Lookup::Found(DispatchState::InProgress { response_id }));
        }
        "completed" | "silent" => "completed",
        "failed" | "cancelled" | "interrupted" | "refused" => "failed",
        _ => return Err(crate::errors::AppError::OAuthAuthorityUnavailable),
    };
    let delivery = raw.get_document("delivery").ok();
    let state = delivery
        .and_then(|d| d.get_str("state").ok())
        .unwrap_or_default();
    let receipt = delivery
        .and_then(|d| d.get_document("preparation").ok())
        .and_then(|p| p.get_document("gateway_receipt").ok());
    let pending = receipt.is_some_and(|r| {
        matches!(r.get_i32("version"), Ok(1 | 2)) && r.get_bool("pending") == Ok(true)
    });
    let local = channel.gateway_source.is_none() && custom_binding.is_none();
    let provider_delivery = if !local {
        "none"
    } else {
        match state {
            "sent" => "confirmed",
            "sending" => "started",
            "unknown" if pending => "started",
            "unknown" => "unconfirmed",
            "refused" if delivery.is_some_and(|d| d.get_bool("gateway_attempted") == Ok(true)) => {
                "unconfirmed"
            }
            _ => "none",
        }
    };
    let suppressed = local && matches!(state, "sending" | "sent" | "unknown" | "refused");
    Ok(Lookup::Found(DispatchState::Finished {
        response_id,
        outcome,
        stored_response: stored.is_some() && !suppressed,
        provider_delivery,
    }))
}
