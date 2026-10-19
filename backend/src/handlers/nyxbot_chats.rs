//! The chats a NyxBot channel bot is in.
//!
//! Every private chat is its own thread; a group, channel or forum topic is
//! one thread its members share, each message attributed to its sender. Per
//! chat the owner chooses whether the agent answers every message or only
//! when it is mentioned or replied to (the default in groups), whether
//! members other than the owner may talk to it, whether the agent may post
//! there on its own, and which agent answers. Members who are not the owner
//! talk as guests: their turns only read with the agent's services and never
//! see the owner's private context (see `mcp_transport::guest_tool_allowed`
//! and `assistant_team::turn_notes`).
use super::*;

use crate::{
    models::channel_conversation::ChannelConversation,
    services::{assistant_team_service as team, channel_adapters::resolve_adapter},
};

pub(super) const MAX_TITLE_CHARS: usize = 80;
const LIST_LIMIT: i64 = 100;
pub(super) const MAX_POST_CHARS: usize = 4000;

/// `private`, `group` or `channel`; anything else counts as a group.
pub(super) fn chat_kind(kind: &str) -> &'static str {
    match kind {
        "private" => "private",
        "channel" => "channel",
        _ => "group",
    }
}

/// A private chat's thread on NyxID's relay: one per chat and sender.
pub(super) fn direct_partition(chat_id: &str, sender_id: &str, thread_id: Option<&str>) -> String {
    format!(
        "direct_{}",
        &sha256_hex(format!(
            "{chat_id}\0{sender_id}\0{}",
            thread_id.unwrap_or_default()
        ))[..32]
    )
}

/// A bot moved from NyxID's relay to the gateway keeps its private chats: the
/// first gateway message of a chat takes over the chat's relay thread (its
/// agent, settings and conversation) under the gateway's conversation, and
/// conversations answering into that chat follow it. The gateway reports the
/// same platform chat and sender IDs NyxID's relay does.
pub(super) async fn adopt_relay_chat(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
    chat_id: &str,
    sender_id: &str,
    thread_id: Option<&str>,
) -> AppResult<()> {
    if sender_id.is_empty() {
        return Ok(());
    }
    let relay_partition = direct_partition(chat_id, sender_id, thread_id);
    let threads = state.db.collection::<NyxbotThread>(THREADS);
    let Some(legacy) = threads
        .find_one(doc! {"channel_id": &row.id, "$or": [
        {"partition": &relay_partition}, {"relay_partition": &relay_partition, "kind": "private"}]})
        .await?
    else {
        return Ok(());
    };
    let legacy = legacy.partition;
    // The gateway conversation carries no chat of its own yet (only its
    // placeholder from `put_conversation`).
    if threads
        .find_one(doc! {"channel_id": &row.id, "partition": partition,
        "kind": {"$ne": null}})
        .await?
        .is_some()
    {
        return Ok(());
    }
    // The gateway may re-create its placeholder in between: once more then.
    let mut attempts = 0;
    loop {
        attempts += 1;
        threads
            .delete_one(doc! {"channel_id": &row.id, "partition": partition, "kind": null})
            .await?;
        match threads
            .update_one(
                doc! {"channel_id": &row.id, "partition": &legacy},
                doc! {"$set": {"partition": partition, "relay_partition": &relay_partition,
                "updated_at": bson::DateTime::now()}},
            )
            .await
        {
            Ok(moved) if moved.matched_count == 1 => break,
            // Another message of the chat got there first.
            Ok(_) => return Ok(()),
            Err(error) if super::is_duplicate(&error) && attempts < 2 => continue,
            Err(error) if super::is_duplicate(&error) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
    let conversations = state
        .db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        );
    for field in ["channel", "reply_channel"] {
        conversations
            .update_many(
                doc! {"user_id": &row.user_id, format!("{field}.nyxbot_channel_id"): &row.id,
                format!("{field}.partition"): &legacy},
                doc! {"$set": {format!("{field}.partition"): partition}},
            )
            .await?;
    }
    Ok(())
}

/// Reuse a gateway-era private chat only with the exact sender/chat/topic
/// mapping supplied by a verified provider event. Ordinary relay bots do no
/// additional reads. Legacy unmapped gateway history stays isolated.
pub(super) async fn partition_after_gateway(
    state: &AppState,
    row: &NyxbotChannel,
    relay_partition: String,
) -> AppResult<String> {
    if row.relay_attempted_at.is_none() {
        return Ok(relay_partition);
    }
    Ok(state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(
            doc! {"channel_id": &row.id, "relay_partition": &relay_partition, "kind": "private"},
        )
        .await?
        .map_or(relay_partition, |chat| chat.partition))
}

/// The shared thread of a group, channel or topic.
pub(super) fn group_partition(chat_id: &str, thread_id: Option<&str>) -> String {
    format!(
        "chat_{}",
        &sha256_hex(format!("{chat_id}\0{}", thread_id.unwrap_or_default()))[..32]
    )
}

/// Private chats answer every message; groups and channels only when
/// mentioned or replied to, unless set to `all`.
pub(super) fn answers_everything(chat: &NyxbotThread) -> bool {
    chat.kind.as_deref() == Some("private") || chat.reply_mode.as_deref() == Some("all")
}

/// Members other than the owner may talk to the agent: when the owner said
/// so, or by default once the owner has talked to the bot there.
fn members_may_talk(chat: &NyxbotThread) -> bool {
    match chat.members.as_deref() {
        Some("everyone") => true,
        Some(_) => false,
        None => chat.owner_seen,
    }
}

fn is_group(chat: &NyxbotThread) -> bool {
    matches!(chat.kind.as_deref(), Some("group" | "channel"))
}

/// Record that the owner talks to the bot in this group: from then on its
/// members may talk to the agent too, unless the owner said otherwise.
pub(super) async fn note_owner_presence(
    state: &AppState,
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    sender_id: &str,
) -> AppResult<NyxbotThread> {
    let mut chat = chat.clone();
    if is_group(&chat) && !chat.owner_seen && row.owner_sender_ids.iter().any(|id| id == sender_id)
    {
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(doc! {"_id": &chat.id}, doc! {"$set": {"owner_seen": true}})
            .await?;
        chat.owner_seen = true;
    }
    Ok(chat)
}

/// A guest's text with every "(owner)" (any ASCII case) unbracketed. ASCII
/// lowercasing keeps every byte offset, so positions carry over.
fn without_owner_mark(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut rest = 0;
    for (at, _) in lower.match_indices("(owner)") {
        out.push_str(&text[rest..at]);
        out.push_str(&text[at + 1..at + 6]);
        rest = at + "(owner)".len();
    }
    out.push_str(&text[rest..]);
    out
}

/// A group message as its thread stores it: the sender's name first, marked
/// `(owner)` for the owner. Names cannot carry the mark and a guest's text is
/// kept on one line, so no one can pass for the owner.
/// A person's name as shared threads show it: one line, without the marks
/// NyxID uses for attribution (`(owner)`, `Name:`).
fn plain_name(name: &str) -> Option<String> {
    let name = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '(' | ')' | '[' | ']' | ':') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!name.is_empty()).then_some(name)
}

pub(super) fn attributed(name: Option<&str>, text: &str, guest: bool) -> String {
    let name = name
        .and_then(plain_name)
        .map(|name| name.chars().take(60).collect::<String>());
    if guest {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        format!(
            "{}: {}",
            name.as_deref().unwrap_or("Someone"),
            without_owner_mark(&text)
        )
    } else {
        format!("{} (owner): {text}", name.as_deref().unwrap_or("Owner"))
    }
}

pub(super) fn clean_title(title: &str) -> Option<String> {
    let title = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
    (!title.is_empty()).then_some(title)
}

/// What NyxID knows about an inbound message's chat.
pub(super) struct ChatFacts {
    pub kind: &'static str,
    pub chat_id: String,
    pub thread_id: Option<String>,
    /// Private chats: the sender is the verified owner.
    pub owner: bool,
    /// The group's name, or the sender's name in a private chat.
    pub title: Option<String>,
}

/// Record the chat of an inbound message (and, for direct channels, the
/// message to reply to). Returns the chat with its settings.
pub(super) async fn record_chat(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
    facts: &ChatFacts,
    last_message_id: Option<&str>,
) -> AppResult<NyxbotThread> {
    record_chat_with_relay(state, row, partition, facts, last_message_id, None).await
}

