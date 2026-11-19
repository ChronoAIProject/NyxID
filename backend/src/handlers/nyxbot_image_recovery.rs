//! Reconstruct only the claimed attachment, never replay a turn or its text.
use super::*;
use crate::{
    handlers::nyxbot as bot,
    models::{
        assistant_conversation::AssistantConversation,
        image_preparation::{ChannelRoute, Route, Source},
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotEvent},
        trigger_run::{COLLECTION_NAME as RUNS, TriggerRun},
    },
    services::{
        assistant_acknowledgement_service as acks, channel_image_preparation as preparation,
    },
};
use bson::doc;
use futures::StreamExt;

struct Target {
    channel: NyxbotChannel,
    thread: NyxbotThread,
    conversation: AssistantConversation,
    post: Option<bot::chats::PostTarget>,
    fence: Option<acks::ChatAuthority>,
    plan: ImageSendPlan,
}

pub(super) fn route(ctx: &ImageSendContext<'_>) -> Route {
    Route::Channel {
        route: Box::new(ChannelRoute {
            conversation_id: ctx.conversation_id.into(),
            turn_id: ctx.turn_id.into(),
            channel_id: ctx.channel.id.clone(),
            thread_id: ctx.thread.id.clone(),
            bot_id: ctx.channel.channel_bot_id.clone(),
            transport: ctx.channel.transport.clone(),
            gateway_version: ctx.channel.gateway_version,
            binding_generation: ctx.thread.follow.binding_generation,
            route_id: ctx.channel.route_id.clone(),
            route_key_id: ctx.channel.route_api_key_id.clone(),
            anchor: ctx.anchor.clone(),
            source: ctx.source.clone(),
        }),
    }
}

async fn target(
    state: &AppState,
    owner: &str,
    route: &ChannelRoute,
    with_plan: bool,
) -> AppResult<Target> {
    target_with(state, owner, route, with_plan, None).await
}

