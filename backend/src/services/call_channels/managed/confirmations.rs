//! One narrow specialist effect: text the carrier-verified caller of this live call.
//! The owner's per-binding opt-in is standing authorization for this Change.
//! Guests cannot decide action cards; this tool grants no account authority and
//! never exists outside an admitted live voice-call turn.
use super::super::{binding, profiles, session};
use super::{admission, conflict, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{channel_adapter::*, managed_comms::*},
    services::{
        audit_service::{self, AuditActor},
        transaction_service,
    },
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::Deserialize;
use serde_json::{Value, json};

pub const TOOL: &str = "call_channel__text_current_caller";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToggleInput {
    pub enabled: bool,
    pub sms_binding_id: Option<String>,
    pub voice_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    pub text: String,
    pub caller_consented: bool,
    pub ordinal: i64,
}

pub async fn configure(
    state: &AppState,
    actor: &AuditActor,
    id: &str,
    input: ToggleInput,
) -> AppResult<()> {
    let v = binding::load(state, id).await?;
    if !input.enabled {
        if v.user_id != actor.user_id
            || v.revision != input.voice_revision
            || !profiles::relay(v.profile)
        {
            return Err(unavailable());
        }
        return disable(state, actor, &v).await;
    }
    if v.user_id != actor.user_id
        || !profiles::relay(v.profile)
        || v.revision != input.voice_revision
    {
        return Err(unavailable());
    }
    let managed = if matches!(v.credential, CredentialBinding::ManagedLine { .. }) {
        Some(admission::read(&state.db, &v.user_id, &v.credential).await?)
    } else {
        None
    };
    let sms_id = if let Some(a) = &managed {
        a.claim.sms_binding_id.clone().ok_or_else(unavailable)?
    } else {
        input.sms_binding_id.ok_or_else(unavailable)?
    };
    let sms = binding::load(state, &sms_id).await?;
    same_resources(&v, &sms)?;
    if managed.is_none()
        && (!matches!(v.credential, CredentialBinding::Connection { .. })
            || !matches!(sms.credential, CredentialBinding::Connection { .. }))
    {
        return Err(unavailable());
    }
    if input.enabled {
        // Two independent provider reads prove both associations with the same
        // number under the credentials that will execute them.
        for b in [&v, &sms] {
            let r = binding::live(state, b).await?;
            for path in binding::resource_paths(b) {
                binding::authorize(state, b, &r, "GET", &path, None).await?;
            }
            binding::verify_resources(
                crate::services::proxy_service::no_redirect_http_client(),
                b,
                &r.key,
                Some(&profiles::binding_origin(b)?),
            )
            .await?;
        }
    }
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,actor,&v,&sms,&managed,input.enabled,state.audit_chain_hmac_key.as_ref().as_ref()),|tx,c|Box::pin(async move{
        let (db,actor,v,sms,managed,on,key)=*c;
        super::super::owner_admission::fence(db,tx,&v.user_id).await?;
        if let Some(a)=managed{admission::fence(db,tx,a,Utc::now()).await?;db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&a.claim.id},doc!{"$set":{"caller_confirmations":on},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;}
        for b in [v,sms] {if db.collection::<Binding>(BINDINGS).update_one(doc!{"_id":&b.id,"user_id":&v.user_id,"revision":b.revision,"verified_credential_identity":&b.verified_credential_identity},doc!{"$inc":{"admission_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}}
        let row=CallerConfirmationBinding{id:uuid::Uuid::new_v4().to_string(),voice_binding_id:v.id.clone(),voice_binding_revision:v.revision,sms_binding_id:sms.id.clone(),sms_binding_revision:sms.revision,voice_credential_identity:v.verified_credential_identity.clone(),sms_credential_identity:sms.verified_credential_identity.clone(),enabled:on,fence:0};
        let mut doc=bson::to_document(&row).map_err(|_|unavailable())?;doc.remove("_id");doc.remove("fence");doc.insert("user_id",&v.user_id);
        db.collection::<bson::Document>(CallerConfirmationBinding::COLLECTION_NAME).update_one(doc!{"voice_binding_id":&v.id},doc!{"$set":doc,"$setOnInsert":{"_id":row.id},"$inc":{"fence":1_i64}}).upsert(true).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"caller_confirmations_configured",json!({"voice_binding_id":v.id,"sms_binding_id":sms.id,"enabled":on})).await?;Ok(())
    })).await
}
fn same_resources(v: &Binding, s: &Binding) -> AppResult<()> {
    if v.user_id != s.user_id
        || profiles::provider(v.profile) != profiles::provider(s.profile)
        || v.account_id != s.account_id
        || v.resource_id != s.resource_id
        || v.phone_number != s.phone_number
        || !profiles::relay(v.profile)
        || !profiles::sms(s.profile)
    {
        return Err(conflict(
            "Voice and SMS must be the same owner's proven account and number",
        ));
    }
    Ok(())
}
async fn pair(state: &AppState, v: &Binding) -> AppResult<Binding> {
    let sms = if matches!(v.credential, CredentialBinding::ManagedLine { .. }) {
        let a = admission::read(&state.db, &v.user_id, &v.credential).await?;
        if !a.claim.caller_confirmations
            || !a.claim.sms_enabled
            || a.claim.voice_binding_id.as_deref() != Some(&v.id)
        {
            return Err(unavailable());
        }
        binding::load(
            state,
            a.claim.sms_binding_id.as_deref().ok_or_else(unavailable)?,
        )
        .await?
    } else {
        let a=state.db.collection::<CallerConfirmationBinding>(CallerConfirmationBinding::COLLECTION_NAME).find_one(doc!{"voice_binding_id":&v.id,"enabled":true,"voice_binding_revision":v.revision,"voice_credential_identity":&v.verified_credential_identity}).await?.ok_or_else(unavailable)?;
        let sms = binding::load(state, &a.sms_binding_id).await?;
        if sms.revision != a.sms_binding_revision
            || sms.verified_credential_identity != a.sms_credential_identity
        {
            return Err(unavailable());
        }
        sms
    };
    same_resources(v, &sms)?;
    binding::live(state, &sms).await?;
    Ok(sms)
}
pub async fn eligible(
    db: &mongodb::Database,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
) -> bool {
    if chat.is_orchestrator()
        || !chat.turn_live
        || chat.turn_stopped
        || chat.collaboration_thread
        || chat.org_agent_access.is_some()
    {
        return false;
    }
    let Some(a) = &chat.channel_adapter_origin else {
        return false;
    };
    let Ok(Some(s))=db.collection::<CallSession>(SESSIONS).find_one(doc!{"_id":&a.session_id,"binding_id":&a.binding_id,"conversation_id":&chat.conversation_id,"active_turn_id":&chat.turn_id,"state":"active","generation":a.generation}).await else{return false;};
    if !profiles::relay(s.profile) {
        return false;
    }
    if let Some(id) = s.managed_claim_id {
        db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
            .find_one(doc! {"_id":id,"caller_confirmations":true,"sms_enabled":true})
            .await
            .ok()
            .flatten()
            .is_some()
    } else {
        db.collection::<CallerConfirmationBinding>(CallerConfirmationBinding::COLLECTION_NAME).find_one(doc!{"voice_binding_id":&s.binding_id,"voice_binding_revision":s.binding_revision,"enabled":true}).await.ok().flatten().is_some()
    }
}
pub fn endpoint() -> crate::services::mcp_service::McpToolEndpoint {
    crate::services::mcp_service::McpToolEndpoint{endpoint_id:TOOL.into(),name:"text_current_caller".into(),description:Some("Send a short confirmation to the current call's carrier-verified caller, only with their consent. The owner’s per-binding toggle provides standing authorization; callers cannot approve cards. Available only during an admitted voice call. No destination parameter. At most two confirmations per call; uncertain sends are never retried. caller_consented is model-asserted consent, not proof.".into()),method:"POST".into(),request_content_type:Some("application/json".into()),request_body_required:true,request_body_schema:Some(json!({"type":"object","properties":{"text":{"type":"string","maxLength":480},"caller_consented":{"type":"boolean"},"ordinal":{"type":"integer","minimum":0,"maximum":1}},"required":["text","caller_consented","ordinal"],"additionalProperties":false})),..Default::default()}
}
/// A separate internal service keeps generated list/search names out of the
/// account namespace. Both discovery entry points use this live evaluator.
pub async fn service_for(
    db: &mongodb::Database,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
) -> Option<crate::services::mcp_service::McpToolService> {
    if !eligible(db, chat).await {
        return None;
    }
    use crate::services::mcp_service::{McpToolService, McpToolSource};
    Some(McpToolService {
        workspace_destinations_pending: false,
        service_id: "call_channel".into(),
        service_name: "Current voice call".into(),
        service_slug: "call_channel".into(),
        description: Some(
            "Owner-authorized confirmations to this call's verified caller only.".into(),
        ),
        service_category: "internal".into(),
        source: McpToolSource::Internal,
        executable: true,
        is_generic_proxy: false,
        invalid_openapi_contract: false,
        recommended_skills: vec![],
        recommended_skill_refs: None,
        skills_revision: None,
        proxy_operation_policy: None,
        durable_endpoint_metadata: Default::default(),
        endpoints: vec![endpoint()],
    })
}
async fn turn_fence(
    state: &AppState,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
) -> AppResult<(Binding, CallSession, Binding)> {
    if chat.is_orchestrator()
        || !chat.turn_live
        || chat.turn_stopped
        || chat.org_agent_access.is_some()
        || chat.collaboration_thread
    {
        return Err(AppError::AssistantTurnRequired);
    }
    let a = chat
        .channel_adapter_origin
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    session::fence_tool(
        &state.db,
        &chat.user_id,
        &chat.conversation_id,
        chat.turn_id
            .as_deref()
            .ok_or(AppError::AssistantTurnRequired)?,
        a,
    )
    .await?;
    let s = session::load(state, &a.session_id).await?;
    if s.state != SessionState::Active
        || !profiles::relay(s.profile)
        || !super::super::protocol::phone(&s.from)
    {
        return Err(AppError::AssistantTurnRequired);
    }
    let v = binding::load(state, &s.binding_id).await?;
    let sms = pair(state, &v).await?;
    Ok((v, s, sms))
}
pub async fn send(
    state: &AppState,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
    invocation: &str,
    i: SendInput,
) -> AppResult<Value> {
    if i.text.is_empty()
        || i.text.len() > 480
        || !i.caller_consented
        || !(0..=1).contains(&i.ordinal)
        || invocation.is_empty()
        || invocation.len() > 256
    {
        return Err(unavailable());
    }
    let (v, s, sms) = turn_fence(state, chat).await?;
    let digest = session::identity(&[&i.text, if i.caller_consented { "true" } else { "false" }]);
    let identity = doc! {"session_id":&s.id,"tool_invocation_id":invocation,"ordinal":i.ordinal};
    let operations = state
        .db
        .collection::<ManagedCallConfirmation>(ManagedCallConfirmation::COLLECTION_NAME);
    if let Some(old) = operations.find_one(identity.clone()).await? {
        if old.payload_digest != digest {
            return Err(conflict(
                "Confirmation invocation already has a different payload",
            ));
        }
        return Ok(json!({"operation_id":old.id,"status":old.status,"replayed":true}));
    }
    let operation_id = uuid::Uuid::new_v4().to_string();
    let parts = profiles::sms_parts(sms.profile, &i.text)?;
    let requests: Vec<_> = (0..parts)
        .map(|part| format!("call-confirmation:{operation_id}:{part}"))
        .collect();
    let r = binding::live(state, &sms).await?;
    let callback = format!(
        "{}/sms-receipts/{operation_id}",
        super::super::coordinator::webhook_url(state, &sms)
    );
    let payload = profiles::sms_payload(&sms, &s.from, &i.text, &callback)?;
    let path = if sms.profile == Profile::TelnyxSms {
        "/v2/messages".into()
    } else {
        format!("/2010-04-01/Accounts/{}/Messages.json", sms.account_id)
    };
    let managed = if matches!(sms.credential, CredentialBinding::ManagedLine { .. }) {
        Some(admission::read(&state.db, &sms.user_id, &sms.credential).await?)
    } else {
        None
    };
    let reference = managed.as_ref().map(|a| AdmittedReference {
        trusted_account_id: a.account.id.clone(),
        credential_service_id: a.account.catalog_service_id.clone(),
        provider: a.line.provider.clone(),
        account_id: a.account.account_id.clone(),
        line_id: a.line.id.clone(),
        number_resource_id: a.line.number_resource_id.clone(),
        application_id: sms.application_id.clone(),
        provider_call_id: None,
        provider_message_id: None,
        call_leg_id: None,
        call_session_id: None,
    });
    let mut meters = Vec::new();
    for request in &requests {
        match super::super::billing::reserve(state, &sms, &r, request).await {
            Ok(m) => meters.push(m),
            Err(e) => {
                super::super::billing::cancel(state, &meters, "confirmation_reservation_failed")
                    .await;
                return Err(e);
            }
        }
    }
    let row = ManagedCallConfirmation {
        id: operation_id.clone(),
        session_id: s.id.clone(),
        tool_invocation_id: invocation.into(),
        ordinal: i.ordinal,
        payload_digest: digest,
        voice_binding_id: v.id.clone(),
        voice_binding_revision: v.revision,
        session_generation: s.generation,
        sms_binding_id: sms.id.clone(),
        sms_binding_revision: sms.revision,
        status: ConfirmationState::Prepared,
        billing_request_ids: requests.clone(),
        model_asserted_caller_consent: i.caller_consented,
        managed_claim_id: s.managed_claim_id.clone(),
        managed_claim_generation: s.managed_claim_generation,
        admitted: reference.clone(),
        created_at: Utc::now(),
    };
    let receipt = EventReceipt {
        id: operation_id.clone(),
        identity_key: session::identity(&[
            "call-confirmation",
            &s.id,
            invocation,
            &i.ordinal.to_string(),
        ]),
        user_id: s.user_id.clone(),
        binding_id: sms.id.clone(),
        binding_revision: sms.revision,
        managed_claim_id: s.managed_claim_id.clone(),
        managed_claim_generation: s.managed_claim_generation,
        admitted_reference: reference.clone(),
        session_id: Some(s.id.clone()),
        event_kind: "sms_send".into(),
        status: "prepared".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: requests.first().cloned(),
        observed_parts: None,
        reserved_parts: Some(parts),
        billing_finalized: false,
        from: Some(sms.phone_number.clone()),
        to: Some(s.from.clone()),
        created_at: Utc::now(),
    };
    let db = transaction_service::durable_database(&state.db);
    let admitted=transaction_service::run_db_only(&db,&(&db,&v,&sms,&s,&row,&receipt,&managed,chat,state.audit_chain_hmac_key.as_ref().as_ref(),&requests),|tx,c|Box::pin(async move {
        let (db,v,sms,s,row,receipt,managed,chat,key,requests)=*c;
        super::super::owner_admission::fence(db,tx,&s.user_id).await?;
        if let Some(a)=managed{admission::fence(db,tx,a,Utc::now()).await?;if !a.claim.caller_confirmations || a.claim.voice_binding_id.as_deref()!=Some(&v.id) || a.claim.sms_binding_id.as_deref()!=Some(&sms.id){return Err(unavailable());}}
        for b in [v,sms] {if db.collection::<Binding>(BINDINGS).update_one(doc!{"_id":&b.id,"enabled":true,"revision":b.revision,"verified_credential_identity":&b.verified_credential_identity},doc!{"$inc":{"admission_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}}
        if managed.is_none() && db.collection::<CallerConfirmationBinding>(CallerConfirmationBinding::COLLECTION_NAME).update_one(doc!{"voice_binding_id":&v.id,"voice_binding_revision":v.revision,"sms_binding_id":&sms.id,"sms_binding_revision":sms.revision,"enabled":true},doc!{"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        let now=bson::DateTime::now();
        if db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"generation":s.generation,"claim_id":&s.claim_id,"state":"active","active_turn_id":&chat.turn_id,"lease_expires_at":{"$gt":now},"expires_at":{"$gt":now},"$expr":{"$lt":[{"$ifNull":["$confirmation_count",0]},2]}},doc!{"$inc":{"confirmation_count":1_i64,"egress_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("This call has used its two confirmation slots"));}
        db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(doc!{"_id":&chat.conversation_id,"credential_api_key_id":&chat.api_key_id,"active_turn.turn_id":&chat.turn_id,"active_turn.stop_requested":{"$ne":true}},doc!{"$inc":{"channel_confirmation_fence":1_i64}}).session(&mut *tx).await?.matched_count.eq(&1).then_some(()).ok_or(AppError::AssistantTurnRequired)?;
        // Admission binds its still-reserved parts; they are never left unforwarded.
        crate::services::billing::meter::forward_in_session(db,tx,requests).await?;
        db.collection::<ManagedCallConfirmation>(ManagedCallConfirmation::COLLECTION_NAME).insert_one(row).session(&mut *tx).await?;
        db.collection::<EventReceipt>(EVENTS).insert_one(receipt).session(&mut *tx).await?;
        if let Some(a)=managed {super::sms::hold_send(db,tx,a,s,receipt).await?;super::costs::register(db,tx,&a.line,&row.id,EffectKind::Confirmation,row.admitted.as_ref().ok_or_else(unavailable)?,row.created_at).await?;}
        audit_service::log_actor_event_in_session(db,tx,key,&AuditActor{user_id:s.user_id.clone(),api_key_id:Some(chat.api_key_id.clone()),api_key_name:None,ip_address:None,user_agent:None},"call_confirmation_admitted",json!({"operation_id":row.id,"session_id":s.id,"voice_binding_id":v.id,"sms_binding_id":sms.id,"ordinal":row.ordinal,"model_asserted_caller_consent":row.model_asserted_caller_consent})).await?;Ok(())
    })).await;
    if let Err(e) = admitted {
        // Resolve unknown commits before any reservation is released. An
        // unresolved lookup leaves the unadmitted parts to the managed sweep.
        if operations.find_one(doc! {"_id":&row.id}).await?.is_none() {
            super::super::billing::cancel(state, &meters, "confirmation_not_admitted").await;
        }
        return Err(e);
    }
    let preflight = async {
        let (live_v, live_s, live_sms) = turn_fence(state, chat).await?;
        if live_v.revision != row.voice_binding_revision
            || live_s.generation != row.session_generation
            || live_sms.id != row.sms_binding_id
            || live_sms.revision != row.sms_binding_revision
        {
            return Err(unavailable());
        }
        let live = binding::live(state, &live_sms).await?;
        if live.identity != r.identity {
            return Err(unavailable());
        }
        binding::authorize(state, &sms, &live, "POST", &path, Some(&payload)).await?;
        Ok(live)
    }
    .await;
    let live = match preflight {
        Ok(r) => r,
        Err(e) => {
            super::super::billing::cancel(state, &meters, "confirmation_failed_before_dispatch")
                .await;
            operations
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"status":"failed_before_dispatch"}},
                )
                .await?;
            state
                .db
                .collection::<EventReceipt>(EVENTS)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"status":"rejected","observed_parts":0_i64}},
                )
                .await?;
            super::super::sms_delivery::reconcile(state, &row.id).await?;
            return Err(e);
        }
    };
    // Persist sending on both records before a byte may leave NyxID. No retry
    // path in this module dispatches an already-existing operation.
    if operations
        .update_one(
            doc! {"_id":&row.id,"status":"prepared"},
            doc! {"$set":{"status":"sending"}},
        )
        .await?
        .matched_count
        != 1
    {
        return Err(conflict("Confirmation dispatch authority changed"));
    }
    state
        .db
        .collection::<EventReceipt>(EVENTS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":"dispatching","dispatched_at":bson::DateTime::now()}},
        )
        .await?;
    let final_check = async {
        let (v2, s2, sms2) = turn_fence(state, chat).await?;
        if v2.revision != row.voice_binding_revision
            || s2.generation != row.session_generation
            || sms2.id != row.sms_binding_id
            || sms2.revision != row.sms_binding_revision
            || sms2.verified_credential_identity != sms.verified_credential_identity
        {
            return Err(unavailable());
        }
        Ok(())
    }
    .await;
    if let Err(error) = final_check {
        super::super::billing::cancel(state, &meters, "confirmation_failed_before_dispatch").await;
        operations
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"status":"failed_before_dispatch"}},
            )
            .await?;
        state
            .db
            .collection::<EventReceipt>(EVENTS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"status":"rejected","observed_parts":0_i64}},
            )
            .await?;
        super::super::sms_delivery::reconcile(state, &row.id).await?;
        return Err(error);
    }
    let profiles::Adapter::Messaging(a) = profiles::Adapter::new(sms.profile) else {
        return Err(unavailable());
    };
    let outcome = a
        .send(
            crate::services::proxy_service::no_redirect_http_client(),
            &sms,
            &live.key,
            &s.from,
            &i.text,
            &callback,
        )
        .await;
    let status = match outcome {
        Ok(accepted) => {
            state.db.collection::<EventReceipt>(EVENTS).update_one(doc!{"_id":&row.id},doc!{"$set":{"status":"accepted","delivery":"accepted","provider_message_id":accepted.id,"observed_parts":accepted.parts}}).await?;
            ConfirmationState::Sent
        }
        Err(profiles::SmsFailure::Rejected(_)) => {
            state
                .db
                .collection::<EventReceipt>(EVENTS)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"status":"rejected","delivery":"failed","observed_parts":0_i64}},
                )
                .await?;
            ConfirmationState::FailedBeforeDispatch
        }
        Err(profiles::SmsFailure::Uncertain) => {
            state
                .db
                .collection::<EventReceipt>(EVENTS)
                .update_one(doc! {"_id":&row.id}, doc! {"$set":{"status":"uncertain"}})
                .await?;
            ConfirmationState::Uncertain
        }
    };
    operations
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":bson::to_bson(&status).map_err(|_|unavailable())?}},
        )
        .await?;
    if super::super::sms_delivery::reconcile(state, &row.id)
        .await
        .is_err()
    {
        tracing::warn!(operation_id=%row.id,"Caller confirmation settlement pending");
    }
    Ok(json!({"operation_id":row.id,"status":status,"replayed":false}))
}

