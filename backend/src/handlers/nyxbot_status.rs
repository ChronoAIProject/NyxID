//! What a NyxBot thread is waiting on, and whether a linked chat app's
//! messages actually reach its agent. Both exist so nothing goes quiet
//! without the owner being able to tell why.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Serialize;

use super::{
    CHANNELS, EVENTS, NyxbotChannel, NyxbotEvent, NyxbotWatch, WATCHES, excerpt, identifier,
};
use crate::{
    AppState,
    errors::AppResult,
    models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
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
            "channel_bot" => items.push(WaitingItem {
                kind: "channel_bot",
                title: format!(
                    "Waiting for your {} bot to be created",
                    platform_name(watch.platform.as_deref().unwrap_or("chat app"))
                ),
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

// ---------------------------------------------------------------------------
// Delivery health
// ---------------------------------------------------------------------------

/// How NyxID judged one inbound message.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Delivered,
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

/// Plain words for a delivery failure code, for the agent and the owner.
pub(crate) fn failure_reason(code: &str, transport: &str) -> String {
    let receiver = if transport == "gateway" {
        "the Agent Event Gateway"
    } else {
        "NyxID's relay"
    };
    if code == "not_received" {
        return format!("{receiver} accepted it but never passed it on");
    }
    if code == "undelivered" {
        return format!("NyxID could not deliver it to {receiver}");
    }
    match code.strip_prefix("refused_") {
        Some(status) => format!("{receiver} refused it (HTTP {status})"),
        None => "it could not be delivered".into(),
    }
}

async fn judge(state: &AppState, row: &NyxbotChannel, message: &ChannelMessage) -> Verdict {
    match message.callback_status.as_deref() {
        Some("failed") => Verdict::Lost(failure_code(message.callback_http_status)),
        // NyxID's own relay receiver accepted it: the agent has it.
        Some("delivered") if row.transport != "gateway" => Verdict::Delivered,
        Some("delivered") => {
            // The gateway may accept a message and drop it on purpose: group
            // messages outside its admission policy, and anything but plain
            // text (it refuses photos and system events with a 202). Only a
            // private plain-text message must reach the provider.
            let private = message.platform_conversation_id.is_some()
                && message.platform_conversation_id == message.sender_platform_id;
            let plain_text = message.content_type == "text"
                && message.attachments.is_empty()
                && message.activity.is_none();
            let age = Utc::now() - message.created_at;
            if !private || !plain_text || age > ChronoDuration::hours(JUDGE_WINDOW_HOURS) {
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
    }
    Ok(())
}

async fn check_delivery(state: &AppState, row: &NyxbotChannel) -> AppResult<()> {
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
        .find(doc! {"conversation_id": route_id, "user_id": &row.user_id,
        "direction": "inbound", "created_at": {"$gt": bson::DateTime::from_chrono(since)}})
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
            crate::services::assistant_team_service::home_thread(
                &state.db,
                &state.encryption_keys,
                &agent,
            )
            .await?
            .id
        }
    };
    let platform = platform_name(&row.platform);
    let text = format!(
        "Messages sent to the {platform} bot {} are not reaching {}: {} (newest at {} UTC). \
        Tell the user plainly that their {platform} messages did not arrive and that this is a \
        NyxID delivery problem, not something they did; they can keep using this chat \
        meanwhile. If it continues, reconnecting the bot (nyxid__connect_channel_bot) rebuilds \
        its link from scratch (the owner stays verified). No confirmation is needed.",
        identifier(&row.bot_label),
        identifier(&agent.name),
        failure_reason(code, &row.transport),
        at.format("%Y-%m-%d %H:%M"),
    );
    super::super::assistant_team::notify(
        state,
        &row.user_id,
        &target,
        vec![crate::services::assistant_team_service::event(
            "channel_delivery_failing",
            text,
            None,
        )],
    )
    .await;
    Ok(())
}
