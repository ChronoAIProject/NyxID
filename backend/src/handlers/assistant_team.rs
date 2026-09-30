//! NyxBot at the HTTP layer: the owner's persistent personal agent and its
//! specialist agents. Server-started turns, event wake-ups with loop guards,
//! permission routing from specialists to NyxBot, the native team and memory
//! tools, and the agent UI endpoints.
//!
//! Every thread is an ordinary conversation, so a server-started turn is the
//! same detached turn a browser starts, run with the owner's identity and
//! billed to the owner.
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, time::Duration};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::AssistantAcknowledgement,
        assistant_agent::{AgentGrants, AssistantAgent},
        assistant_conversation::{AssistantConversation, TurnOrigin},
    },
    mw::{
        auth::{AuthMethod, AuthUser},
        rate_limit::DirectChatPermit,
    },
    services::{
        assistant_acknowledgement_service::{self as acks, ChatAuthority, Decider},
        assistant_nyxagent::{
            self as engine, TurnError, TurnStart, excerpt, identifier, live_turn,
        },
        assistant_settings_service as settings,
        assistant_team_service::{self as team, TeamRefusal},
        assistant_team_tools,
        coordination_service::RateWindowStore,
    },
};

/// Channel turns are bounded per owner, separately from browser turns.
pub const CHANNEL_TURNS_PER_OWNER: u32 = 2;

