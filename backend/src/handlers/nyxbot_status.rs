//! What a NyxBot thread is waiting on, and whether a linked chat app's
//! messages actually reach its agent. Both exist so nothing goes quiet
//! without the owner being able to tell why.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Serialize;

use super::{
    CHANNELS, EVENTS, NyxbotChannel, NyxbotEvent, NyxbotThread, NyxbotWatch, THREADS, WATCHES,
    excerpt, identifier,
};
use crate::{
    AppState,
    errors::AppResult,
    models::channel_message::{COLLECTION_NAME as MESSAGES, CallbackDisposition, ChannelMessage},
};

/// How long the gateway may hold an accepted private message before it
/// counts as lost (it normally calls the provider within seconds).
const NOT_RECEIVED_GRACE_SECS: i64 = 120;
/// At most one "messages are not arriving" notice per channel in this window.
const DELIVERY_NOTICE_HOURS: i64 = 6;
/// Channels checked per sweep, least recently checked first.
const DELIVERY_BATCH: i64 = 100;
/// Inbound messages looked at per channel per sweep, newest first.
const MESSAGES_PER_CHECK: i64 = 20;
/// How far back a channel's first check looks.
const FIRST_LOOKBACK_HOURS: i64 = 1;
/// Admission records outlive this, so older accepted messages are not judged.
const JUDGE_WINDOW_HOURS: i64 = 12;

