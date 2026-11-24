//! Publication borrows exact original sources, never the home's newest turn.
use super::*;
use crate::models::{assistant_conversation::ChannelTurnContext, channel_meta::GrantRef};

pub(crate) const DENIAL_PREFIX: &str = "card-denial:";

pub(crate) struct Selection {
    pub input_id: Option<String>,
    pub creation_turn_id: Option<String>,
    pub requested_turn_id: Option<String>,
    pub contexts: Vec<ChannelTurnContext>,
    pub excerpt: Option<String>,
}

struct Input {
    id: String,
    turn: String,
    role: String,
    text: String,
    proofs: Vec<GrantRef>,
    contexts: Vec<ChannelTurnContext>,
}

async fn input(
    db: &Database,
    tx: &mut ClientSession,
    chat: &ChatAuthority,
    mut filter: bson::Document,
    text: bool,
    latest: bool,
) -> AppResult<Option<Input>> {
    filter.extend(doc! {"conversation_id":&chat.conversation_id,"user_id":&chat.user_id,
        "role":{"$in":["user","orchestrator","event"]},"steering":bson::Bson::Null,"execution_pending":{"$ne":true}});
    let mut projection = doc! {"_id":1,"turn_id":1,"role":1,"meta_grants":1,"meta_contexts":1};
    if text {
        projection.insert("text", 1);
    }
    let Some(row) = db
        .collection::<bson::Document>(MESSAGES)
        .find_one(filter)
        .sort(doc! {"seq":if latest {-1}else{1}})
        .projection(projection)
        .session(tx)
        .await?
    else {
        return Ok(None);
    };
    let proofs = match row.get("meta_grants") {
        None | Some(bson::Bson::Null) => Vec::new(),
        Some(value) => bson::from_bson(value.clone())
            .map_err(|_| super::super::channel_meta_service::grants::incomplete())?,
    };
    let contexts = match row.get("meta_contexts") {
        None | Some(bson::Bson::Null) => Vec::new(),
        Some(value) => bson::from_bson(value.clone())
            .map_err(|_| super::super::channel_meta_service::grants::incomplete())?,
    };
    Ok(Some(Input {
        id: row
            .get_str("_id")
            .map_err(|_| AppError::AssistantTurnRequired)?
            .into(),
        turn: row
            .get_str("turn_id")
            .map_err(|_| AppError::AssistantTurnRequired)?
            .into(),
        role: row
            .get_str("role")
            .map_err(|_| AppError::AssistantTurnRequired)?
            .into(),
        text: row.get_str("text").unwrap_or_default().into(),
        proofs,
        contexts,
    }))
}