/// The owner identity NyxID uses for turns it starts itself (specialist
/// work, wake-ups, channel messages): the same person a browser turn acts for.
pub(crate) fn owner_auth(owner: &str) -> AppResult<AuthUser> {
    let user_id =
        Uuid::parse_str(owner).map_err(|_| AppError::NotFound("Conversation not found".into()))?;
    Ok(AuthUser {
        user_id,
        session_id: None,
        scope: String::new(),
        acting_client_id: None,
        oauth_client_id: None,
        token_jti: None,
        approval_owner_user_id: None,
        auth_method: AuthMethod::Session,
        allow_all_services: true,
        allow_all_nodes: true,
        allowed_service_ids: Vec::new(),
        resource_uris: None,
        allowed_node_ids: Vec::new(),
        api_key_id: None,
        api_key_name: None,
        api_key_credential_id: None,
        api_key_purpose: Default::default(),
        rate_limit_per_second: None,
        rate_limit_burst: None,
        ip_address: None,
        user_agent: None,
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Pool<'a> {
    /// The owner's specialist and event turns, sized by the owner's setting.
    Team { owner: &'a str, limit: u32 },
    /// Channel turns for one owner.
    Channel { owner: &'a str },
}

pub(crate) enum Started {
    Turn {
        conversation: Box<AssistantConversation>,
        receiver: broadcast::Receiver<Value>,
    },
    /// The thread already has a live turn.
    Busy,
    /// The pool has no free slot.
    PoolFull,
}

async fn acquire(state: &AppState, pool: Pool<'_>) -> AppResult<Option<DirectChatPermit>> {
    match pool {
        Pool::Team { owner, limit } => {
            state
                .direct_chat_limiter
                .try_acquire_pool("assistant_team", owner, limit)
                .await
        }
        Pool::Channel { owner } => {
            state
                .direct_chat_limiter
                .try_acquire_pool("assistant_channel", owner, CHANNEL_TURNS_PER_OWNER)
                .await
        }
    }
}

/// Start a turn NyxID initiates. Never queues: a full pool or a live turn is
/// reported so the caller (usually NyxBot) decides what to wait for.
pub(crate) async fn start_server_turn(
    state: &AppState,
    owner: &str,
    start: TurnStart,
    pool: Pool<'_>,
) -> AppResult<Started> {
    let Some(permit) = acquire(state, pool).await? else {
        return Ok(Started::PoolFull);
    };
    start_acquired(state, owner, start, permit).await
}

async fn start_acquired(
    state: &AppState,
    owner: &str,
    start: TurnStart,
    permit: DirectChatPermit,
) -> AppResult<Started> {
    match super::assistant_nyxagent::start_turn(
        state,
        owner_auth(owner)?,
        &start,
        Some(super::assistant_nyxagent::SERVER_TURN_POLICY),
        permit,
    )
    .await
    {
        Ok((conversation, receiver)) => Ok(Started::Turn {
            conversation: Box::new(conversation),
            receiver,
        }),
        Err(AppError::AssistantTurnActive) => Ok(Started::Busy),
        Err(error) => Err(error),
    }
}

pub(crate) async fn team_pool_limit(state: &AppState, owner: &str) -> u32 {
    settings::get(&state.db, owner)
        .await
        .map(|row| row.max_concurrent_subagent_turns.max(1) as u32)
        .unwrap_or(crate::models::assistant_settings::DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS as u32)
}

fn event_turn(conversation_id: &str) -> TurnStart {
    TurnStart {
        conversation_id: Some(conversation_id.to_owned()),
        text: String::new(),
        model: None,
        origin: TurnOrigin::Event,
        channel: None,
        title: None,
        note: None,
        new_id: None,
        agent_id: None,
        report_to: None,
        group_id: None,
        guest: false,
        question_key: None,
        question: None,
        reply_channel: None,
    }
}

/// Start an event turn draining a thread's queue when it is idle. Loop
/// guards: a NyxBot thread stops after `MAX_EVENT_STREAK` consecutive event
/// turns without a user message, and an owner runs at most
/// `EVENT_TURNS_PER_HOUR` event turns per hour. Events stay queued otherwise
/// and reach the agent with its next turn.
pub(crate) async fn wake(state: &AppState, owner: &str, id: &str) {
    let result: AppResult<()> = async {
        let row = engine::get(&state.db, owner, id).await?;
        if row.pending_events.is_empty() || live_turn(&row, Utc::now()).is_some() {
            return Ok(());
        }
        if !row.is_subagent() && row.event_streak >= team::MAX_EVENT_STREAK {
            return Ok(());
        }
        // A destroyed agent never runs again; events that reached it late are dropped.
        if team::agent_for_conversation(&state.db, &row)
            .await?
            .destroyed_at
            .is_some()
        {
            return team::drop_events(&state.db, owner, id).await;
        }
        // Take a pool slot before spending the hourly budget, so a full pool
        // never uses it up. NyxBot's event turns share the pool with working
        // specialists.
        let limit = team_pool_limit(state, owner).await + 1;
        let Some(permit) = acquire(state, Pool::Team { owner, limit }).await? else {
            return Ok(());
        };
        if !RateWindowStore::admit(
            &state.db,
            "assistant_event_turns",
            owner,
            team::EVENT_TURNS_PER_HOUR,
            Duration::from_secs(3600),
        )
        .await?
        .allowed
        {
            return Ok(());
        }
        match start_acquired(state, owner, event_turn(id), permit).await {
            // A drained-then-raced queue or a just-destroyed agent is harmless.
            Ok(_) | Err(AppError::Conflict(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(conversation_id = %id, %error, "NyxBot wake-up deferred");
    }
}

/// NyxBot's home thread, where events land that no thread asked for.
pub(crate) async fn nyxbot_home(state: &AppState, owner: &str) -> AppResult<AssistantConversation> {
    let nyxbot = team::ensure_nyxbot(&state.db, owner).await?;
    team::home_thread(&state.db, &state.encryption_keys, &nyxbot).await
}

/// Queue events on a thread and wake it.
pub(crate) async fn notify(
    state: &AppState,
    owner: &str,
    conversation_id: &str,
    events: Vec<crate::models::assistant_conversation::AgentEvent>,
) {
    if engine::push_events(&state.db, owner, conversation_id, events)
        .await
        .is_ok()
    {
        wake(state, owner, conversation_id).await;
    }
}

/// Type-erased settlement hook. `run_turn` awaits this so the detached turn's
/// future type never depends on the turns it may start.
pub(crate) fn after_turn_boxed(
    state: AppState,
    row: AssistantConversation,
    text: String,
    error: Option<TurnError>,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move { after_turn(&state, &row, &text, error.as_ref()).await })
}

/// Runs after a turn's reply is durable: report NyxBot-assigned specialist
/// work to the NyxBot thread that assigned it, drain queues, and deliver
/// asynchronous replies of channel threads to their chat.
pub(crate) async fn after_turn(
    state: &AppState,
    row: &AssistantConversation,
    text: &str,
    error: Option<&TurnError>,
) {
    let Some(turn) = row.active_turn.as_ref() else {
        return;
    };
    let owner = row.user_id.as_str();
    // Every reply of a hidden group member thread belongs to the group,
    // including event turns (a finished link, a confirmed action).
    if row.group_id.is_some() {
        super::assistant_group::member_settled(state, row, text, error.map(|error| error.code))
            .await;
    }
    // A direct user chat with a specialist does not wake NyxBot; it reads
    // those on its next turn.
    if row.is_subagent()
        && matches!(turn.origin, TurnOrigin::Orchestrator | TurnOrigin::Event)
        && let Some(target) = row.report_to.as_deref()
        && let Ok(agent) = team::agent_for_conversation(&state.db, row).await
    {
        let status = match error.map(|error| error.code) {
            None => "replied".to_owned(),
            Some("cancelled") => "was stopped".to_owned(),
            Some(code) => format!("failed ({code})"),
        };
        let note = format!(
            "Specialist {} {status}. Reply excerpt: \"{}\" Read more with nyxid__read_subagent.",
            identifier(&agent.name),
            excerpt(text, 1200).replace('"', "'")
        );
        notify(
            state,
            owner,
            target,
            vec![team::event("subagent_settled", note, Some(&agent.id))],
        )
        .await;
    }
    // Wake whoever has queued work now that a slot is free.
    wake(state, owner, &row.id).await;
    if let Ok(rows) = team::queued(&state.db, Some(owner)).await {
        for other in rows.iter().filter(|other| other.id != row.id).take(16) {
            wake(state, owner, &other.id).await;
        }
    }
    // Chats that asked the same question while it was being answered get the
    // answer too (taken once, whatever the outcome).
    let also = engine::take_deliveries(&state.db, owner, &row.id)
        .await
        .unwrap_or_default();
    if error.is_some() {
        // They were told an answer would come: say it did not.
        for origin in &also {
            super::nyxbot::deliver_to(
                state,
                row,
                origin,
                "I couldn't finish answering that question. Please ask again.",
            )
            .await;
        }
        return;
    }
    // Only asynchronous event turns reach the chat: a channel turn answers its
    // own event, and a turn the owner starts in the web app stays in the web app.
    if (row.channel.is_some() || row.reply_channel.is_some()) && turn.origin == TurnOrigin::Event {
        super::nyxbot::deliver_update(state, row, text).await;
    }
    for origin in &also {
        super::nyxbot::deliver_to(state, row, origin, text).await;
    }
}

/// Where a specialist's permission request goes: the NyxBot thread that
/// assigned its current work, otherwise NyxBot's home thread.
async fn request_target(state: &AppState, chat: &ChatAuthority) -> AppResult<String> {
    let row = engine::get(&state.db, &chat.user_id, &chat.conversation_id).await?;
    let assigned = row
        .active_turn
        .as_ref()
        .is_some_and(|turn| matches!(turn.origin, TurnOrigin::Orchestrator | TurnOrigin::Event));
    if assigned && let Some(target) = row.report_to {
        return Ok(target);
    }
    Ok(nyxbot_home(state, &chat.user_id).await?.id)
}

/// A specialist asked for permission its grants do not cover: queue an event
/// for NyxBot and wake it.
pub(crate) async fn permission_requested(
    state: &AppState,
    chat: &ChatAuthority,
    request: &AssistantAcknowledgement,
) {
    if chat.is_orchestrator() {
        return;
    }
    let Ok(target) = request_target(state, chat).await else {
        return;
    };
    let target_name = match request.kind.as_str() {
        "service" => format!(
            "service {}",
            identifier(request.service_slug.as_deref().unwrap_or_default())
        ),
        _ => "read-only account access".into(),
    };
    let note = format!(
        "Specialist {} requests {target_name} (request_id {}). It was working on: {} \
        Decide with nyxid__decide_permission: allow only what the user's request needs.",
        identifier(&chat.agent_name),
        request.id,
        request
            .request_excerpt
            .as_deref()
            .map(|text| format!("\"{}\".", excerpt(text, 600).replace('"', "'")))
            .unwrap_or_else(|| "(no text)".into()),
    );
    notify(
        state,
        &chat.user_id,
        &target,
        vec![team::event(
            "permission_requested",
            note,
            Some(&chat.agent_id),
        )],
    )
    .await;
}

/// A specialist's request was decided (by NyxBot or the user): resume the
/// specialist thread that asked.
pub(crate) async fn permission_decided(
    state: &AppState,
    owner: &str,
    request: &AssistantAcknowledgement,
) {
    let allowed = request.status == "allowed";
    let target = match request.kind.as_str() {
        "service" => format!(
            "service {}",
            identifier(request.service_slug.as_deref().unwrap_or_default())
        ),
        _ => "read-only account access".into(),
    };
    let by = match request.decided_by.as_deref() {
        Some("orchestrator") => "NyxBot",
        _ => "The user",
    };
    let reason = request
        .reason
        .as_deref()
        .map(|reason| format!(" Reason: \"{}\".", excerpt(reason, 300).replace('"', "'")))
        .unwrap_or_default();
    let note = format!(
        "{by} {} your request for {target}.{reason} {}",
        if allowed { "allowed" } else { "denied" },
        if allowed {
            "Retry the call now and continue your task."
        } else {
            "Do not retry it; finish what you can and report."
        }
    );
    notify(
        state,
        owner,
        &request.conversation_id,
        vec![team::event("permission_decided", note, None)],
    )
    .await;
}

/// The instructions of a turn for someone other than the owner. NyxBot holds
/// every service of the owner, so it uses none for other people.
fn guest_note(specialist: bool) -> &'static str {
    if specialist {
        "\n\nThis turn answers someone other than the owner (a member of a chat your channel \
        bot is in). Help them with your services (look things up, turn things on or off, \
        create and update), but only the owner can ask for account actions, new connections, \
        more access or deleting anything: NyxID refuses those, so say that only the bot's \
        owner can ask for that. Never reveal the owner's private information (their account, \
        other chats, memory or credentials)."
    } else {
        "\n\nThis turn answers someone other than the owner (a member of a chat the owner's \
        channel bot is in). Answer from the conversation only: you use no tools or services \
        for them, and only the owner can ask you to act. If they need a service, say the \
        owner can give this chat its own agent with just that service. Never reveal the \
        owner's private information (their account, services, other chats, memory or \
        credentials)."
    }
}

/// What the agent's other threads are working on right now, so it neither
/// redoes that work nor starts it twice. Lookup failures only omit it.
async fn in_progress_note(
    state: &AppState,
    row: &AssistantConversation,
    agent: &AssistantAgent,
) -> String {
    let Ok(others) = team::in_progress(&state.db, &row.user_id, &agent.id, &row.id).await else {
        return String::new();
    };
    if others.is_empty() {
        return String::new();
    }
    let this = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.question_key.as_deref());
    let mut note = String::from(
        "\n\nYour other threads are working on these right now; do not start the same work \
        again (if you are asked the same thing, say it is in progress there):",
    );
    for (title, question, key) in others {
        note.push_str(&format!(
            "\n- \"{}\": \"{}\"{}",
            excerpt(&title, 60).replace('"', "'"),
            excerpt(question.as_deref().unwrap_or("(working)"), 200).replace('"', "'"),
            if this.is_some() && key.as_deref() == this {
                " (the same question as this one)"
            } else {
                ""
            }
        ));
    }
    note
}

/// Turn-scoped notes appended to the instructions: drained events, a channel
/// sender's context, the agent's memory, and for NyxBot its roster, direct
/// user chats with specialists and pending permission requests. NyxID-authored
/// and bounded; lookup failures only omit a note.
pub(crate) async fn turn_notes(
    state: &AppState,
    row: &AssistantConversation,
    agent: Option<&AssistantAgent>,
    previous_user_message: Option<DateTime<Utc>>,
) -> String {
    let mut notes = String::new();
    if let Some(turn) = row.active_turn.as_ref() {
        if turn.origin != TurnOrigin::Event && !turn.events.is_empty() {
            notes.push_str(
                "\n\nNyxID events since your previous turn (authored by NyxID; only a quoted \
                owner message is a request from the user):",
            );
            for event in &turn.events {
                notes.push_str("\n- ");
                notes.push_str(&excerpt(&event.text, engine::event_text_limit(event)));
            }
        }
        if let Some(note) = turn.note.as_deref() {
            notes.push_str("\n\n");
            notes.push_str(note);
        }
    }
    // Someone other than the owner is talking: nothing private to the owner
    // (memory, other chats, the team, pending requests) goes into this turn.
    if row.guest_turn {
        notes.push_str(guest_note(row.is_subagent()));
        return notes;
    }
    if let Some(agent) = agent {
        notes.push_str(&team::memory_note(agent));
        // Only the agent's own threads hear about its other chats.
        if row.channel.is_none() {
            notes.push_str(&in_progress_note(state, row, agent).await);
        }
        if let Some(note) = super::assistant_group::group_note(state, row, agent).await {
            notes.push_str("\n\n");
            notes.push_str(&note);
        }
    }
    if row.is_subagent() {
        return notes;
    }
    let owner = row.user_id.as_str();
    notes.push_str(
        &team::roster_note(&state.db, owner)
            .await
            .unwrap_or_default(),
    );
    if let Some(since) = previous_user_message {
        notes.push_str(
            &team::direct_chats_note(&state.db, owner, since)
                .await
                .unwrap_or_default(),
        );
    }
    if let Ok(requests) = team::pending_requests(&state.db, owner).await
        && let Ok(summaries) = team::request_summaries(&state.db, owner, &requests).await
        && !summaries.is_empty()
    {
        notes.push_str(
            "\n\nPending specialist permission requests (decide with nyxid__decide_permission):",
        );
        for request in summaries.iter().take(10) {
            notes.push_str(&format!(
                "\n- request_id {} from {}: {} {}",
                request.request_id,
                identifier(&request.agent),
                identifier(&request.kind),
                identifier(request.service_slug.as_deref().unwrap_or("account"))
            ));
        }
    }
    notes
}

fn refusal(error: &str, instructions: &str) -> (Value, bool) {
    (json!({"error": error, "instructions": instructions}), true)
}

fn text_arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or_default()
}

fn string_list(args: &Value, key: &str) -> Vec<String> {
    args[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn started_json(started: Started) -> Value {
    match started {
        Started::Turn { conversation, .. } => json!({
            "status": "started",
            "turn_id": conversation.active_turn.as_ref().map(|turn| turn.turn_id.clone()),
            "note": "NyxID wakes you with an event when it reports; or use nyxid__wait_for_subagents.",
        }),
        Started::Busy => json!({"status": "busy",
            "note": "It is already working; wait for it before sending more."}),
        Started::PoolFull => json!({"status": "pool_full",
            "note": "Too many specialists are working. Wait for one to finish, then retry."}),
    }
}

/// Give a specialist work in its home thread, reporting to `report_to`.
pub(crate) async fn assign(
    state: &AppState,
    owner: &str,
    agent: &AssistantAgent,
    text: &str,
    report_to: Option<&str>,
) -> AppResult<Started> {
    let home = team::home_thread(&state.db, &state.encryption_keys, agent).await?;
    let limit = team_pool_limit(state, owner).await;
    start_server_turn(
        state,
        owner,
        TurnStart {
            conversation_id: Some(home.id),
            text: text.to_owned(),
            model: None,
            origin: TurnOrigin::Orchestrator,
            channel: None,
            title: None,
            note: None,
            new_id: None,
            agent_id: None,
            report_to: report_to.map(str::to_owned),
            group_id: None,
            guest: false,
            question_key: None,
            question: None,
            reply_channel: None,
        },
        Pool::Team { owner, limit },
    )
    .await
}

/// Resolve `nyxbot` or a live specialist name/ID to an agent.
async fn target_agent(
    state: &AppState,
    owner: &str,
    name: Option<&str>,
) -> AppResult<AssistantAgent> {
    match name.map(str::trim) {
        None | Some("") | Some("nyxbot") | Some("NyxBot") => {
            team::ensure_nyxbot(&state.db, owner).await
        }
        Some(name) => team::live_specialist(&state.db, owner, name).await,
    }
}

/// Dispatch one native NyxBot tool. Returns the MCP result value and whether
/// it is an error result. Team and channel tools need a NyxBot thread key;
/// memory tools belong to every agent.
pub(crate) async fn execute_tool(
    state: &AppState,
    chat: &ChatAuthority,
    tool_name: &str,
    args: &Value,
) -> (Value, bool) {
    let name = tool_name.strip_prefix("nyxid__").unwrap_or_default();
    if !chat.is_orchestrator() && !assistant_team_tools::is_agent_tool(name) {
        return refusal(
            "orchestrator_only",
            "Only NyxBot manages agents and channel bots. Report what you need in your reply.",
        );
    }
    let result = async {
        assistant_team_tools::validate(name, args)?;
        engine::require_enabled(&state.db, &chat.user_id).await?;
        dispatch(state, chat, name, args).await
    }
    .await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            let tool = crate::services::assistant_account_tools::error_result(error);
            (tool.value, true)
        }
    };
    let _ = crate::services::audit_service::log_actor_event(
        state.db.clone(),
        &crate::services::audit_service::AuditActor {
            user_id: chat.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: Some(chat.api_key_id.clone()),
            api_key_name: None,
        },
        "assistant_team_tool_call",
        Some(json!({
            "conversation_id": chat.conversation_id,
            "agent_id": chat.agent_id,
            "tool_name": tool_name,
            "outcome": if outcome.1 {"refused"} else {"success"},
        })),
    )
    .await;
    outcome
}

async fn dispatch(
    state: &AppState,
    chat: &ChatAuthority,
    name: &str,
    args: &Value,
) -> AppResult<(Value, bool)> {
    let db = &state.db;
    let owner = chat.user_id.as_str();
    let caller = chat.conversation_id.as_str();
    Ok(match name {
        "remember" => {
            let note = team::remember(
                db,
                owner,
                &chat.agent_id,
                text_arg(args, "text"),
                args["replace_id"].as_str(),
            )
            .await?;
            (json!({"remembered": note.id}), false)
        }
        "forget" => {
            team::forget(db, owner, &chat.agent_id, text_arg(args, "note_id")).await?;
            (json!({"forgotten": text_arg(args, "note_id")}), false)
        }
        "spawn_subagent" => {
            let targets = team::resolve_targets(
                db,
                state.node_ws_manager.as_ref(),
                owner,
                &string_list(args, "services"),
            )
            .await?;
            let request = team::CreateRequest {
                name: text_arg(args, "name").to_owned(),
                description: text_arg(args, "description").to_owned(),
                display_name: args["display_name"].as_str().map(str::to_owned),
                persona: args["persona"].as_str().map(str::to_owned),
                targets: targets.clone(),
                account_read: args["account_read"].as_bool().unwrap_or(false),
                specialty: args["specialty"].as_str().map(str::to_owned),
                created_by: "nyxbot",
            };
            match team::create_specialist(db, &state.encryption_keys, owner, request).await? {
                Err(TeamRefusal::LimitReached { limit }) => (
                    json!({"error": "limit_reached", "limit": limit,
                        "instructions": "Reuse or destroy a specialist first, or tell the user \
                            they can raise the limit in NyxBot settings."}),
                    true,
                ),
                Err(TeamRefusal::NameTaken) => refusal(
                    "name_taken",
                    "A live specialist already uses this name; message it or pick another name.",
                ),
                Ok((agent, _)) => {
                    let task = match args["task"].as_str() {
                        Some(task) => {
                            started_json(assign(state, owner, &agent, task, Some(caller)).await?)
                        }
                        None => json!({"status": "idle",
                            "note": "Give it work with nyxid__message_subagent."}),
                    };
                    (
                        json!({"subagent": {"id": agent.id, "name": agent.name,
                            "services": targets.slugs, "account_read": agent.grants.account_read},
                            "task": task}),
                        false,
                    )
                }
            }
        }
        "message_subagent" => {
            let agent = team::live_specialist(db, owner, text_arg(args, "subagent")).await?;
            (
                started_json(
                    assign(state, owner, &agent, text_arg(args, "text"), Some(caller)).await?,
                ),
                false,
            )
        }
        "wait_for_subagents" => (wait_for(state, chat, args).await?, false),
        "list_subagents" => {
            let rows = team::summaries(
                db,
                owner,
                false,
                args["include_destroyed"].as_bool().unwrap_or(false),
                300,
            )
            .await?;
            (json!({"subagents": rows}), false)
        }
        "read_subagent" => {
            let agent = team::specialist(db, owner, text_arg(args, "subagent")).await?;
            let rows = team::read(
                db,
                owner,
                &agent,
                args["limit"].as_i64().unwrap_or(team::READ_LIMIT),
            )
            .await?;
            (json!({"subagent": agent.name, "messages": rows}), false)
        }
        "grant_subagent" | "revoke_subagent" => {
            let agent = team::live_specialist(db, owner, text_arg(args, "subagent")).await?;
            let requested = string_list(args, "services");
            let (targets, refused) =
                team::resolve_each_target(db, state.node_ws_manager.as_ref(), owner, &requested)
                    .await?;
            let unchanged = if name == "grant_subagent" {
                "not_granted"
            } else {
                "not_revoked"
            };
            // Nothing usable was asked for: say why instead of changing nothing.
            if !refused.is_empty()
                && targets.service_ids.is_empty()
                && targets.platform_service_ids.is_empty()
                && args.get("account_read").is_none()
            {
                return Ok((
                    json!({"error": unchanged, "subagent": agent.name, unchanged: refused}),
                    true,
                ));
            }
            let targets = AgentGrants {
                service_ids: targets.service_ids,
                platform_service_ids: targets.platform_service_ids,
                account_read: args["account_read"].as_bool().unwrap_or(false),
            };
            let change = if name == "grant_subagent" {
                team::GrantChange::Add(targets)
            } else {
                team::GrantChange::Remove(targets)
            };
            let agent = team::set_grants(db, owner, &agent.id, change).await?;
            let summary = team::summaries(db, owner, false, false, 0)
                .await?
                .into_iter()
                .find(|summary| summary.id == agent.id);
            let mut result = json!({"subagent": agent.name,
                "services": summary.as_ref().map(|s| s.services.clone()),
                "account_read": agent.grants.account_read});
            if !refused.is_empty() {
                result[unchanged] = json!(refused);
            }
            (result, false)
        }
        "update_subagent" => {
            // "nyxbot" updates your own display name and description. Your own
            // persona shapes every NyxBot thread's instructions, so only the
            // owner changes it (agent details), never text you read.
            let agent = target_agent(state, owner, Some(text_arg(args, "subagent"))).await?;
            if agent.is_nyxbot() && args.get("persona").is_some() {
                return Err(AppError::ValidationError(
                    "Only the user can change NyxBot's persona: ask them to edit it in \
                    NyxBot's agent details"
                        .into(),
                ));
            }
            let agent = team::update_agent(
                db,
                owner,
                &agent.id,
                args["name"].as_str(),
                args["description"].as_str(),
                team::AgentStyle {
                    display_name: args["display_name"].as_str(),
                    persona: args["persona"].as_str(),
                },
            )
            .await?;
            (
                json!({"agent": agent.name, "id": agent.id,
                    "display_name": agent.display_name, "persona": agent.persona}),
                false,
            )
        }
        "decide_permission" => {
            let allow = text_arg(args, "decision") == "allow";
            let request_id = text_arg(args, "request_id");
            if Uuid::parse_str(request_id).is_err() {
                return Err(AppError::NotFound("Request not found".into()));
            }
            let row = acks::decide_as(
                db,
                owner,
                None,
                request_id,
                allow,
                Decider::Nyxbot,
                Some(text_arg(args, "reason")),
            )
            .await?;
            acks::audit_decision(
                db,
                &crate::services::audit_service::AuditActor {
                    user_id: owner.into(),
                    ip_address: None,
                    user_agent: None,
                    api_key_id: Some(chat.api_key_id.clone()),
                    api_key_name: None,
                },
                &row,
            )
            .await;
            permission_decided(state, owner, &row).await;
            (
                json!({"request_id": row.id, "status": row.status,
                    "note": "The specialist was resumed with your decision."}),
                false,
            )
        }
        "destroy_subagent" => {
            let agent = team::live_specialist(db, owner, text_arg(args, "subagent")).await?;
            let agent = destroy_agent(state, owner, &agent.id).await?;
            (json!({"destroyed": agent.name}), false)
        }
        "create_group" => {
            let mut ids = Vec::new();
            for name in string_list(args, "members") {
                ids.push(target_agent(state, owner, Some(&name)).await?.id);
            }
            let group = crate::services::assistant_group_service::create(
                db,
                owner,
                text_arg(args, "name"),
                &ids,
                "nyxbot",
            )
            .await?;
            (
                json!({"group": {"id": group.id, "name": group.name},
                    "note": "Post to it with nyxid__post_to_group; the user sees it under Groups."}),
                false,
            )
        }
        "list_groups" => {
            let mut rows = Vec::new();
            for group in crate::services::assistant_group_service::list(db, owner).await? {
                let members =
                    crate::services::assistant_group_service::members(db, owner, &group).await?;
                rows.push(json!({"id": group.id, "name": group.name,
                    "members": members.iter().map(|agent| agent.name.clone()).collect::<Vec<_>>(),
                    "messages": group.message_count}));
            }
            (json!({"groups": rows}), false)
        }
        "post_to_group" => {
            let group =
                crate::services::assistant_group_service::find(db, owner, text_arg(args, "group"))
                    .await?;
            // Inside the group, your reply is your post.
            if engine::get(db, owner, caller).await?.group_id.as_deref() == Some(&group.id) {
                return Err(AppError::ValidationError(
                    "You are in this group: reply directly instead of posting to it".into(),
                ));
            }
            let author = team::ensure_nyxbot(db, owner).await?;
            let (message, addressed) = super::assistant_group::post(
                state,
                owner,
                &group.id,
                text_arg(args, "text"),
                Some(&author),
            )
            .await?;
            // This thread is woken with the members' replies once the group
            // is quiet.
            if !addressed.is_empty() {
                crate::services::assistant_group_service::follow(
                    db,
                    owner,
                    &group.id,
                    caller,
                    message.seq,
                )
                .await?;
            }
            (
                json!({"posted": message.seq, "addressed_agent_ids": addressed,
                "note": if addressed.is_empty() {
                    "Nobody was addressed: @mention members to have them answer."
                } else {
                    "Members answer in the group. End your turn: NyxID wakes you with \
                    their replies when the group is quiet."
                }}),
                false,
            )
        }
        "update_group" => {
            let group =
                crate::services::assistant_group_service::find(db, owner, text_arg(args, "group"))
                    .await?;
            let mut ids = group.member_agent_ids.clone();
            for name in string_list(args, "add") {
                let id = target_agent(state, owner, Some(&name)).await?.id;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            for name in string_list(args, "remove") {
                let id = target_agent(state, owner, Some(&name)).await?.id;
                ids.retain(|member| member != &id);
            }
            let group = crate::services::assistant_group_service::update(
                db,
                owner,
                &group.id,
                args["name"].as_str(),
                Some(&ids),
            )
            .await?;
            (
                json!({"group": {"id": group.id, "name": group.name}}),
                false,
            )
        }
        "delete_group" => {
            let group =
                crate::services::assistant_group_service::find(db, owner, text_arg(args, "group"))
                    .await?;
            crate::services::assistant_group_service::delete(db, owner, &group.id).await?;
            (json!({"deleted": group.name}), false)
        }
        "settings_link" => {
            let area = text_arg(args, "area");
            let path = crate::services::assistant_team_tools::settings_path(
                area,
                args["service"].as_str(),
                args["org_id"].as_str(),
            )
            .ok_or_else(|| AppError::ValidationError("Unknown settings area".into()))?;
            (
                json!({"url": format!("{}{path}",
                    state.config.frontend_url.trim_end_matches('/')),
                    "area": area,
                    "note": "Give the user this link; secrets and sign-in-only changes happen \
                        on that page, never in chat."}),
                false,
            )
        }
        "channel_bot_setup_link" => {
            let agent = target_agent(state, owner, args["agent"].as_str()).await?;
            super::nyxbot::setup_link_tool(
                state,
                owner,
                caller,
                text_arg(args, "platform"),
                args["label"].as_str(),
                &agent,
            )
            .await?
        }
        "connect_channel_bot" => {
            let agent = target_agent(state, owner, args["agent"].as_str()).await?;
            // By id or by label, among the owner's bots and their orgs' bots.
            let reference = args["bot"]
                .as_str()
                .or_else(|| args["bot_id"].as_str())
                .ok_or_else(|| AppError::ValidationError("bot is required".into()))?;
            let bot = super::nyxbot::resolve_bot_ref(state, owner, reference).await?;
            super::nyxbot::connect_tool(state, owner, caller, &bot.id, &agent).await?
        }
        "link_channel_bot" => {
            let agent = target_agent(state, owner, args["agent"].as_str()).await?;
            (
                super::nyxbot::link(state, owner, text_arg(args, "channel_agent_id"), &agent)
                    .await?,
                false,
            )
        }
        "list_channel_agents" => (super::nyxbot::list_tool(state, owner).await?, false),
        "list_channel_chats" => {
            let chats = Box::pin(super::nyxbot::chats::list_chats(
                state,
                owner,
                args["channel_agent_id"].as_str(),
            ))
            .await?;
            (json!({"chats": chats}), false)
        }
        "update_channel_chat" => {
            let agent_id = match args["agent"].as_str() {
                Some("default") => Some("default".to_owned()),
                Some(name) => Some(target_agent(state, owner, Some(name)).await?.id),
                None => None,
            };
            let settings = super::nyxbot::chats::ChatSettings {
                reply_mode: args["reply_mode"].as_str().map(str::to_owned),
                members: args["members"].as_str().map(str::to_owned),
                allow_posts: args["allow_posts"].as_bool(),
                agent_id,
            };
            // Boxed: the gateway update is a large future.
            (
                Box::pin(super::nyxbot::chats::update_chat(
                    state,
                    owner,
                    text_arg(args, "chat_id"),
                    &settings,
                ))
                .await?,
                false,
            )
        }
        "update_channel_access" => (
            Box::pin(super::nyxbot::chats::set_private_chats(
                state,
                owner,
                text_arg(args, "channel_agent_id"),
                text_arg(args, "private_chats"),
            ))
            .await?,
            false,
        ),
        "post_to_chat" => {
            // NyxBot posts to any of the owner's chats; a specialist only to
            // chats it answers.
            let agent = (!chat.is_orchestrator()).then_some(chat.agent_id.as_str());
            // Boxed: the platform send is a large future.
            (
                Box::pin(super::nyxbot::chats::post(
                    state,
                    owner,
                    text_arg(args, "chat_id"),
                    text_arg(args, "text"),
                    agent,
                ))
                .await?,
                false,
            )
        }
        "disconnect_channel_bot" => (
            super::nyxbot::disconnect(state, owner, text_arg(args, "channel_agent_id")).await?,
            false,
        ),
        _ => return Err(AppError::NotFound("NyxBot tool not found".into())),
    })
}

/// Destroy a specialist and disconnect every channel bot linked to it.
pub(crate) async fn destroy_agent(
    state: &AppState,
    owner: &str,
    agent_id: &str,
) -> AppResult<AssistantAgent> {
    let agent = team::destroy(&state.db, owner, agent_id).await?;
    super::nyxbot::chats::release_agent_chats(state, owner, &agent.id).await?;
    for channel in super::nyxbot::list(state, owner).await? {
        if channel.agent_id.as_deref() == Some(agent.id.as_str())
            && let Err(error) = super::nyxbot::disconnect(state, owner, &channel.id).await
        {
            tracing::warn!(%error, "Channel of a destroyed agent was not disconnected");
        }
    }
    Ok(agent)
}

/// Bounded wait. Returns settled replies, who is still running, and pending
/// permission requests; delivered settlements are removed from the calling
/// thread's queue so they do not also wake it later.
async fn wait_for(state: &AppState, chat: &ChatAuthority, args: &Value) -> AppResult<Value> {
    let db = &state.db;
    let owner = chat.user_id.as_str();
    let caller = chat.conversation_id.as_str();
    let timeout = args["timeout_secs"]
        .as_u64()
        .unwrap_or(60)
        .clamp(1, team::MAX_WAIT_SECS);
    let names = string_list(args, "subagents");
    let mut targets = Vec::new();
    if names.is_empty() {
        targets = team::agents(db, owner, false)
            .await?
            .into_iter()
            .filter(|agent| !agent.is_nyxbot())
            .map(|agent| agent.id)
            .collect();
    } else {
        for name in &names {
            targets.push(team::specialist(db, owner, name).await?.id);
        }
    }
    let own_turn = engine::get(db, owner, caller)
        .await?
        .active_turn
        .map(|turn| turn.turn_id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let running = team::running_agents(db, owner).await?;
        let busy = targets.iter().any(|id| running.contains(id));
        let requests = team::pending_requests(db, owner).await?;
        let stopped = match own_turn.as_deref() {
            Some(turn_id) => engine::stop_requested(db, owner, caller, turn_id).await?,
            None => false,
        };
        if !busy || !requests.is_empty() || stopped || tokio::time::Instant::now() >= deadline {
            let summaries = team::summaries(db, owner, false, true, team::REPLY_EXCERPT_CHARS)
                .await?
                .into_iter()
                .filter(|summary| targets.contains(&summary.id))
                .collect::<Vec<_>>();
            let settled: Vec<String> = summaries
                .iter()
                .filter(|summary| summary.status != "running")
                .map(|summary| summary.id.clone())
                .collect();
            team::consume_settled_events(db, owner, caller, &settled).await?;
            return Ok(json!({
                "settled": summaries.iter().filter(|s| s.status != "running").map(|s| json!({
                    "name": s.name, "status": s.status, "reply": s.last_reply,
                })).collect::<Vec<_>>(),
                "running": summaries.iter().filter(|s| s.status == "running")
                    .map(|s| s.name.clone()).collect::<Vec<_>>(),
                "pending_requests": team::request_summaries(db, owner, &requests).await?,
            }));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ---------------------------------------------------------------------------
// Agent UI endpoints (human-only router)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ChannelLinkResponse {
    id: String,
    platform: String,
    bot_label: String,
    status: String,
}

#[derive(Serialize)]
pub struct AgentResponse {
    #[serde(flatten)]
    summary: team::AgentSummary,
    pending_acknowledgements: usize,
    channels: Vec<ChannelLinkResponse>,
}

async fn agent_responses(
    state: &AppState,
    owner: &str,
    include_destroyed: bool,
) -> AppResult<Vec<AgentResponse>> {
    let nyxbot = team::ensure_nyxbot(&state.db, owner).await?;
    let summaries = team::summaries(&state.db, owner, true, include_destroyed, 300).await?;
    let channels = super::nyxbot::list(state, owner).await?;
    Ok(summaries
        .into_iter()
        .map(|summary| {
            let linked = channels
                .iter()
                .filter(|channel| {
                    channel.agent_id.as_deref().unwrap_or(nyxbot.id.as_str()) == summary.id
                })
                .map(|channel| ChannelLinkResponse {
                    id: channel.id.clone(),
                    platform: channel.platform.clone(),
                    bot_label: channel.bot_label.clone(),
                    status: channel.status.clone(),
                })
                .collect();
            AgentResponse {
                pending_acknowledgements: summary.pending_requests.len(),
                channels: linked,
                summary,
            }
        })
        .collect())
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsQuery {
    include_destroyed: Option<bool>,
}

/// The owner's NyxBot and specialists. Creates NyxBot on first use.
pub async fn list_agents(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<AgentsQuery>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let agents = agent_responses(&state, &owner, query.include_destroyed.unwrap_or(false)).await?;
    Ok(Json(json!({
        "agents": agents,
        "limits": SettingsResponse::from(settings::get(&state.db, &owner).await?),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentRequest {
    name: String,
    description: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    persona: Option<String>,
    #[serde(default)]
    services: Vec<String>,
    #[serde(default)]
    account_read: bool,
}

/// The owner creates a specialist directly.
pub async fn create_agent(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateAgentRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let targets = team::resolve_targets(
        &state.db,
        state.node_ws_manager.as_ref(),
        &owner,
        &body.services,
    )
    .await?;
    let request = team::CreateRequest {
        name: body.name,
        description: body.description,
        display_name: body.display_name,
        persona: body.persona,
        targets,
        account_read: body.account_read,
        specialty: None,
        created_by: "user",
    };
    match team::create_specialist(&state.db, &state.encryption_keys, &owner, request).await? {
        Err(TeamRefusal::LimitReached { limit }) => Err(AppError::Conflict(format!(
            "You already have {limit} live agents; destroy one or raise the limit in settings"
        ))),
        Err(TeamRefusal::NameTaken) => Err(AppError::Conflict(
            "A live agent already uses that name".into(),
        )),
        Ok((agent, home)) => Ok((
            StatusCode::CREATED,
            Json(json!({"id": agent.id, "name": agent.name, "home_conversation_id": home.id})),
        )),
    }
}

#[derive(Serialize)]
pub struct MemoryNoteResponse {
    id: String,
    text: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// One agent with its memory, threads and pending requests.
pub async fn get_agent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let agent = team::agent(&state.db, &owner, &id).await?;
    let response = agent_responses(&state, &owner, true)
        .await?
        .into_iter()
        .find(|row| row.summary.id == agent.id)
        .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;
    let now = Utc::now();
    let threads: Vec<Value> = team::threads(&state.db, &agent, 100)
        .await?
        .into_iter()
        .map(|row| {
            let running = live_turn(&row, now).is_some();
            json!({"id": row.id, "title": row.title, "last_message_at": row.updated_at,
                "channel": row.channel.map(|channel| json!({"platform": channel.platform})),
                "running": running})
        })
        .collect();
    let memory: Vec<MemoryNoteResponse> = agent
        .memory
        .into_iter()
        .map(|note| MemoryNoteResponse {
            id: note.id,
            text: note.text,
            created_at: note.created_at,
            updated_at: note.updated_at,
        })
        .collect();
    Ok(Json(
        json!({"agent": response, "memory": memory, "threads": threads}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgentRequest {
    name: Option<String>,
    description: Option<String>,
    /// Empty clears it.
    #[serde(default)]
    display_name: Option<String>,
    /// Empty clears it.
    #[serde(default)]
    persona: Option<String>,
}

pub async fn update_agent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateAgentRequest>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let agent = team::update_agent(
        &state.db,
        &owner,
        &id,
        body.name.as_deref(),
        body.description.as_deref(),
        team::AgentStyle {
            display_name: body.display_name.as_deref(),
            persona: body.persona.as_deref(),
        },
    )
    .await?;
    Ok(Json(json!({"id": agent.id, "name": agent.name,
        "description": agent.description, "display_name": agent.display_name,
        "persona": agent.persona})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantsRequest {
    services: Vec<String>,
    account_read: bool,
}

/// The owner sets a specialist's grants directly (replacing them).
pub async fn set_agent_grants(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<GrantsRequest>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let targets = team::resolve_targets(
        &state.db,
        state.node_ws_manager.as_ref(),
        &owner,
        &body.services,
    )
    .await?;
    let agent = team::set_grants(
        &state.db,
        &owner,
        &id,
        team::GrantChange::Replace(AgentGrants {
            service_ids: targets.service_ids,
            platform_service_ids: targets.platform_service_ids,
            account_read: body.account_read,
        }),
    )
    .await?;
    Ok(Json(json!({"id": agent.id, "services": targets.slugs,
        "account_read": agent.grants.account_read})))
}

pub async fn destroy_agent_route(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let agent = team::agent(&state.db, &owner, &id).await?;
    if agent.is_nyxbot() {
        return Err(AppError::ValidationError(
            "NyxBot cannot be destroyed; delete individual threads instead".into(),
        ));
    }
    let agent = destroy_agent(&state, &owner, &agent.id).await?;
    Ok(Json(
        json!({"id": agent.id, "destroyed_at": agent.destroyed_at}),
    ))
}

/// Permanently delete a destroyed specialist and its threads.
pub async fn delete_agent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    team::purge(&state.db, &owner, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_memory(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, note_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    team::agent(&state.db, &owner, &id).await?;
    team::forget(&state.db, &owner, &id, &note_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SettingsResponse {
    skip_destructive_confirmation: bool,
    max_live_subagents: i32,
    max_concurrent_subagent_turns: i32,
    max_live_subagents_limit: i32,
    max_concurrent_subagent_turns_limit: i32,
    max_group_handoffs: i32,
    max_group_handoffs_per_hour: i32,
    max_group_handoffs_limit: i32,
    max_group_handoffs_per_hour_limit: i32,
}
impl From<crate::models::assistant_settings::AssistantSettings> for SettingsResponse {
    fn from(row: crate::models::assistant_settings::AssistantSettings) -> Self {
        Self {
            skip_destructive_confirmation: row.skip_destructive_confirmation,
            max_live_subagents: row.max_live_subagents,
            max_concurrent_subagent_turns: row.max_concurrent_subagent_turns,
            max_live_subagents_limit: crate::models::assistant_settings::MAX_LIVE_SUBAGENTS_LIMIT,
            max_concurrent_subagent_turns_limit:
                crate::models::assistant_settings::MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT,
            max_group_handoffs: row.max_group_handoffs,
            max_group_handoffs_per_hour: row.max_group_handoffs_per_hour,
            max_group_handoffs_limit: crate::models::assistant_settings::MAX_GROUP_HANDOFFS_LIMIT,
            max_group_handoffs_per_hour_limit:
                crate::models::assistant_settings::MAX_GROUP_HANDOFFS_PER_HOUR_LIMIT,
        }
    }
}

pub async fn get_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<SettingsResponse>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(settings::get(&state.db, &owner).await?.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsRequest {
    skip_destructive_confirmation: Option<bool>,
    max_live_subagents: Option<i32>,
    max_concurrent_subagent_turns: Option<i32>,
    #[serde(default)]
    max_group_handoffs: Option<i32>,
    #[serde(default)]
    max_group_handoffs_per_hour: Option<i32>,
}

pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SettingsRequest>,
) -> AppResult<Json<SettingsResponse>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let before = settings::get(&state.db, &owner).await?;
    let after = settings::update(
        &state.db,
        &owner,
        settings::Update {
            skip_destructive_confirmation: body.skip_destructive_confirmation,
            max_live_subagents: body.max_live_subagents,
            max_concurrent_subagent_turns: body.max_concurrent_subagent_turns,
            max_group_handoffs: body.max_group_handoffs,
            max_group_handoffs_per_hour: body.max_group_handoffs_per_hour,
        },
    )
    .await?;
    settings::audit(&state.db, &owner, &before, &after).await;
    Ok(Json(after.into()))
}

// ---------------------------------------------------------------------------
// Admin: role-to-profile routing (stored now, inactive)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ProfileRoutesResponse {
    /// False until routing is enabled in code; routes are validated and kept.
    active: bool,
    orchestrator: Option<String>,
    subagent: Option<String>,
    channel: Option<String>,
    subagent_roles: std::collections::BTreeMap<String, String>,
    updated_at: Option<DateTime<Utc>>,
}

fn routes_response(
    row: Option<crate::models::assistant_profile_route::AssistantProfileRoutes>,
) -> ProfileRoutesResponse {
    let active = crate::services::assistant_profile_routing::ROUTING_ACTIVE;
    match row {
        Some(row) => ProfileRoutesResponse {
            active,
            orchestrator: row.orchestrator,
            subagent: row.subagent,
            channel: row.channel,
            subagent_roles: row.subagent_roles,
            updated_at: Some(row.updated_at),
        },
        None => ProfileRoutesResponse {
            active,
            orchestrator: None,
            subagent: None,
            channel: None,
            subagent_roles: Default::default(),
            updated_at: None,
        },
    }
}

pub async fn get_profile_routes(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<ProfileRoutesResponse>> {
    crate::handlers::admin_helpers::require_admin(&state, &auth).await?;
    Ok(Json(routes_response(
        crate::services::assistant_profile_routing::get(&state.db).await?,
    )))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRoutesRequest {
    orchestrator: Option<String>,
    subagent: Option<String>,
    channel: Option<String>,
    #[serde(default)]
    subagent_roles: std::collections::BTreeMap<String, String>,
}

pub async fn put_profile_routes(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProfileRoutesRequest>,
) -> AppResult<Json<ProfileRoutesResponse>> {
    crate::handlers::admin_helpers::require_admin(&state, &auth).await?;
    let admin = auth.user_id.to_string();
    let row = crate::services::assistant_profile_routing::set(
        &state.db,
        &admin,
        crate::services::assistant_profile_routing::RoutesUpdate {
            orchestrator: body.orchestrator,
            subagent: body.subagent,
            channel: body.channel,
            subagent_roles: body.subagent_roles,
        },
    )
    .await?;
    let _ = crate::services::audit_service::log_actor_event(
        state.db.clone(),
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        "assistant_profile_routes_updated",
        Some(json!({"roles": row.subagent_roles.len(), "active":
            crate::services::assistant_profile_routing::ROUTING_ACTIVE})),
    )
    .await;
    Ok(Json(routes_response(Some(row))))
}

/// How often NyxID resolves watches and retries deferred wake-ups.
const SWEEP_SECS: u64 = 15;

/// Background task: resolve watches (bots created from setup links, finished
/// connect links) and retry wake-ups whose pool was full (or whose replica
/// restarted) when their events arrived. Agents are persistent, so nothing
/// is destroyed automatically.
pub fn spawn_sweeps(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(SWEEP_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            // Things the owner finished outside the chat queue events first.
            if let Err(error) = super::nyxbot::process_watches(&state).await {
                tracing::debug!(%error, "NyxBot watch sweep deferred");
            }
            // Linked chat apps whose messages stopped reaching their agent.
            if let Err(error) = super::nyxbot::check_deliveries(&state).await {
                tracing::debug!(%error, "NyxBot delivery sweep deferred");
            }
            // Personal bots on a platform the gateway now relays move there.
            if let Err(error) = super::nyxbot::switch_to_gateway(&state).await {
                tracing::debug!(%error, "NyxBot gateway switch-over deferred");
            }
            if let Ok(rows) = team::queued(&state.db, None).await {
                for row in rows {
                    wake(&state, &row.user_id, &row.id).await;
                }
            }
            // Group members addressed while busy (or while the pool was full).
            if let Ok(rows) =
                crate::services::assistant_group_service::with_pending(&state.db).await
            {
                for group in rows {
                    super::assistant_group::advance(&state, &group.user_id, &group.id).await;
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "assistant_team_tests.rs"]
mod tests;
