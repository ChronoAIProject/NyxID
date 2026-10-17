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
    public: Arc<dyn PublicTurnProducer>,
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
        if matches!(destination.authority, ReplyAuthority::Public { .. }) {
            let DeliveryRef::Reply(reply) = reference else {
                return Err(AppError::Forbidden(
                    "Public reply reference required".into(),
                ));
            };
            // Ignore caller body; the dedicated encrypted transcript is authoritative.
            return delivery::deliver_public_reply(
                &self.state,
                self.identity.clone(),
                self.public.clone(),
                destination,
                reply,
                &super::telegram_new_api::TelegramApi::new(&self.state.http_client),
            )
            .await;
        }
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

pub fn compose(
    state: AppState,
    settled_hook: Arc<dyn super::assistant_nyxagent_runtime::OwnerTurnSettledHook>,
) -> Arc<PlatformManagerProducers> {
    let identity = bindings::producer(state.clone());
    let egress = super::assistant_nyxagent_egress::producer(state.clone(), identity);
    let public_runtime = super::platform_public_credential_service::producer(state.clone());
    let execution =
        super::assistant_nyxagent_runtime::producer(state.clone(), egress.clone(), public_runtime);
    assemble(state, execution, egress, settled_hook)
}

/// Every dependency is real. Public execution requires persisted activated deployment proof.
pub fn assemble(
    state: AppState,
    execution: Arc<dyn NyxAgentExecutionProducer>,
    egress: Arc<dyn NyxAgentEgress>,
    settled_hook: Arc<dyn super::assistant_nyxagent_runtime::OwnerTurnSettledHook>,
) -> Arc<PlatformManagerProducers> {
    let identity = bindings::producer(state.clone());
    let ingress = super::platform_manager_ingress_service::producer(state.clone());
    let personal = super::platform_personal_turn_service::producer(
        state.clone(),
        identity.clone(),
        ingress.clone(),
        egress,
        settled_hook,
    );
    let public = super::platform_public_agent_service::producer(state.clone(), execution, ingress);
    let lifecycle = super::platform_bot_runtime_control_service::producer(
        state.clone(),
        personal.clone(),
        public.clone(),
    );
    let transport = Arc::new(Transport {
        state,
        identity: identity.clone(),
        public: public.clone(),
    });
    Arc::new(PlatformManagerProducers {
        identity,
        personal,
        public,
        lifecycle,
        transport,
    })
}

fn fixed_destination(
    manager: ManagerAuthority,
    update: &VerifiedManagerUpdate,
    claim: IngressClaim,
) -> ReplyDestination {
    ReplyDestination {
        manager,
        sender: update.sender.clone(),
        authority: ReplyAuthority::Fixed { ingress: claim },
        expires_at: Utc::now() + Duration::minutes(2),
    }
}

/// Explicit adapter handoff; an opted-in entry never reaches ordinary owner routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagerDispatch {
    NotOptedIn,
    Handled,
    LegacyConfirmation,
}
pub(crate) async fn dispatch_update(
    state: &AppState,
    producers: Option<Arc<PlatformManagerProducers>>,
    bot: &ChannelBot,
    update: &serde_json::Value,
    service: &super::telegram_new_service::TelegramNewService<'_>,
) -> AppResult<ManagerDispatch> {
    let profile = state
        .db
        .collection::<PlatformBotConfig>(PROFILES)
        .find_one(doc! {"manager_bot_id":&bot.platform_bot_id,"native_entry":bson::Bson::Null})
        .await?;
    let Some(profile) = profile else {
        return Ok(ManagerDispatch::NotOptedIn);
    };
    let Some(runtime) =
        super::platform_bot_runtime_control_service::get(&state.db, &profile.id).await?
    else {
        return Ok(ManagerDispatch::NotOptedIn);
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
        return Ok(ManagerDispatch::Handled);
    };
    let text = update["message"]["text"].as_str().unwrap_or("");
    dispatch_verified(
        state,
        producers,
        runtime,
        verified,
        text,
        &username,
        &service.api,
    )
    .await
}

