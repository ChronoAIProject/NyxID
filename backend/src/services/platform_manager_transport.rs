//! Real personal-manager composition and native webhook handoff.
use super::{
    platform_manager_command_service as commands, platform_manager_contract::*,
    platform_manager_delivery as delivery, platform_manager_dispatch_service as dispatch,
    platform_personal_binding_service as bindings,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        channel_bot::ChannelBot,
        platform_bot_config::{COLLECTION_NAME as PROFILES, PlatformBotConfig},
        platform_manager_ingress::{COLLECTION_NAME as INGRESS, PlatformManagerIngress},
    },
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use std::sync::Arc;

struct Transport {
    state: AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
}
#[async_trait]
impl ManagerTransportProducer for Transport {
    async fn admit_manager_update(
        &self,
        db: &Database,
        update: &VerifiedManagerUpdate,
    ) -> AppResult<IngressAdmission> {
        dispatch::admit_manager_update(db, self.identity.as_ref(), update).await
    }
    async fn deliver_platform_reply(
        &self,
        _db: &Database,
        destination: &ReplyDestination,
        reference: &DeliveryRef,
        content: ReplyContent,
    ) -> AppResult<DeliveryOutcome> {
        delivery::deliver(
            &self.state,
            self.identity.clone(),
            destination,
            reference,
            content,
            &super::telegram_new_api::TelegramApi::new(&self.state.http_client),
        )
        .await
    }
}

pub fn compose(state: AppState) -> Arc<PlatformManagerProducers> {
    let identity = bindings::producer(state.clone());
    let egress = super::assistant_nyxagent_egress::producer(state.clone(), identity);
    let public_runtime = super::platform_public_credential_service::producer(state.clone());
    let execution = super::assistant_nyxagent_runtime::producer(
        state.clone(),
        egress.clone(),
        public_runtime.clone(),
    );
    assemble(state, execution, egress, public_runtime)
}

/// Every dependency is real. The public producers remain explicitly unavailable.
pub fn assemble(
    state: AppState,
    execution: Arc<dyn NyxAgentExecutionProducer>,
    egress: Arc<dyn NyxAgentEgress>,
    public_runtime: Arc<dyn PublicNyxAgentRuntime>,
) -> Arc<PlatformManagerProducers> {
    let identity = bindings::producer(state.clone());
    let ingress = super::platform_manager_ingress_service::producer(state.clone());
    let personal = super::platform_personal_turn_service::producer(
        state.clone(),
        identity.clone(),
        ingress.clone(),
        egress.clone(),
    );
    let public = super::platform_public_agent_service::producer(
        state.clone(),
        execution.clone(),
        ingress.clone(),
    );
    let lifecycle = super::platform_bot_runtime_control_service::producer(
        state.clone(),
        personal.clone(),
        public.clone(),
    );
    let transport = Arc::new(Transport {
        state,
        identity: identity.clone(),
    });
    Arc::new(PlatformManagerProducers {
        identity,
        ingress,
        personal,
        public,
        lifecycle,
        transport,
        execution,
        egress,
        public_runtime,
    })
}

fn fixed_destination(
    manager: ManagerAuthority,
    update: &VerifiedManagerUpdate,
    claim: IngressClaim,
) -> ReplyDestination {
    ReplyDestination {
        manager,
        sender_id: update.sender_id,
        chat_id: update.chat_id,
        authority: ReplyAuthority::Fixed { ingress: claim },
        expires_at: Utc::now() + Duration::minutes(2),
    }
}