pub(super) async fn record_chat_with_relay(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
    facts: &ChatFacts,
    last_message_id: Option<&str>,
    relay_message_id: Option<&str>,
) -> AppResult<NyxbotThread> {
    let now = bson::DateTime::now();
    let mut set = doc! {"updated_at": now, "last_message_at": now, "kind": facts.kind,
    "platform_chat_id": &facts.chat_id};
    if facts.kind == "private" {
        set.insert("owner_chat", facts.owner);
    }
    if let Some(thread_id) = facts.thread_id.as_deref() {
        set.insert("platform_thread_id", thread_id);
    }
    if let Some(title) = facts.title.as_deref().and_then(clean_title) {
        set.insert("title", title);
    }
    if let Some(message_id) = last_message_id {
        set.insert("last_message_id", message_id);
    }
    if let Some(message_id) = relay_message_id {
        set.insert("last_relay_message_id", message_id);
    }
    if matches!(row.platform.as_str(), "telegram" | "telegram-new")
        && let Some(message_id) = last_message_id.or(relay_message_id)
    {
        // Best effort: only the stored topic evidence is read, and a missing,
        // partial or legacy message row (or a failed lookup) means "no
        // evidence", which keeps the previous outbound target. It never fails
        // the inbound message.
        let topic = match state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .find_one(doc! {"_id": message_id, "channel_bot_id": &row.channel_bot_id})
            .projection(doc! {"_id": 0, "thread_context.telegram_topic": 1})
            .await
        {
            Ok(source) => source
                .as_ref()
                .and_then(|source| source.get_document("thread_context").ok())
                .and_then(|facts| facts.get("telegram_topic"))
                .and_then(|topic| {
                    bson::from_bson::<crate::models::channel_thread::TelegramTopic>(topic.clone())
                        .ok()
                }),
            Err(error) => {
                tracing::debug!(%error, "Telegram topic evidence unavailable");
                None
            }
        };
        if let Some(topic) = topic {
            set.insert(
                "telegram_topic",
                bson::to_bson(&topic)
                    .map_err(|_| AppError::Internal("Invalid outbound topic".into()))?,
            );
        }
    }
    let threads = state.db.collection::<NyxbotThread>(THREADS);
    // Before NyxID passed each message's own chat type on, a group reached
    // through a default route looked like private chats (one per member):
    // those records go once the group is seen as a group.
    if facts.kind != "private" {
        threads
            .delete_many(doc! {"channel_id": &row.id, "kind": "private",
            "platform_chat_id": &facts.chat_id, "partition": {"$ne": partition}})
            .await?;
    }
    threads
        .find_one_and_update(
            doc! {"channel_id": &row.id, "partition": partition},
            doc! {"$setOnInsert": {"_id": Uuid::new_v4().to_string(), "user_id": &row.user_id,
            "created_at": now}, "$set": set},
        )
        .upsert(true)
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Internal("Channel chat unavailable".into()))
}

/// A private chat's name: "You" for the owner, else the sender's name (never
/// "You"), else the platform and the end of their ID (some platforms send no
/// names).
pub(super) fn private_title(row: &NyxbotChannel, sender_id: &str, name: Option<&str>) -> String {
    if row.owner_sender_ids.iter().any(|id| id == sender_id) {
        return "You".into();
    }
    match name.and_then(clean_title) {
        Some(name) if name.eq_ignore_ascii_case("you") => format!("{name} (guest)"),
        Some(name) => name,
        None => unnamed_sender(&row.platform, sender_id),
    }
}

/// A sender whose platform sends no name (Lark, Feishu): the platform and the
/// end of their ID, so people in a shared thread can be told apart.
pub(super) fn unnamed_sender(platform: &str, sender_id: &str) -> String {
    let tail: String = sender_id
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!(
        "{} user …{tail}",
        crate::services::channel_platform::platform_name(platform)
    )
}