/// Both entry transports provide an admission constructed from authenticated
/// native content. Its canonical text is fixed before any control or model work.
pub(crate) fn dispatch_native_admission(
    state: &AppState,
    producers: Option<Arc<PlatformManagerProducers>>,
    admission: super::platform_entry_receipts::NativeAdmission,
) -> futures::future::BoxFuture<'_, AppResult<()>> {
    // Direct webhooks and gateway Responses share this native-only boundary.
    // Construct the large dispatch future outside their caller poll frames.
    Box::pin(dispatch_native_admission_impl(state, producers, admission))
}

async fn dispatch_native_admission_impl(
    state: &AppState,
    producers: Option<Arc<PlatformManagerProducers>>,
    admission: super::platform_entry_receipts::NativeAdmission,
) -> AppResult<()> {
    let producers =
        producers.ok_or_else(|| AppError::Conflict("Manager runtime is unavailable".into()))?;
    let verified = admission.verified_update();
    let runtime = super::platform_bot_runtime_control_service::get(&state.db, &verified.profile_id)
        .await?
        .ok_or_else(|| AppError::Conflict("Manager runtime is unavailable".into()))?;
    let manager = super::platform_bot_runtime_control_service::policy(&runtime).manager;
    if manager.entry != verified.entry
        || manager.manager_bot_id != verified.manager_bot_id
        || manager.manager_authority_epoch != admission.receipt().manager_authority_epoch
    {
        return Err(AppError::Conflict(
            "Manager native authority changed".into(),
        ));
    }
    let outcome = dispatch_verified(
        state,
        producers,
        runtime,
        verified,
        admission.text(),
        "",
        &super::telegram_new_api::TelegramApi::new(&state.http_client),
    )
    .await?;
    if outcome != ManagerDispatch::Handled {
        return Err(AppError::Conflict(
            "Manager native dispatch is unavailable".into(),
        ));
    }
    Ok(())
}

fn dispatch_verified<'a>(
    state: &'a AppState,
    producers: Arc<PlatformManagerProducers>,
    runtime: crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls,
    verified: VerifiedManagerUpdate,
    text: &'a str,
    username: &'a str,
    api: &'a super::telegram_new_api::TelegramApi<'_>,
) -> futures::future::BoxFuture<'a, AppResult<ManagerDispatch>> {
    Box::pin(dispatch_verified_impl(
        state, producers, runtime, verified, text, username, api,
    ))
}