/// true means this profile is opted in and the update must never reach old guide/
/// owner-channel execution, including malformed, paused and infrastructure cases.
pub(crate) async fn dispatch_update(
    state: &AppState,
    producers: Option<Arc<PlatformManagerProducers>>,
    bot: &ChannelBot,
    update: &serde_json::Value,
    service: &super::telegram_new_service::TelegramNewService<'_>,
) -> AppResult<bool> {
    let profile = state
        .db
        .collection::<PlatformBotConfig>(PROFILES)
        .find_one(doc! {"manager_bot_id":&bot.platform_bot_id})
        .await?;
    let Some(profile) = profile else {
        return Ok(false);
    };
    let Some(runtime) =
        super::platform_bot_runtime_control_service::get(&state.db, &profile.id).await?
    else {
        return Ok(false);
    };
    // Persisted controls are the explicit opt-in marker, even when paused/retired.
    let producers =
        producers.ok_or_else(|| AppError::Conflict("Manager runtime is unavailable".into()))?;
    let (numeric_id, username, values) = service.manager().await?;
    let observation = values
        .get("observation_id")
        .ok_or_else(|| AppError::Conflict("Manager observation is unavailable".into()))?;
    let Some(verified) = dispatch::verified_message(
        &profile.id,
        &bot.platform_bot_id,
        numeric_id,
        observation,
        update,
    ) else {
        return Ok(true);
    };
    let text = update["message"]["text"].as_str().unwrap_or("");
    let verb = text.split_whitespace().next().unwrap_or("");
    let base = verb.split('@').next().unwrap_or("");
    // Legacy setup commands retain their purpose, never becoming personal turns.
    if matches!(base, "/onboarding" | "/setups")
        || (base == "/confirm" && commands::personal_command(text, &username).is_none())
    {
        if verified.command_eligibility != ManagerCommandEligibility::OriginalPrivateMessage {
            return Ok(true);
        }
        return Ok(false); // The HTTP adapter retains legacy setup command handling.
    }
    if let Some(kind) = commands::personal_command(text, &username) {
        if let Some(command) =
            OriginalManagerCommand::from_verified(verified.clone(), TurnInput { text: text.into() })
        {
            if let Some(output) = commands::execute(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                state.audit_chain_hmac_key.as_slice(),
                &command,
                kind,
                &bindings::LifecycleProducer {
                    personal: producers.personal.clone(),
                    public: producers.public.clone(),
                },
            )
            .await?
            {
                let _ = delivery::deliver_command(
                    state,
                    producers.identity.clone(),
                    output,
                    &service.api,
                )
                .await?;
            }
            return Ok(true);
        }
        // Forwarded proof/control is consumed here, never sent to a model.
        refuse_control(state, &producers, &verified).await?;
        return Ok(true);
    }
    if matches!(base, "/link" | "/unlink" | "/confirm")
        || text.contains("nyx_pli_")
        || text.contains("#intent_token=")
    {
        refuse_control(state, &producers, &verified).await?;
        return Ok(true);
    }
    let admission = producers
        .transport
        .admit_manager_update(&state.db, &verified)
        .await?;
    let IngressAdmission::Claimed { ingress, audience } = admission else {
        return Ok(true);
    };
    match audience {
        ManagerAudience::Personal(admitted) => {
            if text.trim().is_empty() || text.starts_with('/') {
                let destination = fixed_destination(
                    super::platform_bot_runtime_control_service::policy(&runtime).manager,
                    &verified,
                    ingress.clone(),
                );
                let _ = producers
                    .transport
                    .deliver_platform_reply(
                        &state.db,
                        &destination,
                        &DeliveryRef::Fixed { ingress },
                        ReplyContent::Fixed(UnavailableReason::EngineUnavailable),
                    )
                    .await?;
                return Ok(true);
            }
            if verified.command_eligibility == ManagerCommandEligibility::OriginalPrivateMessage
                && super::assistant_acknowledgement_service::parse_reply(text).is_some()
            {
                // Read the exact binding's existing thread, never an owner's home.
                let mut session = state.db.client().start_session().await?;
                if let SenderResolution::Personal {
                    conversation_id: Some(id),
                    ..
                } = producers
                    .identity
                    .resolve_sender(&state.db, &mut session, &verified)
                    .await?
                    && let Some(command) = OriginalManagerCommand::from_verified(
                        verified.clone(),
                        TurnInput { text: text.into() },
                    )
                {
                    match producers
                        .personal
                        .decide_platform_reply(&state.db, &admitted, &id, command)
                        .await?
                    {
                        PlatformDecisionOutcome::Decided(_) => return Ok(true),
                        PlatformDecisionOutcome::Unavailable(_) => return Ok(true),
                        PlatformDecisionOutcome::NoPendingDecision => {}
                    }
                }
            }
            let (events, receiver) = tokio::sync::mpsc::channel(16);
            match producers
                .personal
                .start_personal_turn(
                    &state.db,
                    &admitted,
                    TurnInput { text: text.into() },
                    events,
                )
                .await?
            {
                PersonalTurnStart::Started { destination, .. } => {
                    let state = state.clone();
                    tokio::spawn(async move {
                        consume_events(state, producers, destination, receiver).await;
                    });
                }
                PersonalTurnStart::Unavailable(reason) => {
                    let destination =
                        fixed_destination(admitted.authority.manager, &verified, ingress.clone());
                    let _ = producers
                        .transport
                        .deliver_platform_reply(
                            &state.db,
                            &destination,
                            &DeliveryRef::Fixed { ingress },
                            ReplyContent::Fixed(reason),
                        )
                        .await?;
                }
            }
        }
        ManagerAudience::Fixed(reason) => {
            let destination = fixed_destination(
                super::platform_bot_runtime_control_service::policy(&runtime).manager,
                &verified,
                ingress.clone(),
            );
            let _ = producers
                .transport
                .deliver_platform_reply(
                    &state.db,
                    &destination,
                    &DeliveryRef::Fixed { ingress },
                    ReplyContent::Fixed(reason),
                )
                .await?;
        }
        ManagerAudience::PrivateChatHandoff => {
            let destination = fixed_destination(
                super::platform_bot_runtime_control_service::policy(&runtime).manager,
                &verified,
                ingress.clone(),
            );
            let _ = producers
                .transport
                .deliver_platform_reply(
                    &state.db,
                    &destination,
                    &DeliveryRef::Fixed { ingress },
                    ReplyContent::PrivateChatHandoff,
                )
                .await?;
        }
        ManagerAudience::Public(_) => {
            // No public runtime proof exists. Never enter owner preparation.
            let destination = fixed_destination(
                super::platform_bot_runtime_control_service::policy(&runtime).manager,
                &verified,
                ingress.clone(),
            );
            let _ = producers
                .transport
                .deliver_platform_reply(
                    &state.db,
                    &destination,
                    &DeliveryRef::Fixed { ingress },
                    ReplyContent::Fixed(UnavailableReason::PublicNotReady),
                )
                .await?;
        }
        ManagerAudience::Control => {
            return Err(AppError::Conflict("Control receipt cannot execute".into()));
        }
    }
    Ok(true)
}

