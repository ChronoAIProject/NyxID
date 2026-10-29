//! Durable identities and claims. Started turns and uncertain effects are never replayed.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as THREADS, ChannelOrigin, TurnOrigin,
        },
        channel_adapter::*,
    },
    services::{api_key_mutation_service as transactions, assistant_nyxagent as engine},
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use sha2::{Digest, Sha256};
pub fn identity(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_be_bytes());
        h.update(p.as_bytes());
    }
    hex::encode(h.finalize())
}
pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    for (collection, keys, name) in [
        (
            BINDINGS,
            doc! {"profile":1,"account_id":1,"resource_id":1},
            "adapter_resource_profile",
        ),
        (
            ALLOCATIONS,
            doc! {"provider":1,"account_id":1,"resource_id":1},
            "adapter_resource_owner",
        ),
        (
            ALLOCATIONS,
            doc! {"provider":1,"account_id":1,"phone_number":1},
            "adapter_number_owner",
        ),
        (
            ALLOCATIONS,
            doc! {"provider":1,"account_id":1,"application_id":1},
            "adapter_application_owner",
        ),
        (
            ACCOUNTS,
            doc! {"provider":1,"account_id":1},
            "adapter_account_trust",
        ),
        (
            SESSIONS,
            doc! {"identity_key":1},
            "adapter_session_identity",
        ),
        (EVENTS, doc! {"identity_key":1}, "adapter_event_identity"),
        (
            CLAIMS,
            doc! {"identity_key":1},
            "adapter_global_resource_identity",
        ),
    ] {
        db.collection::<bson::Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_string())
                            .unique(true)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    db.collection::<EventReceipt>(EVENTS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"binding_id":1,"provider_message_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("adapter_send_provider_identity".to_string())
                        .unique(true)
                        .partial_filter_expression(
                            doc! {"event_kind":"sms_send","provider_message_id":{"$type":"string"}},
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<CallSession>(SESSIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"state":1,"lease_expires_at":1})
                .build(),
        )
        .await?;
    for (collection, keys, name) in [
        (BINDINGS, doc! {"user_id":1,"_id":1}, "adapter_owner_page"),
        (
            SESSIONS,
            doc! {"binding_id":1,"state":1,"billing_finalized":1},
            "adapter_binding_cleanup",
        ),
        (
            SESSIONS,
            doc! {"user_id":1,"conversation_id":1},
            "adapter_owner_conversation",
        ),
        (
            SESSIONS,
            doc! {"recovery_checked_at":1,"_id":1,"state":1,"lease_expires_at":1},
            "adapter_recovery_order",
        ),
        (
            EVENTS,
            doc! {"binding_id":1,"event_kind":1,"provider_message_id":1,"created_at":1},
            "adapter_receipt_join",
        ),
        (
            EVENTS,
            doc! {"binding_id":1,"event_kind":1,"billing_finalized":1},
            "adapter_binding_usage_cleanup",
        ),
        (
            EVENTS,
            doc! {"event_kind":1,"billing_finalized":1,"recovery_checked_at":1,"_id":1},
            "adapter_sms_recovery_order",
        ),
        (CLAIMS, doc! {"user_id":1}, "adapter_owner_claims"),
    ] {
        db.collection::<bson::Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().name(name.to_owned()).build())
                    .build(),
            )
            .await?;
    }
    Ok(())
}
pub async fn prepare(
    state: &AppState,
    b: &Binding,
    call: &str,
    from: &str,
    to: &str,
) -> AppResult<(CallSession, bool)> {
    let identity_key = identity(&[super::profiles::provider(b.profile), &b.account_id, call]);
    let now = Utc::now();
    let row = CallSession {
        id: uuid::Uuid::new_v4().to_string(),
        identity_key: identity_key.clone(),
        user_id: b.user_id.clone(),
        binding_id: b.id.clone(),
        binding_revision: b.revision,
        profile: b.profile,
        contract_version: 1,
        credential_identity: b.verified_credential_identity.clone(),
        provider_call_id: call.into(),
        provider_session_id: None,
        from: from.into(),
        to: to.into(),
        conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        state: SessionState::Prepared,
        claim_id: None,
        answered: false,
        answer_dispatched: false,
        relay_dispatched: false,
        hangup_dispatched: false,
        cap_dispatched: false,
        cap_confirmed: false,
        bootstrap_dispatched: false,
        generation: 0,
        next_utterance: 0,
        custom_history: Vec::new(),
        active_turn_id: None,
        reply_turn_id: None,
        socket_ticket_hash: None,
        billing_request_id: format!("adapter-call:{}", uuid::Uuid::new_v4()),
        observed_seconds: 0,
        ended_at: None,
        terminal_evidence: None,
        billing_finalized: false,
        delivery: "generated_not_confirmed".into(),
        heard_prefix_chars: None,
        heard_prefix_matches_sent: false,
        created_at: now,
        connected_at: None,
        lease_expires_at: now + Duration::seconds(30),
        expires_at: now + Duration::seconds(1800),
    };
    let mut tx = state.db.client().start_session().await?;
    let db = state.db.clone();
    let binding = b.clone();
    let inserted = row.clone();
    let result=tx.start_transaction().and_run2(async move|tx|{
        let result:AppResult<()>=async{
            super::owner_admission::fence(&db,tx,&binding.user_id).await?;
            let authority=db.collection::<Binding>(BINDINGS).update_one(doc!{"_id":&binding.id,"enabled":true,"revision":binding.revision,"verified_credential_identity":&binding.verified_credential_identity},doc!{"$inc":{"admission_fence":1}}).session(&mut *tx).await?;
            if authority.matched_count!=1{return Err(AppError::AssistantTurnRequired);}
            db.collection::<CallSession>(SESSIONS).insert_one(&inserted).session(&mut *tx).await?;Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await;
    let fresh = match result {
        Ok(()) => true,
        Err(e) if super::binding::duplicate(&e) => false,
        Err(e) => return Err(transactions::map_transaction_error(e)),
    };
    let saved = by_call(state, b, call).await?;
    if saved.binding_id != b.id
        || saved.binding_revision != b.revision
        || saved.from != from
        || saved.to != to
    {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Call resource mismatch".into(),
        ));
    }
    Ok((saved, fresh))
}
pub async fn by_call(state: &AppState, b: &Binding, call: &str) -> AppResult<CallSession> {
    state.db.collection::<CallSession>(SESSIONS).find_one(doc!{"identity_key":identity(&[super::profiles::provider(b.profile),&b.account_id,call]),"binding_id":&b.id}).await?.ok_or_else(||AppError::NotFound("Call session unavailable".into()))
}
pub async fn load(state: &AppState, id: &str) -> AppResult<CallSession> {
    state
        .db
        .collection::<CallSession>(SESSIONS)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(|| AppError::NotFound("Call session unavailable".into()))
}

/// Persist an effect identity only while its session and owner still admit work.
/// Disable/finish and owner deletion write these same records before checking
/// pending evidence, so an effect cannot appear after their idle snapshot.
pub(super) async fn insert_sms_send(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
    row: &EventReceipt,
) -> AppResult<()> {
    let db = state.db.clone();
    let b = b.clone();
    let s = s.clone();
    let row = row.clone();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction().and_run2(async move |tx| {
        let result: AppResult<()> = async {
            super::owner_admission::fence(&db, tx, &b.user_id).await?;
            let binding = db.collection::<Binding>(BINDINGS).update_one(
                doc! {"_id":&b.id,"enabled":true,"revision":s.binding_revision,"verified_credential_identity":&s.credential_identity},
                doc! {"$inc":{"admission_fence":1_i64}},
            ).session(&mut *tx).await?;
            let session = db.collection::<CallSession>(SESSIONS).update_one(
                doc! {"_id":&s.id,"user_id":&b.user_id,"binding_revision":s.binding_revision,"generation":s.generation,"claim_id":&s.claim_id,"state":"active","expires_at":{"$gt":bson::DateTime::now()},"lease_expires_at":{"$gt":bson::DateTime::now()}},
                doc! {"$inc":{"egress_fence":1_i64}},
            ).session(&mut *tx).await?;
            if binding.matched_count != 1 || session.matched_count != 1 {
                return Err(AppError::AssistantTurnRequired);
            }
            db.collection::<EventReceipt>(EVENTS).insert_one(&row).session(&mut *tx).await?;
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

pub(super) async fn activate_noncall(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
    ticket_hash: Option<String>,
) -> AppResult<CallSession> {
    if super::profiles::relay(s.profile) {
        return Err(AppError::ChannelPlatformSendUnsupported);
    }
    let db = state.db.clone();
    let b = b.clone();
    let prepared = s.clone();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction().and_run2(async move |tx| {
        let result: AppResult<()> = async {
            super::owner_admission::fence(&db, tx, &b.user_id).await?;
            let binding = db.collection::<Binding>(BINDINGS).update_one(
                doc! {"_id":&b.id,"enabled":true,"revision":prepared.binding_revision},
                doc! {"$inc":{"admission_fence":1_i64}},
            ).session(&mut *tx).await?;
            let session = db.collection::<CallSession>(SESSIONS).update_one(
                doc! {"_id":&prepared.id,"state":"prepared","generation":prepared.generation,"binding_revision":prepared.binding_revision,"ended_at":bson::Bson::Null,"billing_finalized":false,"lease_expires_at":{"$gt":bson::DateTime::now()}},
                doc! {"$set":{"state":"active","claim_id":&prepared.id,"socket_ticket_hash":&ticket_hash,"lease_expires_at":bson::DateTime::from_chrono(prepared.expires_at)}},
            ).session(&mut *tx).await?;
            if binding.matched_count != 1 || session.matched_count != 1 {
                return Err(AppError::AssistantTurnRequired);
            }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
    let active = load(state, &s.id).await?;
    fence(state, &active).await?;
    Ok(active)
}
pub async fn command(state: &AppState, s: &CallSession, field: &str) -> AppResult<bool> {
    if !matches!(
        field,
        "answer_dispatched"
            | "relay_dispatched"
            | "hangup_dispatched"
            | "cap_dispatched"
            | "bootstrap_dispatched"
    ) {
        return Err(AppError::BadRequest("Invalid call command".into()));
    }
    let mut f = doc! {"_id":&s.id,"binding_revision":s.binding_revision};
    f.insert(field, false);
    if field == "relay_dispatched" {
        f.insert("answered", true);
    }
    let mut set = doc! {};
    set.insert(field, true);
    Ok(state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_one(f, doc! {"$set":set})
        .await?
        .modified_count
        == 1)
}
pub async fn receipt(state: &AppState, b: &Binding, event: &str, kind: &str) -> AppResult<bool> {
    let row = EventReceipt {
        id: uuid::Uuid::new_v4().to_string(),
        identity_key: identity(&[&b.id, &b.revision.to_string(), event]),
        user_id: b.user_id.clone(),
        binding_id: b.id.clone(),
        binding_revision: b.revision,
        session_id: None,
        event_kind: kind.into(),
        status: "admitted".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: None,
        observed_parts: None,
        reserved_parts: None,
        billing_finalized: false,
        from: None,
        to: None,
        created_at: Utc::now(),
    };
    let db = state.db.clone();
    let b = b.clone();
    let mut tx = db.client().start_session().await?;
    let result = tx
        .start_transaction()
        .and_run2(async move |tx| {
            let result: AppResult<()> = async {
                super::binding::retain_receipt(&db, tx, &b).await?;
                db.collection::<EventReceipt>(EVENTS)
                    .insert_one(&row)
                    .session(&mut *tx)
                    .await?;
                Ok(())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await;
    match result {
        Ok(_) => Ok(true),
        Err(e) if super::binding::duplicate(&e) => Ok(false),
        Err(e) => Err(transactions::map_transaction_error(e)),
    }
}
pub async fn admit(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
    event: &str,
    text: &str,
    history: Option<Vec<String>>,
) -> AppResult<engine::OwnerTurnClaim> {
    if text.is_empty() || text.len() > 8192 {
        return Err(AppError::ValidationError("Utterance exceeds limit".into()));
    }
    super::binding::live(state, b).await?;
    let mut tx = state.db.client().start_session().await?;
    let db = state.db.clone();
    let keys = state.encryption_keys.clone();
    let b = b.clone();
    let s = s.clone();
    let text = text.to_owned();
    let event = event.to_owned();
    let ids = engine::OwnerTurnIds {
        conversation_id: s.conversation_id.clone(),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let claim=tx.start_transaction().and_run2(async move |tx| {
        let result:AppResult<_>=async {
            super::owner_admission::fence(&db,tx,&b.user_id).await?;
            let live=db.collection::<Binding>(BINDINGS).find_one(doc!{"_id":&b.id,"enabled":true,"revision":b.revision}).session(&mut *tx).await?.ok_or(AppError::AssistantTurnRequired)?;
            // Write fence binding revision against concurrent disable/reconfiguration.
            db.collection::<Binding>(BINDINGS).update_one(doc!{"_id":&live.id,"revision":live.revision},doc!{"$set":{"updated_at":bson::DateTime::from_chrono(Utc::now())}}).session(&mut *tx).await?;
            let agent=db.collection::<crate::models::assistant_agent::AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME).find_one(doc!{"_id":&b.agent_id,"user_id":&b.user_id,"destroyed_at":bson::Bson::Null}).session(&mut *tx).await?.ok_or(AppError::AssistantTurnRequired)?;
            if agent.kind!=crate::models::assistant_agent::AgentKind::Specialist{return Err(AppError::AssistantTurnRequired);}
            if let Some(history)=&history {
                if history.len()>256 || (!s.custom_history.is_empty() && (history.len()<=s.custom_history.len() || !history.starts_with(&s.custom_history))) {return Err(AppError::AssistantTurnRequired);}
                let history_write=db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"custom_history":bson::to_bson(&s.custom_history).map_err(|_|AppError::AssistantTurnRequired)?},doc!{"$set":{"custom_history":bson::to_bson(history).map_err(|_|AppError::AssistantTurnRequired)?}}).session(&mut *tx).await?;
                if history_write.modified_count!=1{return Err(AppError::AssistantTurnRequired);}
            }
            let updated=db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"binding_revision":b.revision,"generation":s.generation,"state":{"$in":["prepared","active"]},"expires_at":{"$gt":bson::DateTime::from_chrono(Utc::now())}},doc!{"$inc":{"next_utterance":1},"$set":{"active_turn_id":&ids.turn_id,"reply_turn_id":&ids.turn_id,"delivery":"generated_not_confirmed","heard_prefix_chars":bson::Bson::Null,"heard_prefix_matches_sent":false}}).session(&mut *tx).await?;
            if updated.modified_count!=1{return Err(AppError::AssistantTurnRequired);}
            let receipt=EventReceipt{id:uuid::Uuid::new_v4().to_string(),identity_key:identity(&[&b.id,&b.revision.to_string(),&s.id,&event]),user_id:b.user_id.clone(),binding_id:b.id.clone(),binding_revision:b.revision,session_id:Some(s.id.clone()),event_kind:"utterance".into(),status:"started".into(),provider_message_id:None,delivery:"unknown".into(),billing_request_id:None,observed_parts:None,reserved_parts:None,billing_finalized:false,from:None,to:None,created_at:Utc::now()};
            db.collection::<EventReceipt>(EVENTS).insert_one(receipt).session(&mut *tx).await?;
            let existing=db.collection::<AssistantConversation>(THREADS).find_one(doc!{"_id":&s.conversation_id,"user_id":&b.user_id}).session(&mut *tx).await?.is_some();
            let start=engine::TurnStart{channel_uploads:Vec::new(),invocation_id:None,
                media_event_ids: Vec::new(),acknowledgement_id:None,retry_of:None,channel_event_id:None,org_access:None,attachment_ids:vec![],group_attachments:vec![],group_request_id:None,trigger:None,conversation_id:existing.then(||s.conversation_id.clone()),text:text.clone(),model:None,origin:TurnOrigin::Channel,channel:Some(ChannelOrigin{native_event_id:None,thread:None,nyxbot_channel_id:b.id.clone(),partition:s.id.clone(),platform:super::profiles::id(b.profile).into()}),title:Some("External guest conversation".into()),note:None,new_id:Some(s.conversation_id.clone()),agent_id:Some(b.agent_id.clone()),report_to:None,group_id:None,guest:true,question_key:None,question:Some(text.clone()),reply_channel:None};
            // Provider playback differs from model generation. Rebuild context from
            // retained transcript evidence instead of reusing unconfirmed audio history.
            db.collection::<AssistantConversation>(THREADS).update_one(doc!{"_id":&s.conversation_id,"user_id":&b.user_id},doc!{"$set":{"nyxagent_session_id":bson::Bson::Null,"nyxagent_last_response_id":bson::Bson::Null,"nyxagent_instruction_binding":bson::Bson::Null,"context_reset_at":bson::DateTime::now(),"context_reset_reason":"channel_delivery"}}).session(&mut *tx).await?;
            let mut claim=Box::pin(engine::begin_owner_turn_in_session(&db,tx,&keys,&b.user_id,&start,&ids,&agent,None,None,None)).await?.into_started()?;
            let authority=TurnAuthority{binding_id:b.id.clone(),session_id:s.id.clone(),binding_revision:b.revision,generation:s.generation,credential_identity:s.credential_identity.clone()};
            db.collection::<AssistantConversation>(THREADS).update_one(doc!{"_id":&ids.conversation_id,"active_turn.turn_id":&ids.turn_id},doc!{"$set":{"active_turn.channel_adapter_origin":bson::to_bson(&authority).map_err(|_|AppError::AssistantTurnRequired)?}}).session(&mut *tx).await?;
            if let Some(turn)=claim.conversation.active_turn.as_mut(){turn.channel_adapter_origin=Some(authority);}
            Ok(claim)
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
    claim.log_reclaimed();
    Ok(claim)
}
pub async fn fence(state: &AppState, s: &CallSession) -> AppResult<()> {
    let row = load(state, &s.id).await?;
    if row.generation != s.generation
        || row.claim_id != s.claim_id
        || row.binding_revision != s.binding_revision
        || row.state == SessionState::Closed
        || row.state == SessionState::Uncertain
        || Utc::now() > row.expires_at
    {
        return Err(AppError::AssistantTurnRequired);
    }
    let b = super::binding::load(state, &s.binding_id).await?;
    if b.revision != row.binding_revision
        || b.verified_credential_identity != row.credential_identity
        || b.agent_id.is_empty()
        || Utc::now() > row.lease_expires_at
    {
        return Err(AppError::AssistantTurnRequired);
    }
    super::binding::read_authority(&state.db, &b).await?;
    super::binding::validate_agent(state, &b.user_id, &b.agent_id).await?;
    engine::require_enabled(&state.db, &b.user_id).await?;
    Ok(())
}
pub async fn interrupt(state: &AppState, s: &CallSession) -> AppResult<CallSession> {
    state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_one(
            doc! {"_id":&s.id,"generation":s.generation},
            doc! {"$inc":{"generation":1}},
        )
        .await?;
    if s.active_turn_id.is_some() {
        engine::request_stop(&state.db, &s.user_id, &s.conversation_id).await?;
    }
    load(state, &s.id).await
}
pub async fn expired(state: &AppState) -> AppResult<Vec<CallSession>> {
    Ok(state.db.collection::<CallSession>(SESSIONS).find(doc!{"$or":[{"state":{"$in":["prepared","active","uncertain"]},"lease_expires_at":{"$lte":bson::DateTime::from_chrono(Utc::now())}},{"state":"closed","billing_finalized":false}]}).sort(doc!{"recovery_checked_at":1,"_id":1}).limit(100).await?.try_collect().await?)
}

/// Tool callbacks revalidate this mandatory fence regardless of rollout flags.
/// No adapter reads occur for ordinary conversations.
pub async fn fence_tool(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    a: &TurnAuthority,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let _session=db.collection::<CallSession>(SESSIONS).find_one(doc!{"_id":&a.session_id,"user_id":owner,"binding_id":&a.binding_id,"binding_revision":a.binding_revision,"credential_identity":&a.credential_identity,"generation":a.generation,"active_turn_id":turn,"conversation_id":conversation,"state":{"$in":["prepared","active"]},"expires_at":{"$gt":now},"lease_expires_at":{"$gt":now}}).await?.ok_or(AppError::AssistantTurnRequired)?;
    let b=db.collection::<Binding>(BINDINGS).find_one(doc!{"_id":&a.binding_id,"user_id":owner,"enabled":true,"revision":a.binding_revision,"verified_credential_identity":&a.credential_identity}).await?.ok_or(AppError::AssistantTurnRequired)?;
    super::binding::read_authority(db, &b).await?;
    let row=db.collection::<AssistantConversation>(THREADS).find_one(doc!{"_id":conversation,"user_id":owner,"active_turn.turn_id":turn,"active_turn.stop_requested":{"$ne":true}}).await?.ok_or(AppError::AssistantTurnRequired)?;
    if row
        .active_turn
        .as_ref()
        .and_then(|t| t.channel_adapter_origin.as_ref())
        != Some(a)
    {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(())
}

/// Freeze the first authoritative terminal observation atomically with its quantity.
pub async fn terminal(
    state: &AppState,
    s: &CallSession,
    at: chrono::DateTime<Utc>,
    seconds: Option<i64>,
    evidence: &str,
) -> AppResult<CallSession> {
    let current = load(state, &s.id).await?;
    let at = at.min(Utc::now()).max(current.created_at);
    let quantity = seconds.unwrap_or_else(|| {
        current
            .connected_at
            .map(|start| (at - start).num_seconds().max(0))
            .unwrap_or(0)
    });
    if !(0..=86400).contains(&quantity) {
        return Err(AppError::BadRequest(
            "Invalid terminal call duration".into(),
        ));
    }
    state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"ended_at":bson::Bson::Null},doc!{"$set":{"state":"closed","ended_at":bson::DateTime::from_chrono(at),"observed_seconds":quantity,"terminal_evidence":evidence},"$inc":{"generation":1}}).await?;
    load(state, &s.id).await
}

pub async fn record_interruption(
    state: &AppState,
    s: &CallSession,
    prefix: &str,
    sent: &str,
    duration: u64,
    reply_turn: Option<&str>,
) -> AppResult<()> {
    let verified = reply_turn.is_some()
        && !prefix.is_empty()
        && sent.starts_with(prefix)
        && duration <= 1800000;
    let evidence = crate::models::assistant_message::ChannelDelivery {
        status: if verified {
            "interrupted_prefix"
        } else {
            "interruption_unverified"
        }
        .into(),
        heard_prefix_chars: verified.then_some(prefix.chars().count() as i64),
        heard_prefix_matches_sent: verified,
    };
    state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"generation":s.generation,"reply_turn_id":reply_turn},doc!{"$set":{"delivery":&evidence.status,"heard_prefix_chars":evidence.heard_prefix_chars,"heard_prefix_matches_sent":verified}}).await?;
    if let Some(turn) = reply_turn {
        state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME).update_many(doc!{"conversation_id":&s.conversation_id,"turn_id":turn,"role":"assistant"},doc!{"$set":{"turn_metadata.channel_delivery":bson::to_bson(&evidence).map_err(|_|AppError::AssistantTurnRequired)?}}).await?;
    }
    Ok(())
}

pub async fn finish_noncall(state: &AppState, id: &str, reason: &str) -> AppResult<()> {
    let s = load(state, id).await?;
    if super::profiles::relay(s.profile) {
        return Err(AppError::BadRequest(
            "Carrier sessions require terminal evidence".into(),
        ));
    }
    let _ = engine::request_stop(&state.db, &s.user_id, &s.conversation_id).await;
    state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":id,"ended_at":bson::Bson::Null},doc!{"$set":{"state":"closed","ended_at":bson::DateTime::now(),"terminal_evidence":reason,"billing_finalized":true},"$inc":{"generation":1}}).await?;
    Ok(())
}