async fn dispatch_verified_impl(
    state: &AppState,
    producers: Arc<PlatformManagerProducers>,
    runtime: crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls,
    verified: VerifiedManagerUpdate,
    text: &str,
    username: &str,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<ManagerDispatch> {
    let verb = text.split_whitespace().next().unwrap_or("");
    let base = verb.split('@').next().unwrap_or("");
    let kind = super::platform_manager_management::parse(text, username)
        .map(commands::PersonalCommandKind::Management)
        .or_else(|| commands::personal_command(text, username));
    if let Some(kind) = kind {
        if let Some(command) =
            OriginalManagerCommand::from_verified(verified.clone(), TurnInput { text: text.into() })
        {
            let result = commands::execute(
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
            .await;
            let output = match result {
                Ok(output) => output,
                Err(error) if expected_failure(&error).is_some() => {
                    tracing::info!(
                        error_code = error.error_code(),
                        "Personal manager control refused"
                    );
                    refuse_control(state, &producers, &verified).await?;
                    return Ok(ManagerDispatch::Handled);
                }
                Err(error) => return Err(error),
            };
            if let Some(output) = output {
                let _ = delivery::deliver_command(state, producers.identity.clone(), output, api)
                    .await?;
            }
            return Ok(ManagerDispatch::Handled);
        }
        // Forwarded proof/control is consumed here, never sent to a model.
        refuse_control(state, &producers, &verified).await?;
        return Ok(ManagerDispatch::Handled);
    }
    // An upgrade must still finish an already-issued onboarding-only proof.
    // This explicit handoff never selects a personal account or ordinary route.
    if matches!(verified.entry, EntryIdentity::TelegramV1 { .. })
        && legacy_confirmation(text, username)
        && OriginalManagerCommand::from_verified(verified.clone(), TurnInput { text: text.into() })
            .is_some()
    {
        return Ok(ManagerDispatch::LegacyConfirmation);
    }
    if matches!(
        base,
        "/link"
            | "/login"
            | "/unlink"
            | "/logout"
            | "/status"
            | "/help"
            | "/confirm"
            | "/onboarding"
            | "/setups"
            | "/setup"
            | "/bots"
            | "/bot"
    ) || text.contains("nyx_pli_")
        || text.contains("#intent_token=")
    {
        refuse_control(state, &producers, &verified).await?;
        return Ok(ManagerDispatch::Handled);
    }
    let admission = producers
        .transport
        .admit_manager_update(&state.db, &verified)
        .await?;
    let (ingress, audience) = match admission {
        IngressAdmission::Claimed { ingress, audience } => (ingress, audience),
        IngressAdmission::Duplicate { receipt_id } => {
            tracing::debug!(
                receipt_id,
                "Manager update already admitted; preserving original audience"
            );
            return Ok(ManagerDispatch::Handled);
        }
    };
    match audience {
        ManagerAudience::Personal(admitted) => {
            let live = producers
                .identity
                .runtime_policy(&state.db, &verified.profile_id)
                .await?;
            let unavailable = match live {
                None => Some(UnavailableReason::ManagerUnavailable),
                Some(policy) if policy.emergency_stopped => {
                    Some(UnavailableReason::EmergencyStopped)
                }
                Some(policy) if !policy.personal_enabled => {
                    Some(UnavailableReason::PersonalDisabled)
                }
                Some(policy)
                    if policy.manager != admitted.authority.manager
                        || policy.personal_control_revision
                            != admitted.authority.personal_control_revision =>
                {
                    Some(UnavailableReason::OriginRevoked)
                }
                Some(_) => None,
            };
            if let Some(reason) = unavailable {
                send_fixed(
                    state,
                    &producers,
                    &verified,
                    &ingress,
                    admitted.authority.manager,
                    reason,
                )
                .await?;
                return Ok(ManagerDispatch::Handled);
            }
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
                return Ok(ManagerDispatch::Handled);
            }
            if verified.command_eligibility == ManagerCommandEligibility::OriginalPrivateMessage
                && super::assistant_acknowledgement_service::parse_reply(text).is_some()
            {
                // Read the exact binding's existing thread, never an owner's home.
                let mut session = state.db.client().start_session().await?;
                if let SenderResolution::Personal {
                    authority,
                    nyxbot_agent_id: Some(agent_id),
                    conversation_id: Some(id),
                } = producers
                    .identity
                    .resolve_sender(&state.db, &mut session, &verified)
                    .await?
                    && let Some(command) = OriginalManagerCommand::from_verified(
                        verified.clone(),
                        TurnInput { text: text.into() },
                    )
                {
                    let thread_matches = state
                        .db
                        .collection::<crate::models::assistant_conversation::AssistantConversation>(
                            crate::models::assistant_conversation::COLLECTION_NAME,
                        )
                        .find_one(doc! {
                            "_id": &id,
                            "user_id": &admitted.authority.user_id,
                            "platform_thread.nyxbot_agent_id": &agent_id,
                            "platform_thread.binding_id": &admitted.authority.binding_id,
                            "platform_thread.binding_generation": admitted.authority.binding_generation,
                        })
                        .session(&mut session)
                        .await?
                        .is_some();
                    if *authority != admitted.authority || !thread_matches {
                        send_fixed(
                            state,
                            &producers,
                            &verified,
                            &ingress,
                            admitted.authority.manager,
                            UnavailableReason::OriginRevoked,
                        )
                        .await?;
                        return Ok(ManagerDispatch::Handled);
                    }
                    match producers
                        .personal
                        .decide_platform_reply(&state.db, &admitted, &id, command)
                        .await?
                    {
                        PlatformDecisionOutcome::Decided(decision) => {
                            if decision.requested_turn.conversation_id != id
                                || decision.acknowledgement_id.is_empty()
                            {
                                return Err(AppError::Conflict(
                                    "Personal decision thread changed".into(),
                                ));
                            }
                            tracing::info!(
                                acknowledgement_id = decision.acknowledgement_id,
                                allowed = decision.allowed,
                                "Personal manager card decision committed"
                            );
                            return Ok(ManagerDispatch::Handled);
                        }
                        PlatformDecisionOutcome::Unavailable(reason) => {
                            send_fixed(
                                state,
                                &producers,
                                &verified,
                                &ingress,
                                admitted.authority.manager,
                                reason,
                            )
                            .await?;
                            return Ok(ManagerDispatch::Handled);
                        }
                        PlatformDecisionOutcome::NoPendingDecision => {}
                    }
                }
            }
            let (events, receiver) = tokio::sync::mpsc::channel(16);
            let started = producers
                .personal
                .start_personal_turn(
                    &state.db,
                    &admitted,
                    TurnInput { text: text.into() },
                    events,
                )
                .await;
            let started = match started {
                Ok(started) => started,
                Err(error) => {
                    let Some(reason) = expected_failure(&error) else {
                        return Err(error);
                    };
                    tracing::info!(
                        error_code = error.error_code(),
                        "Personal manager turn admission refused"
                    );
                    PersonalTurnStart::Unavailable(reason)
                }
            };
            match started {
                PersonalTurnStart::Started { turn, destination } => {
                    if !matches!(&destination.authority,ReplyAuthority::Personal {turn:expected,..}|ReplyAuthority::Public {turn:expected,..} if *expected==turn)
                    {
                        return Err(AppError::Conflict("Personal delivery turn changed".into()));
                    }
                    let state = state.clone();
                    crate::services::platform_native_reply_service::spawn_in_native_scope(
                        async move {
                            consume_events(state, producers, destination, receiver).await;
                        },
                    );
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
        ManagerAudience::Public(admitted) => {
            let context = match producers
                .public
                .prepare_public_context(&state.db, &admitted)
                .await
            {
                Ok(context) => context,
                Err(error) => {
                    let Some(reason) = public_failure(&error) else {
                        return Err(error);
                    };
                    send_fixed(
                        state,
                        &producers,
                        &verified,
                        &ingress,
                        admitted.manager,
                        reason,
                    )
                    .await?;
                    return Ok(ManagerDispatch::Handled);
                }
            };
            let mut session = state.db.client().start_session().await?;
            let fenced = session
                .start_transaction()
                .and_run(
                    (&producers.public, &context),
                    |session, (public, context)| {
                        Box::pin(async move {
                            super::api_key_mutation_service::transaction_result(
                                public
                                    .fence_public_execution(&state.db, session, context)
                                    .await,
                            )
                        })
                    },
                )
                .await
                .map_err(super::api_key_mutation_service::map_transaction_error)?;
            if let FenceOutcome::Unavailable(reason) = fenced {
                send_fixed(
                    state,
                    &producers,
                    &verified,
                    &ingress,
                    admitted.manager,
                    reason,
                )
                .await?;
                return Ok(ManagerDispatch::Handled);
            }
            let destination = ReplyDestination {
                manager: admitted.manager.clone(),
                sender: admitted.sender.clone(),
                authority: ReplyAuthority::Public {
                    ingress: admitted,
                    public_session_id: context.public_session_id.clone(),
                    turn: context.turn.clone(),
                },
                expires_at: super::platform_public_credential_service::public_reply_expiry(
                    &state.db, &context,
                )
                .await?,
            };
            // Establish the bounded receiver before the execution producer spawns.
            let (events, receiver) = tokio::sync::mpsc::channel(16);
            let expected = context.turn.clone();
            let turn = producers
                .public
                .run_public_turn(
                    &state.db,
                    context,
                    TurnInput {
                        text: text.to_owned(),
                    },
                    events,
                )
                .await;
            let turn = match turn {
                Ok(turn) => turn,
                Err(error) => {
                    let Some(reason) = public_failure(&error) else {
                        return Err(error);
                    };
                    send_fixed(
                        state,
                        &producers,
                        &verified,
                        &ingress,
                        destination.manager,
                        reason,
                    )
                    .await?;
                    return Ok(ManagerDispatch::Handled);
                }
            };
            if turn != expected {
                return Err(AppError::Conflict("Public execution turn changed".into()));
            }
            crate::services::platform_native_reply_service::spawn_in_native_scope(consume_events(
                state.clone(),
                producers.clone(),
                destination,
                receiver,
            ));
        }
        ManagerAudience::Control => {
            return Err(AppError::Conflict("Control receipt cannot execute".into()));
        }
    }
    Ok(ManagerDispatch::Handled)
}

fn legacy_confirmation(text: &str, username: &str) -> bool {
    let mut words = text.split_whitespace();
    let Some(verb) = words.next() else {
        return false;
    };
    let (base, target) = verb
        .split_once('@')
        .map_or((verb, None), |(base, target)| (base, Some(target)));
    if base != "/confirm" || target.is_some_and(|target| !target.eq_ignore_ascii_case(username)) {
        return false;
    }
    let code = text.trim_start().strip_prefix(verb).unwrap_or("").trim();
    // Personal proofs have their own parser and must never become legacy proofs.
    !code
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("P-"))
        && super::platform_binding_service::normalize_code(code).is_some()
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

async fn send_fixed(
    state: &AppState,
    producers: &PlatformManagerProducers,
    update: &VerifiedManagerUpdate,
    ingress: &IngressClaim,
    manager: ManagerAuthority,
    reason: UnavailableReason,
) -> AppResult<()> {
    let destination = fixed_destination(manager, update, ingress.clone());
    producers
        .transport
        .deliver_platform_reply(
            &state.db,
            &destination,
            &DeliveryRef::Fixed {
                ingress: ingress.clone(),
            },
            ReplyContent::Fixed(reason),
        )
        .await?;
    Ok(())
}

fn public_failure(error: &AppError) -> Option<UnavailableReason> {
    Some(match error {
        AppError::RateLimited => UnavailableReason::PublicQuotaLimited,
        AppError::InsufficientCredits | AppError::WalletSuspended => {
            UnavailableReason::SponsorUnavailable
        }
        AppError::Conflict(_) | AppError::Forbidden(_) => UnavailableReason::PublicNotReady,
        _ => return None,
    })
}

fn expected_failure(error: &AppError) -> Option<UnavailableReason> {
    Some(match error {
        AppError::AssistantTurnActive
        | AppError::ServiceConcurrencyLimited
        | AppError::RateLimited => UnavailableReason::CapacityLimited,
        AppError::InsufficientCredits | AppError::WalletSuspended => {
            UnavailableReason::InsufficientFunds
        }
        AppError::Forbidden(_)
        | AppError::ForbiddenWithGuidance { .. }
        | AppError::PlanEntitlementRequired(_) => UnavailableReason::OwnerUnavailable,
        AppError::ValidationError(_)
        | AppError::BadRequest(_)
        | AppError::AssistantModelUnavailable => UnavailableReason::EngineUnavailable,
        AppError::Conflict(_) | AppError::NotFound(_) => UnavailableReason::OriginRevoked,
        _ => return None,
    })
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
            ExecutionEvent::Settled { turn } => {
                if !matches!(&destination.authority,ReplyAuthority::Personal {turn:expected,..}|ReplyAuthority::Public {turn:expected,..} if *expected==turn)
                {
                    tracing::warn!("Manager execution emitted settlement for another turn");
                    continue;
                }
                break;
            }
        }
    }
}

pub(crate) async fn deliver_saved(
    state: &AppState,
    producers: &PlatformManagerProducers,
    destination: &ReplyDestination,
    reply: &ReplyRef,
) -> AppResult<DeliveryOutcome> {
    if matches!(destination.authority, ReplyAuthority::Public { .. }) {
        if let Some(expires_at) =
            super::platform_public_credential_service::public_failure_notice_expiry(
                &state.db,
                destination,
                reply,
            )
            .await?
        {
            let ReplyAuthority::Public { ingress, .. } = &destination.authority else {
                unreachable!();
            };
            let fixed = ReplyDestination {
                manager: destination.manager.clone(),
                sender: destination.sender.clone(),
                authority: ReplyAuthority::Fixed {
                    ingress: ingress.ingress.clone(),
                },
                expires_at,
            };
            return producers
                .transport
                .deliver_platform_reply(
                    &state.db,
                    &fixed,
                    &DeliveryRef::Fixed {
                        ingress: ingress.ingress.clone(),
                    },
                    ReplyContent::Fixed(UnavailableReason::PublicTurnFailed),
                )
                .await;
        }
        // Public transcripts never enter owner history or credential paths.
        return delivery::deliver_public_reply(
            state,
            producers.identity.clone(),
            producers.public.clone(),
            destination,
            reply,
            &super::telegram_new_api::TelegramApi::new(&state.http_client),
        )
        .await;
    }
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
    let destination = delivery::captured_destination(&saved)?
        .ok_or_else(|| AppError::Conflict("Reply remains in NyxID".into()))?;
    let body = delivery::render_personal(state, &saved, None).await?;
    producers
        .transport
        .deliver_platform_reply(
            &state.db,
            &destination,
            &DeliveryRef::Reply(reply.clone()),
            ReplyContent::Text(body),
        )
        .await
}

pub fn spawn_sweeps(state: AppState, producers: Arc<PlatformManagerProducers>) {
    crate::services::platform_native_reply_service::spawn_in_native_scope(async move {
        let mut ticks = tokio::time::interval(std::time::Duration::from_secs(15));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut pending = false;
        loop {
            if pending {
                // Drain backlog in bounded passes while yielding between passes.
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                ticks.tick().await;
            }
            pending = false;
            match super::platform_personal_owner_lifecycle::recover_pending(&state.db, 32).await {
                Ok(visited) => {
                    pending |= visited == 32;
                    if visited > 0 {
                        tracing::debug!(
                            visited,
                            "Personal account retirement recovery visited owners"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    error_code = error.error_code(),
                    "Personal account retirement recovery failed"
                ),
            }
            pending |= recovery_progress(
                "personal lifecycle",
                producers
                    .lifecycle
                    .recover_personal_lifecycle(&state.db, producers.personal.as_ref(), 100)
                    .await,
            );
            pending |= recovery_progress(
                "public lifecycle",
                super::platform_public_agent_service::sweep_public_lifecycle(&state, 100).await,
            );
            pending |= recovery_progress(
                "public budget",
                super::platform_public_budget_service::sweep(&state.db, 100).await,
            );
            pending |= recovery_progress("saved replies", recover_saved_replies(&state, 100).await);
            if let Err(error) = recover_ingress(&state, &producers, 100).await {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Manager ingress recovery failed"
                );
            }
            pending |= recovery_progress(
                "delivery",
                delivery::sweep(
                    &state,
                    producers.identity.clone(),
                    producers.public.clone(),
                    100,
                )
                .await,
            );
        }
    });
}

fn recovery_progress(component: &'static str, result: AppResult<RecoveryBatch>) -> bool {
    match result {
        Ok(batch) => {
            if batch.processed > 0 || batch.has_more {
                tracing::debug!(
                    component,
                    processed = batch.processed,
                    has_more = batch.has_more,
                    "Manager recovery batch completed"
                );
            }
            batch.has_more
        }
        Err(error) => {
            tracing::warn!(
                component,
                error_code = error.error_code(),
                "Manager recovery batch failed"
            );
            false
        }
    }
}

async fn recover_saved_replies(state: &AppState, limit: u32) -> AppResult<RecoveryBatch> {
    use futures::TryStreamExt;
    let batch = limit.min(100);
    if batch == 0 {
        return Ok(RecoveryBatch {
            processed: 0,
            has_more: false,
        });
    }
    let messages = state.db.collection::<AssistantMessage>(MESSAGES);
    let mut pending=messages.find(doc! {"role":"assistant","platform_reply.kind":"captured","platform_delivery_recovery_queued":{"$ne":true}})
        .sort(doc! {"created_at":1,"_id":1}).limit(i64::from(batch)).await?;
    let mut processed = 0;
    while let Some(message) = pending.try_next().await? {
        let result: AppResult<()> = async {
            let row = state
                .db
                .collection::<crate::models::assistant_conversation::AssistantConversation>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&message.conversation_id,"user_id":&message.user_id})
                .await?
                .ok_or_else(|| AppError::NotFound("Retained conversation unavailable".into()))?;
            delivery::saved_personal_reply(
                state,
                &row,
                &ReplyRef {
                    turn: TurnRef {
                        conversation_id: message.conversation_id.clone(),
                        turn_id: message.turn_id.clone(),
                    },
                    reply_id: message.id.clone(),
                },
            )
            .await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {}
            Err(
                AppError::Conflict(_)
                | AppError::NotFound(_)
                | AppError::Forbidden(_)
                | AppError::ValidationError(_),
            ) => {
                messages
                    .update_one(
                        doc! {"_id":&message.id},
                        doc! {"$set":{"platform_delivery_recovery_queued":true}},
                    )
                    .await?;
            }
            Err(error) => tracing::warn!(
                error_code = error.error_code(),
                "Saved manager reply recovery remains pending"
            ),
        }
        processed += 1;
    }
    Ok(RecoveryBatch {
        processed,
        has_more: processed == batch,
    })
}

async fn recover_ingress(
    state: &AppState,
    producers: &PlatformManagerProducers,
    limit: u32,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let batch = limit.min(100);
    if batch == 0 {
        return Ok(());
    }
    let receipts_collection = state.db.collection::<PlatformManagerIngress>(INGRESS);
    let mut receipts=receipts_collection.find(doc! {"phase":"settled","reply_id":{"$type":"string"},"delivery_recovery_queued":{"$ne":true}}).sort(doc! {"settled_at":1,"_id":1}).limit(i64::from(batch)).await?;
    while let Some(receipt) = receipts.try_next().await? {
        let result: AppResult<()> = async {
            let reply_id = receipt
                .reply_id
                .as_ref()
                .ok_or_else(|| AppError::Conflict("Missing reply reference".into()))?;
            if let crate::models::platform_manager_ingress::IngressAudience::Public {public_policy_revision}=&receipt.audience {
                let execution=receipt.execution.as_ref().ok_or_else(||AppError::Conflict("Missing public execution".into()))?;
                let retained=state.db.collection::<crate::models::platform_public_turn::PlatformPublicTurn>(crate::models::platform_public_turn::COLLECTION_NAME)
                    .find_one(doc! {"_id":&execution.turn_id,"receipt_id":&receipt.id,"claim_id":&receipt.claim_id,"reply_id":reply_id}).await?
                    .ok_or_else(||AppError::NotFound("Retained public reply unavailable".into()))?;
                let manager=ManagerAuthority {profile_id:receipt.profile_id.clone(),manager_bot_id:receipt.manager_bot_id.clone(),entry:receipt.entry.clone(),manager_authority_epoch:receipt.manager_authority_epoch};
                let turn=TurnRef {conversation_id:execution.conversation_id.clone(),turn_id:execution.turn_id.clone()};
                let admitted=PublicIngress {manager:manager.clone(),sender:receipt.sender.clone(),public_policy_revision:*public_policy_revision,
                    ingress:IngressClaim {receipt_id:receipt.id.clone(),claim_id:receipt.claim_id.clone()}};
                let destination=ReplyDestination {manager,sender:receipt.sender.clone(),expires_at:retained.expires_at,
                    authority:ReplyAuthority::Public {ingress:admitted,public_session_id:retained.public_session_id,turn:turn.clone()}};
                deliver_saved(state,producers,&destination,&ReplyRef {turn,reply_id:reply_id.clone()}).await?;
                return Ok(());
            }
            let message = state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"_id":reply_id})
                .await?
                .ok_or_else(|| AppError::NotFound("Retained reply unavailable".into()))?;
            let turn=TurnRef {conversation_id:message.conversation_id.clone(),turn_id:message.turn_id.clone()};
            let destination=delivery::captured_destination(&message)?.ok_or_else(||AppError::Conflict("Reply remains in NyxID".into()))?;
            let _ = deliver_saved(
                state,
                producers,
                &destination,
                &ReplyRef {
                    turn,
                    reply_id: reply_id.clone(),
                },
            )
            .await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {}
            Err(AppError::NotFound(_) | AppError::Conflict(_)) => {
                // Missing/deleted/altered proof is terminal, never reconstructed.
                receipts_collection
                    .update_one(
                        doc! {"_id":&receipt.id,"claim_id":&receipt.claim_id,"phase":"settled"},
                        doc! {"$set":{"delivery_recovery_queued":true}},
                    )
                    .await?;
            }
            Err(error) => tracing::warn!(
                error_code = error.error_code(),
                "Manager reply recovery remains pending"
            ),
        }
    }
    let mut expired = receipts_collection
        .find(doc! {"phase":"admitted","start_before":{"$lte":bson::DateTime::now()}})
        .sort(doc! {"start_before":1,"_id":1})
        .limit(i64::from(batch))
        .await?;
    while let Some(receipt) = expired.try_next().await? {
        receipts_collection.update_one(doc! {"_id":receipt.id,"phase":"admitted","start_before":{"$lte":bson::DateTime::now()}},doc! {"$set":{"phase":"interrupted","interrupted_reason":"admission_expired","settled_at":bson::DateTime::now()}}).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "platform_manager_journey_tests.rs"]
mod journey_tests;

#[cfg(test)]
mod future_size_tests {
    use super::*;

    fn admission_future_size<F: std::future::Future>(
        _: impl FnOnce(
            &'static AppState,
            Option<Arc<PlatformManagerProducers>>,
            super::super::platform_entry_receipts::NativeAdmission,
        ) -> F,
    ) -> usize {
        std::mem::size_of::<F>()
    }

    fn verified_future_size<F: std::future::Future>(
        _: impl FnOnce(
            &'static AppState,
            Arc<PlatformManagerProducers>,
            crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls,
            VerifiedManagerUpdate,
            &'static str,
            &'static str,
            &'static super::super::telegram_new_api::TelegramApi<'static>,
        ) -> F,
    ) -> usize {
        std::mem::size_of::<F>()
    }

    #[test]
    fn verified_dispatch_keeps_legacy_transport_caller_future_small() {
        let size = verified_future_size(dispatch_verified);
        assert!(
            size < 1024,
            "verified dispatch caller future grew to {size} bytes"
        );
    }

    #[test]
    fn native_admission_keeps_transport_caller_futures_small() {
        let size = admission_future_size(dispatch_native_admission);
        assert!(size < 1024, "native admission future grew to {size} bytes");
    }
}