/// Lark and Feishu write each mention into the text as a key (`@_user_1`);
/// the event lists who it names. Put the names in (in one pass, longest key
/// first), so the agent reads who was addressed. Other payloads are left as
/// they are.
pub(super) fn named_mentions(text: &str, raw: &Value) -> String {
    if text.len() > 64 * 1024 {
        return text.to_owned();
    }
    let Some(mentions) = raw["event"]["message"]["mentions"]
        .as_array()
        .filter(|m| m.len() <= crate::services::channel_native_mentions::MAX_ENTITIES)
    else {
        return text.to_owned();
    };
    let mut named: Vec<(&str, String)> = mentions
        .iter()
        .filter_map(|mention| {
            let key = mention["key"]
                .as_str()
                .filter(|key| key.starts_with("@_user_") && key.len() <= 64)?;
            let name = plain_name(mention["name"].as_str().filter(|name| name.len() <= 1024)?)?;
            Some((key, name.chars().take(60).collect()))
        })
        .collect();
    if named.is_empty() {
        return text.to_owned();
    }
    let conflicting: Vec<&str> = named
        .iter()
        .filter_map(|(key, name)| {
            named
                .iter()
                .any(|(other, label)| key == other && name != label)
                .then_some(*key)
        })
        .collect();
    named.retain(|(key, _)| !conflicting.contains(key));
    named.sort_by_key(|(key, _)| std::cmp::Reverse(key.len()));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("@_") {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match named.iter().find(|(key, _)| {
            rest.starts_with(key)
                && rest[key.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_')
        }) {
            Some((key, name)) => {
                out.push('@');
                out.push_str(name);
                rest = &rest[key.len()..];
            }
            None => {
                out.push_str("@_");
                rest = &rest[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

async fn fetch_title(state: &AppState, bot_id: &str, chat_id: &str) -> AppResult<Option<String>> {
    let bot = channel_bot_service::get_bot(&state.db, bot_id).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let token = crate::services::channel_credentials::resolve_bot_token(
        &state.db,
        &state.encryption_keys,
        adapter.as_ref(),
        &bot,
    )
    .await?;
    Ok(adapter
        .chat_title(
            &state.http_client,
            &crate::services::channel_platform::BotCredentials {
                billing: None,
                token: &token,
                platform_bot_id: Some(&bot.platform_bot_id),
                platform_secrets: None,
            },
            chat_id,
        )
        .await?
        .as_deref()
        .and_then(clean_title))
}

async fn store_title(state: &AppState, chat_id: &str, title: &str) -> AppResult<()> {
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": chat_id, "title": bson::Bson::Null},
            doc! {"$set": {"title": title}},
        )
        .await?;
    Ok(())
}

/// Look a group's name up now (bounded), for a thread about to be created.
pub(super) async fn look_up_title(
    state: &AppState,
    row: &NyxbotChannel,
    chat: &NyxbotThread,
) -> Option<String> {
    let chat_id = chat.platform_chat_id.as_deref()?;
    let lookup = fetch_title(state, &row.channel_bot_id, chat_id);
    match tokio::time::timeout(Duration::from_secs(3), lookup).await {
        Ok(Ok(Some(title))) => {
            let _ = store_title(state, &chat.id, &title).await;
            Some(title)
        }
        Ok(Err(error)) => {
            tracing::debug!(%error, "Channel chat title not looked up");
            None
        }
        _ => None,
    }
}

/// Look a group's name up once, when the message did not carry it. Best
/// effort: failures only leave the chat untitled.
pub(super) fn spawn_title_lookup(state: &AppState, row: &NyxbotChannel, chat: &NyxbotThread) {
    if chat.title.is_some() || !is_group(chat) {
        return;
    }
    let Some(chat_id) = chat.platform_chat_id.clone() else {
        return;
    };
    let state = state.clone();
    let bot_id = row.channel_bot_id.clone();
    let id = chat.id.clone();
    tokio::spawn(async move {
        let result: AppResult<()> = async {
            if let Some(title) = fetch_title(&state, &bot_id, &chat_id).await? {
                store_title(&state, &id, &title).await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::debug!(%error, "Channel chat title not looked up");
        }
    });
}

/// Whether a direct message in a group mentions the bot or replies to it,
/// read from the platform's payload. `None` when the payload does not say:
/// then only the owner is answered (as before chats had settings).
/// `bot_user_id`: the bot's own user ID on its platform when known (Lark:
/// its `open_id`), so a mention of someone else is not a mention of the bot.
pub(super) fn raw_addressed(
    bot: &ChannelBot,
    raw: &Value,
    bot_user_id: Option<&str>,
) -> Option<bool> {
    match canonical_platform(&bot.platform) {
        "telegram" => {
            let message = native_message(raw)?;
            let bot_id = bot.platform_bot_id.as_str();
            let is_bot = |user: &Value| {
                user["id"].as_i64().map(|id| id.to_string()).as_deref() == Some(bot_id)
            };
            Some(
                is_bot(&message["reply_to_message"]["from"])
                    || crate::services::channel_native_mentions::telegram(message, bot).bot,
            )
        }
        "lark" | "feishu" => Some(
            crate::services::channel_native_mentions::lark(&raw["event"]["message"], bot_user_id)
                .bot,
        ),

        // Slack says so with its own event type; other channel messages
        // cannot be told apart here.
        "slack" => (raw["event"]["type"] == "app_mention").then_some(true),
        // Slash commands and component clicks are always for the bot.
        "discord" if matches!(raw["type"].as_u64(), Some(2 | 3)) && raw.get("author").is_none() => {
            Some(true)
        }
        "discord" if raw.get("author").is_some() => {
            let bot_id = bot.platform_bot_id.as_str();
            Some(
                raw["mentions"].as_array().is_some_and(|users| {
                    users.iter().any(|user| user["id"].as_str() == Some(bot_id))
                }) || raw["referenced_message"]["author"]["id"].as_str() == Some(bot_id),
            )
        }
        _ => None,
    }
}

/// Select the same authenticated native message variant used by Telegram
/// parsing and sender classification. Telegram adapters accept these three
/// update variants in this order; edited channel posts are not part of the
/// NyxID Telegram ingress contract.
pub(super) fn native_message(raw: &Value) -> Option<&Value> {
    ["message", "edited_message", "channel_post"]
        .into_iter()
        .filter_map(|key| raw.get(key))
        .next()
}

/// Whether `reply_to` (a platform message ID in `chat_id`) is one the bot sent.
pub(super) async fn replies_to_bot(
    state: &AppState,
    bot: &ChannelBot,
    chat_id: &str,
    reply_to: Option<&str>,
) -> AppResult<bool> {
    let Some(reply_to) = reply_to.filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    Ok(state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(
            doc! {"platform": &bot.platform, "platform_message_id": reply_to,
            "direction": "outbound", "channel_bot_id": &bot.id,
            "platform_conversation_id": chat_id},
        )
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

/// The platform message a gateway event replies to, read from NyxID's own
/// record of the inbound message (the gateway's activity does not say).
pub(super) async fn inbound_reply_to(
    state: &AppState,
    row: &NyxbotChannel,
    message_id: &str,
) -> AppResult<Option<String>> {
    Ok(state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(
            doc! {"_id": message_id, "channel_bot_id": &row.channel_bot_id,
            "direction": "inbound"},
        )
        .projection(doc! {"reply_to_platform_message_id": 1})
        .await?
        .and_then(|message| {
            message
                .get_str("reply_to_platform_message_id")
                .ok()
                .map(str::to_owned)
        }))
}

pub(super) use crate::services::channel_turn_delivery::{Admission, admission};

/// Tell a group, at most daily, why the agent is not answering its members
/// yet. Returns the reply when it is due.
pub(super) async fn waiting_hint(
    state: &AppState,
    chat: &NyxbotThread,
) -> AppResult<Option<String>> {
    let now = Utc::now();
    let due = bson::DateTime::from_chrono(now - ChronoDuration::hours(24));
    let claimed = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": &chat.id, "$or": [{"guest_hint_at": bson::Bson::Null},
            {"guest_hint_at": {"$lt": due}}]},
            doc! {"$set": {"guest_hint_at": bson::DateTime::from_chrono(now)}},
        )
        .await?;
    Ok((claimed.modified_count == 1)
        .then(|| "I'll answer everyone here once my owner has talked to me in this chat.".into()))
}

/// How the agent should picture a chat in its instructions.
pub(super) fn describe(chat: &NyxbotThread) -> String {
    let title = chat
        .title
        .as_deref()
        .map(|title| format!(" \"{}\"", excerpt(title, 60).replace('"', "'")))
        .unwrap_or_default();
    match chat.kind.as_deref() {
        Some("private") => "a private chat".into(),
        Some("channel") => format!("the channel{title}"),
        _ => format!("the group chat{title}"),
    }
}

/// Move a rebuilt connection's chats (and their threads' channel) and its
/// private-chat access to the new connection. A new gateway channel names its
/// private conversations anew (`conv_...`), so those start fresh threads, as
/// they always did. When the connection now reaches another agent, chats
/// without their own agent start new threads with it (as relinking does).
pub(super) async fn carry_over(
    state: &AppState,
    owner: &str,
    from: &NyxbotChannel,
    to: &str,
    agent_changed: bool,
) -> AppResult<()> {
    crate::services::channel_thread_follow_service::carry_over(
        &state.db,
        owner,
        &from.id,
        to,
        agent_changed,
    )
    .await
}

// ---------------------------------------------------------------------------
// Gateway admission
// ---------------------------------------------------------------------------

/// Gateway group admission for this channel: every group message when one
/// of its chats answers everything, otherwise only mentions and replies.
async fn wanted_groups(state: &AppState, row: &NyxbotChannel) -> AppResult<&'static str> {
    let all = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id": &row.id, "reply_mode": "all",
        "kind": {"$in": ["group", "channel"]}})
        .await?
        .is_some();
    Ok(if all { "all" } else { GATEWAY_GROUPS_DEFAULT })
}

/// How long the sweep waits before retrying an update the gateway refused.
const GATEWAY_RETRY_MINUTES: i64 = 10;

/// Bring the gateway's group admission in line with the chats' settings.
/// Returns a stable error code when the gateway refused the update. The
/// sweep (`force = false`) backs off after a refusal; a user's change does not.
pub(super) async fn sync_gateway_groups(
    state: &AppState,
    row: &NyxbotChannel,
    force: bool,
) -> AppResult<Option<&'static str>> {
    if row.transport != "gateway" {
        return Ok(None);
    }
    // Compare with the stored admission as of now, not the caller's copy.
    let mut current = load_channel(state, &row.user_id, &row.id).await?;
    if current.status != "active" || current.transport != "gateway" {
        return Ok(None);
    }
    let prior_identity = current.gateway_bot_id.clone();
    super::gateway_bot_mentions::discover(state, &mut current).await?;
    let identity_changed = prior_identity != current.gateway_bot_id;
    super::gateway_threads::discover(state, &mut current).await?;
    let row = &current;
    if row.status != "active" || row.transport != "gateway" {
        return Ok(None);
    }
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
    let bots = super::gateway_bot_mentions::desired(row, &bot);
    let wanted = wanted_groups(state, row).await?;
    let threads = super::gateway_threads::desired(state, row).await?;
    if row
        .gateway_groups
        .as_deref()
        .unwrap_or(GATEWAY_GROUPS_DEFAULT)
        == wanted
        && threads == row.gateway_threads
        && bots == row.gateway_bot_mentions.enabled
        && !identity_changed
        && row.gateway_no_progress
    {
        return Ok(None);
    }
    if !force
        && row
            .gateway_groups_retry_at
            .is_some_and(|at| at > Utc::now())
    {
        return Ok(Some("gateway_retry_pending"));
    }
    let code = push_gateway_groups(state, row, wanted, &threads, identity_changed).await?;
    let update = match code {
        None => doc! {"$unset": {"gateway_groups_retry_at": ""}},
        Some(_) => doc! {"$set": {"gateway_groups_retry_at": bson::DateTime::from_chrono(
        Utc::now() + ChronoDuration::minutes(GATEWAY_RETRY_MINUTES))}},
    };
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(doc! {"_id": &row.id}, update)
        .await?;
    Ok(code)
}

async fn push_gateway_groups(
    state: &AppState,
    row: &NyxbotChannel,
    wanted: &'static str,
    threads: &crate::models::nyxbot_channel::GatewayThreadSupport,
    identity_changed: bool,
) -> AppResult<Option<&'static str>> {
    let (Some(channel_id), Some(expected_version), Some(route_id)) = (
        row.gateway_channel_id.as_deref(),
        row.gateway_version,
        row.route_id.as_deref(),
    ) else {
        return Ok(Some("gateway_channel_missing"));
    };
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
    let creator = creator_bearer(state, &row.user_id)?;
    let bots = super::gateway_bot_mentions::desired(row, &bot);
    let reconciliation = GatewayReconciliation {
        wanted,
        threads,
        bots,
        identity_changed,
    };
    let mut update = gateway_policy(state, row, &bot, &[route_id.to_owned()], wanted);
    if threads.version == Some(1) {
        update["reply"]["thread_contract"] = super::gateway_threads::policy(threads);
    } else if let Some(reply) = update["reply"].as_object_mut() {
        reply.remove("thread_contract");
    }
    update["expected_version"] = json!(expected_version);
    // Proof belongs to the acknowledged binding and policy. Invalidate it
    // before a replacement write, including errors or an uncertain response.
    let Some(filter) = gateway_local_cas_filter(row, expected_version) else {
        return Ok(Some("gateway_reconcile_conflict"));
    };
    if state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(filter, doc! {"$set": {"gateway_no_progress": false}})
        .await?
        .matched_count
        != 1
    {
        return Ok(Some("gateway_reconcile_conflict"));
    }
    let response = gateway_call(
        state,
        reqwest::Method::PUT,
        &format!("/channels/{}", urlencode(channel_id)),
        creator.as_str(),
        Some(&update),
        None,
    )
    .await;
    match response {
        Ok(response) if response.status == 200 => {
            if !gateway_policy_acknowledged(&response.body, &update) {
                // A successful PUT without a trustworthy echo is an
                // uncertain write. Read the authenticated record before
                // retrying so an accepted update cannot leave our CAS version
                // permanently stale.
                return reconcile_gateway_groups(
                    state,
                    row,
                    reconciliation,
                    creator.as_str(),
                    &update,
                )
                .await;
            }
            let Some(version) = response.body["version"].as_i64().filter(|value| *value > 0) else {
                return reconcile_gateway_groups(
                    state,
                    row,
                    reconciliation,
                    creator.as_str(),
                    &update,
                )
                .await;
            };
            persist_gateway_groups(state, row, expected_version, wanted, threads, bots, version)
                .await
        }
        Ok(response) if response.status == 409 => {
            // A concurrent writer may already have applied the desired
            // definition. Reconcile the fresh CAS version before deciding
            // whether another PUT is needed.
            reconcile_gateway_groups(state, row, reconciliation, creator.as_str(), &update).await
        }
        Ok(response) => Ok(Some(gateway_error_code(&response))),
        Err(_) => Ok(Some("gateway_unavailable")),
    }
}

struct GatewayReconciliation<'a> {
    wanted: &'static str,
    threads: &'a crate::models::nyxbot_channel::GatewayThreadSupport,
    bots: bool,
    identity_changed: bool,
}

/// Compare only the provider fields that NyxID authored. Gateways may add
/// harmless definition metadata, but identity, admission and route scope must
/// echo exactly before NyxID enables bot delivery.
fn gateway_policy_acknowledged(body: &Value, requested: &Value) -> bool {
    let Some(definition) = body.get("definition") else {
        return false;
    };
    let Some(actual_sources) = definition.get("sources").and_then(Value::as_array) else {
        return false;
    };
    let Some(expected_sources) = requested.get("sources").and_then(Value::as_array) else {
        return false;
    };
    if actual_sources.len() != 1 || expected_sources.len() != 1 {
        return false;
    }
    let Some(actual) = definition.pointer("/sources/0") else {
        return false;
    };
    let Some(expected) = requested.pointer("/sources/0") else {
        return false;
    };
    if actual.get("trust_normalized").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    [
        "/type",
        "/issuer",
        "/key_id",
        "/platform",
        "/route_ids",
        "/bot_id",
        "/bot_username",
        "/admission",
    ]
    .iter()
    .all(|pointer| actual.pointer(pointer) == expected.pointer(pointer))
        && definition
            .pointer("/reply/ack_after_ms")
            .and_then(Value::as_u64)
            == Some(0)
        && definition.pointer("/reply/thread_contract")
            == requested.pointer("/reply/thread_contract")
}