// Preserve unknown labels verbatim here, including the generic "chat app".
fn platform_name(platform: &str) -> &str {
    match platform {
        "telegram" | "telegram-new" => "Telegram",
        "discord" => "Discord",
        "slack" => "Slack",
        "lark" => "Lark",
        "feishu" => "Feishu",
        "whatsapp" => "WhatsApp",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Waiting
// ---------------------------------------------------------------------------

/// Something outside the chat the thread is waiting for. NyxID resumes the
/// thread by itself when it happens.
#[derive(Debug, Serialize)]
pub struct WaitingItem {
    /// `channel_bot`, `connect_link`, or `owner_verification`.
    pub kind: &'static str,
    pub title: String,
    /// What NyxID has seen so far, when that explains a long wait.
    pub detail: Option<String>,
    pub since: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub(crate) async fn waiting(
    state: &AppState,
    owner: &str,
    conversation_id: &str,
) -> AppResult<Vec<WaitingItem>> {
    let now = bson::DateTime::now();
    let watches: Vec<NyxbotWatch> = state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .find(doc! {"user_id": owner, "conversation_id": conversation_id,
        "status": "pending", "expires_at": {"$gt": now}})
        .sort(doc! {"created_at": 1})
        .limit(10)
        .await?
        .try_collect()
        .await?;
    let mut items = Vec::new();
    for watch in watches {
        match watch.kind.as_str() {
            "trigger_created" => items.push(WaitingItem {
                kind: "trigger_created",
                title: "Waiting for your webhook trigger to be created".into(),
                detail: None,
                since: watch.created_at,
                expires_at: Some(watch.expires_at),
            }),
            "channel_bot" => items.push(WaitingItem {
                kind: "channel_bot",
                title: format!(
                    "Waiting for your {} bot to be created",
                    platform_name(watch.platform.as_deref().unwrap_or("chat app"))
                ),
                detail: None,
                since: watch.created_at,
                expires_at: Some(watch.expires_at),
            }),
            "connect_link" => {
                let Some(link) = connect_link(state, owner, &watch).await? else {
                    continue;
                };
                items.push(WaitingItem {
                    kind: "connect_link",
                    title: format!(
                        "Waiting for you to finish connecting {}",
                        service_name(state, &link.service_slug).await
                    ),
                    detail: None,
                    since: watch.created_at,
                    // A flow started before expiry may still finish in its grace.
                    expires_at: Some(link.expires_at).filter(|at| *at > Utc::now()),
                });
            }
            _ => {}
        }
    }
    let channels: Vec<NyxbotChannel> = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find(
            doc! {"user_id": owner, "source_conversation_id": conversation_id,
            "status": "active", "owner_sender_ids": {"$size": 0},
            "link_code_expires_at": {"$gt": now}},
        )
        .limit(10)
        .await?
        .try_collect()
        .await?;
    for row in channels {
        let bot = row
            .bot_username
            .as_deref()
            .map(|name| format!("@{}", name.trim_start_matches('@')))
            .unwrap_or_else(|| excerpt(row.bot_label.trim(), 60));
        items.push(WaitingItem {
            kind: "owner_verification",
            title: format!(
                "Waiting for you to verify your {} account with {bot}",
                platform_name(&row.platform)
            ),
            detail: match super::org_access_holds(state, &row).await? {
                true => verification_hint(state, &row).await?,
                false => None,
            },
            since: row.updated_at,
            expires_at: row.link_code_expires_at,
        });
    }
    Ok(items)
}

/// A watched connect link that is still open.
async fn connect_link(
    state: &AppState,
    owner: &str,
    watch: &NyxbotWatch,
) -> AppResult<Option<crate::models::connect_link::ConnectLink>> {
    use crate::models::connect_link::{COLLECTION_NAME, ConnectLink, ConnectLinkStatus};
    let Some(id) = watch.connect_link_id.as_deref() else {
        return Ok(None);
    };
    Ok(state
        .db
        .collection::<ConnectLink>(COLLECTION_NAME)
        .find_one(doc! {"_id": id, "user_id": owner})
        .await?
        .filter(|link| link.status == ConnectLinkStatus::Pending))
}

async fn service_name(state: &AppState, slug: &str) -> String {
    state
        .db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"slug": slug})
        .projection(doc! {"name": 1})
        .await
        .ok()
        .flatten()
        .and_then(|row| {
            row.get_str("name")
                .ok()
                .map(|name| excerpt(name.trim(), 60))
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| excerpt(slug, 60))
}

/// While the owner has not verified a channel, what NyxID saw from its bot
/// since the current code was issued: nothing at all (the platform is not
/// delivering), a message another route took, or one that reached the agent
/// without the code. `None` once verified or when no code is out.
pub(crate) async fn verification_hint(
    state: &AppState,
    row: &NyxbotChannel,
) -> AppResult<Option<String>> {
    let Some(expires_at) = row.link_code_expires_at else {
        return Ok(None);
    };
    if !row.owner_sender_ids.is_empty() || row.status != "active" || expires_at <= Utc::now() {
        return Ok(None);
    }
    let issued = expires_at - ChronoDuration::hours(super::LINK_CODE_TTL_HOURS);
    let newest = state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(
            doc! {"user_id": super::bot_owner(row), "channel_bot_id": &row.channel_bot_id,
            "direction": "inbound", "created_at": {"$gt": bson::DateTime::from_chrono(issued)}},
        )
        .sort(doc! {"created_at": -1})
        .await?;
    let platform = platform_name(&row.platform);
    Ok(Some(match newest {
        None => match twin_bot(state, row).await? {
            // The same app registered twice: the platform delivers to one.
            Some(twin) => format!(
                "NyxID has not received any message for this bot, but another of your NyxID \
                bots, {} ({}), is registered for the same {platform} app, and the app sends \
                events to only one of them. Point the app's event subscription at this bot \
                ({}) and publish a new app version, or link the agent to {} instead and \
                delete the duplicate.",
                excerpt(twin.label.trim(), 60),
                twin.id,
                webhook_url(state, row),
                excerpt(twin.label.trim(), 60),
            ),
            None if manual_webhook(&row.platform) => format!(
                "NyxID has not received any message from this bot yet. If the code was already \
                sent, check the event subscription (Request URL) in the {platform} developer \
                console: it must be {} and the app version must be published.",
                webhook_url(state, row),
            ),
            None => "NyxID has not received any message from this bot yet. If the code was \
                already sent, check the bot's event subscription on its page in NyxID."
                .to_owned(),
        },
        Some(message) if row.route_id.as_deref() != Some(message.conversation_id.as_str()) => {
            if row.bot_owner_id.is_some() {
                // A shared org bot: the newest message may be anyone's.
                "Messages to this organization bot reach NyxID, but the newest went to another \
                route on it. If it was yours, a chat-specific route takes your chat; remove it \
                so your messages reach the agent."
                    .to_owned()
            } else {
                "A message reached NyxID, but another route on this bot (from an earlier setup) \
                took it, so it never reached the agent. Remove that route to fix it."
                    .to_owned()
            }
        }
        Some(_) => "A message reached the agent, but not the current code in a private chat \
            with the bot."
            .to_owned(),
    }))
}

/// Platforms whose event subscription URL the owner enters by hand (their
/// stored platform is never canonicalised, so the URL below is exact).
pub(crate) fn manual_webhook(platform: &str) -> bool {
    matches!(platform, "lark" | "feishu")
}

/// Where the platform must deliver this bot's events (exact for platforms
/// with `manual_webhook`).
pub(crate) fn webhook_url(state: &AppState, row: &NyxbotChannel) -> String {
    format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        state.config.base_url.trim_end_matches('/'),
        row.platform,
        row.channel_bot_id
    )
}

/// Another active bot the owner manages (theirs or an org's they administer)
/// registered for the same platform app (same app ID, or the same platform
/// bot ID): only one of them can receive the app's events.
pub(crate) async fn twin_bot(
    state: &AppState,
    row: &NyxbotChannel,
) -> AppResult<Option<crate::models::channel_bot::ChannelBot>> {
    use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
    let bots = state.db.collection::<ChannelBot>(BOTS);
    let Some(bot) = bots
        .find_one(doc! {"_id": &row.channel_bot_id, "user_id": super::bot_owner(row)})
        .await?
    else {
        return Ok(None);
    };
    let app_id = bot.app_id.as_deref().filter(|id| !id.is_empty());
    let platform_bot_id = Some(bot.platform_bot_id.as_str()).filter(|id| !id.is_empty());
    if app_id.is_none() && platform_bot_id.is_none() {
        return Ok(None);
    }
    let family = super::canonical_platform(&bot.platform);
    // Every bot the owner manages (theirs and their orgs'), newest first: the
    // other registration may well belong to an org.
    Ok(
        crate::services::channel_bot_service::list_all_bots(&state.db, &row.user_id)
            .await?
            .into_iter()
            .find(|other| {
                other.id != bot.id
                    && super::canonical_platform(&other.platform) == family
                    && ((app_id.is_some() && other.app_id.as_deref() == app_id)
                        || (platform_bot_id.is_some()
                            && Some(other.platform_bot_id.as_str()) == platform_bot_id))
            }),
    )
}

// ---------------------------------------------------------------------------
// Delivery health
// ---------------------------------------------------------------------------

/// How NyxID judged one inbound message.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Delivered,
    Interrupted,
    MediaFailed(&'static str),
    /// Stable code: `refused_{status}`, `undelivered`, or `not_received`.
    Lost(String),
    /// Says nothing about delivery (e.g. a group message or photo the
    /// gateway may drop on purpose): passed over without changing the status.
    Skip,
    /// Too early to tell; judge it on a later sweep.
    Unknown,
}

fn failure_code(http_status: Option<u16>) -> String {
    match http_status {
        Some(status) => format!("refused_{status}"),
        None => "undelivered".into(),
    }
}

/// First failed selector in source order determines the channel's summary.
/// Never copy an arbitrary persisted status into API errors or UI prose.
fn media_failure_code(status: &str) -> &'static str {
    match status {
        "too_large" => "media_too_large",
        "source_unavailable" => "media_source_unavailable",
        "unsupported_or_invalid" => "media_unsupported_or_invalid",
        "rate_limited" => "media_rate_limited",
        "fetch_or_store_failed" => "media_fetch_or_store_failed",
        "timeout" | "not_processed: timeout" => "media_timeout",
        "not_processed: stopped" => "media_stopped",
        "not_processed: expired" => "media_expired",
        _ => "media_failed",
    }
}

