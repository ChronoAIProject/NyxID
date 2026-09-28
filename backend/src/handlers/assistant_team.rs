//! NyxBot team orchestration at the HTTP layer: server-started turns, event
//! wake-ups with loop guards, permission routing between a subagent and its
//! orchestrator, the orchestrator's native tools, and the team UI endpoints.
//!
//! Every agent is an ordinary conversation, so a server-started turn is the
//! same detached turn a browser starts, run with the owner's identity and
//! billed to the owner.
use axum::{
    Json,
    extract::{Path, State},
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
        assistant_conversation::{AgentRole, AssistantConversation, SubagentGrants, TurnOrigin},
    },
    mw::{
        auth::{AuthMethod, AuthUser},
        rate_limit::DirectChatPermit,
    },
    services::{
        assistant_acknowledgement_service::{self as acks, ChatAuthority, Decider},
        assistant_nyxagent::{self as engine, TurnError, TurnStart, excerpt, identifier, live_turn},
        assistant_settings_service as settings,
        assistant_team_service::{self as team, TeamRefusal},
        assistant_team_tools,
        coordination_service::RateWindowStore,
    },
};

/// Channel turns are bounded per owner, separately from browser turns.
pub const CHANNEL_TURNS_PER_OWNER: u32 = 2;

/// The owner identity NyxID uses for turns it starts itself (subagent work,
/// wake-ups, channel messages): the same person a browser turn acts for.
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
    /// A team's subagent and event turns, sized by the owner's setting.
    Team { team_id: &'a str, limit: u32 },
    /// Channel turns for one owner.
    Channel { owner: &'a str },
}

pub(crate) enum Started {
    Turn {
        conversation: Box<AssistantConversation>,
        receiver: broadcast::Receiver<Value>,
    },
    /// The target already has a live turn.
    Busy,
    /// The pool has no free slot.
    PoolFull,
}

async fn acquire(state: &AppState, pool: Pool<'_>) -> AppResult<Option<DirectChatPermit>> {
    match pool {
        Pool::Team { team_id, limit } => {
            state
                .direct_chat_limiter
                .try_acquire_pool("assistant_team", team_id, limit)
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
/// reported so the caller (usually the orchestrator) decides what to wait for.
pub(crate) async fn start_server_turn(
    state: &AppState,
    owner: &str,
    start: TurnStart,
    pool: Pool<'_>,
) -> AppResult<Started> {
    let Some(permit) = acquire(state, pool).await? else {
        return Ok(Started::PoolFull);
    };
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

async fn team_pool_limit(state: &AppState, owner: &str) -> u32 {
    settings::get(&state.db, owner)
        .await
        .map(|row| row.max_concurrent_subagent_turns.max(1) as u32)
        .unwrap_or(crate::models::assistant_settings::DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS as u32)
}

/// Start an event turn draining the agent's queue when it is idle. Loop
/// guards: an orchestrator stops after `MAX_EVENT_STREAK` consecutive event
/// turns without a user message, and a team runs at most
/// `EVENT_TURNS_PER_HOUR` event turns per hour. Events stay queued otherwise
/// and reach the agent with its next turn.
pub(crate) async fn wake(state: &AppState, owner: &str, id: &str) {
    let result: AppResult<()> = async {
        let row = engine::get(&state.db, owner, id).await?;
        if row.destroyed_at.is_some()
            || row.pending_events.is_empty()
            || live_turn(&row, Utc::now()).is_some()
        {
            return Ok(());
        }
        let team_id = row.team_root().to_owned();
        if row.role == AgentRole::Orchestrator && row.event_streak >= team::MAX_EVENT_STREAK {
            return Ok(());
        }
        if !RateWindowStore::admit(
            &state.db,
            "assistant_event_turns",
            &team_id,
            team::EVENT_TURNS_PER_HOUR,
            Duration::from_secs(3600),
        )
        .await?
        .allowed
        {
            return Ok(());
        }
        // The orchestrator itself shares the pool with its working subagents.
        let limit = team_pool_limit(state, owner).await + 1;
        let start = TurnStart {
            conversation_id: Some(id.to_owned()),
            text: String::new(),
            model: None,
            origin: TurnOrigin::Event,
            channel: None,
            title: None,
            note: None,
            new_id: None,
        };
        match start_server_turn(
            state,
            owner,
            start,
            Pool::Team {
                team_id: &team_id,
                limit,
            },
        )
        .await
        {
            // A drained-then-raced queue is harmless.
            Ok(_) | Err(AppError::Conflict(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(conversation_id = %id, %error, "NyxBot wake-up deferred");
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

/// Runs after a turn's reply is durable: report orchestrator-assigned
/// subagent work to the orchestrator, drain queues, and deliver asynchronous
/// replies of channel conversations to their chat.
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
    // A direct user chat with a subagent does not wake the orchestrator; it
    // reads those on its next turn.
    if row.is_subagent()
        && matches!(turn.origin, TurnOrigin::Orchestrator | TurnOrigin::Event)
        && let Some(team_id) = row.team_id.as_deref()
    {
        let name = identifier(row.agent_name.as_deref().unwrap_or_default());
        let status = match error.map(|error| error.code) {
            None => "replied".to_owned(),
            Some("cancelled") => "was stopped".to_owned(),
            Some(code) => format!("failed ({code})"),
        };
        let note = format!(
            "Subagent {name} {status}. Reply excerpt: \"{}\" Read more with nyxid__read_subagent.",
            excerpt(text, 1200).replace('"', "'")
        );
        let _ = engine::push_events(
            &state.db,
            owner,
            team_id,
            vec![team::event("subagent_settled", note, Some(&row.id))],
        )
        .await;
    }
    // Wake whoever has queued work now that a slot is free: this agent,
    // its orchestrator, and teammates.
    wake(state, owner, &row.id).await;
    let team_id = row.team_root().to_owned();
    if team_id != row.id {
        wake(state, owner, &team_id).await;
    }
    if let Ok(members) = team::members(&state.db, owner, std::slice::from_ref(&team_id)).await {
        for member in members.iter().filter(|member| {
            member.id != row.id && member.destroyed_at.is_none() && !member.pending_events.is_empty()
        }) {
            wake(state, owner, &member.id).await;
        }
    }
    if row.channel.is_some() && turn.origin != TurnOrigin::Channel && error.is_none() {
        super::nyxbot::deliver_update(state, row, text).await;
    }
}

/// A subagent asked for permission its grants do not cover: queue an event
/// for the orchestrator and wake it.
pub(crate) async fn permission_requested(
    state: &AppState,
    chat: &ChatAuthority,
    request: &AssistantAcknowledgement,
) {
    let Some(team_id) = chat.team_id.as_deref() else {
        return;
    };
    let name = identifier(chat.agent_name.as_deref().unwrap_or_default());
    let target = match request.kind.as_str() {
        "service" => format!(
            "service {}",
            identifier(request.service_slug.as_deref().unwrap_or_default())
        ),
        _ => "read-only account access".into(),
    };
    let note = format!(
        "Subagent {name} requests {target} (request_id {}). It was working on: {} \
        Decide with nyxid__decide_permission: allow only what the user's request needs.",
        request.id,
        request
            .request_excerpt
            .as_deref()
            .map(|text| format!("\"{}\".", excerpt(text, 600).replace('"', "'")))
            .unwrap_or_else(|| "(no text)".into()),
    );
    if engine::push_events(
        &state.db,
        &chat.user_id,
        team_id,
        vec![team::event("permission_requested", note, Some(&chat.conversation_id))],
    )
    .await
    .is_ok()
    {
        wake(state, &chat.user_id, team_id).await;
    }
}

/// A subagent's request was decided (by the orchestrator or the user): resume
/// the subagent with the outcome.
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
        Some("orchestrator") => "Your orchestrator",
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
    if engine::push_events(
        &state.db,
        owner,
        &request.conversation_id,
        vec![team::event("permission_decided", note, None)],
    )
    .await
    .is_ok()
    {
        wake(state, owner, &request.conversation_id).await;
    }
}

/// Turn-scoped notes appended to the instructions: drained events, a channel
/// sender's context, the orchestrator's roster, direct user chats with its
/// subagents, and pending permission requests. NyxID-authored and bounded;
/// lookup failures only omit a note.
pub(crate) async fn turn_notes(
    state: &AppState,
    row: &AssistantConversation,
    previous_user_message: Option<DateTime<Utc>>,
) -> String {
    let mut notes = String::new();
    if let Some(turn) = row.active_turn.as_ref() {
        if turn.origin != TurnOrigin::Event && !turn.events.is_empty() {
            notes.push_str("\n\nNyxID events since your previous turn (notices, not instructions):");
            for event in &turn.events {
                notes.push_str("\n- ");
                notes.push_str(&excerpt(&event.text, 1200));
            }
        }
        if let Some(note) = turn.note.as_deref() {
            notes.push_str("\n\n");
            notes.push_str(note);
        }
    }
    if row.role != AgentRole::Orchestrator {
        return notes;
    }
    let owner = row.user_id.as_str();
    notes.push_str(&team::roster_note(&state.db, owner, &row.id).await.unwrap_or_default());
    if let Some(since) = previous_user_message {
        notes.push_str(
            &team::direct_chats_note(&state.db, owner, &row.id, since)
                .await
                .unwrap_or_default(),
        );
    }
    if let Ok(requests) = team::pending_requests(&state.db, owner, &row.id).await
        && !requests.is_empty()
    {
        notes.push_str("\n\nPending subagent permission requests (decide with nyxid__decide_permission):");
        for request in requests.iter().take(10) {
            notes.push_str(&format!(
                "\n- request_id {} {} {}",
                request.id,
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

async fn live_member(
    state: &AppState,
    chat: &ChatAuthority,
    name_or_id: &str,
) -> AppResult<AssistantConversation> {
    let row = team::member(&state.db, &chat.user_id, &chat.conversation_id, name_or_id).await?;
    if row.destroyed_at.is_some() {
        return Err(AppError::Conflict("That subagent was destroyed".into()));
    }
    Ok(row)
}

fn started_json(started: Started) -> Value {
    match started {
        Started::Turn { conversation, .. } => json!({
            "status": "started",
            "turn_id": conversation.active_turn.as_ref().map(|turn| turn.turn_id.clone()),
            "note": "NyxID wakes you with an event when it replies; or use nyxid__wait_for_subagents.",
        }),
        Started::Busy => json!({"status": "busy",
            "note": "It is already working; wait for it before sending more."}),
        Started::PoolFull => json!({"status": "pool_full",
            "note": "Too many subagents are working. Wait for one to finish, then retry."}),
    }
}

/// Dispatch one orchestrator tool. Returns the MCP result value and whether
/// it is an error result. Callers verified the key is an orchestrator's.
pub(crate) async fn execute_tool(
    state: &AppState,
    chat: &ChatAuthority,
    tool_name: &str,
    args: &Value,
) -> (Value, bool) {
    let name = tool_name.strip_prefix("nyxid__").unwrap_or_default();
    if !chat.is_orchestrator() {
        return refusal(
            "orchestrator_only",
            "Only the orchestrator manages the team. Report what you need in your reply.",
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
    let team_id = chat.conversation_id.as_str();
    Ok(match name {
        "spawn_subagent" => {
            let orchestrator = team::orchestrator(db, owner, team_id).await?;
            let targets = team::resolve_targets(
                db,
                state.node_ws_manager.as_ref(),
                owner,
                &string_list(args, "services"),
            )
            .await?;
            let request = team::SpawnRequest {
                name: text_arg(args, "name").to_owned(),
                charter: text_arg(args, "charter").to_owned(),
                targets: targets.clone(),
                account_read: args["account_read"].as_bool().unwrap_or(false),
                specialty: args["specialty"].as_str().map(str::to_owned),
            };
            match team::spawn(db, &state.encryption_keys, owner, &orchestrator, request).await? {
                Err(TeamRefusal::LimitReached { limit }) => (
                    json!({"error": "limit_reached", "limit": limit,
                        "instructions": "Destroy an idle subagent first, or tell the user they \
                            can raise the limit in NyxBot settings."}),
                    true,
                ),
                Err(TeamRefusal::NameTaken) => refusal(
                    "name_taken",
                    "A live subagent already uses this name; pick another or message it.",
                ),
                Ok((row, _)) => {
                    let task = match args["task"].as_str() {
                        Some(task) => {
                            let limit = team_pool_limit(state, owner).await;
                            let start = TurnStart {
                                conversation_id: Some(row.id.clone()),
                                text: task.to_owned(),
                                model: None,
                                origin: TurnOrigin::Orchestrator,
                                channel: None,
                                title: None,
                                note: None,
                                new_id: None,
                            };
                            started_json(
                                start_server_turn(
                                    state,
                                    owner,
                                    start,
                                    Pool::Team { team_id, limit },
                                )
                                .await?,
                            )
                        }
                        None => json!({"status": "idle",
                            "note": "Give it work with nyxid__message_subagent."}),
                    };
                    (
                        json!({"subagent": {"id": row.id, "name": row.agent_name,
                            "services": targets.slugs, "account_read": row.grants.account_read},
                            "task": task}),
                        false,
                    )
                }
            }
        }
        "message_subagent" => {
            let member = live_member(state, chat, text_arg(args, "subagent")).await?;
            let limit = team_pool_limit(state, owner).await;
            let start = TurnStart {
                conversation_id: Some(member.id.clone()),
                text: text_arg(args, "text").to_owned(),
                model: None,
                origin: TurnOrigin::Orchestrator,
                channel: None,
                title: None,
                note: None,
                new_id: None,
            };
            (
                started_json(
                    start_server_turn(state, owner, start, Pool::Team { team_id, limit }).await?,
                ),
                false,
            )
        }
        "wait_for_subagents" => (wait_for(state, chat, args).await?, false),
        "list_subagents" => {
            let rows = team::summaries(
                db,
                owner,
                team_id,
                args["include_destroyed"].as_bool().unwrap_or(false),
                300,
            )
            .await?;
            (json!({"subagents": rows}), false)
        }
        "read_subagent" => {
            let member = team::member(db, owner, team_id, text_arg(args, "subagent")).await?;
            let rows = team::read(
                db,
                owner,
                &member,
                args["limit"].as_i64().unwrap_or(team::READ_LIMIT),
            )
            .await?;
            (
                json!({"subagent": member.agent_name, "messages": rows}),
                false,
            )
        }
        "grant_subagent" | "revoke_subagent" => {
            let member = live_member(state, chat, text_arg(args, "subagent")).await?;
            let targets = team::resolve_targets(
                db,
                state.node_ws_manager.as_ref(),
                owner,
                &string_list(args, "services"),
            )
            .await?;
            let mut grants: SubagentGrants = member.grants.clone();
            let grant = name == "grant_subagent";
            for (list, ids) in [
                (&mut grants.service_ids, &targets.service_ids),
                (&mut grants.platform_service_ids, &targets.platform_service_ids),
            ] {
                if grant {
                    for id in ids {
                        if !list.contains(id) {
                            list.push(id.clone());
                        }
                    }
                } else {
                    list.retain(|id| !ids.contains(id));
                }
            }
            if let Some(account_read) = args["account_read"].as_bool() {
                grants.account_read = if grant {
                    grants.account_read || account_read
                } else {
                    grants.account_read && !account_read
                };
            }
            let row = team::set_grants(db, owner, team_id, &member.id, grants).await?;
            let summary = team::summaries(db, owner, team_id, false, 0)
                .await?
                .into_iter()
                .find(|summary| summary.id == row.id);
            (
                json!({"subagent": row.agent_name,
                    "services": summary.as_ref().map(|s| s.services.clone()),
                    "account_read": row.grants.account_read}),
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
                Decider::Orchestrator { team_id },
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
                    "note": "The subagent was resumed with your decision."}),
                false,
            )
        }
        "destroy_subagent" => {
            let member = live_member(state, chat, text_arg(args, "subagent")).await?;
            let row = team::destroy(db, owner, team_id, &member.id).await?;
            (json!({"destroyed": row.agent_name}), false)
        }
        "connect_channel_bot" => {
            super::nyxbot::connect_tool(state, owner, team_id, text_arg(args, "bot_id")).await?
        }
        "list_channel_agents" => (super::nyxbot::list_tool(state, owner).await?, false),
        "disconnect_channel_bot" => (
            super::nyxbot::disconnect(state, owner, text_arg(args, "channel_agent_id")).await?,
            false,
        ),
        _ => return Err(AppError::NotFound("NyxBot tool not found".into())),
    })
}

/// Bounded wait. Returns settled replies, who is still running, and pending
/// permission requests; delivered settlements are removed from the
/// orchestrator's queue so they do not also wake it later.
async fn wait_for(state: &AppState, chat: &ChatAuthority, args: &Value) -> AppResult<Value> {
    let db = &state.db;
    let owner = chat.user_id.as_str();
    let team_id = chat.conversation_id.as_str();
    let timeout = args["timeout_secs"]
        .as_u64()
        .unwrap_or(60)
        .clamp(1, team::MAX_WAIT_SECS);
    let names = string_list(args, "subagents");
    let mut targets = Vec::new();
    if names.is_empty() {
        targets = team::members(db, owner, &[team_id.to_owned()])
            .await?
            .into_iter()
            .filter(|row| row.destroyed_at.is_none())
            .map(|row| row.id)
            .collect();
    } else {
        for name in &names {
            targets.push(team::member(db, owner, team_id, name).await?.id);
        }
    }
    let own_turn = engine::get(db, owner, team_id)
        .await?
        .active_turn
        .map(|turn| turn.turn_id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let now = Utc::now();
        let rows = team::members(db, owner, &[team_id.to_owned()]).await?;
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|row| targets.contains(&row.id))
            .collect();
        let running: Vec<_> = rows
            .iter()
            .filter(|row| row.destroyed_at.is_none() && live_turn(row, now).is_some())
            .collect();
        let requests = team::pending_requests(db, owner, team_id).await?;
        let stopped = match own_turn.as_deref() {
            Some(turn_id) => engine::stop_requested(db, owner, team_id, turn_id).await?,
            None => false,
        };
        if running.is_empty()
            || !requests.is_empty()
            || stopped
            || tokio::time::Instant::now() >= deadline
        {
            let summaries = team::summaries(db, owner, team_id, true, team::REPLY_EXCERPT_CHARS)
                .await?
                .into_iter()
                .filter(|summary| targets.contains(&summary.id))
                .collect::<Vec<_>>();
            let settled: Vec<String> = summaries
                .iter()
                .filter(|summary| summary.status != "running")
                .map(|summary| summary.id.clone())
                .collect();
            team::consume_settled_events(db, owner, team_id, &settled).await?;
            let names: std::collections::HashMap<String, String> = rows
                .iter()
                .map(|row| (row.id.clone(), row.agent_name.clone().unwrap_or_default()))
                .collect();
            return Ok(json!({
                "settled": summaries.iter().filter(|s| s.status != "running").map(|s| json!({
                    "name": s.name, "status": s.status, "reply": s.last_reply,
                })).collect::<Vec<_>>(),
                "running": summaries.iter().filter(|s| s.status == "running")
                    .map(|s| s.name.clone()).collect::<Vec<_>>(),
                "pending_requests": requests.iter().map(|request| team::request_summary(
                    request,
                    names.get(&request.conversation_id).map(String::as_str).unwrap_or("subagent"),
                )).collect::<Vec<_>>(),
            }));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ---------------------------------------------------------------------------
// Team UI endpoints (human-only router)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct TeamResponse {
    orchestrator_id: String,
    members: Vec<team::MemberSummary>,
    pending_requests: Vec<team::RequestSummary>,
    limits: SettingsResponse,
}

pub async fn get_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<TeamResponse>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let row = engine::get(&state.db, &owner, &id).await?;
    let team_id = row.team_root().to_owned();
    let members = team::summaries(&state.db, &owner, &team_id, true, 600).await?;
    let names: std::collections::HashMap<&str, &str> = members
        .iter()
        .map(|member| (member.id.as_str(), member.name.as_str()))
        .collect();
    let requests = team::pending_requests(&state.db, &owner, &team_id).await?;
    let pending_requests = requests
        .iter()
        .map(|request| {
            team::request_summary(
                request,
                names
                    .get(request.conversation_id.as_str())
                    .copied()
                    .unwrap_or("subagent"),
            )
        })
        .collect();
    Ok(Json(TeamResponse {
        orchestrator_id: team_id,
        pending_requests,
        members,
        limits: settings::get(&state.db, &owner).await?.into(),
    }))
}

/// Destroy a subagent from the UI. The transcript stays read-only.
pub async fn destroy_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let row = engine::get(&state.db, &owner, &id).await?;
    let Some(team_id) = row.team_id.clone() else {
        return Err(AppError::ValidationError(
            "Only subagents can be destroyed; delete the chat instead".into(),
        ));
    };
    let row = team::destroy(&state.db, &owner, &team_id, &row.id).await?;
    Ok(Json(json!({"id": row.id, "destroyed_at": row.destroyed_at})))
}

#[derive(Serialize)]
pub struct SettingsResponse {
    skip_destructive_confirmation: bool,
    max_live_subagents: i32,
    max_concurrent_subagent_turns: i32,
    max_live_subagents_limit: i32,
    max_concurrent_subagent_turns_limit: i32,
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

/// Background sweep: destroy subagents idle for `IDLE_DESTROY_DAYS` and
/// retry wake-ups whose pool was full when their events arrived.
pub fn spawn_sweeps(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        let mut ticks: u64 = 0;
        loop {
            interval.tick().await;
            ticks += 1;
            if ticks % (team::IDLE_SWEEP_INTERVAL_SECS / 60) == 0
                && let Err(error) = team::sweep_idle(&state.db).await
            {
                tracing::warn!(%error, "NyxBot idle subagent sweep failed");
            }
            if let Ok(rows) = team::queued(&state.db).await {
                for row in rows {
                    wake(&state, &row.user_id, &row.id).await;
                }
            }
        }
    });
}