async fn refuse_control(
    state: &AppState,
    producers: &Arc<PlatformManagerProducers>,
    update: &VerifiedManagerUpdate,
) -> AppResult<()> {
    if let IngressAdmission::Claimed { ingress, .. } = producers
        .transport
        .admit_manager_update(&state.db, update)
        .await?
        && let Some(runtime) = producers
            .identity
            .runtime_policy(&state.db, &update.profile_id)
            .await?
    {
        let destination = fixed_destination(runtime.manager, update, ingress.clone());
        let _ = producers
            .transport
            .deliver_platform_reply(
                &state.db,
                &destination,
                &DeliveryRef::Fixed { ingress },
                ReplyContent::Fixed(UnavailableReason::ModeDisabled),
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn consume_events(
    state: AppState,
    producers: Arc<PlatformManagerProducers>,
    destination: ReplyDestination,
    mut events: tokio::sync::mpsc::Receiver<ExecutionEvent>,
) {
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_secs(1800), events.recv()).await
    {
        match event {
            ExecutionEvent::ReplyReady(reply) => {
                if let Err(error) = deliver_saved(&state, &producers, &destination, &reply).await {
                    tracing::warn!(
                        error_code = error.error_code(),
                        "Personal manager saved reply delivery deferred or refused"
                    );
                }
            }
            ExecutionEvent::Settled { .. } => break,
        }
    }
}

pub(crate) async fn deliver_saved(
    state: &AppState,
    producers: &PlatformManagerProducers,
    destination: &ReplyDestination,
    reply: &ReplyRef,
) -> AppResult<DeliveryOutcome> {
    let ReplyAuthority::Personal { origin, turn } = &destination.authority else {
        return Err(AppError::Forbidden(
            "Personal reply authority required".into(),
        ));
    };
    if *turn != reply.turn {
        return Err(AppError::Conflict("Reply turn changed".into()));
    }
    let saved=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {"_id":&reply.reply_id,"user_id":&origin.authority.user_id,"conversation_id":&reply.turn.conversation_id,"turn_id":&reply.turn.turn_id,"role":"assistant"}).await?
        .ok_or_else(||AppError::NotFound("Retained reply unavailable".into()))?;
    producers
        .transport
        .deliver_platform_reply(
            &state.db,
            destination,
            &DeliveryRef::Reply(reply.clone()),
            ReplyContent::Text(delivery::render_saved(&saved, &state.config.frontend_url)),
        )
        .await
}

pub fn spawn_sweeps(state: AppState, producers: Arc<PlatformManagerProducers>) {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(std::time::Duration::from_secs(15));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            if let Err(error) = producers
                .lifecycle
                .recover_personal_lifecycle(&state.db, producers.personal.as_ref(), 100)
                .await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Personal manager lifecycle recovery failed"
                );
            }
            if let Err(error) =
                super::platform_public_agent_service::sweep_public_lifecycle(&state, 100).await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Public manager lifecycle recovery failed"
                );
            }
            if let Err(error) = recover_ingress(&state, &producers, 100).await {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Manager ingress recovery failed"
                );
            }
            if let Err(error) = delivery::sweep(&state, producers.identity.clone(), 100).await {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Manager delivery recovery failed"
                );
            }
        }
    });
}

