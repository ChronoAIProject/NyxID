//! Intercept opted-in native bots after authenticated adapter preparation and
//! paid activity admission, before any ordinary route or owner-token producer.
use super::{
    platform_entry_identity::refused, platform_manager_contract::PlatformManagerProducers,
};
use crate::{
    AppState,
    errors::AppResult,
    models::{
        channel_bot::ChannelBot,
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        platform_bot_config::{COLLECTION_NAME as PROFILES, PlatformBotConfig},
        platform_entry_identity::EntryTransport,
    },
};
use axum::http::HeaderMap;
use bson::{Document, doc};
use std::sync::Arc;

pub(crate) enum NativeWebhook {
    Ordinary,
    Consumed(Option<serde_json::Value>),
}

pub(crate) async fn intercept(
    state: &AppState,
    bot_id: &str,
    expected_platform: &str,
    headers: &HeaderMap,
    body: &[u8],
    producers: Option<Arc<PlatformManagerProducers>>,
) -> AppResult<NativeWebhook> {
    let Some(raw) = state
        .db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":bot_id})
        .await?
    else {
        return Ok(NativeWebhook::Ordinary);
    };
    // Native markers are sticky through pause/retirement and missing controls.
    // A malformed selected entry is consumed; it can never regain owner routing.
    if raw.contains_key("platform_entry_pending") {
        return Err(refused());
    }
    if !raw.contains_key("platform_entry_version") {
        return Ok(NativeWebhook::Ordinary);
    }
    let version = raw
        .get_i32("platform_entry_version")
        .ok()
        .map(i64::from)
        .or_else(|| raw.get_i64("platform_entry_version").ok());
    if version != Some(2) {
        return Err(refused());
    }
    intercept_native(
        state,
        bot_id,
        expected_platform,
        headers,
        body,
        producers,
        raw,
    )
    .await
}

fn intercept_native<'a>(
    state: &'a AppState,
    bot_id: &'a str,
    expected_platform: &'a str,
    headers: &'a HeaderMap,
    body: &'a [u8],
    producers: Option<Arc<PlatformManagerProducers>>,
    raw: Document,
) -> futures::future::BoxFuture<'a, AppResult<NativeWebhook>> {
    // Allocate native preparation and dispatch only after sticky v2 selection.
    // The ordinary webhook future retains just the marker lookup and this pointer.
    Box::pin(intercept_native_impl(
        state,
        bot_id,
        expected_platform,
        headers,
        body,
        producers,
        raw,
    ))
}

