//! Saved model-Deny notices use a real, exclusive event turn and ordinary
//! durable reply settlement. Effect-Deny keeps its existing model explanation.
use super::{assistant_acknowledgement_service as acks, assistant_nyxagent as engine};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::AssistantAcknowledgement,
        assistant_conversation::{
            AgentEvent, AssistantConversation, PlatformReplyRouting, TurnOrigin,
        },
        assistant_message::AssistantMessage,
        assistant_model_control::{COLLECTION_NAME as ROOTS, ModelControlRoot},
        assistant_model_denial_notice::{COLLECTION_NAME as NOTICES, ModelDenialNotice},
    },
};
use bson::{Document, doc};
use chrono::{Duration, Utc};
use mongodb::{ClientSession, Database};
use serde_json::json;

pub(crate) const EVENT_KIND: &str = "model_denied_notice";
const TEXT: &str = "You denied this model request. I have stopped this continuation and will not call the model to explain that decision. You can start a separate request when you are ready.";
fn refused() -> AppError {
    AppError::Conflict("Saved model-Deny notice authority is unavailable".into())
}
fn binding(source: &AssistantMessage) -> AppResult<String> {
    Ok(super::mcp_service::canonical_sha256(
        serde_json::to_value(source).map_err(|_| refused())?,
    ))
}
pub(crate) fn is_notice(event: &AgentEvent) -> bool {
    event.kind == EVENT_KIND
}

/// Classify from saved server execution evidence and its complete original
/// argument digest. A summary, model name or mutable client path is no proof.
async fn model_source_turn(
    db: &Database,
    card: &AssistantAcknowledgement,
    session: &mut ClientSession,
) -> AppResult<Option<(Option<String>, bool)>> {
    if card.tool_name.as_deref() == Some("nyxid.model_control_v1") {
        let root=db.collection::<ModelControlRoot>(ROOTS).find_one(doc!{"acknowledgement_id":&card.id,
            "user_id":&card.user_id,"conversation_id":&card.conversation_id,"api_key_id":&card.api_key_id})
            .session(&mut *session).await?;
        let valid = root.as_ref().is_some_and(|root| {
            root.card_arguments.as_ref().is_some_and(|args| {
                card.arguments_digest.as_deref() == Some(acks::arguments_digest(args).as_str())
                    && args["root_id"] == root.id
                    // Deny retirement only: neither value grants dispatch compatibility.
                    && args["protocol"].as_str().is_some_and(|protocol|
                        matches!(protocol, super::assistant_model_control_protocol::PROTOCOL
                            | super::assistant_model_control_protocol::RETIRED_PROTOCOL))
            })
        });
        return Ok(Some((
            root.and_then(|r| r.turn_ids.first().cloned()),
            valid,
        )));
    }
    if card.tool_name.as_deref() != Some("nyxid.service_execution") {
        return Ok(None);
    }
    let saved = db
        .collection::<Document>(super::assistant_service_execution::COLLECTION)
        .find_one(doc! {
            "_id":&card.id,"kind":"card_binding","user_id":&card.user_id,
            "conversation_id":&card.conversation_id,"api_key_id":&card.api_key_id,
        })
        .session(&mut *session)
        .await?;
    let Some(saved) = saved else {
        return Ok(Some((None, false)));
    };
    let operation = saved.get_document("operation").ok();
    let args = json!({"connection_id":saved.get_str("connection_id").ok(),
        "effective_owner_id":saved.get_str("owner_id").ok(),"execution_binding":saved.get_str("binding").ok(),
        "operation":bson::from_bson::<serde_json::Value>(bson::Bson::Document(operation.cloned().unwrap_or_default())).map_err(|_|refused())?});
    let valid = ["connection_id", "owner_id", "binding"]
        .iter()
        .all(|key| saved.get_str(key).is_ok())
        && card.arguments_digest.as_deref() == Some(acks::arguments_digest(&args).as_str());
    // Only a complete, digest-bound known server effect classification keeps
    // the existing effect explanation. Missing/unknown evidence must never
    // become permission to ask a possibly denied model to explain itself.
    match operation.and_then(|o| o.get_str("kind").ok()) {
        Some("http") if valid => return Ok(None),
        Some("ssh") if valid && operation.is_some_and(known_ssh_effect) => return Ok(None),
        Some("legacy_inference" | "raw_inference" | "inference") => {}
        _ => return Ok(Some((None, false))),
    }
    Ok(Some((
        saved.get_str("turn_id").ok().map(str::to_owned),
        valid,
    )))
}