async fn persist_gateway_groups(
    state: &AppState,
    row: &NyxbotChannel,
    expected_version: i64,
    wanted: &'static str,
    threads: &crate::models::nyxbot_channel::GatewayThreadSupport,
    bots: bool,
    version: i64,
) -> AppResult<Option<&'static str>> {
    let Some(filter) = gateway_local_cas_filter(row, expected_version) else {
        return Ok(Some("gateway_reconcile_conflict"));
    };
    let updated = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            filter,
            doc! {"$set": {"gateway_version": version, "gateway_groups": wanted, "gateway_no_progress": true,
            "gateway_bot_mentions.enabled": bots,
            "gateway_bot_id": &row.gateway_bot_id,
            "gateway_threads": bson::to_bson(threads).map_err(|_| crate::services::channel_thread_follow_service::not_found())?,
            "updated_at": bson::DateTime::now()}},
        )
        .await?;
    Ok((updated.matched_count != 1).then_some("gateway_reconcile_conflict"))
}

/// Fence local gateway state updates against the same live transport binding
/// that was used for the remote request. A reconnect or route switch must
/// win over a late acknowledgement from the old channel.
pub(super) fn gateway_local_cas_filter(
    row: &NyxbotChannel,
    expected_version: i64,
) -> Option<mongodb::bson::Document> {
    let (Some(channel_id), Some(route_id)) =
        (row.gateway_channel_id.as_deref(), row.route_id.as_deref())
    else {
        return None;
    };
    (row.transport == "gateway" && row.status == "active").then(|| {
        doc! {
            "_id": &row.id,
            "status": "active",
            "transport": "gateway",
            "gateway_version": expected_version,
            "gateway_channel_id": channel_id,
            "route_id": route_id,
            "route_api_key_id": &row.route_api_key_id,
            "agent_api_key_id": &row.agent_api_key_id,
            "binding_id": &row.binding_id,
        }
    })
}

async fn reconcile_gateway_groups(
    state: &AppState,
    row: &NyxbotChannel,
    policy: GatewayReconciliation<'_>,
    creator: &str,
    requested: &Value,
) -> AppResult<Option<&'static str>> {
    let Some(channel_id) = row.gateway_channel_id.as_deref() else {
        return Ok(Some("gateway_channel_missing"));
    };
    let fresh = gateway_call(
        state,
        reqwest::Method::GET,
        &format!("/channels/{}", urlencode(channel_id)),
        creator,
        None,
        None,
    )
    .await;
    let Ok(fresh) = fresh else {
        return Ok(Some("gateway_unavailable"));
    };
    if fresh.status != 200 {
        return Ok(Some(gateway_error_code(&fresh)));
    }
    let Some(version) = fresh.body["version"].as_i64().filter(|value| *value > 0) else {
        return Ok(Some("gateway_version_unavailable"));
    };
    let acknowledged = gateway_policy_acknowledged(&fresh.body, requested);
    if acknowledged {
        return persist_gateway_groups(
            state,
            row,
            row.gateway_version.unwrap_or_default(),
            policy.wanted,
            policy.threads,
            policy.bots,
            version,
        )
        .await;
    }
    // Persist a fresh CAS version even when the policy echo is unsafe. The
    // next scheduled reconciliation can retry against the accepted write;
    // bot enablement remains disabled until a complete echo is observed.
    let Some(filter) = gateway_local_cas_filter(row, row.gateway_version.unwrap_or_default())
    else {
        return Ok(Some("gateway_reconcile_conflict"));
    };
    let updated = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            filter,
            doc! {"$set": {"gateway_version": version, "gateway_no_progress": false, "updated_at": bson::DateTime::now()}},
        )
        .await?;
    if updated.matched_count != 1 {
        return Ok(Some("gateway_reconcile_conflict"));
    }
    if policy.identity_changed {
        Ok(Some("gateway_receiving_identity_unavailable"))
    } else if policy.bots {
        Ok(Some("gateway_native_bot_mentions_unavailable"))
    } else if policy.threads.version == Some(1) {
        Ok(Some("gateway_thread_contract_unavailable"))
    } else {
        Ok(Some("gateway_policy_unavailable"))
    }
}

// ---------------------------------------------------------------------------
// Listing, settings and posting
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ChannelChatResponse {
    parent_chat_id: Option<String>,
    id: String,
    channel_agent_id: String,
    platform: String,
    bot_label: String,
    /// `private`, `group` or `channel`; `None` until the chat speaks again
    /// (chats recorded before this was tracked).
    kind: Option<String>,
    title: Option<String>,
    /// The agent this chat reaches instead of the channel's, when set.
    agent_id: Option<String>,
    /// `mention` or `all` for groups and channels; `all` for private chats.
    reply_mode: &'static str,
    /// Who may talk to the agent: `everyone` or `owner` (for private chats,
    /// the bot's `private_chats`).
    members: String,
    /// Groups: the owner's explicit choice (`everyone` or `owner`); `None`
    /// follows the default (members once the owner has talked there).
    members_setting: Option<String>,
    /// Groups: the owner has talked to the bot there.
    owner_seen: bool,
    /// Private chats: the owner's own chat with the bot.
    owner: bool,
    allow_posts: bool,
    conversation_id: Option<String>,
    last_message_at: Option<chrono::DateTime<Utc>>,
    /// An authenticated platform message registered this chat, independently of a reply.
    registration_evidence: &'static str,
    registered_at: chrono::DateTime<Utc>,
    has_thread_history: bool,
    threads: &'static str,
    threads_setting: Option<String>,
    thread_capabilities: crate::services::channel_platform::ThreadCapabilities,
    followed_thread_count: i64,
    follow_readiness: &'static str,
    follow_guidance: Option<String>,
}

fn chat_response(row: &NyxbotChannel, chat: &NyxbotThread) -> ChannelChatResponse {
    ChannelChatResponse {
        parent_chat_id: chat.follow.parent_chat_id.clone(),
        has_thread_history: chat.follow.has_thread_history,
        follow_guidance: None,
        threads: "off",
        threads_setting: chat.follow.threads.clone(),
        thread_capabilities: Default::default(),
        followed_thread_count: 0,
        follow_readiness: "unavailable",
        id: chat.id.clone(),
        channel_agent_id: row.id.clone(),
        platform: row.platform.clone(),
        bot_label: row.bot_label.clone(),
        kind: chat.kind.clone(),
        title: chat.title.clone(),
        agent_id: chat.agent_id.clone(),
        reply_mode: if answers_everything(chat) {
            "all"
        } else {
            "mention"
        },
        members: if chat.kind.as_deref() == Some("private") {
            row.private_chats.clone().unwrap_or_else(|| "owner".into())
        } else if members_may_talk(chat) {
            "everyone".into()
        } else {
            "owner".into()
        },
        members_setting: chat.members.clone(),
        owner_seen: chat.owner_seen,
        owner: chat.owner_chat,
        allow_posts: chat.allow_posts,
        conversation_id: chat.conversation_id.clone(),
        last_message_at: chat.last_message_at,
        registration_evidence: if chat.last_message_at.is_some() {
            "authenticated_inbound"
        } else {
            "legacy_unknown"
        },
        registered_at: chat.created_at,
    }
}

fn follow_response(
    state: &AppState,
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    on: bool,
    count: i64,
) -> ChannelChatResponse {
    let mut dto = chat_response(row, chat);
    dto.thread_capabilities = super::thread_controls::capabilities(state, row, on, is_group(chat));
    dto.threads = if on && chat.follow.threads.as_deref().unwrap_or("follow") == "follow" {
        "follow"
    } else {
        "off"
    };
    dto.follow_guidance = if on && is_group(chat) {
        Some(if row.transport == "gateway" {
            "Thread follow is unavailable on this gateway version.".into()
        } else if let Ok(a) = crate::services::channel_adapters::resolve_adapter(
            &row.platform,
            &state.token_exchange_cache,
        ) {
            if a.thread_capabilities().thread_follow {
                a.thread_follow_guidance().into()
            } else {
                "This platform does not support thread follow.".into()
            }
        } else {
            "Thread follow is unavailable.".into()
        })
    } else {
        None
    };
    dto.followed_thread_count = count;
    dto.follow_readiness = if dto.thread_capabilities.thread_follow {
        "ready"
    } else {
        "unavailable"
    };
    dto
}

/// Chats of the owner's channels (optionally one channel), most recent
/// first. Gateway sender partitions of a group are not chats.
pub(crate) async fn list_chats(
    state: &AppState,
    owner: &str,
    channel_id: Option<&str>,
) -> AppResult<Vec<ChannelChatResponse>> {
    let channels = list(state, owner).await?;
    let channels: Vec<NyxbotChannel> = channels
        .into_iter()
        .filter(|row| channel_id.is_none_or(|id| row.id == id))
        .collect();
    if let Some(id) = channel_id
        && channels.is_empty()
    {
        return Err(AppError::NotFound(format!("Channel {id} not found")));
    }
    let ids: Vec<&str> = channels.iter().map(|row| row.id.as_str()).collect();
    let chats: Vec<NyxbotThread> = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find(doc! {"user_id": owner, "channel_id": {"$in": ids}, "record_scope":{"$ne":"platform_thread"},
        "$or": [{"kind": {"$ne": bson::Bson::Null}},
            {"conversation_id": {"$ne": bson::Bson::Null}}]})
        .sort(doc! {"last_message_at": -1, "updated_at": -1})
        .limit(LIST_LIMIT)
        .await?
        .try_collect()
        .await?;
    let on = super::thread_follow::enabled(state, owner).await?;
    let counts = crate::services::channel_thread_follow_service::counts(
        &state.db,
        owner,
        &chats.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
    )
    .await?;
    Ok(chats
        .iter()
        .filter_map(|chat| {
            let row = channels.iter().find(|row| row.id == chat.channel_id)?;
            Some(follow_response(
                state,
                row,
                chat,
                on,
                *counts.get(&chat.id).unwrap_or(&0),
            ))
        })
        .collect())
}