async fn target_with(
    state: &AppState,
    owner: &str,
    route: &ChannelRoute,
    with_plan: bool,
    checked: Option<&crate::services::platform_bot_authority::PersonalRouteCheck>,
) -> AppResult<Target> {
    // A recovered attachment belongs to an already admitted logical post;
    // revalidate its target without spending another initiate limiter token.
    let conversation = engine::get(&state.db, owner, &route.conversation_id).await?;
    let mut fence = None;
    let mut post = None;
    let mut scheduled_plan = None;
    let (channel, thread) = match &route.source {
        Source::Event { event_id } => {
            let event = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"_id":event_id})
                .await?
                .ok_or_else(preparation::lost)?;
            let d = event.delivery.as_ref().ok_or_else(preparation::lost)?;
            if event.conversation_id.as_deref() != Some(route.conversation_id.as_str())
                || event.turn_id.as_deref() != Some(route.turn_id.as_str())
                || crate::services::org_channel_service::event_actor(&event) != owner
                || d.state == "refused"
            {
                return Err(preparation::lost());
            }
            bot::late_delivery::eligible_with(state, &event, d, &conversation, checked)
                .await?
                .ok_or_else(preparation::lost)?
        }
        Source::Supplemental { origin } => {
            if origin.thread.is_some() {
                let thread = crate::services::channel_thread_follow_service::validate_delivery(
                    &state.db,
                    owner,
                    origin,
                    &conversation.id,
                )
                .await?;
                let channel = crate::services::channel_thread_follow_service::access(
                    &state.db,
                    &thread.user_id,
                    &route.channel_id,
                )
                .await?;
                (channel, thread)
            } else {
                crate::services::channel_turn_delivery::target_with(
                    &state.db,
                    &conversation,
                    origin,
                    checked,
                )
                .await?
                .ok_or_else(preparation::lost)?
            }
        }
        Source::Post {
            api_key_id,
            key_version,
        } => {
            let chat =
                acks::chat_for_key_version(&state.db, owner, Some(api_key_id), *key_version, None)
                    .await?
                    .ok_or_else(preparation::lost)?;
            if chat.conversation_id != route.conversation_id
                || chat.turn_id.as_deref() != Some(route.turn_id.as_str())
                || !bot::chats::post_turn_is_live(state, &chat).await?
            {
                return Err(AppError::AssistantTurnRequired);
            }
            let transport_owner = chat
                .org_agent_access
                .as_ref()
                .and_then(|a| a.channel())
                .map_or(owner, |a| a.channel().user_id.as_str());
            let destination = bot::chats::load_post_with(
                state,
                transport_owner,
                &route.thread_id,
                (!chat.is_orchestrator()
                    && !crate::services::assistant_service_execution::is_shared_chat(&chat))
                .then_some(chat.agent_id.as_str()),
                false,
                checked,
            )
            .await?;
            if (crate::services::assistant_service_execution::is_shared_chat(&chat)
                && destination.bot.user_id != owner)
                || (destination.thread.kind.as_deref() == Some("private")
                    && chat
                        .org_agent_access
                        .as_ref()
                        .and_then(|a| a.channel())
                        .is_some_and(|a| a.execution().actor_id != owner))
            {
                return Err(preparation::lost());
            }
            let pair = (destination.channel.clone(), destination.thread.clone());
            fence = Some(chat);
            post = Some(destination);
            pair
        }
        Source::Automation { run_id } => {
            let raw = state.db.collection::<bson::Document>(RUNS).find_one(doc! {"_id":run_id,"user_id":owner,
                "thread_id":&route.conversation_id,
                "$expr":{"$gt":[{"$ifNull":["$result_delivery.expires_at","$expires_at"]},"$$NOW"]}}).await?.ok_or_else(preparation::lost)?;
            let run: TriggerRun = bson::from_document(raw).map_err(|_| preparation::lost())?;
            if run.deliver_to
                != (crate::models::trigger_schedule::DeliverTo::Chat {
                    chat_id: route.thread_id.clone(),
                })
            {
                return Err(preparation::lost());
            }
            scheduled_plan = Some(ImageSendPlan {
                attachments: run
                    .image_refs
                    .iter()
                    .filter(|r| {
                        r.conversation_id == route.conversation_id && r.turn_id == route.turn_id
                    })
                    .map(|r| r.attachment.clone())
                    .collect(),
                delegated: run
                    .delegated_image_refs
                    .iter()
                    .filter(|r| {
                        r.recipient_turn == route.turn_id
                            && r.assignment.recipient_conversation == route.conversation_id
                    })
                    .cloned()
                    .collect(),
            });
            let destination = bot::chats::load_post_with(
                state,
                owner,
                &route.thread_id,
                run.agent_id
                    .as_deref()
                    .filter(|_| conversation.is_subagent()),
                false,
                checked,
            )
            .await?;
            let pair = (destination.channel.clone(), destination.thread.clone());
            post = Some(destination);
            pair
        }
    };
    if channel.id != route.channel_id
        || thread.id != route.thread_id
        || channel.channel_bot_id != route.bot_id
        || channel.transport != route.transport
        || channel.gateway_version != route.gateway_version
        || thread.follow.binding_generation != route.binding_generation
        || channel.route_id != route.route_id
        || channel.route_api_key_id != route.route_key_id
        || channel.gateway_source.is_some()
    {
        return Err(preparation::lost());
    }
    let plan = if !with_plan {
        ImageSendPlan {
            attachments: vec![],
            delegated: vec![],
        }
    } else if let Some(plan) = scheduled_plan {
        plan
    } else if let Some(turn) = conversation
        .active_turn
        .as_ref()
        .filter(|t| t.turn_id == route.turn_id)
    {
        ImageSendPlan {
            attachments: turn.attachments.clone(),
            delegated: turn.delegated_images.references.clone(),
        }
    } else {
        ImageSendPlan::settled(&state.db, owner, &route.conversation_id, &route.turn_id).await?
    };
    Ok(Target {
        channel,
        thread,
        conversation,
        post,
        fence,
        plan,
    })
}