fn media_verdict(event: &NyxbotEvent) -> Verdict {
    if let Some(media) = &event.media {
        if media.interrupted {
            return Verdict::Interrupted;
        }
        if let Some(outcome) = media
            .items
            .iter()
            .filter_map(|item| item.outcome.as_ref())
            .find(|outcome| outcome.status != "bound")
        {
            return Verdict::MediaFailed(media_failure_code(&outcome.status));
        }
        // A turn binding precedes ingestion. Advancing delivery_seen_at now
        // would permanently hide an ingestion failure recorded by the run.
        if media.items.iter().any(|item| item.outcome.is_none())
            && !matches!(event.status.as_str(), "completed" | "failed" | "refused")
        {
            return Verdict::Unknown;
        }
    }
    if event.conversation_id.is_some()
        || matches!(event.status.as_str(), "completed" | "failed" | "refused")
    {
        Verdict::Delivered
    } else {
        Verdict::Unknown
    }
}

/// Plain words for a delivery failure code, for the agent and the owner.
pub(crate) fn failure_reason(code: &str, transport: &str) -> String {
    let receiver = if transport == "gateway" {
        "the Agent Event Gateway"
    } else {
        "NyxID's relay"
    };
    if code == "media_interrupted" {
        return "a photo or file was accepted but interrupted before processing".into();
    }
    let media_reason = match code {
        "media_too_large" => Some("a photo or file exceeded the size limit"),
        "media_source_unavailable" => {
            Some("a photo or file source was unavailable or no longer authorized")
        }
        "media_unsupported_or_invalid" => {
            Some("a photo or file had an unsupported or invalid format")
        }
        "media_rate_limited" => {
            Some("a photo or file could not be processed because the upload rate limit was reached")
        }
        "media_fetch_or_store_failed" => Some("a photo or file could not be downloaded or stored"),
        "media_timeout" => Some("photo or file processing timed out"),
        "media_stopped" => Some("the turn stopped before a photo or file was processed"),
        "media_expired" => Some("a photo or file expired in the queue before processing"),
        "media_failed" => Some("a photo or file could not be processed"),
        _ => None,
    };
    if let Some(reason) = media_reason {
        return reason.into();
    }
    if code == "not_received" {
        return format!("{receiver} accepted it but never passed it on");
    }
    if code == "routed_elsewhere" {
        return "another route on this bot, from an earlier setup, sent it to a different agent"
            .into();
    }
    if code == "undelivered" {
        return format!("NyxID could not deliver it to {receiver}");
    }
    match code.strip_prefix("refused_") {
        Some(status) => format!("{receiver} refused it (HTTP {status})"),
        None => "it could not be delivered".into(),
    }
}

