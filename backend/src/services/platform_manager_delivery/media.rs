//! Manager media uses the same source/audience v2 claims as channel replies.
use super::*;
use crate::{
    models::nyxbot_image_delivery::ImageDeliveryState,
    services::{
        channel_image_delivery as claims,
        channel_platform::{
            BotCredentials, MaterializedAttachment, MediaKind, OutboundReply, PlatformAdapter,
        },
    },
};
struct SecretBytes(Zeroizing<Vec<u8>>);
impl AsRef<[u8]> for SecretBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn send_saved(
    state: &AppState,
    row: &Delivery,
    claim: &str,
    body: &str,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
    token: &str,
    api: &super::super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<String> {
    send_saved_inner(state, row, claim, body, identity, public, token, api, None).await
}

#[allow(clippy::too_many_arguments)]
async fn send_saved_inner(
    state: &AppState,
    row: &Delivery,
    claim: &str,
    body: &str,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
    token: &str,
    api: &super::super::telegram_new_api::TelegramApi<'_>,
    only: Option<&crate::models::nyxbot_image_delivery::NyxbotImageDelivery>,
) -> AppResult<String> {
    let DeliveryReference::Reply {
        reply_id,
        conversation_id,
        turn_id,
    } = &row.reference
    else {
        return Ok(String::new());
    };
    let owner = &row.origin.as_ref().ok_or_else(refused)?.thread.user_id;
    let message=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {"_id":reply_id,"user_id":owner,"conversation_id":conversation_id,"turn_id":turn_id}).await?.ok_or_else(refused)?;
    let profile = state
        .db
        .collection::<PlatformBotConfig>(PROFILES)
        .find_one(doc! {"_id":&row.profile_id})
        .await?
        .ok_or_else(refused)?;
    let chat = scope::actor_string(&row.sender);
    let mut reply = OutboundReply {
        text: None,
        attachments: vec![],
        metadata: Some(serde_json::json!({"protect_content":true})),
        reply_to_platform_message_id: None,
    };
    let audience = crate::services::channel_image_audience::resolve(
        &profile.channel_bot_id,
        "telegram",
        &chat,
        &reply,
    );
    if only.is_some_and(|image| {
        image.user_id != *owner
            || image.conversation_id != *conversation_id
            || image.audience_key != audience
            || !message
                .attachments
                .iter()
                .any(|a| a.id == image.attachment_id)
    }) {
        return Err(refused());
    }
    let adapter =
        crate::services::channel_adapters::telegram::TelegramAdapter::for_api_base(api.base_url);
    let deliveries = state.db.collection::<bson::Document>(COLLECTION_NAME);
    if only.is_none()
        && let Some(saved) = deliveries.find_one(doc! {"_id":&row.id}).await?
        && let Ok(missing) = saved.get_i64("multipart_media_missing")
    {
        return Ok(failure_note(
            missing,
            &state.config.frontend_url,
            conversation_id,
        ));
    }
    // Legacy batches used the owner's NyxBot channel row, while v2 audience
    // keys use the provider bot. Preserve both identities across conversion.
    let aliases: Vec<bson::Document> = state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find(doc! {"user_id":owner,"channel_bot_id":&profile.channel_bot_id})
        .projection(doc! {"_id":1})
        .limit(65)
        .await?
        .try_collect()
        .await?;
    if aliases.len() > 64 {
        return Err(refused());
    }
    let mut legacy_channels = vec![profile.channel_bot_id.clone()];
    legacy_channels.extend(
        aliases
            .iter()
            .filter_map(|row| row.get_str("_id").ok())
            .map(str::to_owned),
    );
    let mut missing = 0i64;
    let mut uncertain = false;
    let mut seen = std::collections::HashSet::new();
    for (index, attachment) in message
        .attachments
        .iter()
        .filter(|a| {
            a.origin == "tool"
                && a.chat_delivery != Some(false)
                && !(a.chat_delivery.is_none() && a.label == "Machine screenshot")
                && seen.insert(a.id.as_str())
        })
        .take(crate::services::assistant_nyxagent::MAX_TURN_ATTACHMENTS)
        .enumerate()
    {
        if only.is_some_and(|image| image.attachment_id != attachment.id) {
            continue;
        }
        if uncertain {
            missing += 1;
            continue;
        }
        if !validate_media(
            state,
            row,
            only.is_none().then_some(claim),
            body,
            identity.as_ref(),
            public.as_deref(),
        )
        .await?
        {
            missing += 1;
            continue;
        }
        let mut legacy = None;
        for channel in &legacy_channels {
            if let Some(outcome) = claims::read_legacy(
                &state.db,
                owner,
                conversation_id,
                turn_id,
                channel,
                &attachment.id,
            )
            .await?
            {
                let rank = |state| match state {
                    ImageDeliveryState::Sent => 0,
                    ImageDeliveryState::Unknown | ImageDeliveryState::Sending => 1,
                    _ => 2,
                };
                if legacy
                    .as_ref()
                    .is_none_or(|previous: &claims::LegacyImageOutcome| {
                        rank(outcome.state) < rank(previous.state)
                    })
                {
                    legacy = Some(outcome);
                }
            }
        }
        let acquired = if let Some(outcome) = legacy {
            claims::ImageClaim::Legacy(outcome)
        } else {
            claims::prepare_for_turn(
                &state.db,
                owner,
                conversation_id,
                turn_id,
                &profile.channel_bot_id,
                &attachment.id,
                &audience,
                &crate::models::image_preparation::Route::Manager {
                    delivery_id: row.id.clone(),
                },
            )
            .await?
        };
        let (id, attempt) = match acquired {
            claims::ImageClaim::Dispatch { id, attempt } => (id, attempt),
            claims::ImageClaim::Existing(existing) => {
                if existing.state != ImageDeliveryState::Sent {
                    missing += 1;
                    uncertain |= matches!(
                        existing.state,
                        ImageDeliveryState::Sending | ImageDeliveryState::Unknown
                    );
                }
                continue;
            }
            claims::ImageClaim::Legacy(existing) => {
                if existing.state != ImageDeliveryState::Sent {
                    missing += 1;
                    uncertain |= matches!(
                        existing.state,
                        ImageDeliveryState::Sending | ImageDeliveryState::Unknown
                    );
                }
                continue;
            }
            claims::ImageClaim::Unknown => {
                missing += 1;
                uncertain = true;
                continue;
            }
        };
        #[cfg(test)]
        crate::services::durable_wake_tests::after("manager_image_preparation_claimed").await;
        let loaded = async {
            if !claims::source_exists(&state.db, owner, conversation_id, turn_id, &attachment.id)
                .await?
            {
                return Err(refused());
            }
            let (mime, bytes) = crate::services::assistant_nyxagent::read_attachment(
                &state.db,
                &state.encryption_keys,
                owner,
                conversation_id,
                &attachment.id,
            )
            .await?;
            let bytes = Zeroizing::new(bytes);
            if mime != attachment.content_type || bytes.len() as i64 != attachment.size {
                return Err(refused());
            }
            if bytes.len() as u64 > state.config.channel_media_max_bytes {
                return Err(AppError::ChannelMediaTooLarge);
            }
            Ok((mime, bytes))
        }
        .await;
        let (status, code) = match loaded {
            Err(_) => (ImageDeliveryState::Failed, Some("unavailable")),
            Ok((mime, bytes)) => {
                let admission = async {
                    if only.is_none() {
                        pace_part(state, row, claim).await?;
                    } else if !part_rate_available(state, row).await? {
                        return Err(AppError::RateLimited);
                    }
                    validate_media(
                        state,
                        row,
                        only.is_none().then_some(claim),
                        body,
                        identity.as_ref(),
                        public.as_deref(),
                    )
                    .await
                }
                .await;
                match admission {
                    Ok(true) => {}
                    Ok(false) => {
                        let _ = claims::release(&state.db, &id, &attempt).await;
                        missing += 1;
                        continue;
                    }
                    Err(error) => {
                        // No provider I/O has started; retain a retryable claim
                        // when a preflight database/authority check fails.
                        let _ = claims::release(&state.db, &id, &attempt).await;
                        return Err(error);
                    }
                }
                let image = mime.starts_with("image/");
                let extension = match mime.as_str() {
                    "image/png" => ".png",
                    "image/jpeg" => ".jpg",
                    "image/webp" => ".webp",
                    "image/gif" => ".gif",
                    "application/pdf" => ".pdf",
                    "text/plain" => ".txt",
                    "text/csv" => ".csv",
                    "application/json" => ".json",
                    _ => "",
                };
                reply.attachments = vec![MaterializedAttachment {
                    kind: if image {
                        MediaKind::Image
                    } else {
                        MediaKind::File
                    },
                    bytes: bytes::Bytes::from_owner(SecretBytes(bytes)),
                    filename: Some(format!("attachment-{}{extension}", index + 1)),
                    mime_type: Some(mime),
                    caption: None,
                }];
                let credentials = BotCredentials {
                    token,
                    platform_bot_id: Some(&row.manager_bot_id),
                    platform_secrets: None,
                    billing: None,
                };
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(35),
                    crate::services::channel_image_preparation::with_effect(
                        media_guard(
                            state,
                            row,
                            only.is_none().then_some(claim),
                            body,
                            identity.clone(),
                            public.clone(),
                            &id,
                            &attempt,
                        ),
                        adapter.send_reply(api.http, &credentials, &chat, &reply),
                    ),
                )
                .await;
                match result {
                    Ok(Ok(Some(_))) => (ImageDeliveryState::Sent, None),
                    Ok(Err(error)) if error.retry_after_seconds().is_some() => {
                        if matches!(
                            claims::release_refused(&state.db, &id, &attempt).await?,
                            claims::ImageCompletion::Recorded
                        ) {
                            return Err(error);
                        }
                        uncertain = true;
                        (ImageDeliveryState::Unknown, None)
                    }
                    Ok(Err(error))
                        if !crate::services::channel_media_service::is_delivery_uncertain(
                            &error,
                        ) =>
                    {
                        (ImageDeliveryState::Failed, Some("platform_error"))
                    }
                    _ => {
                        uncertain = true;
                        (ImageDeliveryState::Unknown, None)
                    }
                }
            }
        };
        let recorded = claims::finish(&state.db, &id, &attempt, status, code).await?;
        if !matches!(recorded, claims::ImageCompletion::Recorded) {
            uncertain = true;
            missing += 1;
        } else if status != ImageDeliveryState::Sent {
            missing += 1;
        }
    }
    if only.is_none()
        && deliveries
            .update_one(
                doc! {"_id":&row.id,"state":"dispatching","claim_id":claim},
                doc! {"$set":{"multipart_media_missing":missing}},
            )
            .await?
            .matched_count
            != 1
    {
        return Err(super::super::channel_text_delivery::unconfirmed());
    }
    Ok(failure_note(
        missing,
        &state.config.frontend_url,
        conversation_id,
    ))
}