pub(super) fn guard(
    state: &AppState,
    owner: &str,
    route: &Route,
    audience: &str,
    id: &str,
    attempt: &str,
    delegated: Option<&crate::models::assistant_delegated_image::DeliveryReference>,
) -> crate::services::channel_platform::PostEffectGuard {
    let state = state.clone();
    let owner = owner.to_owned();
    let route = route.clone();
    let audience = audience.to_owned();
    let id = id.to_owned();
    let attempt = attempt.to_owned();
    let delegated = delegated.cloned();
    std::sync::Arc::new(move || {
        let state = state.clone();
        let owner = owner.clone();
        let route = route.clone();
        let audience = audience.clone();
        let id = id.clone();
        let attempt = attempt.clone();
        let delegated = delegated.clone();
        Box::pin(async move {
            let Route::Channel { route } = route else {
                return Err(preparation::lost());
            };
            let checked = crate::services::platform_bot_authority::read_personal_routes(
                &state.db,
                &route.bot_id,
            )
            .await?;
            let current =
                Box::pin(target_with(&state, &owner, &route, false, Some(&checked))).await?;
            let ctx = context(&owner, &route, &current);
            if audience_key_with(&state, &ctx, Some(&checked)).await? != audience
                || !delegated_allowed(&state, &ctx, delegated.as_ref()).await
            {
                return Err(preparation::lost());
            }
            preparation::before_effect(&state.db, &id, &attempt).await
        })
    })
}

fn context<'a>(
    owner: &'a str,
    route: &'a ChannelRoute,
    current: &'a Target,
) -> ImageSendContext<'a> {
    ImageSendContext {
        owner,
        conversation_id: &route.conversation_id,
        turn_id: &route.turn_id,
        channel: &current.channel,
        thread: &current.thread,
        anchor: route.anchor.clone(),
        source: route.source.clone(),
        audience: ImageAudience {
            owner_private: !current.conversation.guest_turn
                && current.thread.owner_chat
                && current.thread.kind.as_deref() == Some("private"),
        },
        deadline: None,
        initiated: current.post.as_ref(),
        fence: current.fence.as_ref(),
    }
}

async fn recover(state: &AppState, row: &NyxbotImageDelivery) -> AppResult<()> {
    let prep = row.preparation.as_ref().ok_or_else(preparation::lost)?;
    if prep.phase == "sending" {
        if let Some(attempt) = &row.attempt {
            preparation::retire_dispatch(&state.db, &row.id, attempt).await?;
        }
        return Ok(());
    }
    if !matches!(prep.phase.as_str(), "preparing" | "released") {
        return Ok(());
    }
    match &prep.route {
        Route::Manager { .. } => {
            crate::services::platform_manager_delivery::recover_attachment(state, row).await
        }
        Route::Channel { route } => {
            let mut current = Box::pin(target(state, &row.user_id, route, true)).await?;
            current.plan.attachments.retain(|a| {
                a.id == row.attachment_id && row.conversation_id == route.conversation_id
            });
            current.plan.delegated.retain(|r| {
                r.attachment.id == row.attachment_id
                    && r.assignment.source_conversation == row.conversation_id
            });
            if candidates(&current.plan).is_empty() {
                return Err(preparation::lost());
            }
            let ctx = context(&row.user_id, route, &current);
            if audience_key(state, &ctx).await? != row.audience_key {
                return Err(preparation::lost());
            }
            send_turn_images(state, ctx, &current.plan).await?;
            Ok(())
        }
    }
}

pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    let rows = preparation::due_rows(&state.db).await?;
    futures::stream::iter(rows)
        .for_each_concurrent(4, |row| async move {
            let Some(_work) = state.shutdown.background() else {
                return;
            };
            if let Err(error) = Box::pin(recover(state, &row)).await {
                if let Some(attempt) = &row.attempt {
                    if matches!(
                        error,
                        AppError::NotFound(_)
                            | AppError::Forbidden(_)
                            | AppError::Conflict(_)
                            | AppError::Unauthorized(_)
                            | AppError::AssistantTurnRequired
                            | AppError::AssistantAttachmentExpired
                            | AppError::ChannelBotInactive(_)
                            | AppError::ChannelBotNotFound(_)
                    ) {
                        let _ = preparation::park(&state.db, &row.id, attempt).await;
                    } else {
                        let _ = claims::release(&state.db, &row.id, attempt).await;
                    }
                }
                tracing::warn!(
                    code = error.error_code(),
                    "Attachment recovery deferred or retired"
                );
            }
        })
        .await;
    Ok(())
}