/// Whether an inbound message came from a private chat. Telegram's private
/// chat ID is the sender's; elsewhere (Lark's `oc_` chats) NyxID knows from the
/// chat it recorded.
async fn private_message(state: &AppState, row: &NyxbotChannel, message: &ChannelMessage) -> bool {
    let Some(chat_id) = message.platform_conversation_id.as_deref() else {
        return false;
    };
    if message.sender_platform_id.as_deref() == Some(chat_id) {
        return true;
    }
    row.platform != "telegram"
        && state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"user_id": &row.user_id, "channel_id": &row.id,
            "platform_chat_id": chat_id, "kind": "private"})
            .await
            .is_ok_and(|chat| chat.is_some())
}

async fn judge(state: &AppState, row: &NyxbotChannel, message: &ChannelMessage) -> Verdict {
    // Resolve media aliases before transport/content shortcuts. A suppressed
    // redelivery inherits the winner and can never trigger Lost/resend advice.
    let alias = message.duplicate_of_media_key.as_deref();
    let filter = match alias {
        Some(key) => doc! {"user_id":&row.user_id,"media.key":key},
        None => {
            doc! {"channel_id":&row.id,"user_id":&row.user_id,"event_id":&message.id,"media.key":{"$type":"string"}}
        }
    };
    match state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(filter)
        .await
    {
        Ok(Some(event)) => {
            return media_verdict(&event);
        }
        Ok(None) if alias.is_some() => return Verdict::Skip,
        Err(_) => return Verdict::Unknown,
        _ => {}
    }
    match message.callback_status.as_deref() {
        Some("failed") => Verdict::Lost(failure_code(message.callback_http_status)),
        // NyxID's own relay receiver accepted it: the agent has it.
        Some("delivered") if row.transport != "gateway" => Verdict::Delivered,
        Some("delivered") => {
            if matches!(
                message.callback_disposition,
                Some(CallbackDisposition::Refused { .. })
            ) {
                return Verdict::Skip;
            }
            let plain_text = message.content_type == "text"
                && message.attachments.is_empty()
                && message.embedded_attachments.is_empty()
                && message.activity.is_none();
            let eligible_media = message.platform == row.platform
                && matches!(row.platform.as_str(), "lark" | "feishu")
                && matches!(
                    message.callback_disposition,
                    Some(CallbackDisposition::Accepted)
                )
                && message.activity.is_none()
                && (!message.attachments.is_empty() || !message.embedded_attachments.is_empty())
                && message
                    .attachments
                    .iter()
                    .chain(&message.embedded_attachments)
                    .all(|item| matches!(item.content_type.as_str(), "image" | "file"))
                && message
                    .sender_platform_id
                    .as_ref()
                    .is_some_and(|sender| row.owner_sender_ids.contains(sender));
            let supported_media = if eligible_media {
                match crate::services::feature_flag_service::personal_flag_enabled(
                    &state.db,
                    &row.user_id,
                    crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
                )
                .await
                {
                    Ok(enabled) => enabled,
                    Err(_) => return Verdict::Unknown,
                }
            } else {
                false
            };
            let age = Utc::now() - message.created_at;
            if !(plain_text || supported_media)
                || age > ChronoDuration::hours(JUDGE_WINDOW_HOURS)
                || !private_message(state, row, message).await
            {
                return Verdict::Skip;
            }
            // The gateway's event ID for a relayed message is NyxID's own
            // message ID, so the admission record matches it exactly.
            let received = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"channel_id": &row.id, "event_id": &message.id})
                .await;
            match received {
                Ok(Some(_)) => Verdict::Delivered,
                Ok(None) if age > ChronoDuration::seconds(NOT_RECEIVED_GRACE_SECS) => {
                    Verdict::Lost("not_received".into())
                }
                _ => Verdict::Unknown,
            }
        }
        _ => Verdict::Unknown,
    }
}

/// Check a batch of linked chat apps: if their newest messages are not
/// reaching the agent, flag the channel and tell the agent once, so it can
/// explain to the owner instead of the chat going quiet.
pub(crate) async fn check_deliveries(state: &AppState) -> AppResult<()> {
    let rows: Vec<NyxbotChannel> = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find(doc! {"status": "active", "route_id": {"$ne": null}})
        .sort(doc! {"delivery_checked_at": 1, "created_at": 1})
        .limit(DELIVERY_BATCH)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_many(
            doc! {"_id": {"$in": ids}},
            doc! {"$set": {"delivery_checked_at": bson::DateTime::now()}},
        )
        .await?;
    for row in rows {
        if let Err(error) = check_delivery(state, &row).await {
            tracing::debug!(%error, "NyxBot delivery check deferred");
        }
        // A gateway update that failed or raced is retried here.
        match super::chats::sync_gateway_groups(state, &row, false).await {
            Ok(Some(code)) => tracing::debug!(code, "NyxBot gateway group admission pending"),
            Ok(None) => {}
            Err(error) => tracing::debug!(%error, "NyxBot gateway group admission deferred"),
        }
    }
    Ok(())
}