fn known_ssh_effect(operation: &Document) -> bool {
    let digest = |field: &str| {
        operation.get_str(field).is_ok_and(|value| {
            value.strip_prefix("sha256:").is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        })
    };
    match operation.get_str("retry_tool").ok() {
        Some("nyx__ssh_exec") => operation.len() == 3 && digest("arguments"),
        Some(kind @ ("SSH certificate" | "SSH terminal session" | "SSH tunnel session")) => {
            operation.len() == 5
                && operation
                    .get_str("session_kind")
                    .is_ok_and(|saved| saved == kind)
                && digest("parameters")
                && operation
                    .get_i64("ttl_secs")
                    .or_else(|_| operation.get_i32("ttl_secs").map(i64::from))
                    .is_ok_and(|ttl| (1..=3600).contains(&ttl))
        }
        _ => false,
    }
}

/// Joins the original decision transaction. Even a process exit before the
/// handler wake leaves either queued work or an explicit unsupported notice.
pub(crate) async fn decision_in_session(
    db: &Database,
    card: &AssistantAcknowledgement,
    session: &mut ClientSession,
) -> AppResult<()> {
    if card.status != "denied"
        || card.decider != "user"
        || card.decided_by.as_deref() != Some("user")
        || card.kind != "action"
        || card.trigger_run_id.is_some()
        || card.voice_request_id.is_some()
        || card.invocation_id.is_some()
    {
        return Ok(());
    }
    let Some((turn, proven)) = model_source_turn(db, card, session).await? else {
        return Ok(());
    };
    let rows = db.collection::<ModelDenialNotice>(NOTICES);
    if rows
        .find_one(doc! {"_id":&card.id})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let source = if let Some(turn) = turn {
        db.collection::<AssistantMessage>("assistant_messages").find_one(doc!{
        "user_id":&card.user_id,"conversation_id":&card.conversation_id,"turn_id":turn,
        "role":"user","author":"owner","execution_pending":{"$ne":true},"steering":bson::Bson::Null,
    }).session(&mut *session).await?
    } else {
        None
    };
    let conversation = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {
        "_id":&card.conversation_id,"user_id":&card.user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    let now = Utc::now();
    let mut notice = ModelDenialNotice {
        id: card.id.clone(),
        user_id: card.user_id.clone(),
        conversation_id: card.conversation_id.clone(),
        api_key_id: card.api_key_id.clone(),
        card_arguments_digest: card.arguments_digest.clone().unwrap_or_default(),
        source_input_id: source.as_ref().map(|s| s.id.clone()),
        source_input_binding: source.as_ref().map(binding).transpose()?,
        platform_reply: source.as_ref().and_then(|s| s.platform_reply.clone()),
        state: "unsupported".into(),
        claimed_turn_id: None,
        reply_id: None,
        reason: Some("unproven_saved_binding".into()),
        created_at: now,
        expires_at: card.expires_at.min(now + Duration::minutes(10)),
    };
    if proven && let Some(source) = source {
        let supported = if let Some(PlatformReplyRouting::Captured {
            origin,
            turn_id,
            expires_at,
        }) = &source.platform_reply
        {
            notice.expires_at = notice.expires_at.min(*expires_at);
            source.platform_origin.as_ref() == Some(origin.as_ref())
                && turn_id == &source.turn_id
                && conversation.platform_thread.as_ref() == Some(&origin.thread)
        } else if source.platform_reply.is_none() {
            source.origin == Some(TurnOrigin::User)
                && source.platform_origin.is_none()
                && conversation.platform_thread.is_none()
        } else {
            false
        };
        if supported
            && conversation.credential_api_key_id == card.api_key_id
            && !conversation.guest_turn
            && conversation.group_id.is_none()
            && conversation.channel.is_none()
            && notice.expires_at > now
        {
            notice.state = "pending".into();
            notice.reason = None;
        } else {
            notice.reason = Some("saved_destination_unsupported_or_expired".into());
        }
    }
    rows.insert_one(&notice).session(&mut *session).await?;
    enqueue_in_session(db, &notice, session).await
}

fn event(notice: &ModelDenialNotice) -> AgentEvent {
    let mut event = super::assistant_team_service::bound_event(EVENT_KIND, TEXT.into(), None, None);
    event.id = notice.id.clone();
    event
}
async fn enqueue_in_session(
    db: &Database,
    notice: &ModelDenialNotice,
    session: &mut ClientSession,
) -> AppResult<()> {
    if notice.state != "pending" || notice.expires_at <= Utc::now() {
        return Ok(());
    }
    let rows = db.collection::<AssistantConversation>("assistant_conversations");
    let Some(mut row) = rows
        .find_one(doc! {"_id":&notice.conversation_id,"user_id":&notice.user_id})
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    if row.pending_events.iter().any(|e| e.id == notice.id)
        || row.pending_events.len() >= engine::MAX_PENDING_EVENTS
    {
        return Ok(());
    }
    row.pending_events.push(event(notice));
    rows.update_one(
        doc! {"_id":&row.id,"user_id":&row.user_id},
        doc! {"$set":{"pending_events":bson::to_bson(&row.pending_events).map_err(|_|refused())?}},
    )
    .session(session)
    .await?;
    Ok(())
}

pub(crate) async fn exists(db: &Database, card: &str) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(NOTICES)
        .find_one(doc! {"_id":card})
        .await?
        .is_some())
}