async fn disable(state: &AppState, actor: &AuditActor, v: &Binding) -> AppResult<()> {
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,actor,v,state.audit_chain_hmac_key.as_ref().as_ref()),|tx,c|Box::pin(async move {
        let(db,actor,v,key)=*c;
        super::super::owner_admission::fence(db,tx,&v.user_id).await?;
        if db.collection::<Binding>(BINDINGS).update_one(doc!{"_id":&v.id,"user_id":&actor.user_id,"revision":v.revision},doc!{"$inc":{"admission_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        if let CredentialBinding::ManagedLine{managed_claim_id,..}=&v.credential {
            db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":managed_claim_id,"user_id":&actor.user_id,"voice_binding_id":&v.id},doc!{"$set":{"caller_confirmations":false},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        }
        db.collection::<CallerConfirmationBinding>(CallerConfirmationBinding::COLLECTION_NAME).update_one(doc!{"voice_binding_id":&v.id},doc!{"$set":{"enabled":false},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"caller_confirmations_configured",json!({"voice_binding_id":v.id,"enabled":false})).await?;Ok(())
    })).await
}
/// A prepared operation has never acquired its dispatch barrier. Claiming it
/// here fences the original writer before releasing proven-unsent funding.
pub async fn sweep(state: &AppState) -> AppResult<()> {
    use futures::TryStreamExt;
    let operations = state
        .db
        .collection::<ManagedCallConfirmation>(ManagedCallConfirmation::COLLECTION_NAME);
    let rows:Vec<_>=operations.find(doc!{"status":"prepared","created_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(5))}}).limit(50).await?.try_collect().await?;
    for r in rows {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        if operations
            .update_one(
                doc! {"_id":&r.id,"status":"prepared"},
                doc! {"$set":{"status":"failed_before_dispatch"}},
            )
            .await?
            .matched_count
            != 1
        {
            continue;
        }
        state
            .db
            .collection::<EventReceipt>(EVENTS)
            .update_one(
                doc! {"_id":&r.id,"status":"prepared"},
                doc! {"$set":{"status":"rejected","observed_parts":0_i64}},
            )
            .await?;
        if super::super::sms_delivery::reconcile(state, &r.id)
            .await
            .is_err()
        {
            tracing::warn!(operation_id=%r.id,"Prepared caller confirmation recovery pending");
        }
    }
    Ok(())
}