async fn check_delivery(state: &AppState, row: &NyxbotChannel) -> AppResult<()> {
    // A demoted or removed org admin's link to the org's bot is released,
    // even if no message ever arrives to trigger it.
    if !super::org_access_holds(state, row).await? {
        return super::release_org_channel(state, row).await;
    }
    let Some(route_id) = row.route_id.as_deref() else {
        return Ok(());
    };
    // The first check looks back briefly only, so failures from before a
    // fix (or before this check existed) raise no stale alarm.
    let since = row.delivery_seen_at.unwrap_or_else(|| {
        row.created_at
            .max(Utc::now() - ChronoDuration::hours(FIRST_LOOKBACK_HOURS))
    });
    let messages: Vec<ChannelMessage> = state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find(
            doc! {"conversation_id": route_id, "user_id": super::bot_owner(row),
            "direction": "inbound", "created_at": {"$gt": bson::DateTime::from_chrono(since)}},
        )
        .sort(doc! {"created_at": -1})
        .limit(MESSAGES_PER_CHECK)
        .await?
        .try_collect()
        .await?;
    // Newest first: the newest judged message decides. Messages still in
    // flight are judged on a later sweep, so the check never moves past them;
    // skipped messages older than all of those only move the check forward.
    let mut verdict: Option<(Verdict, DateTime<Utc>)> = None;
    let mut skipped_to: Option<DateTime<Utc>> = None;
    let mut in_flight = false;
    for message in &messages {
        match judge(state, row, message).await {
            Verdict::Unknown => in_flight = true,
            Verdict::Skip => {
                if skipped_to.is_none() || in_flight {
                    skipped_to = Some(message.created_at);
                    in_flight = false;
                }
            }
            judged => {
                verdict = Some((judged, message.created_at));
                break;
            }
        }
    }
    // The owner's private chat captured by another route on the bot (a
    // chat-specific route beats this channel's default route): newer than
    // anything judged here, that is what the owner experiences.
    let owner_chat_elsewhere = state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(
            doc! {"user_id": super::bot_owner(row), "channel_bot_id": &row.channel_bot_id,
            "direction": "inbound", "conversation_id": {"$ne": route_id},
            "created_at": {"$gt": bson::DateTime::from_chrono(since)}},
        )
        .sort(doc! {"created_at": -1})
        .await?
        .filter(|message| {
            let private = message.platform_conversation_id.is_some()
                && message.platform_conversation_id == message.sender_platform_id;
            // Until the owner verifies, only a personal bot's private chats can
            // be assumed to be theirs; an org bot's may be any member's.
            let owner = match row.owner_sender_ids.is_empty() {
                true => row.bot_owner_id.is_none(),
                false => message
                    .sender_platform_id
                    .as_ref()
                    .is_some_and(|sender| row.owner_sender_ids.contains(sender)),
            };
            private && owner
        });
    if let Some(message) = owner_chat_elsewhere
        && verdict
            .as_ref()
            .is_none_or(|(_, at)| message.created_at > *at)
    {
        verdict = Some((Verdict::Lost("routed_elsewhere".into()), message.created_at));
    }
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    let active = doc! {"_id": &row.id, "status": "active"};
    let Some((verdict, seen_at)) = verdict else {
        if let Some(at) = skipped_to.filter(|_| !in_flight) {
            channels
                .update_one(
                    active,
                    doc! {"$set": {"delivery_seen_at": bson::DateTime::from_chrono(at)}},
                )
                .await?;
        }
        return Ok(());
    };
    let seen = bson::DateTime::from_chrono(seen_at);
    match verdict {
        Verdict::Interrupted | Verdict::MediaFailed(_) => {
            let (status, code) = match verdict {
                Verdict::MediaFailed(code) => ("media_failed", code),
                _ => ("interrupted", "media_interrupted"),
            };
            channels
                .update_one(
                    active,
                    doc! {"$set":{"delivery_status":status,
                    "delivery_error":code,"delivery_failed_at":seen,"delivery_seen_at":seen}},
                )
                .await?;
        }
        Verdict::Lost(code) => {
            let Some(before) = channels
                .find_one_and_update(
                    active,
                    doc! {"$set": {"delivery_status": "failing", "delivery_error": &code,
                    "delivery_failed_at": seen, "delivery_seen_at": seen}},
                )
                .await?
            else {
                // Disconnected meanwhile: nothing to flag or announce.
                return Ok(());
            };
            if before.delivery_status.as_deref() != Some("failing") {
                super::audit(
                    state,
                    &row.user_id,
                    "nyxbot_channel_delivery_failing",
                    serde_json::json!({"channel_agent_id": &row.id, "platform": &row.platform,
                        "error_code": &code}),
                )
                .await;
            }
            // Every newly lost message may tell the agent; the notice claim
            // throttles it to once per window, so a relapse or a lasting
            // failure is never left unannounced for long.
            notify_failing(state, row, &code, seen_at).await?;
        }
        _ => {
            channels
                .update_one(
                    active,
                    doc! {"$set": {"delivery_status": "ok", "delivery_seen_at": seen},
                    "$unset": {"delivery_error": "", "delivery_failed_at": ""}},
                )
                .await?;
        }
    }
    Ok(())
}