/// Bounded recovery in the existing event worker. Queue-full and process-exit
/// gaps do not lose the notice. A committed reply is terminal; an expired
/// claimed turn without any reply may requeue the SAME durable notice.
pub(crate) async fn recover(db: &Database) -> AppResult<()> {
    use futures::TryStreamExt;
    let pending: Vec<ModelDenialNotice> = db
        .collection(NOTICES)
        .find(doc! {"state":{"$in":["pending","claimed"]}})
        .sort(doc! {"created_at":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for expected in pending {
        let mut session = db.client().start_session().await?;
        session.start_transaction().and_run2(async |session| {
            let result=Box::pin(async {
                let notices=db.collection::<ModelDenialNotice>(NOTICES);
                let Some(mut notice)=notices.find_one(doc!{"_id":&expected.id,"state":{"$in":["pending","claimed"]}})
                    .session(&mut *session).await? else {return Ok(());};
                let conversation=db.collection::<AssistantConversation>("assistant_conversations").find_one(doc!{
                    "_id":&notice.conversation_id,"user_id":&notice.user_id}).session(&mut *session).await?;
                // Settlement (including the orphan sweep) owns an admitted
                // turn until it clears active_turn. Never retire its notice
                // underneath the shared reply transaction, even after expiry
                // or revocation. execute still checks live authority.
                if notice.state=="claimed" && conversation.as_ref().is_some_and(|row|
                    row.active_turn.as_ref().is_some_and(|turn| Some(&turn.turn_id)==notice.claimed_turn_id.as_ref())) {
                    return Ok(());
                }
                let key=db.collection::<Document>("api_keys").find_one(doc!{"_id":&notice.api_key_id,
                    "user_id":&notice.user_id,"is_active":true}).session(&mut *session).await?;
                if notice.expires_at<=Utc::now() || key.is_none() || conversation.as_ref().is_none_or(|row|
                    row.credential_api_key_id!=notice.api_key_id) {
                    notice.state="unsupported".into();notice.reason=Some("expired_or_revoked".into());
                } else if notice.state=="claimed" {
                    let saved=db.collection::<AssistantMessage>("assistant_messages").find_one(doc!{
                        "user_id":&notice.user_id,"conversation_id":&notice.conversation_id,
                        "turn_id":&notice.claimed_turn_id,"role":"assistant"}).session(&mut *session).await?;
                    if let Some(saved)=saved {
                        let completed=saved.status=="completed" && saved.text==TEXT;
                        // Reclaim may have saved a failed no-reply turn before
                        // this recovery pass. Stop/revocation/partial output
                        // cannot resurrect successful notice work.
                        if saved.status=="failed" && saved.error_code.as_deref()==Some("turn_lost")
                            && saved.text.is_empty() && source_valid_in_session(db,&notice,session).await? {
                            notice.state="pending".into();notice.claimed_turn_id=None;notice.reply_id=None;notice.reason=None;
                        } else {
                            notice.state=if completed {"settled"} else {"unsupported"}.into();
                            notice.reply_id=Some(saved.id);notice.reason=(!completed).then(||"turn_did_not_complete".into());
                        }
                    } else {notice.state="pending".into();notice.claimed_turn_id=None;}
                }
                notices.replace_one(doc!{"_id":&notice.id},&notice).session(&mut *session).await?;
                if notice.state=="pending" {enqueue_in_session(db,&notice,session).await?;}
                else {db.collection::<Document>("assistant_conversations").update_one(doc!{
                    "_id":&notice.conversation_id,"user_id":&notice.user_id},doc!{"$pull":{"pending_events":{"id":&notice.id,"kind":EVENT_KIND}}})
                    .session(&mut *session).await?;}
                Ok(())
            }).await;
            super::api_key_mutation_service::transaction_result(result)
        }).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    }
    Ok(())
}

pub(crate) async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<ModelDenialNotice>(NOTICES)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"state":1,"created_at":1})
                .build(),
        )
        .await?;
    Ok(())
}