async fn recover_ingress(
    state: &AppState,
    producers: &PlatformManagerProducers,
    limit: u32,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let mut receipts=state.db.collection::<PlatformManagerIngress>(INGRESS).find(doc! {"phase":"settled","reply_id":{"$type":"string"},"settled_at":{"$gt":bson::DateTime::from_chrono(Utc::now()-Duration::hours(1))}}).sort(doc! {"settled_at":-1}).limit(i64::from(limit.min(100))).await?;
    while let Some(receipt) = receipts.try_next().await? {
        let Some(reply_id) = receipt.reply_id else {
            continue;
        };
        let Some(message) = state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"_id":&reply_id})
            .await?
        else {
            continue;
        };
        let Some(origin) = message.platform_origin.as_ref() else {
            continue;
        };
        let origin = super::platform_personal_turn_service::contract_origin(origin)?;
        let turn = TurnRef {
            conversation_id: message.conversation_id,
            turn_id: message.turn_id,
        };
        let destination = ReplyDestination {
            manager: origin.authority.manager.clone(),
            sender_id: origin.authority.sender_id,
            chat_id: origin.authority.chat_id,
            authority: ReplyAuthority::Personal {
                origin,
                turn: turn.clone(),
            },
            expires_at: receipt.admitted_at + Duration::minutes(30),
        };
        let reply = ReplyRef { turn, reply_id };
        // Refusals are recorded by delivery and do not stop other owners' recovery.
        let _ = deliver_saved(state, producers, &destination, &reply).await?;
    }
    state.db.collection::<PlatformManagerIngress>(INGRESS).update_many(doc! {"phase":"admitted","start_before":{"$lte":bson::DateTime::now()}},doc! {"$set":{"phase":"interrupted","interrupted_reason":"admission_expired","settled_at":bson::DateTime::now()}}).await?;
    Ok(())
}
