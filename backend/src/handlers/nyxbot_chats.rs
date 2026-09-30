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
    let legacy = direct_partition(chat_id, sender_id, thread_id);
    let threads = state.db.collection::<NyxbotThread>(THREADS);
    if threads
        .find_one(doc! {"channel_id": &row.id, "partition": &legacy})
        .await?
        .is_none()
    {
        return Ok(());
    }
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
                doc! {"$set": {"partition": partition, "updated_at": bson::DateTime::now()}},
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
    format!("{} user …{tail}", super::platform_name(platform))
}

/// Lark and Feishu write each mention into the text as a key (`@_user_1`);
/// the event lists who it names. Put the names in (in one pass, longest key
/// first), so the agent reads who was addressed. Other payloads are left as
/// they are.
pub(super) fn named_mentions(text: &str, raw: &Value) -> String {
    let Some(mentions) = raw["event"]["message"]["mentions"].as_array() else {
        return text.to_owned();
    };
    let mut named: Vec<(&str, String)> = mentions
        .iter()
        .filter_map(|mention| {
            let key = mention["key"]
                .as_str()
                .filter(|key| key.starts_with("@_"))?;
            let name = plain_name(mention["name"].as_str()?)?;
            Some((key, name.chars().take(60).collect()))
        })
        .collect();
    if named.is_empty() {
        return text.to_owned();
    }
    named.sort_by_key(|(key, _)| std::cmp::Reverse(key.len()));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("@_") {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match named.iter().find(|(key, _)| rest.starts_with(key)) {
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

fn mentions_username(text: &str, username: &str) -> bool {
    if username.is_empty() {
        return false;
    }
    let text = text.to_lowercase();
    let needle = format!("@{}", username.to_lowercase());
    text.match_indices(&needle).any(|(at, _)| {
        text[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '_'))
    })
}

/// Whether a direct message in a group mentions the bot or replies to it,
/// read from the platform's payload. `None` when the payload does not say:
/// then only the owner is answered (as before chats had settings).
pub(super) fn raw_addressed(bot: &ChannelBot, raw: &Value) -> Option<bool> {
    match canonical_platform(&bot.platform) {
        "telegram" => {
            let message = ["message", "edited_message", "channel_post"]
                .iter()
                .find_map(|key| raw.get(*key))?;
            let bot_id = bot.platform_bot_id.as_str();
            let is_bot = |user: &Value| {
                user["id"].as_i64().map(|id| id.to_string()).as_deref() == Some(bot_id)
            };
            let text = message["text"]
                .as_str()
                .or_else(|| message["caption"].as_str())
                .unwrap_or_default();
            let entities = message["entities"]
                .as_array()
                .or_else(|| message["caption_entities"].as_array());
            Some(
                is_bot(&message["reply_to_message"]["from"])
                    || mentions_username(text, bot.platform_bot_username.trim_start_matches('@'))
                    || entities.is_some_and(|entities| {
                        entities.iter().any(|entity| {
                            entity["type"] == "text_mention" && is_bot(&entity["user"])
                        })
                    }),
            )
        }
        // Lark delivers group messages that do not @mention the bot only to
        // apps granted every group message; any mention counts.
        "lark" | "feishu" => Some(
            raw["event"]["message"]["mentions"]
                .as_array()
                .is_some_and(|mentions| !mentions.is_empty()),
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

/// Who an inbound chat message is from, as far as the agent is concerned.
pub(super) enum Admission {
    /// The verified owner.
    Owner,
    /// Someone else the chat lets talk to the agent.
    Guest,
    /// A private chat with someone the bot does not answer.
    Refuse,
    /// Not for the agent (not addressed, or members may not talk).
    Silent,
    /// A member addressed the agent in a group the owner has not talked in
    /// yet (and has not set who may talk).
    Waiting,
}

pub(super) fn admission(
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    sender_id: &str,
    addressed: Option<bool>,
) -> Admission {
    let owner = row.owner_sender_ids.iter().any(|id| id == sender_id);
    // When the platform cannot tell, the owner's messages count as addressed
    // and nobody else's do.
    let addressed = addressed.unwrap_or(owner);
    if chat.kind.as_deref() == Some("private") {
        return if owner {
            Admission::Owner
        } else if row.private_chats.as_deref() == Some("everyone") {
            Admission::Guest
        } else {
            Admission::Refuse
        };
    }
    if !addressed && !answers_everything(chat) {
        Admission::Silent
    } else if owner {
        Admission::Owner
    } else if members_may_talk(chat) {
        Admission::Guest
    } else if chat.members.is_none() && addressed && chat.kind.as_deref() == Some("group") {
        // A broadcast channel's posts come from the channel, never the owner:
        // no promise that cannot come true.
        Admission::Waiting
    } else {
        Admission::Silent
    }
}

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
    let from = from.id.as_str();
    let stable = doc! {"$not": {"$regex": "^conv_"}};
    let threads = state.db.collection::<NyxbotThread>(THREADS);
    if agent_changed {
        threads
            .update_many(
                doc! {"channel_id": from, "user_id": owner, "agent_id": bson::Bson::Null},
                doc! {"$set": {"conversation_id": bson::Bson::Null}},
            )
            .await?;
    }
    threads
        .update_many(
            doc! {"channel_id": from, "user_id": owner, "partition": stable.clone()},
            doc! {"$set": {"channel_id": to}},
        )
        .await?;
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_many(
            doc! {"user_id": owner, "channel.nyxbot_channel_id": from,
            "channel.partition": stable},
            doc! {"$set": {"channel.nyxbot_channel_id": to}},
        )
        .await?;
    Ok(())
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
    let row = &load_channel(state, &row.user_id, &row.id).await?;
    if row.status != "active" {
        return Ok(None);
    }
    let wanted = wanted_groups(state, row).await?;
    if row
        .gateway_groups
        .as_deref()
        .unwrap_or(GATEWAY_GROUPS_DEFAULT)
        == wanted
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
    let code = push_gateway_groups(state, row, wanted).await?;
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
) -> AppResult<Option<&'static str>> {
    let (Some(channel_id), Some(version), Some(route_id)) = (
        row.gateway_channel_id.as_deref(),
        row.gateway_version,
        row.route_id.as_deref(),
    ) else {
        return Ok(Some("gateway_channel_missing"));
    };
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
    let creator = creator_bearer(state, &row.user_id)?;
    let mut update = gateway_policy(state, row, &bot, &[route_id.to_owned()], wanted);
    update["expected_version"] = json!(version);
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
            let version = response.body["version"].as_i64().unwrap_or(version + 1);
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .update_one(
                    doc! {"_id": &row.id},
                    doc! {"$set": {"gateway_version": version, "gateway_groups": wanted,
                    "updated_at": bson::DateTime::now()}},
                )
                .await?;
            Ok(None)
        }
        Ok(response) => Ok(Some(gateway_error_code(&response))),
        Err(_) => Ok(Some("gateway_unavailable")),
    }
}

// ---------------------------------------------------------------------------
// Listing, settings and posting
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ChannelChatResponse {
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
}

fn chat_response(row: &NyxbotChannel, chat: &NyxbotThread) -> ChannelChatResponse {
    ChannelChatResponse {
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
    }
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
        .find(doc! {"user_id": owner, "channel_id": {"$in": ids},
        "$or": [{"kind": {"$ne": bson::Bson::Null}},
            {"conversation_id": {"$ne": bson::Bson::Null}}]})
        .sort(doc! {"last_message_at": -1, "updated_at": -1})
        .limit(LIST_LIMIT)
        .await?
        .try_collect()
        .await?;
    Ok(chats
        .iter()
        .filter_map(|chat| {
            channels
                .iter()
                .find(|row| row.id == chat.channel_id)
                .map(|row| chat_response(row, chat))
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
        .find_one(doc! {"_id": chat_id, "user_id": owner})
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
    pub reply_mode: Option<String>,
    pub members: Option<String>,
    pub allow_posts: Option<bool>,
    /// An agent ID, or `default` for the channel's agent.
    pub agent_id: Option<String>,
}

pub(crate) async fn update_chat(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    settings: &ChatSettings,
) -> AppResult<Value> {
    let (row, chat) = load_chat(state, owner, chat_id).await?;
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
    let updated = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one_and_update(doc! {"_id": &chat.id, "user_id": owner}, update)
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;
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
            "allow_posts": settings.allow_posts, "agent_changed": agent_changed.is_some()}),
    )
    .await;
    let mut result = json!({"chat": chat_response(&row, &updated)});
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
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id, "user_id": owner},
            doc! {"$set": {"private_chats": private_chats, "updated_at": bson::DateTime::now()}},
        )
        .await?;
    audit(
        state,
        owner,
        "nyxbot_channel_access_updated",
        json!({"channel_agent_id": &row.id, "private_chats": private_chats}),
    )
    .await;
    Ok(json!({"channel_agent_id": row.id, "private_chats": private_chats}))
}