fn failure_note(missing: i64, frontend: &str, conversation_id: &str) -> String {
    if missing == 0 {
        String::new()
    } else {
        format!(
            "\n\n{missing} attachment(s) could not be confirmed delivered. Review them in NyxID: {}",
            conversation_url(frontend, conversation_id)
        )
    }
}

async fn validate_media(
    state: &AppState,
    row: &Delivery,
    claim: Option<&str>,
    body: &str,
    identity: &dyn ManagerIdentityProducer,
    public: Option<&dyn PublicTurnProducer>,
) -> AppResult<bool> {
    if let Some(claim) = claim {
        return revalidate_part(state, row, claim, body, identity, public).await;
    }
    let mut session = state.db.client().start_session().await?;
    let result = session
        .start_transaction()
        .and_run(
            (state, row, body, identity, public),
            |session, (state, row, body, identity, public)| {
                Box::pin(async move {
                    let result: AppResult<bool> = async {
                        if state
                            .db
                            .collection::<bson::Document>(COLLECTION_NAME)
                            .find_one(doc! {
                                "_id":&row.id,"state":{"$ne":"refused"},
                                "$expr":{"$gt":["$expires_at","$$NOW"]},
                            })
                            .session(&mut *session)
                            .await?
                            .is_none()
                        {
                            return Ok(false);
                        }
                        Ok(
                            validate_in_session(state, *identity, *public, session, row, body)
                                .await?
                                .is_some(),
                        )
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error);
    match result {
        Err(AppError::Conflict(_) | AppError::NotFound(_) | AppError::Forbidden(_)) => Ok(false),
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
fn media_guard(
    state: &AppState,
    row: &Delivery,
    claim: Option<&str>,
    body: &str,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
    id: &str,
    attempt: &str,
) -> crate::services::channel_platform::PostEffectGuard {
    let state = state.clone();
    let row = row.clone();
    let claim = claim.map(str::to_owned);
    let body = Arc::new(Zeroizing::new(body.to_owned()));
    let id = id.to_owned();
    let attempt = attempt.to_owned();
    Arc::new(move || {
        let state = state.clone();
        let row = row.clone();
        let claim = claim.clone();
        let body = body.clone();
        let identity = identity.clone();
        let public = public.clone();
        let id = id.clone();
        let attempt = attempt.clone();
        Box::pin(async move {
            if !validate_media(
                &state,
                &row,
                claim.as_deref(),
                body.as_str(),
                identity.as_ref(),
                public.as_deref(),
            )
            .await?
            {
                return Err(refused());
            }
            crate::services::channel_image_preparation::before_effect(&state.db, &id, &attempt)
                .await
        })
    })
}

pub(super) async fn recover(
    state: &AppState,
    image: &crate::models::nyxbot_image_delivery::NyxbotImageDelivery,
    api: &super::super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<()> {
    let Some(crate::models::image_preparation::Preparation {
        route: crate::models::image_preparation::Route::Manager { delivery_id },
        ..
    }) = image.preparation.as_deref()
    else {
        return Err(refused());
    };
    let row = state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .find_one(doc! {"_id":delivery_id})
        .await?
        .ok_or_else(refused)?;
    let DeliveryReference::Reply { reply_id, .. } = &row.reference else {
        return Err(refused());
    };
    let message = state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id":reply_id})
        .await?
        .ok_or_else(refused)?;
    let body = Zeroizing::new(render_personal(state, &message, None).await?);
    let identity = super::super::platform_personal_binding_service::producer(state.clone());
    if !validate_media(state, &row, None, &body, identity.as_ref(), None).await? {
        return Err(refused());
    }
    let mut session = state.db.client().start_session().await?;
    let material = session
        .start_transaction()
        .and_run(
            (state, &row, &body, &identity),
            |session, (state, row, body, identity)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        let observation =
                            validate_in_session(state, identity.as_ref(), None, session, row, body)
                                .await?
                                .ok_or_else(refused)?;
                        capture_material_in_session(
                            state,
                            session,
                            row,
                            "attachment-recovery",
                            &observation,
                            None,
                        )
                        .await
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    let CapturedSend::Ready(CapturedMaterial::Telegram(credential)) = material else {
        return Err(refused());
    };
    let values = super::super::platform_credential_service::decrypt_platform_snapshot(
        &state.encryption_keys,
        Some(&credential),
    )
    .await?;
    let token = values
        .get(super::super::channel_adapters::telegram_new::MANAGER_TOKEN)
        .ok_or_else(refused)?;
    if token
        .split(':')
        .next()
        .and_then(|id| id.parse::<i64>().ok())
        .map(|id| id.to_string())
        .as_deref()
        != Some(&row.manager_bot_id)
    {
        return Err(refused());
    }
    send_saved_inner(
        state,
        &row,
        "",
        &body,
        identity,
        None,
        token,
        api,
        Some(image),
    )
    .await?;
    Ok(())
}