/// Event turns select exactly one notice. Other turns leave all notices queued;
/// neither path may render a denied model request into a later model input.
pub(crate) fn select_events(
    row: &mut AssistantConversation,
    origin: TurnOrigin,
) -> Vec<AgentEvent> {
    if origin == TurnOrigin::Event
        && let Some(index) = row.pending_events.iter().position(is_notice)
    {
        return vec![row.pending_events.remove(index)];
    }
    let (notices, ordinary): (Vec<_>, Vec<_>) = std::mem::take(&mut row.pending_events)
        .into_iter()
        .partition(is_notice);
    row.pending_events = ordinary;
    let events = super::async_service_operation::select_events(row, origin);
    row.pending_events.extend(notices);
    events
}

/// Claim inside the real admitted turn transaction and retain the ORIGINAL
/// captured reply ceiling, independent of fresh execution's ThreadEvent origin.
pub(crate) async fn claim_in_session(
    db: &Database,
    row: &mut AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(turn) = row.active_turn.as_mut() else {
        return Ok(());
    };
    let Some(marker) = turn.events.iter().find(|e| is_notice(e)) else {
        return Ok(());
    };
    if turn.origin != TurnOrigin::Event || turn.events.len() != 1 {
        return Err(refused());
    }
    let notice=db.collection::<ModelDenialNotice>(NOTICES).find_one(doc!{"_id":&marker.id,"user_id":&row.user_id,
        "conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,"state":"pending","expires_at":{"$gt":bson::DateTime::now()}})
        .session(&mut *session).await?.ok_or_else(refused)?;
    let source = source_in_session(db, &notice, session).await?;
    match (&notice.platform_reply, turn.platform_origin.as_ref()) {
        (
            Some(PlatformReplyRouting::Captured {
                origin, expires_at, ..
            }),
            Some(fresh),
        ) if *expires_at > Utc::now()
            && origin.thread == fresh.thread
            && origin.manager_authority_epoch == fresh.manager_authority_epoch
            && origin.personal_control_revision == fresh.personal_control_revision
            && origin.sender == fresh.sender => {}
        (None, None)
            if source.origin == Some(TurnOrigin::User)
                && row.platform_thread.is_none()
                && row.channel.is_none() => {}
        _ => return Err(refused()),
    }
    turn.platform_reply = notice.platform_reply.clone();
    turn.also_deliver.clear();
    db.collection::<Document>(NOTICES)
        .update_one(
            doc! {"_id":&notice.id,"state":"pending"},
            doc! {"$set":{"state":"claimed","claimed_turn_id":&turn.turn_id}},
        )
        .session(session)
        .await?;
    Ok(())
}
async fn source_in_session(
    db: &Database,
    notice: &ModelDenialNotice,
    session: &mut ClientSession,
) -> AppResult<AssistantMessage> {
    let source=db.collection::<AssistantMessage>("assistant_messages").find_one(doc!{"_id":&notice.source_input_id,
        "user_id":&notice.user_id,"conversation_id":&notice.conversation_id,"role":"user","author":"owner"})
        .session(&mut *session).await?.ok_or_else(refused)?;
    if notice.source_input_binding.as_deref() != Some(binding(&source)?.as_str())
        || source.platform_reply != notice.platform_reply
    {
        return Err(refused());
    }
    let card=db.collection::<AssistantAcknowledgement>("assistant_acknowledgements").find_one(doc!{"_id":&notice.id,
        "user_id":&notice.user_id,"conversation_id":&notice.conversation_id,"api_key_id":&notice.api_key_id,
        "status":"denied","decided_by":"user","arguments_digest":&notice.card_arguments_digest})
        .session(&mut *session).await?.ok_or_else(refused)?;
    if !model_source_turn(db, &card, session)
        .await?
        .is_some_and(|(turn, valid)| valid && turn.as_deref() == Some(source.turn_id.as_str()))
    {
        return Err(refused());
    }
    Ok(source)
}

async fn source_valid_in_session(
    db: &Database,
    notice: &ModelDenialNotice,
    session: &mut ClientSession,
) -> AppResult<bool> {
    match source_in_session(db, notice, session).await {
        Ok(_) => Ok(true),
        Err(AppError::Conflict(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Runs before begin_model_attempt, after the ordinary execution context is
/// established. It returns a real result for shared finish_turn settlement.
pub(crate) async fn execute(
    db: &Database,
    row: &AssistantConversation,
) -> AppResult<Option<engine::TurnResult>> {
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(None);
    };
    let Some(marker) = turn.events.iter().find(|e| is_notice(e)) else {
        return Ok(None);
    };
    if turn.origin != TurnOrigin::Event || turn.events.len() != 1 {
        return Err(refused());
    }
    let chat = acks::for_key(db, &row.user_id, Some(&row.credential_api_key_id))
        .await?
        .ok_or_else(refused)?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async |session| {
        let result=async {
            let (live,_,_)=acks::fence(db,&chat,session).await?;
            super::assistant_access_policy::require_same_live_turn(&chat,&live)?;
            if chat.turn_id.as_deref()!=Some(turn.turn_id.as_str()) {return Err(refused());}
            let notice=db.collection::<ModelDenialNotice>(NOTICES).find_one(doc!{"_id":&marker.id,"user_id":&row.user_id,
                "conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,"state":"claimed",
                "claimed_turn_id":&turn.turn_id,"expires_at":{"$gt":bson::DateTime::now()}}).session(&mut *session).await?.ok_or_else(refused)?;
            source_in_session(db,&notice,session).await?;
            if live.active_turn.as_ref().is_none_or(|active|active.events.len()!=1
                || active.events[0].id!=notice.id || active.events[0].kind!=EVENT_KIND || active.platform_reply!=notice.platform_reply)
            {return Err(refused());}
            Ok(())
        }.await;super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    Ok(Some(engine::TurnResult {
        text: TEXT.into(),
        session_id: None,
        response_id: None,
        error: None,
    }))
}

/// Same transaction as the ordinary durable reply; an ambiguous commit cannot
/// create another notice reply. Cancellation remains cancellation, not success.
pub(crate) async fn settled_in_session(
    db: &Database,
    row: &AssistantConversation,
    reply_id: &str,
    reply_text: &str,
    error: Option<&engine::TurnError>,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(());
    };
    let Some(marker) = turn.events.iter().find(|e| is_notice(e)) else {
        return Ok(());
    };
    let notices = db.collection::<ModelDenialNotice>(NOTICES);
    let notice = notices
        .find_one(doc! {"_id":&marker.id,"user_id":&row.user_id,
        "conversation_id":&row.id,"claimed_turn_id":&turn.turn_id})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    // Old recovery writers may already have retired an admitted notice. Its
    // cancellation/failure must still settle, without promoting it to success.
    if notice.state == "unsupported" && error.is_some() {
        return Ok(());
    }
    if notice.state != "claimed" {
        return Err(refused());
    }
    let no_reply_crash = error.is_some_and(|error| error.code == "turn_lost")
        && reply_text.is_empty()
        && !turn.stop_requested
        && turn
            .reply
            .as_ref()
            .is_none_or(|reply| reply.text.is_empty())
        && turn.model_attempt_id.is_none()
        && turn.model_requests_active == 0
        && notice.expires_at > Utc::now()
        && row.credential_api_key_id == notice.api_key_id;
    let key_alive = no_reply_crash
        && db
            .collection::<Document>("api_keys")
            .find_one(doc! {"_id":&notice.api_key_id,"user_id":&notice.user_id,"is_active":true})
            .session(&mut *session)
            .await?
            .is_some();
    let requeue = key_alive && source_valid_in_session(db, &notice, session).await?;
    let changed = db
        .collection::<Document>(NOTICES)
        .update_one(
            doc! {"_id":&marker.id,"user_id":&row.user_id,
            "conversation_id":&row.id,"state":"claimed","claimed_turn_id":&turn.turn_id},
            doc! {"$set":{"state":if requeue {"pending"} else if error.is_some() {"unsupported"} else {"settled"},
            "claimed_turn_id":if requeue {None} else {Some(turn.turn_id.as_str())},
            "reply_id":if requeue {None} else {Some(reply_id)},
            "reason":if error.is_some() && !requeue {Some("turn_did_not_complete")} else {None::<&str>}}},
        )
        .session(session)
        .await?;
    if changed.matched_count != 1 {
        return Err(refused());
    }
    Ok(())
}