async fn load_chat(
    state: &AppState,
    owner: &str,
    chat_id: &str,
) -> AppResult<(NyxbotChannel, NyxbotThread)> {
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id": chat_id, "user_id": owner,"record_scope":{"$ne":"platform_thread"}})
        .await?
        .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;
    let row = load_channel(state, owner, &chat.channel_id).await?;
    if !matches!(row.status.as_str(), "pending" | "active") {
        return Err(AppError::NotFound("Chat not found".into()));
    }
    Ok((row, chat))
}

/// New per-chat settings; absent fields keep their value.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSettings {
    #[serde(default, deserialize_with = "non_null_threads")]
    pub threads: Option<String>,
    pub reply_mode: Option<String>,
    pub members: Option<String>,
    pub allow_posts: Option<bool>,
    /// An agent ID, or `default` for the channel's agent.
    pub agent_id: Option<String>,
}

fn non_null_threads<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    <String as serde::Deserialize>::deserialize(d).map(Some)
}

pub(crate) async fn update_chat(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    settings: &ChatSettings,
) -> AppResult<Value> {
    let (row, chat) = load_chat(state, owner, chat_id).await?;
    crate::services::platform_bot_authority::require_personal_routes(
        &state.db,
        &row.channel_bot_id,
    )
    .await?;
    let group = is_group(&chat);
    if chat.kind.is_none() && (settings.reply_mode.is_some() || settings.members.is_some()) {
        return Err(AppError::Conflict(
            "NyxID has not seen a message in this chat since chats got their own settings; \
            send one there first"
                .into(),
        ));
    }
    let mut set = doc! {"updated_at": bson::DateTime::now()};
    let mut unset = doc! {};
    if let Some(threads) = settings.threads.as_deref() {
        if !matches!(threads, "follow" | "off") || !group {
            return Err(AppError::ValidationError(
                "threads must be follow or off on a group chat".into(),
            ));
        }
        set.insert("threads", threads);
    }
    if let Some(mode) = settings.reply_mode.as_deref() {
        if !matches!(mode, "mention" | "all") {
            return Err(AppError::ValidationError(
                "reply_mode must be mention or all".into(),
            ));
        }
        if !group {
            return Err(AppError::ValidationError(
                "Private chats always answer every message".into(),
            ));
        }
        set.insert("reply_mode", mode);
    }
    if let Some(members) = settings.members.as_deref() {
        if !matches!(members, "everyone" | "owner" | "default") {
            return Err(AppError::ValidationError(
                "members must be everyone, owner or default".into(),
            ));
        }
        if !group {
            return Err(AppError::ValidationError(
                "Who may talk in private chats is set on the channel bot (private_chats)".into(),
            ));
        }
        if members == "default" {
            unset.insert("members", "");
        } else {
            set.insert("members", members);
        }
    }
    if let Some(allow) = settings.allow_posts {
        if allow && chat.platform_chat_id.is_none() {
            return Err(AppError::Conflict(
                "NyxID has not seen this chat's address yet; send a message there first".into(),
            ));
        }
        set.insert("allow_posts", allow);
    }
    let mut agent_changed = None;
    if let Some(agent) = settings.agent_id.as_deref() {
        let current = chat.agent_id.as_deref();
        if agent == "default" {
            if current.is_some() {
                unset.insert("agent_id", "");
                agent_changed = Some("default".to_owned());
            }
        } else {
            let agent = team::agent(&state.db, owner, agent).await?;
            if agent.user_id != owner {
                return Err(AppError::Forbidden(
                    "Organization agents require private member threads".into(),
                ));
            }
            if agent.destroyed_at.is_some() {
                return Err(AppError::Conflict("That agent was destroyed".into()));
            }
            if current != Some(agent.id.as_str()) {
                set.insert("agent_id", &agent.id);
                agent_changed = Some(agent.name.clone());
            }
        }
        // The chat starts a new thread with its new agent; the old thread
        // stays in the previous agent's history.
        if agent_changed.is_some() {
            set.insert("conversation_id", bson::Bson::Null);
        }
    }
    let mut update = doc! {"$set": set};
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    if agent_changed.is_some() {
        update.insert("$inc", doc! {"binding_generation":1});
    }
    let updated = crate::services::channel_thread_follow_service::update_settings(
        &state.db,
        owner,
        &row.id,
        &chat.id,
        update,
        settings.threads.as_deref() == Some("off"),
        agent_changed.is_some(),
    )
    .await?;
    let gateway_error = if settings.reply_mode.is_some() {
        sync_gateway_groups(state, &row, true).await?
    } else {
        None
    };
    audit(
        state,
        owner,
        "nyxbot_channel_chat_updated",
        json!({"channel_agent_id": &row.id, "chat_id": &chat.id,
            "reply_mode": settings.reply_mode, "members": settings.members,
            "allow_posts": settings.allow_posts, "threads":settings.threads, "agent_changed": agent_changed.is_some()}),
    )
    .await;
    let on = super::thread_follow::enabled(state, owner).await?;
    let counts = crate::services::channel_thread_follow_service::counts(
        &state.db,
        owner,
        std::slice::from_ref(&updated.id),
    )
    .await?;
    let mut result = json!({"chat": follow_response(state,&row,&updated,on,*counts.get(&updated.id).unwrap_or(&0))});
    if let Some(code) = gateway_error {
        result["warning"] = json!(format!(
            "Saved, but the gateway did not accept the change ({code}); the bot keeps \
            answering only mentions and replies there until it does. Try again later."
        ));
    } else if settings.reply_mode.as_deref() == Some("all")
        && matches!(canonical_platform(&row.platform), "lark" | "feishu")
    {
        result["note"] = json!(
            "Lark and Feishu deliver group messages that do not @mention the bot only to apps \
            with the permission to read all group messages; add it in the developer console."
        );
    } else if settings.reply_mode.as_deref() == Some("all") && row.platform == "telegram" {
        result["note"] = json!(
            "Telegram bots only see every group message when privacy mode is off (BotFather \
            /setprivacy) or the bot is a group admin."
        );
    }
    if let Some(agent) = agent_changed {
        result["agent"] = json!(agent);
    }
    Ok(result)
}

/// Who may talk to the agent in the bot's private chats.
pub(crate) async fn set_private_chats(
    state: &AppState,
    owner: &str,
    channel_id: &str,
    private_chats: &str,
) -> AppResult<Value> {
    if !matches!(private_chats, "owner" | "everyone") {
        return Err(AppError::ValidationError(
            "private_chats must be owner or everyone".into(),
        ));
    }
    let row = load_channel(state, owner, channel_id).await?;
    // Narrowing to the owner stays available on any channel; only widening is fenced.
    let mut filter = doc! {"_id": &row.id, "user_id": owner};
    if private_chats == "everyone" {
        crate::services::platform_bot_authority::require_personal_routes(
            &state.db,
            &row.channel_bot_id,
        )
        .await?;
        filter.insert("status", doc! {"$in": ["pending", "active"]});
    }
    let updated = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            filter,
            doc! {"$set": {"private_chats": private_chats, "updated_at": bson::DateTime::now()}},
        )
        .await?;
    if updated.matched_count != 1 {
        return Err(AppError::Conflict("Channel authority changed".into()));
    }
    audit(
        state,
        owner,
        "nyxbot_channel_access_updated",
        json!({"channel_agent_id": &row.id, "private_chats": private_chats}),
    )
    .await;
    Ok(json!({"channel_agent_id": row.id, "private_chats": private_chats}))
}

/// Validate an automation destination with the same posting ACL as native tools.
pub(crate) async fn validate_post(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    agent_id: Option<&str>,
) -> AppResult<()> {
    let (row, chat) = load_chat(state, owner, chat_id).await?;
    if row.status != "active" {
        return Err(AppError::Conflict("The channel bot is not active".into()));
    }
    if let Some(agent_id) = agent_id {
        let answering = match chat.agent_id.as_deref().or(row.agent_id.as_deref()) {
            Some(id) => id.to_owned(),
            None => team::ensure_nyxbot(&state.db, owner).await?.id,
        };
        if answering != agent_id {
            return Err(AppError::Forbidden(
                "Only the agent that answers this chat (or NyxBot) posts there".into(),
            ));
        }
    }
    if !chat.allow_posts {
        return Err(AppError::Forbidden(
            "Posting is off for this chat; the owner can allow it".into(),
        ));
    }
    if !org_access_holds(state, &row).await? {
        return Err(AppError::Forbidden("org_access_lost".into()));
    }
    if chat.platform_chat_id.is_none() {
        return Err(AppError::ValidationError(
            "Chat has no known address".into(),
        ));
    }
    Ok(())
}

/// Post a message the agent writes on its own into a chat that allows it.
pub(crate) async fn post(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    text: &str,
    agent_id: Option<&str>,
) -> AppResult<Value> {
    post_inner(state, owner, chat_id, text, agent_id, None).await
}

/// This gate is unconditional, including when the general turn-gate flag is off.
pub(super) async fn post_turn_is_live(
    state: &AppState,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
) -> AppResult<bool> {
    if chat.guest || chat.turn_id.is_none() {
        return Ok(false);
    }
    let active_key = state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": &chat.api_key_id, "user_id": &chat.user_id,
        "is_active": true, "$or": [{"expires_at": bson::Bson::Null},
            {"expires_at": {"$gt": bson::DateTime::now()}}]})
        .projection(doc! {"_id": 1})
        .await?
        .is_some();
    if !active_key {
        return Ok(false);
    }
    // None deliberately forces fresh owner/org/group ACL resolution.
    let current = crate::services::assistant_acknowledgement_service::for_key(
        &state.db,
        &chat.user_id,
        Some(&chat.api_key_id),
    )
    .await?;
    Ok(current.is_some_and(|current| {
        current.turn_live
            && !current.turn_stopped
            && !current.guest
            && current.conversation_id == chat.conversation_id
            && current.agent_id == chat.agent_id
            && current.role == chat.role
            && current.turn_id == chat.turn_id
            && current.report_to == chat.report_to
            && current.with_nyxbot == chat.with_nyxbot
    }))
}

