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

fn members_may_talk(chat: &NyxbotThread) -> bool {
    chat.members.as_deref() != Some("owner")
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
    if let Some(thread_id) = facts.thread_id.as_deref() {
        set.insert("platform_thread_id", thread_id);
    }
    if let Some(title) = facts.title.as_deref().and_then(clean_title) {
        set.insert("title", title);
    }
    if let Some(message_id) = last_message_id {
        set.insert("last_message_id", message_id);
    }
    state
        .db
        .collection::<NyxbotThread>(THREADS)
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

/// Look a group's name up once, when the message did not carry it. Best
/// effort: failures only leave the chat untitled.
pub(super) fn spawn_title_lookup(state: &AppState, row: &NyxbotChannel, chat: &NyxbotThread) {
    if chat.title.is_some() || chat.kind.as_deref() == Some("private") {
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
            let bot = channel_bot_service::get_bot(&state.db, &bot_id).await?;
            let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
            let token = crate::services::channel_credentials::resolve_bot_token(
                &state.db,
                &state.encryption_keys,
                adapter.as_ref(),
                &bot,
            )
            .await?;
            let title = adapter
                .chat_title(
                    &state.http_client,
                    &crate::services::channel_platform::BotCredentials {
                        billing: None,
                        token: &token,
                        platform_bot_id: Some(&bot.platform_bot_id),
                        platform_secrets: None,
                    },
                    &chat_id,
                )
                .await?;
            if let Some(title) = title.as_deref().and_then(clean_title) {
                state
                    .db
                    .collection::<NyxbotThread>(THREADS)
                    .update_one(
                        doc! {"_id": &id, "title": bson::Bson::Null},
                        doc! {"$set": {"title": title}},
                    )
                    .await?;
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
/// read from the platform's payload. `None` when the platform's payload does
/// not say (every message then counts as addressed).
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
                        entities
                            .iter()
                            .any(|entity| entity["type"] == "text_mention" && is_bot(&entity["user"]))
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
        .find_one(doc! {"platform": &bot.platform, "platform_message_id": reply_to,
        "direction": "outbound", "channel_bot_id": &bot.id,
        "platform_conversation_id": chat_id})
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
        .collection::<crate::models::channel_message::ChannelMessage>(
            crate::models::channel_message::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": message_id, "channel_bot_id": &row.channel_bot_id,
        "direction": "inbound"})
        .await?
        .and_then(|message| message.reply_to_platform_message_id))
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
}

pub(super) fn admission(
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    sender_id: &str,
    addressed: bool,
) -> Admission {
    let owner = row.owner_sender_ids.iter().any(|id| id == sender_id);
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
    } else {
        Admission::Silent
    }
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

/// Bring the gateway's group admission in line with the chats' settings.
/// Returns a stable error code when the gateway refused the update.
pub(super) async fn sync_gateway_groups(
    state: &AppState,
    row: &NyxbotChannel,
) -> AppResult<Option<&'static str>> {
    if row.transport != "gateway" || row.status != "active" {
        return Ok(None);
    }
    let wanted = wanted_groups(state, row).await?;
    if row.gateway_groups.as_deref().unwrap_or(GATEWAY_GROUPS_DEFAULT) == wanted {
        return Ok(None);
    }
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
    /// Groups and channels: `everyone` or `owner`.
    members: &'static str,
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
        members: if members_may_talk(chat) {
            "everyone"
        } else {
            "owner"
        },
        allow_posts: chat.allow_posts,
        conversation_id: chat.conversation_id.clone(),
        last_message_at: chat.last_message_at,
    }
}

/// Chats of the owner's channels (optionally one channel), most recent
/// first. Gateway sender partitions of a group are not chats.
pub(super) async fn list_chats(
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

pub(super) async fn update_chat(
    state: &AppState,
    owner: &str,
    chat_id: &str,
    settings: &ChatSettings,
) -> AppResult<Value> {
    let (row, chat) = load_chat(state, owner, chat_id).await?;
    let group = chat.kind.as_deref().is_some_and(|kind| kind != "private");
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
        if !matches!(members, "everyone" | "owner") {
            return Err(AppError::ValidationError(
                "members must be everyone or owner".into(),
            ));
        }
        if !group {
            return Err(AppError::ValidationError(
                "Who may talk in private chats is set on the channel bot (private_chats)".into(),
            ));
        }
        set.insert("members", members);
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
        sync_gateway_groups(state, &row).await?
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
pub(super) async fn set_private_chats(
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
pub(super) async fn post(
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
    super::channel_relay::check_initiate_rate_limit(state, &chat.id).await?;
    let key_owner = bot_owner(&row);
    let key = key_service::get_api_key(&state.db, key_owner, &row.route_api_key_id).await?;
    let mut auth = super::assistant_team::owner_auth(key_owner)?;
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
    let sent = super::channel_relay::deliver_initiated_message(
        state,
        &HeaderMap::new(),
        &auth,
        &conversation,
        &bot,
        super::channel_relay::SendMessageRequest {
            conversation_id: conversation.id.clone(),
            message: super::channel_relay::AsyncReplyBody {
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