/// Tell the channel's agent (once per window) that messages are lost.
async fn notify_failing(
    state: &AppState,
    row: &NyxbotChannel,
    code: &str,
    at: DateTime<Utc>,
) -> AppResult<()> {
    let cutoff =
        bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(DELIVERY_NOTICE_HOURS));
    let claimed = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id, "$or": [{"delivery_notified_at": null},
            {"delivery_notified_at": {"$lt": cutoff}}]},
            doc! {"$set": {"delivery_notified_at": bson::DateTime::now()}},
        )
        .await?
        .modified_count
        == 1;
    if !claimed {
        return Ok(());
    }
    let agent = match row.agent_id.as_deref() {
        Some(id) => crate::services::assistant_team_service::agent(&state.db, &row.user_id, id)
            .await
            .ok(),
        None => None,
    };
    let agent = match agent {
        Some(agent) if agent.destroyed_at.is_none() => agent,
        _ => {
            crate::services::assistant_team_service::ensure_nyxbot(&state.db, &row.user_id).await?
        }
    };
    // The thread that set the bot up, if it still exists; else the agent's home.
    let source = match row.source_conversation_id.as_deref() {
        Some(id) => state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id": id, "user_id": &row.user_id})
            .projection(doc! {"_id": 1})
            .await?
            .map(|_| id.to_owned()),
        None => None,
    };
    let target = match source {
        Some(id) => id,
        None => {
            crate::services::assistant_team_service::home_thread_for(
                &state.db,
                &state.encryption_keys,
                &row.user_id,
                &agent,
            )
            .await?
            .id
        }
    };
    let platform = platform_name(&row.platform);
    let remedy = if code == "routed_elsewhere" {
        "List the bot's routes with nyxid__list_channel_routes, show the user which one takes \
        their chat, and with their OK remove it with nyxid__delete_channel_route; then ask them \
        to send their message (or code) again."
    } else {
        "Tell the user this is a NyxID delivery problem, not something they did; they can keep \
        using this chat meanwhile. If it continues, reconnecting the bot \
        (nyxid__connect_channel_bot) rebuilds its link from scratch (the owner stays verified)."
    };
    let text = format!(
        "Messages sent to the {platform} bot {} are not reaching {}: {} (newest at {} UTC). \
        Tell the user plainly that their {platform} messages did not arrive. {remedy} No \
        confirmation is needed.",
        identifier(&row.bot_label),
        identifier(&agent.name),
        failure_reason(code, &row.transport),
        at.format("%Y-%m-%d %H:%M"),
    );
    super::super::assistant_team::notify(
        state,
        &row.user_id,
        &target,
        vec![crate::services::assistant_team_service::bound_event(
            "channel_delivery_failing",
            text,
            None,
            None,
        )],
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::channel_message::{CallbackRefusal, StoredAttachment},
        test_utils::{connect_transaction_test_database, test_app_state, test_user},
    };
    use uuid::Uuid;

    async fn fixture(name: &str) -> (AppState, NyxbotChannel, ChannelMessage) {
        let db = connect_transaction_test_database(name).await;
        let owner = Uuid::new_v4().to_string();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&owner, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let now = bson::DateTime::now();
        let row: NyxbotChannel = bson::from_document(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": &owner,
            "channel_bot_id": Uuid::new_v4().to_string(), "platform":"lark", "bot_label":"Receipts",
            "transport":"gateway", "status":"active", "route_api_key_id":"key",
            "route_id": Uuid::new_v4().to_string(), "owner_sender_ids":["human"],
            "created_at":bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(1)), "updated_at":now,
        }).unwrap();
        let bot: crate::models::channel_bot::ChannelBot = bson::from_document(doc! {
            "_id": &row.channel_bot_id, "user_id": &owner, "platform": "lark",
            "label": "Receipts", "platform_bot_id": "bot", "platform_bot_username": "receipts",
            "bot_token_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic, bytes: vec![],
            },
            "webhook_secret_hash": "hash", "webhook_registered": true, "status": "active",
            "is_active": true, "created_at": now, "updated_at": now,
        })
        .unwrap();
        db.collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .insert_one(bot)
        .await
        .unwrap();
        db.collection::<NyxbotChannel>(CHANNELS)
            .insert_one(&row)
            .await
            .unwrap();
        let thread = doc! {
            "_id":Uuid::new_v4().to_string(), "user_id":&owner, "channel_id":&row.id,
            "partition":"private", "kind":"private", "platform_chat_id":"oc_private",
            "created_at":now,"updated_at":now,
        };
        db.collection::<bson::Document>(THREADS)
            .insert_one(thread)
            .await
            .unwrap();
        let mut message: ChannelMessage = bson::from_document(doc! {
            "_id":Uuid::new_v4().to_string(), "user_id":&owner,"conversation_id":&row.route_id,
            "channel_bot_id":&row.channel_bot_id,"direction":"inbound","platform":"lark",
            "platform_conversation_id":"oc_private","sender_platform_id":"human",
            "content_type":"image","callback_status":"delivered","callback_http_status":202,
            "created_at":bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(180)),
        })
        .unwrap();
        message.callback_disposition = Some(CallbackDisposition::Accepted);
        message.attachments = vec![StoredAttachment {
            content_type: "image".into(),
            provider_ref: "opaque".into(),
            platform_message_id: None,
            file_key: None,
            image_key: None,
            filename: None,
            mime_type: None,
            size_bytes: None,
        }];
        (test_app_state(db), row, message)
    }

    async fn enable_media(state: &AppState, row: &NyxbotChannel) {
        state
            .db
            .collection::<bson::Document>("feature_flag_overrides")
            .insert_one(doc! {
                "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
                "flag_key":crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
                "target_kind":"user","target_key":&row.user_id,"enabled":true,"updated_by":"test",
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            })
            .await
            .unwrap();
    }

    fn event(row: &NyxbotChannel, message: &ChannelMessage) -> NyxbotEvent {
        bson::from_document(doc! {
            "_id":Uuid::new_v4().to_string(),"channel_id":&row.id,"user_id":&row.user_id,
            "partition":"private","event_id":&message.id,"status":"running",
            "conversation_id":"conversation","created_at":bson::DateTime::now(),
            "expires_at":bson::DateTime::from_chrono(Utc::now() + ChronoDuration::days(7)),
            "media":{"key":"a".repeat(64),"declared":1_i32,"platform_chat_id":"oc_private",
                "items":[{"index":0_i32,"embedded":false,"kind":"image"}]},
        })
        .unwrap()
    }

    #[tokio::test]
    async fn channel_media_status_callback_eligibility_and_grace_matrix() {
        let (state, mut row, mut message) = fixture("media_status_eligibility").await;
        assert_eq!(
            judge(&state, &row, &message).await,
            Verdict::Skip,
            "media disabled"
        );
        enable_media(&state, &row).await;
        for platform in ["lark", "feishu"] {
            row.platform = platform.into();
            message.platform = platform.into();
            assert_eq!(
                judge(&state, &row, &message).await,
                Verdict::Lost("not_received".into()),
                "accepted private image"
            );
            message.embedded_attachments = std::mem::take(&mut message.attachments);
            message.content_type = "unknown".into();
            assert_eq!(
                judge(&state, &row, &message).await,
                Verdict::Lost("not_received".into()),
                "accepted private post"
            );
            message.attachments = std::mem::take(&mut message.embedded_attachments);
            message.content_type = "image".into();
        }
        for reason in [
            CallbackRefusal::ContentUnsupported,
            CallbackRefusal::AttachmentUnsupported,
            CallbackRefusal::GroupNotAddressed,
        ] {
            message.callback_disposition = Some(CallbackDisposition::Refused { reason });
            assert_eq!(judge(&state, &row, &message).await, Verdict::Skip);
        }
        for disposition in [None, Some(CallbackDisposition::Unknown)] {
            message.callback_disposition = disposition;
            assert_eq!(
                judge(&state, &row, &message).await,
                Verdict::Skip,
                "legacy or malformed response"
            );
        }
        message.callback_disposition = Some(CallbackDisposition::Accepted);
        let attachment = message.attachments.pop().unwrap();
        for kind in ["sticker", "unknown"] {
            message.content_type = kind.into();
            assert_eq!(
                judge(&state, &row, &message).await,
                Verdict::Skip,
                "sticker or malformed post"
            );
        }
        message.attachments.push(attachment);
        message.content_type = "image".into();
        message.platform_conversation_id = Some("oc_group".into());
        assert_eq!(
            judge(&state, &row, &message).await,
            Verdict::Skip,
            "unaddressed group"
        );
        message.platform_conversation_id = Some("oc_private".into());
        message.sender_platform_id = Some("guest".into());
        assert_eq!(
            judge(&state, &row, &message).await,
            Verdict::Skip,
            "guests do not ingest"
        );
        message.sender_platform_id = Some("human".into());
        message.created_at = Utc::now() - ChronoDuration::seconds(10);
        assert_eq!(judge(&state, &row, &message).await, Verdict::Unknown);
        message.created_at = Utc::now() - ChronoDuration::hours(13);
        assert_eq!(judge(&state, &row, &message).await, Verdict::Skip);
        message.attachments.clear();
        message.content_type = "text".into();
        message.callback_disposition = None;
        message.created_at = Utc::now() - ChronoDuration::seconds(180);
        assert_eq!(
            judge(&state, &row, &message).await,
            Verdict::Lost("not_received".into()),
            "legacy text unchanged"
        );
        row.transport = "direct".into();
        assert_eq!(judge(&state, &row, &message).await, Verdict::Delivered);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn channel_media_status_waits_for_outcomes_and_reports_failure_classes() {
        let (state, row, message) = fixture("media_status_outcomes").await;
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(&message)
            .await
            .unwrap();
        let mut event = event(&row, &message);
        let events = state.db.collection::<NyxbotEvent>(EVENTS);
        events.insert_one(&event).await.unwrap();
        assert_eq!(
            judge(&state, &row, &message).await,
            Verdict::Unknown,
            "binding alone is not ingestion success"
        );
        check_delivery(&state, &row).await.unwrap();
        let pending = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            pending.delivery_seen_at.is_none(),
            "later outcome must still be judged"
        );
        for (status, code) in [
            ("too_large", "media_too_large"),
            ("source_unavailable", "media_source_unavailable"),
            ("unsupported_or_invalid", "media_unsupported_or_invalid"),
            ("rate_limited", "media_rate_limited"),
            ("fetch_or_store_failed", "media_fetch_or_store_failed"),
            ("timeout", "media_timeout"),
            ("not_processed: timeout", "media_timeout"),
            ("not_processed: stopped", "media_stopped"),
            ("not_processed: expired", "media_expired"),
            ("not_processed: failed", "media_failed"),
            ("untrusted status text", "media_failed"),
        ] {
            event.media.as_mut().unwrap().items[0].outcome =
                Some(crate::models::assistant_channel_media::MediaOutcome {
                    event_id: event.id.clone(),
                    event_key: "a".repeat(64),
                    index: Some(0),
                    embedded: false,
                    status: status.into(),
                    upload_id: None,
                });
            events
                .replace_one(doc! {"_id":&event.id}, &event)
                .await
                .unwrap();
            assert_eq!(
                judge(&state, &row, &message).await,
                Verdict::MediaFailed(code),
                "{status}"
            );
            assert_ne!(failure_reason(code, "gateway"), "it could not be delivered");
            assert!(!failure_reason(code, "gateway").contains("untrusted"));
        }
        check_delivery(&state, &row).await.unwrap();
        let failed = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.delivery_status.as_deref(), Some("media_failed"));
        assert_eq!(failed.delivery_error.as_deref(), Some("media_failed"));
        assert!(
            failed.delivery_notified_at.is_none(),
            "ingestion failures do not claim delivery loss"
        );
        event.media.as_mut().unwrap().items[0]
            .outcome
            .as_mut()
            .unwrap()
            .status = "bound".into();
        events
            .replace_one(doc! {"_id":&event.id}, &event)
            .await
            .unwrap();
        assert_eq!(judge(&state, &row, &message).await, Verdict::Delivered);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn channel_media_status_duplicates_reuse_failed_interrupted_and_pending_winners() {
        let (state, row, mut message) = fixture("media_status_duplicates").await;
        let mut event = event(&row, &message);
        event.event_id = "winning-relay-uuid".into();
        message.duplicate_of_media_key = Some("a".repeat(64));
        let events = state.db.collection::<NyxbotEvent>(EVENTS);
        events.insert_one(&event).await.unwrap();
        assert_eq!(judge(&state, &row, &message).await, Verdict::Unknown);
        event.media.as_mut().unwrap().interrupted = true;
        events
            .replace_one(doc! {"_id":&event.id}, &event)
            .await
            .unwrap();
        assert_eq!(judge(&state, &row, &message).await, Verdict::Interrupted);
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(&message)
            .await
            .unwrap();
        check_delivery(&state, &row).await.unwrap();
        let interrupted = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(interrupted.delivery_status.as_deref(), Some("interrupted"));
        assert_eq!(
            interrupted.delivery_error.as_deref(),
            Some("media_interrupted")
        );
        assert!(interrupted.delivery_notified_at.is_none());
        event.media.as_mut().unwrap().interrupted = false;
        event.media.as_mut().unwrap().items[0].outcome =
            Some(crate::models::assistant_channel_media::MediaOutcome {
                event_id: event.id.clone(),
                event_key: "a".repeat(64),
                index: Some(0),
                embedded: false,
                status: "source_unavailable".into(),
                upload_id: None,
            });
        events
            .replace_one(doc! {"_id":&event.id}, &event)
            .await
            .unwrap();
        assert_eq!(
            judge(&state, &row, &message).await,
            Verdict::MediaFailed("media_source_unavailable")
        );
        events.delete_one(doc! {"_id":&event.id}).await.unwrap();
        assert_eq!(
            judge(&state, &row, &message).await,
            Verdict::Skip,
            "expired winner never becomes Lost"
        );
        state.db.drop().await.unwrap();
    }
}