pub(crate) async fn post_current_turn(
    state: &AppState,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
    chat_id: &str,
    text: &str,
    include_images: bool,
) -> AppResult<Value> {
    if chat.guest {
        return Err(AppError::Forbidden(
            "Only the owner can post to another chat".into(),
        ));
    }
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_POST_CHARS {
        return Err(AppError::ValidationError(format!(
            "text must have 1 to {MAX_POST_CHARS} characters"
        )));
    }
    if !post_turn_is_live(state, chat).await? {
        return Err(AppError::AssistantTurnRequired);
    }
    let target = prepare_post(
        state,
        &chat.user_id,
        chat_id,
        (!chat.is_orchestrator()).then_some(chat.agent_id.as_str()),
    )
    .await?;
    let mut report = super::images::ImageReport::default();
    if include_images {
        let turn_id = chat
            .turn_id
            .as_deref()
            .ok_or(AppError::AssistantTurnRequired)?;
        let row = state
            .db
            .collection::<crate::models::assistant_conversation::AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .find_one(doc! {
                "_id":&chat.conversation_id,"user_id":&chat.user_id,"guest_turn":false,
                "active_turn.turn_id":turn_id,"active_turn.stop_requested":false,
            })
            .await?;
        if let Some(row) = row.filter(|row| engine::live_turn(row, Utc::now()).is_some()) {
            let plan = super::images::ImageSendPlan {
                attachments: row
                    .active_turn
                    .map(|turn| turn.attachments)
                    .unwrap_or_default(),
            };
            report = super::images::report_or_failure(
                state,
                super::images::ImageSendContext {
                    owner: &chat.user_id,
                    conversation_id: &chat.conversation_id,
                    turn_id,
                    channel: &target.channel,
                    thread: &target.thread,
                    anchor: super::images::ImageAnchor::Initiated {
                        chat_id: chat_id.into(),
                    },
                    audience: super::images::ImageAudience {
                        owner_private: target.thread.owner_chat
                            && target.thread.kind.as_deref() == Some("private"),
                    },
                    deadline: None,
                    initiated: Some(&target),
                    fence: Some(chat),
                },
                &plan,
            )
            .await;
        }
    }
    let images = super::images::image_result(&report);
    if !post_turn_is_live(state, chat).await.unwrap_or(false) {
        return Ok(json!({"text":"cancelled","images":images,"chat_id":chat_id}));
    }
    let result = send_post_with_fence(
        state,
        &target,
        crate::services::channel_platform::OutboundReply {
            text: Some(super::images::append_failure(text, &report)),
            metadata: None,
            attachments: Vec::new(),
            reply_to_platform_message_id: None,
        },
        Some(chat),
    )
    .await;
    let (status, message_id) = match result {
        Ok(sent) => ("sent", Some(sent.message_id)),
        Err(AppError::AssistantTurnRequired) => ("cancelled", None),
        Err(error)
            if matches!(
                super::images::classify_outcome(&error),
                super::images::ImageOutcome::Unknown
            ) =>
        {
            ("unknown", None)
        }
        Err(_) => ("failed", None),
    };
    audit(
        state,
        &chat.user_id,
        "nyxbot_channel_chat_posted",
        json!({
            "channel_agent_id": target.channel.id,
            "chat_id": chat_id,
            "message_id": message_id,
            "text_status": status,
        }),
    )
    .await;
    Ok(json!({
        "status": if status == "sent" { "posted" } else { status },
        "text": status,
        "images": images,
        "chat_id": chat_id,
        "message_id": message_id,
    }))
}

pub(crate) async fn post_automation(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    text: &str,
    agent_id: Option<&str>,
    thread_id: &str,
) -> AppResult<Value> {
    post_inner(state, owner, chat_id, text, agent_id, Some(thread_id)).await
}

pub(crate) struct PostTarget {
    pub agent_id: Option<String>,
    pub channel: NyxbotChannel,
    pub thread: NyxbotThread,
    pub bot: ChannelBot,
    pub auth: crate::mw::auth::AuthUser,
    pub conversation: ChannelConversation,
    pub adapter: std::sync::Arc<dyn crate::services::channel_platform::PlatformAdapter>,
}

/// A single logical post consumes one limiter token here. Its bounded image
/// components reuse this admitted target and take no additional tokens.
pub(super) async fn prepare_post(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    agent_id: Option<&str>,
) -> AppResult<PostTarget> {
    let (row, chat) = load_chat(state, owner, chat_id).await?;
    if row.status != "active" {
        return Err(AppError::Conflict("The channel bot is not active".into()));
    }
    if let Some(agent_id) = agent_id {
        let answering = match chat.agent_id.as_deref().or(row.agent_id.as_deref()) {
            Some(id) => id.to_owned(),
            None => team::ensure_nyxbot(&state.db, owner).await?.id,
        };
        if answering != agent_id {
            return Err(AppError::Forbidden(
                "Only the agent that answers this chat (or NyxBot) posts there".into(),
            ));
        }
    }
    if !chat.allow_posts {
        return Err(AppError::Forbidden(
            "Posting is off for this chat; the owner can allow it".into(),
        ));
    }
    let platform_chat_id = chat
        .platform_chat_id
        .clone()
        .ok_or_else(|| AppError::Conflict("NyxID has not seen this chat's address yet".into()))?;
    let route_id = row
        .route_id
        .clone()
        .ok_or_else(|| AppError::Conflict("The channel bot has no route".into()))?;
    if !org_access_holds(state, &row).await? {
        release_org_channel(state, &row).await?;
        return Err(AppError::Forbidden("org_access_lost".into()));
    }
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
    if !bot.is_active || bot.status == "suspended" {
        return Err(AppError::ChannelBotInactive(
            "Bot has been deactivated".to_string(),
        ));
    }
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let adapter: std::sync::Arc<dyn crate::services::channel_platform::PlatformAdapter> =
        std::sync::Arc::from(adapter);
    #[cfg(test)]
    let adapter = crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .get(&bot.id)
        .cloned()
        .unwrap_or(adapter);
    crate::handlers::channel_relay::check_initiate_rate_limit(state, &chat.id).await?;
    let auth = super::route_key_auth(state, &row).await?;
    let now = Utc::now();
    // The bot's own route, addressed at this chat; nothing is stored.
    let conversation = ChannelConversation {
        activity_callback: None,
        id: route_id,
        user_id: bot.user_id.clone(),
        channel_bot_id: Some(bot.id.clone()),
        platform: bot.platform.clone(),
        platform_conversation_id: platform_chat_id,
        platform_conversation_type: chat.kind.clone().unwrap_or_else(|| "group".into()),
        platform_sender_id: None,
        agent_api_key_id: row.route_api_key_id.clone(),
        default_agent: false,
        allow_agent_initiated: true,
        is_active: true,
        last_message_at: None,
        created_at: now,
        updated_at: now,
    };
    Ok(PostTarget {
        agent_id: agent_id.map(str::to_owned),
        channel: row,
        thread: chat,
        bot,
        auth,
        conversation,
        adapter,
    })
}

pub(super) async fn send_post(
    state: &AppState,
    target: &PostTarget,
    outbound: crate::services::channel_platform::OutboundReply,
) -> AppResult<crate::handlers::channel_relay::AsyncReplyResponse> {
    send_post_with_fence(state, target, outbound, None).await
}

pub(super) async fn send_post_with_fence(
    state: &AppState,
    target: &PostTarget,
    mut outbound: crate::services::channel_platform::OutboundReply,
    fence: Option<&crate::services::assistant_acknowledgement_service::ChatAuthority>,
) -> AppResult<crate::handlers::channel_relay::AsyncReplyResponse> {
    validate_post(
        state,
        &target.channel.user_id,
        &target.thread.id,
        target.agent_id.as_deref(),
    )
    .await?;
    let (channel, thread) = load_chat(state, &target.channel.user_id, &target.thread.id).await?;
    if channel.channel_bot_id != target.bot.id
        || thread.platform_chat_id != target.thread.platform_chat_id
        || thread.platform_thread_id != target.thread.platform_thread_id
        || thread.telegram_topic != target.thread.telegram_topic
    {
        return Err(AppError::Conflict("Post target changed".into()));
    }
    let bot = channel_bot_service::get_bot(&state.db, &target.bot.id).await?;
    if !bot.is_active || bot.status == "suspended" {
        return Err(AppError::ChannelBotInactive(
            "Bot has been deactivated".into(),
        ));
    }
    target.adapter.reply_context(
        crate::services::channel_image_audience::chat_thread(&target.thread),
        target.thread.created_at,
        &mut outbound.metadata,
    );
    if let Some(chat) = fence
        && !post_turn_is_live(state, chat).await?
    {
        return Err(AppError::AssistantTurnRequired);
    }
    crate::handlers::channel_relay::deliver_prepared_initiated_message(
        state,
        &HeaderMap::new(),
        &target.auth,
        &target.conversation,
        &target.bot,
        outbound,
        target.adapter.as_ref(),
    )
    .await
}