fn add_contexts(
    target: &mut Vec<ChannelTurnContext>,
    contexts: &[ChannelTurnContext],
) -> AppResult<()> {
    for context in contexts {
        let proof = context
            .meta_grant
            .as_ref()
            .ok_or_else(super::super::channel_meta_service::grants::incomplete)?;
        if proof.bot_id != context.channel_bot_id {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
        if !target
            .iter()
            .any(|existing| existing.meta_grant.as_ref() == Some(proof))
        {
            target.push(context.clone());
        }
        if target.len() > 100 {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
    }
    Ok(())
}

/// One retained initiating input proves that a captured turn exists. A normal
/// card reads it once; recovery uses the saved origin rather than a live turn.
pub(crate) async fn select(
    db: &Database,
    tx: &mut ClientSession,
    chat: &ChatAuthority,
    conversation: &AssistantConversation,
    origin: Option<&RequestOrigin>,
    text: bool,
) -> AppResult<Selection> {
    let captured = origin
        .and_then(|origin| origin.creation_turn_id.as_deref())
        .or_else(|| {
            origin
                .is_none()
                .then_some(chat.turn_id.as_deref())
                .flatten()
        });
    let first = if let Some(turn) = captured {
        Some(
            input(db, tx, chat, doc! {"turn_id":turn}, text, false)
                .await?
                .ok_or(AppError::AssistantTurnRequired)?,
        )
    } else {
        None
    };
    if origin.is_none() && first.as_ref().is_some_and(|input| !input.proofs.is_empty()) {
        super::super::assistant_access_policy::require_same_live_turn(chat, conversation)?;
    }
    let active = conversation
        .active_turn
        .as_ref()
        .filter(|turn| Some(turn.turn_id.as_str()) == captured);
    let active_contexts = if active.is_some() {
        super::super::async_service_operation::meta_lineage::contexts(conversation)?
    } else {
        Vec::new()
    };
    let mut contexts = Vec::new();
    if let Some(origin) = origin {
        add_contexts(&mut contexts, &origin.meta_contexts)?;
    } else {
        add_contexts(&mut contexts, &active_contexts)?;
    }
    if let Some(first) = &first {
        add_contexts(&mut contexts, &first.contexts)?;
    }
    let mut requested = origin.and_then(|origin| origin.requested_turn_id.clone());
    let mut input_id = origin.and_then(|origin| origin.initiating_input_id.clone());
    // A denial explains an existing question; it never creates another ask.
    if origin.is_none()
        && let Some(event) = active.and_then(|turn| {
            turn.events
                .iter()
                .find(|event| event.kind == "action_denied")
        })
        && let Some(id) = event
            .question_key
            .as_deref()
            .and_then(|key| key.strip_prefix(DENIAL_PREFIX))
    {
        let card=db.collection::<AssistantAcknowledgement>(ACKS).find_one(doc! {"_id":id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"status":"denied"})
            .session(&mut *tx).await?.ok_or(AppError::AssistantTurnRequired)?;
        requested = card.requested_turn_id;
        add_contexts(&mut contexts, &card.meta_contexts)?;
    }
    let selected = if let Some(turn) = requested.as_deref() {
        if first.as_ref().is_some_and(|input| input.turn == turn) {
            None
        } else {
            let mut filter = doc! {"turn_id":turn};
            if let Some(id) = &input_id {
                filter.insert("_id", id);
            }
            Some(
                input(db, tx, chat, filter, text, false)
                    .await?
                    .ok_or(AppError::AssistantTurnRequired)?,
            )
        }
    } else if origin.is_some() {
        None
    } else if first
        .as_ref()
        .is_some_and(|input| input.role != "event" || !input.proofs.is_empty())
        || !active_contexts.is_empty()
        || active
            .is_some_and(|turn| turn.trigger_run_id.is_some() || turn.automation_work.is_some())
    {
        None
    } else {
        let mut filter = doc! {"role":{"$in":["user","orchestrator"]}};
        if let Some(seq) = active.and_then(|turn| turn.initiating_message_seq) {
            filter.insert("seq", doc! {"$lte":seq});
        }
        // `input` supplies the common role floor; keep the legacy role choice
        // in an explicit conjunction for an independent ordinary event.
        filter = doc! {"$and":[filter,{"role":{"$in":["user","orchestrator"]}}]};
        input(db, tx, chat, filter, text, true).await?
    };
    let initiating = selected.as_ref().or(first.as_ref());
    if let Some(initiating) = initiating {
        requested = Some(initiating.turn.clone());
        input_id = Some(initiating.id.clone());
        add_contexts(&mut contexts, &initiating.contexts)?;
    }
    for witness in first.iter().chain(selected.iter()) {
        for proof in &witness.proofs {
            if !contexts
                .iter()
                .any(|context| context.meta_grant.as_ref() == Some(proof))
            {
                return Err(super::super::channel_meta_service::grants::incomplete());
            }
        }
    }
    for context in &contexts {
        if !first.iter().chain(selected.iter()).any(|witness| {
            witness
                .proofs
                .iter()
                .any(|proof| context.meta_grant.as_ref() == Some(proof))
        }) {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
    }
    if let Some(origin) = origin.filter(|origin| !origin.meta_contexts.is_empty()) {
        for context in &contexts {
            if !origin
                .meta_contexts
                .iter()
                .any(|stored| stored.meta_grant == context.meta_grant)
            {
                return Err(super::super::channel_meta_service::grants::incomplete());
            }
        }
    }
    Ok(Selection {
        input_id,
        creation_turn_id: captured.map(str::to_owned).or_else(|| {
            conversation
                .active_turn
                .as_ref()
                .map(|turn| turn.turn_id.clone())
        }),
        requested_turn_id: requested,
        contexts,
        excerpt: initiating.map(|input| {
            format!(
                "{} said: {}",
                input.role,
                super::super::assistant_nyxagent::excerpt(&input.text, 600)
            )
        }),
    })
}

pub(crate) async fn fence_selection(
    db: &Database,
    tx: &mut ClientSession,
    source: &Selection,
) -> AppResult<()> {
    super::super::async_service_operation::meta_lineage::fence(db, tx, &source.contexts, true).await
}

pub(crate) async fn continuation_current(
    db: &Database,
    tx: &mut ClientSession,
    owner: &str,
    conversation: &str,
    event: &crate::models::assistant_conversation::AgentEvent,
) -> AppResult<bool> {
    let id = event
        .question_key
        .as_deref()
        .and_then(|key| key.strip_prefix(DENIAL_PREFIX))
        .ok_or_else(super::super::channel_meta_service::grants::incomplete)?;
    let Some(card) = db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find_one(doc! {"_id":id,"user_id":owner,"conversation_id":conversation,"status":"denied"})
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
    };
    for context in &event.channel_contexts {
        if !card
            .meta_contexts
            .iter()
            .any(|stored| stored.meta_grant == context.meta_grant)
        {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
        if !super::super::async_service_operation::meta_lineage::current(db, tx, context).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) async fn insert_receipt(
    db: &Database,
    chat: &ChatAuthority,
    row: &AssistantAcknowledgement,
) -> AppResult<()> {
    super::super::transaction_service::run_db_only(db, &(db, chat, row), |tx, &(db, chat, row)| {
        Box::pin(async move {
            super::super::erasure_admission::fence_in_session(db, &chat.user_id, tx).await?;
            let changed = db
                .collection::<bson::Document>(CONVERSATIONS)
                .update_one(
                    doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id},
                    doc! {"$inc":{"receipt_fence":1_i64}},
                )
                .session(&mut *tx)
                .await?;
            if changed.matched_count != 1 {
                return Err(AppError::AssistantTurnRequired);
            }
            if let Some(turn) = chat.turn_id.as_deref() {
                let source = input(db, tx, chat, doc! {"turn_id":turn}, false, false)
                    .await?
                    .ok_or(AppError::AssistantTurnRequired)?;
                for proof in source.proofs {
                    super::super::channel_meta_service::scopes::fence_reference(
                        db, tx, &proof, false,
                    )
                    .await?;
                }
            }
            db.collection::<AssistantAcknowledgement>(ACKS)
                .insert_one(row)
                .session(tx)
                .await?;
            Ok(())
        })
    })
    .await
}

/// A decision callback may resume after deletion. Reread and fence the exact
/// stored card before copying its summary into a new event.
pub(crate) async fn publish_owner_denial(
    db: &Database,
    owner: &str,
    id: &str,
    conversation: &str,
) -> AppResult<bool> {
    super::super::transaction_service::run_db_only(db,&(db,owner,id,conversation),|tx,&(db,owner,id,conversation)|Box::pin(async move {
        let Some(row)=db.collection::<AssistantAcknowledgement>(ACKS).find_one_and_update(
            doc! {"_id":id,"user_id":owner,"conversation_id":conversation,"status":"denied"},doc! {"$inc":{"continuation_fence":1_i64}})
            .session(&mut *tx).await? else {return Ok(false)};
        if row.automation_work.is_some() || row.decider!="user" || row.trigger_run_id.is_some()
            || row.voice_request_id.is_some() || row.invocation_id.is_some() {return Ok(false)}
        super::super::erasure_admission::fence_in_session(db,owner,tx).await?;
        for turn in [row.creation_turn_id.as_deref(),row.requested_turn_id.as_deref()].into_iter().flatten() {
            for proof in super::super::channel_meta_service::scopes::assistant_turn_references(db,tx,owner,conversation,turn).await? {
                if !row.meta_contexts.iter().any(|context|context.meta_grant.as_ref()==Some(&proof)) {
                    return Err(super::super::channel_meta_service::grants::incomplete());
                }
            }
        }
        super::super::async_service_operation::meta_lineage::fence(db,tx,&row.meta_contexts,true).await?;
        let mut event=crate::models::assistant_acknowledgement::bind_continuation(row.routing.as_ref(),
            super::super::assistant_team_service::event("action_denied",format!(
                "The owner denied: {}. Do not execute or retry this action. Explain plainly what you skipped. {}",
                super::super::assistant_nyxagent::excerpt(&row.summary,2000),decision_narration(false)),None));
        event.id=Uuid::new_v5(&Uuid::NAMESPACE_OID,format!("nyxid:card-denial:{}",row.id).as_bytes()).to_string();
        event.execution_task=row.execution_task.clone();
        event.channel_contexts=row.meta_contexts.clone();
        event.question_key=Some(format!("{DENIAL_PREFIX}{}",row.id));
        super::super::assistant_nyxagent::append_events_in_session(db,owner,conversation,&[event],tx).await
    })).await
}