/// Post a message the agent writes on its own into a chat that allows it.
pub(crate) async fn post(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    text: &str,
    agent_id: Option<&str>,
) -> AppResult<Value> {
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
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_POST_CHARS {
        return Err(AppError::ValidationError(format!(
            "text must have 1 to {MAX_POST_CHARS} characters"
        )));
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
    crate::handlers::channel_relay::check_initiate_rate_limit(state, &chat.id).await?;
    let key_owner = bot_owner(&row);
    let key = key_service::get_api_key(&state.db, key_owner, &row.route_api_key_id).await?;
    let mut auth = crate::handlers::assistant_team::owner_auth(key_owner)?;
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    auth.api_key_id = Some(key.id.clone());
    auth.api_key_name = Some(key.name.clone());
    auth.scope = key.scopes.clone();
    auth.allow_all_services = false;
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
    let sent = crate::handlers::channel_relay::deliver_initiated_message(
        state,
        &HeaderMap::new(),
        &auth,
        &conversation,
        &bot,
        crate::handlers::channel_relay::SendMessageRequest {
            conversation_id: conversation.id.clone(),
            message: crate::handlers::channel_relay::AsyncReplyBody {
                text: Some(text.to_owned()),
                metadata: None,
                attachments: Vec::new(),
            },
            idempotency_key: None,
        },
        adapter.as_ref(),
    )
    .await?;
    audit(
        state,
        owner,
        "nyxbot_channel_chat_posted",
        json!({"channel_agent_id": &row.id, "chat_id": &chat.id,
            "message_id": &sent.message_id}),
    )
    .await;
    Ok(json!({"status": "posted", "chat_id": chat.id, "message_id": sent.message_id}))
}

/// Chats given to a destroyed agent go back to their bot's agent (in new
/// threads).
pub(crate) async fn release_agent_chats(
    state: &AppState,
    owner: &str,
    agent_id: &str,
) -> AppResult<()> {
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_many(
            doc! {"user_id": owner, "agent_id": agent_id},
            doc! {"$unset": {"agent_id": ""}, "$set": {"conversation_id": bson::Bson::Null}},
        )
        .await?;
    Ok(())
}

/// A channel thread's bot and chat, for thread listings.
pub(crate) struct ChatDetails {
    pub bot_label: String,
    pub chat_id: Option<String>,
    pub kind: Option<String>,
    pub title: Option<String>,
}

/// Bot labels and chats of the channel threads among `rows`, by
/// conversation ID. Two queries, whatever the page size.
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
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let (row, _) = load_chat(&state, &owner, &chat_id).await?;
    if row.id != id {
        return Err(AppError::NotFound("Chat not found".into()));
    }
    Ok(Json(update_chat(&state, &owner, &chat_id, &body).await?))
}