async fn post_inner(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    text: &str,
    agent_id: Option<&str>,
    full_thread: Option<&str>,
) -> AppResult<Value> {
    let text = text.trim();
    if text.is_empty() || (full_thread.is_none() && text.chars().count() > MAX_POST_CHARS) {
        return Err(AppError::ValidationError(format!(
            "text must have 1 to {MAX_POST_CHARS} characters"
        )));
    }
    let target = prepare_post(state, owner, chat_id, agent_id).await?;
    let text = full_thread.map_or_else(
        || text.to_owned(),
        |thread| {
            automation_reply(
                &target.bot.platform,
                text,
                &format!(
                    "{}/assistant?c={thread}",
                    state.config.frontend_url.trim_end_matches('/')
                ),
            )
        },
    );
    let sent = send_post(
        state,
        &target,
        crate::services::channel_platform::OutboundReply {
            text: Some(text),
            metadata: None,
            attachments: Vec::new(),
            reply_to_platform_message_id: None,
        },
    )
    .await?;
    audit(
        state,
        owner,
        "nyxbot_channel_chat_posted",
        json!({
            "channel_agent_id": &target.channel.id,
            "chat_id": &target.thread.id,
            "message_id": &sent.message_id,
        }),
    )
    .await;
    Ok(json!({
        "status": "posted",
        "chat_id": target.thread.id,
        "message_id": sent.message_id,
    }))
}

pub(crate) async fn post_automation_images(
    state: &AppState,
    run: &crate::models::trigger_run::TriggerRun,
    chat_id: &str,
    agent_id: Option<&str>,
    conversation_id: &str,
    text: &str,
) -> AppResult<Value> {
    let target = prepare_post(state, &run.user_id, chat_id, agent_id).await?;
    let mut report = super::images::ImageReport {
        app_visible: target.thread.owner_chat && target.thread.kind.as_deref() == Some("private"),
        automation: true,
        ..Default::default()
    };
    let mut outbound = crate::services::channel_platform::OutboundReply {
        text: None,
        metadata: None,
        attachments: Vec::new(),
        reply_to_platform_message_id: None,
    };
    target.adapter.reply_context(
        crate::services::channel_image_audience::chat_thread(&target.thread),
        target.thread.created_at,
        &mut outbound.metadata,
    );
    let audience = crate::services::channel_image_audience::resolve(
        &target.bot.id,
        &target.bot.platform,
        &target.conversation.platform_conversation_id,
        &outbound,
    );
    let mut budget = engine::MAX_TURN_ATTACHMENTS;
    let mut omitted = run.image_refs_truncated;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(135);
    let mut seen = std::collections::HashSet::new();
    let mut stop = None;
    let mut last_send: Option<std::time::Instant> = None;
    for item in run.image_refs.iter().rev() {
        if !seen.insert(item.attachment.id.as_str()) {
            continue;
        }
        if run.thread_id.as_deref() != Some(item.conversation_id.as_str())
            || !engine::chat_tool_image(&item.attachment)
        {
            report.outcomes.push(super::images::ImageItemResult {
                attachment_id: item.attachment.id.clone(),
                status: "failed".into(),
                code: Some("unavailable".into()),
            });
            report.failed += 1;
            report.attempted += 1;
            continue;
        }
        let metadata = match engine::turn_tool_attachments(
            &state.db,
            &run.user_id,
            &item.conversation_id,
            &item.turn_id,
        )
        .await
        {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(
                    run_id = run.id,
                    code = error.error_code(),
                    "Automation image metadata unavailable"
                );
                Vec::new()
            }
        };
        if !metadata.iter().any(|attachment| {
            attachment.id == item.attachment.id
                && attachment.content_type == item.attachment.content_type
                && attachment.size == item.attachment.size
                && engine::chat_tool_image(attachment)
        }) {
            report.outcomes.push(super::images::ImageItemResult {
                attachment_id: item.attachment.id.clone(),
                status: "failed".into(),
                code: Some("unavailable".into()),
            });
            report.failed += 1;
            report.attempted += 1;
            continue;
        }
        match crate::services::channel_image_delivery::read_legacy(
            &state.db,
            &run.user_id,
            &item.conversation_id,
            &item.turn_id,
            &target.channel.id,
            &item.attachment.id,
        )
        .await
        {
            Ok(Some(outcome)) => {
                report
                    .outcomes
                    .push(super::images::legacy_item(&item.attachment.id, outcome));
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                report.outcomes.push(super::images::ImageItemResult {
                    attachment_id: item.attachment.id.clone(),
                    status: "unknown".into(),
                    code: None,
                });
                report.unknown += 1;
                report.attempted += 1;
                stop = Some("after_unknown");
                continue;
            }
        }
        let existing = match crate::services::channel_image_delivery::read(
            &state.db,
            &run.user_id,
            &item.attachment.id,
            &audience,
        )
        .await
        {
            Ok(existing) => existing,
            Err(_) => {
                report.outcomes.push(super::images::ImageItemResult {
                    attachment_id: item.attachment.id.clone(),
                    status: "unknown".into(),
                    code: None,
                });
                report.unknown += 1;
                report.attempted += 1;
                stop = Some("after_unknown");
                continue;
            }
        };
        let pending = existing.as_ref().is_none_or(|claim| {
            claim.state == crate::models::nyxbot_image_delivery::ImageDeliveryState::Pending
        });
        if let Some(existing) = existing.as_ref().filter(|_| !pending) {
            report.outcomes.push(super::images::existing_item(existing));
            continue;
        }
        if pending && budget == 0 {
            omitted = omitted.saturating_add(1);
            continue;
        }
        if pending
            && matches!(target.bot.platform.as_str(), "telegram" | "telegram-new")
            && target.thread.kind.as_deref() != Some("private")
            && let Some(last) = last_send
        {
            tokio::time::sleep(
                (last + std::time::Duration::from_secs(1))
                    .saturating_duration_since(std::time::Instant::now()),
            )
            .await;
        }
        if pending {
            last_send = Some(std::time::Instant::now());
        }
        let one = super::images::report_or_failure_with_stop(
            state,
            super::images::ImageSendContext {
                owner: &run.user_id,
                conversation_id: &item.conversation_id,
                turn_id: &item.turn_id,
                channel: &target.channel,
                thread: &target.thread,
                anchor: super::images::ImageAnchor::Initiated {
                    chat_id: chat_id.into(),
                },
                audience: super::images::ImageAudience {
                    owner_private: report.app_visible,
                },
                deadline: Some(deadline),
                initiated: Some(&target),
                fence: None,
            },
            &super::images::ImageSendPlan {
                attachments: vec![item.attachment.clone()],
            },
            stop,
        )
        .await;
        if pending && (one.claimed > 0 || one.unknown > 0) {
            budget -= 1;
        }
        if one.unknown > 0 {
            stop = Some("after_unknown");
        }
        if one
            .outcomes
            .iter()
            .any(|item| item.code.as_deref() == Some("billing"))
        {
            stop = Some("billing");
        }
        report.attempted += one.attempted;
        report.sent += one.sent;
        report.failed += one.failed;
        report.unknown += one.unknown;
        report.skipped += one.skipped;
        report.unsupported |= one.unsupported;
        report.outcomes.extend(one.outcomes);
    }
    let mut note = super::images::failure_note(&report);
    if omitted > 0 {
        note.push_str(&format!("\n\n{omitted} additional images were omitted from this delivery. Check the automation's thread in the app; older images may have expired."));
    }
    let reply = automation_reply_with_note(
        &target.bot.platform,
        text,
        &format!(
            "{}/assistant?c={conversation_id}",
            state.config.frontend_url.trim_end_matches('/')
        ),
        &note,
    );
    let sent = send_post(
        state,
        &target,
        crate::services::channel_platform::OutboundReply {
            text: Some(reply),
            metadata: None,
            attachments: Vec::new(),
            reply_to_platform_message_id: None,
        },
    )
    .await?;
    audit(
        state,
        &run.user_id,
        "nyxbot_channel_chat_posted",
        json!({
            "channel_agent_id": target.channel.id,
            "chat_id": chat_id,
            "message_id": sent.message_id,
        }),
    )
    .await;
    Ok(json!({
        "status": "posted",
        "chat_id": chat_id,
        "message_id": sent.message_id,
        "images": super::images::image_result(&report),
    }))
}

/// Chats given to a destroyed agent go back to their bot's agent (in new
/// threads).
pub(crate) async fn release_agent_chats(
    state: &AppState,
    owner: &str,
    agent_id: &str,
) -> AppResult<()> {
    crate::services::channel_thread_follow_service::release_agent(&state.db, owner, agent_id).await
}

/// A channel thread's bot and chat, for thread listings.
pub(crate) struct ChatDetails {
    pub parent_chat_id: Option<String>,
    pub thread_id: Option<String>,
    pub parent_title: Option<String>,
    pub bot_label: String,
    pub chat_id: Option<String>,
    pub kind: Option<String>,
    pub title: Option<String>,
}

/// Bot labels and chats of the channel threads among `rows`, by
/// conversation ID. Bounded batch queries, whatever the page size.
pub(crate) async fn thread_details(
    state: &AppState,
    owner: &str,
    rows: &[&crate::models::assistant_conversation::AssistantConversation],
) -> AppResult<std::collections::HashMap<String, ChatDetails>> {
    let mut details = std::collections::HashMap::new();
    let mut channel_ids: Vec<&str> = rows
        .iter()
        .filter_map(|row| row.channel.as_ref())
        .map(|origin| origin.nyxbot_channel_id.as_str())
        .collect();
    if channel_ids.is_empty() {
        return Ok(details);
    }
    channel_ids.sort_unstable();
    channel_ids.dedup();
    let channels: Vec<NyxbotChannel> = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find(doc! {"_id": {"$in": &channel_ids}, "user_id": owner})
        .await?
        .try_collect()
        .await?;
    let conversation_ids: Vec<&str> = rows
        .iter()
        .filter(|row| row.channel.is_some())
        .map(|row| row.id.as_str())
        .collect();
    let chats: Vec<NyxbotThread> = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find(doc! {"user_id": owner, "channel_id": {"$in": &channel_ids},
        "conversation_id": {"$in": &conversation_ids}})
        .await?
        .try_collect()
        .await?;
    let parents:Vec<NyxbotThread>=state.db.collection(THREADS).find(doc! {"user_id":owner,
        "_id":{"$in":chats.iter().filter_map(|c|c.follow.parent_chat_id.as_deref()).collect::<Vec<_>>()}})
        .await?.try_collect().await?;
    for row in rows {
        let Some(origin) = row.channel.as_ref() else {
            continue;
        };
        let Some(channel) = channels
            .iter()
            .find(|channel| channel.id == origin.nyxbot_channel_id)
        else {
            continue;
        };
        let chat = chats.iter().find(|chat| {
            chat.channel_id == channel.id && chat.conversation_id.as_deref() == Some(&row.id)
        });
        details.insert(
            row.id.clone(),
            ChatDetails {
                parent_chat_id: chat.and_then(|c| c.follow.parent_chat_id.clone()),
                thread_id: chat
                    .filter(|c| crate::services::channel_thread_follow_service::is_child(c))
                    .map(|c| c.id.clone()),
                parent_title: chat
                    .and_then(|c| {
                        parents
                            .iter()
                            .find(|p| Some(&p.id) == c.follow.parent_chat_id.as_ref())
                    })
                    .and_then(|p| p.title.clone()),
                bot_label: channel.bot_label.clone(),
                chat_id: chat.map(|chat| chat.id.clone()),
                kind: chat.and_then(|chat| chat.kind.clone()),
                title: chat.and_then(|chat| chat.title.clone()),
            },
        );
    }
    Ok(details)
}