async fn intercept_native_impl(
    state: &AppState,
    bot_id: &str,
    expected_platform: &str,
    headers: &HeaderMap,
    body: &[u8],
    producers: Option<Arc<PlatformManagerProducers>>,
    raw: Document,
) -> AppResult<NativeWebhook> {
    let bot: ChannelBot = bson::from_document(raw.clone()).map_err(|_| refused())?;
    if bot.platform != expected_platform || !bot.is_active || bot.status != "active" {
        return Err(refused());
    }
    let profile_id = raw.get_str("platform_profile_id").map_err(|_| refused())?;
    let profile = state.db.collection::<PlatformBotConfig>(PROFILES)
        .find_one(doc! {"_id":profile_id,"channel_bot_id":bot_id,"identity_version":2,"deleted_at":bson::Bson::Null})
        .await?.ok_or_else(refused)?;
    let configuration = profile.native_entry.as_ref().ok_or_else(refused)?;
    super::platform_entry_identity::validate_configuration(configuration)?;
    if configuration.identity.channel_bot_id != bot.id
        || configuration.identity.platform_bot_id != bot.platform_bot_id
        || profile.manager_bot_id != bot.platform_bot_id
        || profile.execution_profile
            != crate::models::platform_bot_config::PlatformExecutionProfile::NativeEntry
    {
        return Err(refused());
    }
    let adapter =
        super::platform_native_reply_service::send_adapter(state, configuration.identity.platform)?;
    let batch = super::platform_entry_native::prepare(
        &state.db,
        &state.encryption_keys,
        adapter.as_ref(),
        &bot,
        headers,
        body,
        configuration,
    )
    .await?;
    if let Some(challenge) = &batch.prepared().challenge_response {
        return Ok(NativeWebhook::Consumed(Some(challenge.clone())));
    }
    // Billing applies to every authenticated parsed X activity, even when it is
    // not eligible for personal/public/control authority. Never mint owner tokens.
    for message in batch.messages() {
        if let Some(billing) = super::channel_billing_service::ChannelBilling::for_bot(
            &state.db,
            &state.billing,
            &bot,
            None,
        ) {
            let activity = adapter.activity_metadata(message);
            let charged = if activity
                .as_ref()
                .is_some_and(|value| value.kind == "encrypted_chat")
            {
                billing.received_chat(&message.platform_message_id).await
            } else if super::channel_adapters::x::is_public_conversation(&message.conversation_id) {
                billing.received_post(&message.platform_message_id).await
            } else {
                billing.received(&message.platform_message_id).await
            };
            if let Err(error) = charged {
                if super::channel_billing_service::blocks_channel(&error) {
                    let _ = super::channel_billing_service::suspend(
                        &super::channel_inbound_service::InboundDeps::from(state),
                        &bot,
                        adapter.as_ref(),
                    )
                    .await;
                }
                return Err(error);
            }
        }
    }
    if !batch.messages().is_empty() {
        super::channel_bot_service::record_inbound_success(&state.db, &bot).await;
    }
    let controls = super::platform_bot_runtime_control_service::get(&state.db, profile_id)
        .await?
        .ok_or_else(refused)?;
    for (index, message) in batch.messages().iter().enumerate() {
        if batch.fact(index).is_none() {
            continue;
        }
        let relay_message_id = uuid::Uuid::new_v4().to_string();
        let mut session = state.db.client().start_session().await?;
        let admission = session
            .start_transaction()
            .and_run(
                (
                    state,
                    &profile,
                    &controls,
                    configuration,
                    &batch,
                    &bot,
                    index,
                    message,
                    &relay_message_id,
                ),
                |session,
                 (
                    state,
                    profile,
                    controls,
                    configuration,
                    batch,
                    bot,
                    index,
                    message,
                    relay_message_id,
                )| {
                    Box::pin(async move {
                        super::api_key_mutation_service::transaction_result(
                            async {
                                let admission = super::platform_entry_receipts::record_in_session(
                                    &state.db,
                                    session,
                                    &state.encryption_keys,
                                    state.auth_device_hmac_key.as_slice(),
                                    super::platform_entry_receipts::NativeReceiptInput {
                                        profile_id: &profile.id,
                                        epoch: controls.manager_authority_epoch,
                                        configuration,
                                        batch,
                                        index: *index,
                                        relay_message_id,
                                    },
                                )
                                .await?;
                                let id = &admission.receipt().id;
                                if state
                                    .db
                                    .collection::<ChannelMessage>(MESSAGES)
                                    .find_one(doc! {"_id":id})
                                    .session(&mut *session)
                                    .await?
                                    .is_none()
                                {
                                    let mut metadata =
                                        super::channel_relay_service::inbound_metadata(
                                            &profile.channel_bot_id,
                                            &profile.id,
                                            &bot.user_id,
                                            &bot.platform,
                                            message,
                                            "",
                                            id,
                                        );
                                    // Content/attachments/raw native bodies remain transient. The
                                    // separately encrypted reply handle owns interaction capability.
                                    metadata.attachments.clear();
                                    metadata.thread_id = None;
                                    metadata.agent_api_key_id = None;
                                    metadata.sender_display_name = None;
                                    metadata.callback_status = Some("pending".into());
                                    state
                                        .db
                                        .collection::<ChannelMessage>(MESSAGES)
                                        .insert_one(metadata)
                                        .session(&mut *session)
                                        .await?;
                                }
                                Ok(admission)
                            }
                            .await,
                        )
                    })
                },
            )
            .await
            .map_err(super::api_key_mutation_service::map_transaction_error)?;
        match &configuration.transport {
            EntryTransport::Direct => {
                super::platform_manager_transport::dispatch_native_admission(
                    state,
                    producers.clone(),
                    admission,
                )
                .await?
            }
            EntryTransport::NyxagentGateway { .. } => {
                super::platform_entry_gateway_service::forward_native(state, &profile, &admission)
                    .await?
            }
        }
    }
    Ok(NativeWebhook::Consumed(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webhook_future_size<F: std::future::Future>(
        _: impl FnOnce(
            &'static AppState,
            &'static str,
            &'static str,
            &'static HeaderMap,
            &'static [u8],
            Option<Arc<PlatformManagerProducers>>,
        ) -> F,
    ) -> usize {
        // Inspect the future type without constructing state or polling a webhook.
        std::mem::size_of::<F>()
    }

    #[test]
    fn native_selection_keeps_ordinary_webhook_futures_small() {
        let interceptor = webhook_future_size(intercept);
        assert!(
            interceptor < 16 * 1024,
            "native interceptor future grew to {interceptor} bytes"
        );
        let handler = webhook_future_size(
            crate::handlers::channel_webhooks::handle_webhook_inner_with_producers,
        );
        assert!(
            handler < 16 * 1024,
            "ordinary webhook future grew to {handler} bytes"
        );
    }
}
