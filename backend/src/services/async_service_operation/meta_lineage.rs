//! Grant provenance on the original watch, then its existing queued event.
use super::*;
use crate::models::assistant_conversation::ChannelTurnContext;

pub(crate) async fn capture(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<Vec<ChannelTurnContext>> {
    let mut contexts: Vec<ChannelTurnContext> = Vec::new();
    for source in row.active_turn.iter().flat_map(|turn| {
        turn.channel_context
            .iter()
            .chain(turn.events.iter().flat_map(|event| &event.channel_contexts))
    }) {
        if source.meta_grant.is_none()
            || contexts
                .iter()
                .any(|existing| existing.meta_grant == source.meta_grant)
        {
            continue;
        }
        if contexts.len() >= 100 {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
        let mut context = source.clone();
        context.display_name = None;
        context.conversation_label = None;
        context.text_completeness = None;
        context.org_origin = None;
        context.original_message = false;
        contexts.push(context);
    }
    fence(db, tx, &contexts, true).await?;
    Ok(contexts)
}

pub(crate) async fn fence(
    db: &Database,
    tx: &mut ClientSession,
    contexts: &[ChannelTurnContext],
    fresh: bool,
) -> AppResult<()> {
    if contexts.len() > 100 {
        return Err(super::super::channel_meta_service::grants::incomplete());
    }
    for context in contexts {
        let proof = context
            .meta_grant
            .as_ref()
            .ok_or_else(super::super::channel_meta_service::grants::incomplete)?;
        if context.channel_bot_id != proof.bot_id {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
        super::super::channel_meta_service::scopes::fence_reference(db, tx, proof, fresh).await?;
    }
    Ok(())
}

/// A successful exact read can establish permanent loss. Storage errors and
/// malformed or missing scope evidence propagate for retry/repair instead.
async fn current(
    db: &Database,
    tx: &mut ClientSession,
    context: &ChannelTurnContext,
) -> AppResult<bool> {
    use super::super::channel_meta_service::grants;
    use crate::models::{channel_bot::COLLECTION_NAME as BOTS, channel_meta::GRANT_SCOPES};
    use bson::{Bson, Document};
    let proof = context.meta_grant.as_ref().ok_or_else(grants::incomplete)?;
    if proof.generation < 0 || proof.bot_id != context.channel_bot_id {
        return Err(grants::incomplete());
    }
    let scope = db
        .collection::<Document>(GRANT_SCOPES)
        .find_one(doc! {"bot_id":&proof.bot_id,"grant_id":&proof.grant_id})
        .session(&mut *tx)
        .await?
        .ok_or_else(grants::incomplete)?;
    if scope.get_str("status") == Ok("completed") {
        if grants::completed(&scope, &proof.grant_id)? {
            return Ok(false);
        }
        return Err(grants::incomplete());
    }
    let from = scope
        .get_i64("from_generation")
        .map_err(|_| grants::incomplete())?;
    let through = match scope.get("to_generation") {
        None | Some(Bson::Null) => None,
        Some(Bson::Int64(value)) => Some(*value),
        _ => return Err(grants::incomplete()),
    };
    if from < 0
        || proof.generation < from
        || through.is_some_and(|to| to < from || proof.generation > to)
    {
        return Err(grants::incomplete());
    }
    match scope.get_str("status") {
        Ok("sealed" | "retired") => return Ok(false),
        Ok("current") => {}
        _ => return Err(grants::incomplete()),
    }
    match scope.get("erasure_requested") {
        Some(Bson::Boolean(true)) => return Ok(false),
        None | Some(Bson::Boolean(false)) => {}
        _ => return Err(grants::incomplete()),
    }
    let Some(parent) = db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":&proof.bot_id})
        .projection(doc! {"is_active":1,"meta_binding":1})
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
    };
    let binding = parent
        .get_document("meta_binding")
        .map_err(|_| grants::incomplete())?;
    let grant = match binding.get("grant_id") {
        None if proof.grant_id.is_empty() => "",
        Some(Bson::String(value)) => value.as_str(),
        _ => return Err(grants::incomplete()),
    };
    let generation = binding
        .get_i64("generation")
        .map_err(|_| grants::incomplete())?;
    let active = parent
        .get_bool("is_active")
        .map_err(|_| grants::incomplete())?;
    if generation < 0 {
        return Err(grants::incomplete());
    }
    if grant != proof.grant_id || generation != proof.generation || !active {
        return Ok(false);
    }
    for field in [
        "erasure_requested",
        "provider_revoked",
        "operator_suspended",
        "grant_reconnect_required",
    ] {
        match binding.get(field) {
            Some(Bson::Boolean(true)) => return Ok(false),
            None | Some(Bson::Boolean(false)) => {}
            _ => return Err(grants::incomplete()),
        }
    }
    if let Some(identity) = scope.get("identity").filter(|value| **value != Bson::Null) {
        let identity = identity.as_str().ok_or_else(grants::incomplete)?;
        if let Some(authority) = db
            .collection::<Document>("channel_meta_grant_authority")
            .find_one(doc! {"identity":identity})
            .session(&mut *tx)
            .await?
        {
            for field in ["erasure_cutoff", "revoked_at"] {
                let cutoff = match authority.get(field) {
                    None | Some(Bson::Null) => continue,
                    Some(Bson::DateTime(value)) => {
                        if field == "revoked_at" {
                            bson::DateTime::from_millis(value.timestamp_millis() + 1000)
                        } else {
                            *value
                        }
                    }
                    _ => return Err(grants::incomplete()),
                };
                let authorized = scope
                    .get_datetime("authorized_at")
                    .map_err(|_| grants::incomplete())?;
                if authorized < &cutoff {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// Cancel only local result delivery. No remote cancellation is inferred.
pub(super) async fn retire_in_session(
    db: &Database,
    tx: &mut ClientSession,
    w: &AsyncServiceOperation,
) -> AppResult<bool> {
    if w.meta_contexts.is_empty() || !matches!(w.state.as_str(), "ready" | "queued") {
        return Ok(false);
    }
    let Some(live) = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(lease_filter(w))
        .session(&mut *tx)
        .await?
    else {
        return Ok(true);
    };
    if live.meta_contexts.len() > 100 {
        return Err(super::super::channel_meta_service::grants::incomplete());
    }
    let now = Utc::now();
    let mut reason = if now >= live.expires_at.min(live.created_at + Duration::hours(4)) {
        Some("result_expired")
    } else if now >= live.deadline {
        Some("timeout")
    } else {
        None
    };
    if reason.is_none() {
        for context in &live.meta_contexts {
            if !current(db, tx, context).await? {
                reason = Some("authority_lost");
                break;
            }
        }
    }
    let Some(reason) = reason else {
        return Ok(false);
    };
    let changed=db.collection::<bson::Document>(WATCHES).update_one(lease_filter(&live),doc! {
        "$unset":{"secret_context_versions.result_encrypted":""},"$set":{"state":"cancelled","reason":reason,
            "result_encrypted":null,"lease_id":null,"lease_until":null}}).session(&mut *tx).await?;
    if changed.matched_count == 1 {
        db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&live.conversation_id,"user_id":&live.user_id},doc! {
            "$pull":{"pending_events":{"$or":[{"kind":EVENT_KIND,"id":&live.id},
                {"kind":super::super::cma_operation::APPROVAL_EVENT_KIND,"question_key":cma_approval_watch_key(&live.id)}]}}})
            .session(&mut *tx).await?;
    }
    Ok(true)
}

pub(crate) async fn retire_unavailable(
    db: &Database,
    w: &AsyncServiceOperation,
) -> AppResult<bool> {
    if w.meta_contexts.is_empty() || !matches!(w.state.as_str(), "ready" | "queued") {
        return Ok(false);
    }
    super::super::transaction_service::run_db_only(db, &(db, w), |tx, &(db, w)| {
        Box::pin(retire_in_session(db, tx, w))
    })
    .await
}