// ---------------------------------------------------------------------------
// Human UI endpoints
// ---------------------------------------------------------------------------

pub async fn list_channel_chats(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    crate::handlers::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let chats = list_chats(&state, &owner, Some(&id)).await?;
    Ok(Json(json!({"chats": chats})))
}

pub async fn update_channel_chat(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path((id, chat_id)): Path<(String, String)>,
    Json(body): Json<ChatSettings>,
) -> AppResult<Json<Value>> {
    crate::handlers::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let (row, _) = load_chat(&state, &owner, &chat_id).await?;
    if row.id != id {
        return Err(AppError::NotFound("Chat not found".into()));
    }
    Ok(Json(update_chat(&state, &owner, &chat_id, &body).await?))
}

/// Fit a single proactive message to the destination's text contract. The full
/// result stays in its durable thread; shortening is always explicit.
fn automation_reply(platform: &str, text: &str, thread_url: &str) -> String {
    automation_reply_with_note(platform, text, thread_url, "")
}

fn automation_reply_with_note(platform: &str, text: &str, thread_url: &str, note: &str) -> String {
    let limit = match platform {
        "discord" => 2000,
        "telegram" | "telegram-new" | "whatsapp" => 4096,
        "slack" => 40_000,
        "x" => 10_000,
        // Lark's JSON text content is limited to 30 KB.
        "lark" | "feishu" => 30 * 1024,
        _ => MAX_POST_CHARS,
    };
    let json_bytes = matches!(platform, "lark" | "feishu");
    let cost = |value: &str| {
        if json_bytes {
            serde_json::json!({"text": value}).to_string().len()
        } else {
            value.encode_utf16().count()
        }
    };
    let full = format!("{text}{note}");
    if cost(&full) <= limit {
        return full;
    }
    let suffix = format!("\n\n[Reply shortened. Read the full reply: {thread_url}]{note}");
    let mut result = String::new();
    let budget = limit.saturating_sub(cost(&suffix));
    let mut used = 0;
    for ch in text.chars() {
        let units = if json_bytes {
            serde_json::to_string(&ch.to_string()).map_or(6, |v| v.len().saturating_sub(2))
        } else {
            ch.len_utf16()
        };
        if used + units > budget {
            break;
        }
        result.push(ch);
        used += units;
    }
    result.push_str(&suffix);
    result
}

#[cfg(test)]
mod automation_reply_tests {
    use super::automation_reply;
    #[test]
    fn respects_platform_limits_and_links_shortened_unicode_replies() {
        for (platform, limit) in [("telegram", 4096), ("discord", 2000), ("slack", 40_000)] {
            let reply = automation_reply(
                platform,
                &"🦉".repeat(25_000),
                "https://nyx.id/assistant?c=thread",
            );
            assert!(reply.encode_utf16().count() <= limit);
            assert!(reply.contains("Reply shortened"));
            assert!(reply.contains("https://nyx.id/assistant?c=thread"));
        }
        assert_eq!(
            automation_reply("discord", "All done", "https://nyx.id"),
            "All done"
        );
    }
}

/// Native actor kind from authenticated provider data, never a display name.
pub(super) fn native_sender_kind(
    platform: &str,
    raw: &Value,
) -> crate::models::channel_thread::ThreadSenderKind {
    use crate::models::channel_thread::ThreadSenderKind::*;
    match canonical_platform(platform) {
        "telegram" => {
            let m = native_message(raw);
            match m {
                // Telegram channel posts have no user `from`; preserve their
                // authenticated legacy guest delivery. `native_owner_eligible`
                // below keeps them away from owner and bot-budget privileges.
                Some(m) if raw.get("channel_post").is_some() && m.get("from").is_none() => Human,
                Some(m) if m.get("sender_chat").is_none() => match m["from"]["is_bot"].as_bool() {
                    Some(false) => Human,
                    Some(true) => Bot,
                    None => Unknown,
                },
                _ => Unknown,
            }
        }
        "lark" | "feishu" => {
            if raw["header"]["event_type"].as_str() == Some("card.action.trigger") {
                let operator = &raw["event"]["operator"];
                let valid_operator = operator["open_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("ou_") && (4..=128).contains(&id.len()));
                return if valid_operator { Human } else { Unknown };
            }
            match raw["event"]["sender"]["sender_type"].as_str() {
                Some("user") => Human,
                Some("app") => Bot,
                _ => Unknown,
            }
        }
        // Existing adapter-specific admission remains unchanged outside this feature.
        _ => Human,
    }
}

/// A Telegram channel post has an authenticated provider envelope but no
/// user actor. It keeps legacy guest handling without owner linking/presence
/// or replenishing the automated group-hop allowance.
pub(super) fn native_owner_eligible(platform: &str, raw: &Value) -> bool {
    !(canonical_platform(platform) == "telegram"
        && raw.get("channel_post").is_some()
        && native_message(raw).is_some_and(|message| message.get("from").is_none()))
}

/// A native bot can address only the authenticated receiving installation.
/// Identity namespaces are explicit; an app ID or display name cannot stand
/// in for a Lark open_id. Missing identity closes automated admission.
pub(super) fn foreign_bot(bot: &ChannelBot, own_open_id: Option<&str>, sender: &str) -> bool {
    let own = match canonical_platform(&bot.platform) {
        "telegram" => bot
            .platform_bot_id
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(|_| bot.platform_bot_id.as_str()),
        "lark" | "feishu" => own_open_id.filter(|id| id.starts_with("ou_")),
        _ => None,
    };
    own.is_some_and(|own| {
        sender != own
            && match canonical_platform(&bot.platform) {
                "telegram" => sender
                    .parse::<i64>()
                    .is_ok_and(|id| id > 0 && id.to_string() == sender),
                "lark" | "feishu" => {
                    sender.starts_with("ou_") && sender.len() > 3 && sender.len() <= 128
                }
                _ => false,
            }
    })
}

pub(super) fn bot_guest_allowed(chat: &NyxbotThread, addressed: Option<bool>) -> bool {
    matches!(chat.kind.as_deref(), Some("group"))
        && addressed == Some(true)
        && members_may_talk(chat)
}

pub(super) async fn existing_bot_chat(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
) -> AppResult<Option<NyxbotThread>> {
    Ok(state.db.collection::<NyxbotThread>(THREADS).find_one(doc! {
        "user_id":&row.user_id,"channel_id":&row.id,"partition":partition,
        "record_scope":{"$ne":crate::services::channel_thread_follow_service::SCOPE},"kind":"group"
    }).await?.filter(members_may_talk))
}

#[cfg(test)]
mod gateway_policy_tests {
    use super::gateway_policy_acknowledged;
    use serde_json::{Value, json};

    fn requested() -> Value {
        json!({
            "sources": [{
                "type": "nyxid_relay",
                "issuer": "https://nyx.example",
                "key_id": "route-key",
                "platform": "lark",
                "route_ids": ["route-1"],
                "bot_id": "ou_receiver",
                "admission": {"type": "scoped", "groups": "mention_or_reply_to_bot"}
            }],
            "reply": {"ack_after_ms": 0, "thread_contract": null}
        })
    }

    #[test]
    fn acknowledged_gateway_policy_requires_one_raw_verified_source() {
        let requested = requested();
        assert!(gateway_policy_acknowledged(
            &json!({"version": 2, "definition": requested}),
            &requested,
        ));

        let mut progress = requested.clone();
        progress["reply"]["ack_after_ms"] = json!(1500);
        assert!(!gateway_policy_acknowledged(
            &json!({"version":2,"definition":progress}),
            &requested
        ));
        let mut unknown = requested.clone();
        unknown["reply"]
            .as_object_mut()
            .unwrap()
            .remove("ack_after_ms");
        assert!(!gateway_policy_acknowledged(
            &json!({"version":2,"definition":unknown}),
            &requested
        ));

        let mut normalized = requested.clone();
        normalized["sources"][0]["trust_normalized"] = json!(true);
        assert!(!gateway_policy_acknowledged(
            &json!({"version": 2, "definition": normalized}),
            &requested,
        ));

        let mut extra_source = requested.clone();
        extra_source["sources"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "unexpected"}));
        assert!(!gateway_policy_acknowledged(
            &json!({"version": 2, "definition": extra_source}),
            &requested,
        ));

        let mut changed_route = requested.clone();
        changed_route["sources"][0]["route_ids"] = json!(["other-route"]);
        assert!(!gateway_policy_acknowledged(
            &json!({"version": 2, "definition": changed_route}),
            &requested,
        ));
    }
}
