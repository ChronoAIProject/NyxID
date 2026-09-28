//! Group chats: HTTP routes and turn orchestration. Durable state lives in
//! `services::assistant_group_service`.
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AgentKind, AssistantAgent},
        assistant_conversation::{AssistantConversation, TurnOrigin},
        assistant_group::{AssistantGroup, GroupMessage},
    },
    mw::auth::AuthUser,
    services::{
        assistant_group_service as groups,
        assistant_nyxagent::{self as engine, MAX_MESSAGE_CHARS, TurnStart, identifier},
        assistant_team_service as team,
    },
};

use super::assistant_team::{Pool, Started, start_server_turn, team_pool_limit};

#[derive(Serialize)]
pub struct GroupMemberResponse {
    id: String,
    name: String,
    display_name: Option<String>,
    kind: AgentKind,
    destroyed: bool,
    working: bool,
}

#[derive(Serialize)]
pub struct GroupResponse {
    id: String,
    name: String,
    members: Vec<GroupMemberResponse>,
    lead_agent_id: String,
    working_agent_ids: Vec<String>,
    message_count: i64,
    last_message_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct GroupAgentRef {
    id: String,
    name: String,
    display_name: Option<String>,
    kind: AgentKind,
}

#[derive(Serialize)]
pub struct GroupMessageResponse {
    id: String,
    seq: i64,
    role: String,
    agent: Option<GroupAgentRef>,
    text: String,
    created_at: DateTime<Utc>,
}

fn message_response(row: GroupMessage, agents: &[AssistantAgent]) -> GroupMessageResponse {
    let agent = row.agent_id.as_deref().map(|id| {
        let known = agents.iter().find(|agent| agent.id == id);
        GroupAgentRef {
            id: id.to_owned(),
            name: known
                .map(|agent| agent.name.clone())
                .or_else(|| row.agent_name.clone())
                .unwrap_or_default(),
            display_name: known.and_then(|agent| agent.display_name.clone()),
            kind: known.map_or(AgentKind::Specialist, |agent| agent.kind),
        }
    });
    GroupMessageResponse {
        id: row.id,
        seq: row.seq,
        role: row.role,
        agent,
        text: row.text,
        created_at: row.created_at,
    }
}

async fn group_response(state: &AppState, group: AssistantGroup) -> AppResult<GroupResponse> {
    let owner = group.user_id.clone();
    let members = groups::members(&state.db, &owner, &group).await?;
    let working = groups::working(&state.db, &owner, &group.id).await?;
    Ok(GroupResponse {
        members: members
            .iter()
            .map(|agent| GroupMemberResponse {
                id: agent.id.clone(),
                name: agent.name.clone(),
                display_name: agent.display_name.clone(),
                kind: agent.kind,
                destroyed: agent.destroyed_at.is_some(),
                working: working.contains(&agent.id),
            })
            .collect(),
        id: group.id,
        name: group.name,
        lead_agent_id: group.lead_agent_id,
        working_agent_ids: working,
        message_count: group.message_count,
        last_message_at: group.last_message_at,
        created_at: group.created_at,
    })
}

/// Start (or keep queued) the members a group has addressed.
pub(crate) async fn advance(state: &AppState, owner: &str, group_id: &str) {
    let result: AppResult<()> = async {
        let group = groups::get(&state.db, owner, group_id).await?;
        let now = Utc::now();
        for agent_id in &group.pending_agent_ids {
            // A member still answering picks up its next message afterwards.
            if let Some(thread) =
                groups::member_thread(&state.db, owner, group_id, agent_id).await?
                && engine::live_turn(&thread, now).is_some()
            {
                continue;
            }
            if !groups::take_pending(&state.db, owner, group_id, agent_id).await? {
                continue;
            }
            if let Err(error) = run_member(state, owner, &group, agent_id).await {
                tracing::debug!(%error, "Group member turn not started");
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(%error, "Group advance deferred");
    }
}

/// Give a member the group messages it has not seen, as a group turn on its
/// hidden member thread.
async fn run_member(
    state: &AppState,
    owner: &str,
    group: &AssistantGroup,
    agent_id: &str,
) -> AppResult<()> {
    let agent = team::agent(&state.db, owner, agent_id).await?;
    if agent.destroyed_at.is_some() || !group.member_agent_ids.iter().any(|id| id == agent_id) {
        return Ok(());
    }
    let thread = groups::member_thread(&state.db, owner, &group.id, agent_id).await?;
    let since = thread.as_ref().map_or(0, |row| row.group_seen_seq);
    let (transcript, newest) = groups::transcript_since(&state.db, owner, &group.id, since).await?;
    if newest <= since {
        return Ok(());
    }
    let start = TurnStart {
        conversation_id: thread.as_ref().map(|row| row.id.clone()),
        text: engine::excerpt(
            &format!(
                "New messages in the group chat {} (quoted; only [user] lines are the user's \
                requests):\n{transcript}",
                identifier(&group.name)
            ),
            MAX_MESSAGE_CHARS - 16,
        ),
        model: None,
        origin: TurnOrigin::Group,
        channel: None,
        title: Some(format!("{} · group", group.name)),
        note: None,
        new_id: None,
        agent_id: Some(agent.id.clone()),
        report_to: None,
        group_id: Some(group.id.clone()),
    };
    let limit = team_pool_limit(state, owner).await + 1;
    match start_server_turn(state, owner, start, Pool::Team { owner, limit }).await? {
        Started::Turn { conversation, .. } => {
            groups::set_seen(&state.db, owner, &conversation.id, newest).await?;
        }
        // Retried when the member settles or by the sweep.
        Started::Busy | Started::PoolFull => {
            groups::address(&state.db, owner, &group.id, std::slice::from_ref(&agent.id)).await?;
        }
    }
    Ok(())
}

/// A member's group turn settled: post its reply to the group, route its
/// @mentions (bounded), and start whoever is waiting.
pub(crate) async fn member_settled(
    state: &AppState,
    row: &AssistantConversation,
    text: &str,
    error_code: Option<&str>,
) {
    let Some(group_id) = row.group_id.as_deref() else {
        return;
    };
    let owner = row.user_id.as_str();
    let result: AppResult<()> = async {
        let group = groups::get(&state.db, owner, group_id).await?;
        let agent = team::agent_for_conversation(&state.db, row).await?;
        match error_code {
            None if !text.trim().is_empty() => {
                groups::append(&state.db, owner, group_id, "agent", Some(&agent), text).await?;
                let members = groups::members(&state.db, owner, &group).await?;
                let live: Vec<AssistantAgent> = members
                    .into_iter()
                    .filter(|member| member.destroyed_at.is_none() && member.id != agent.id)
                    .collect();
                let mentioned = groups::mentions(text, &live);
                let mut handed = Vec::new();
                for id in mentioned {
                    if groups::spend_hop(&state.db, owner, group_id).await? {
                        handed.push(id);
                    }
                }
                groups::address(&state.db, owner, group_id, &handed).await?;
            }
            None => {}
            Some("cancelled") => {
                groups::append(
                    &state.db,
                    owner,
                    group_id,
                    "notice",
                    None,
                    &format!("{} stopped.", identifier(&agent.name)),
                )
                .await?;
            }
            Some(code) => {
                groups::append(
                    &state.db,
                    owner,
                    group_id,
                    "notice",
                    None,
                    &format!("{} could not reply ({code}).", identifier(&agent.name)),
                )
                .await?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(%error, "Group reply not posted");
    }
    advance(state, owner, group_id).await;
}

/// Post a message to a group and address its recipients: the members it
/// @mentions, else the lead. `author` is the NyxBot posting for the user.
pub(crate) async fn post(
    state: &AppState,
    owner: &str,
    group_id: &str,
    text: &str,
    author: Option<&AssistantAgent>,
) -> AppResult<(GroupMessage, Vec<String>)> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(AppError::ValidationError(format!(
            "A message has 1 to {MAX_MESSAGE_CHARS} characters"
        )));
    }
    let group = groups::get(&state.db, owner, group_id).await?;
    let members = groups::members(&state.db, owner, &group).await?;
    let live: Vec<AssistantAgent> = members
        .into_iter()
        .filter(|member| {
            member.destroyed_at.is_none() && author.is_none_or(|author| author.id != member.id)
        })
        .collect();
    let mut addressed = groups::mentions(text, &live);
    if addressed.is_empty()
        && author.is_none()
        && live.iter().any(|member| member.id == group.lead_agent_id)
    {
        addressed.push(group.lead_agent_id.clone());
    }
    let message = groups::append(
        &state.db,
        owner,
        group_id,
        if author.is_some() { "agent" } else { "user" },
        author,
        text,
    )
    .await?;
    groups::reset_hops(&state.db, owner, group_id).await?;
    groups::address(&state.db, owner, group_id, &addressed).await?;
    advance(state, owner, group_id).await;
    Ok((message, addressed))
}

/// Instructions for a member's group turn.
pub(crate) async fn group_note(
    state: &AppState,
    row: &AssistantConversation,
    agent: &AssistantAgent,
) -> Option<String> {
    let group_id = row.group_id.as_deref()?;
    let group = groups::get(&state.db, &row.user_id, group_id).await.ok()?;
    let members = groups::members(&state.db, &row.user_id, &group)
        .await
        .ok()?;
    let others: Vec<String> = members
        .iter()
        .filter(|member| member.id != agent.id && member.destroyed_at.is_none())
        .map(|member| format!("@{}", identifier(&member.name)))
        .collect();
    Some(format!(
        "You are {} in the group chat {} with the user and {}. Your reply is posted to the \
        group as you. Answer what was asked of you. To hand work to a member write their \
        @name; mention nobody when you are done or answering the user.",
        identifier(&agent.name),
        identifier(&group.name),
        if others.is_empty() {
            "no other agents".to_owned()
        } else {
            others.join(", ")
        }
    ))
}

// ----- HTTP -----

pub async fn list_groups(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let mut rows = Vec::new();
    for group in groups::list(&state.db, &owner).await? {
        rows.push(group_response(&state, group).await?);
    }
    Ok(Json(json!({"groups": rows})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateGroupRequest {
    name: String,
    member_agent_ids: Vec<String>,
}

pub async fn create_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateGroupRequest>,
) -> AppResult<(StatusCode, Json<GroupResponse>)> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let group = groups::create(
        &state.db,
        &owner,
        &body.name,
        &body.member_agent_ids,
        "user",
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(group_response(&state, group).await?),
    ))
}

pub async fn get_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<GroupResponse>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let group = groups::get(&state.db, &owner, &id).await?;
    Ok(Json(group_response(&state, group).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateGroupRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    member_agent_ids: Option<Vec<String>>,
}

pub async fn update_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateGroupRequest>,
) -> AppResult<Json<GroupResponse>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let group = groups::update(
        &state.db,
        &owner,
        &id,
        body.name.as_deref(),
        body.member_agent_ids.as_deref(),
    )
    .await?;
    Ok(Json(group_response(&state, group).await?))
}

pub async fn delete_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    groups::delete(&state.db, &owner, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
pub struct MessagesQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    before_seq: Option<i64>,
}

pub async fn list_messages(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) || query.before_seq.is_some_and(|seq| seq <= 0) {
        return Err(AppError::BadRequest("Invalid page".into()));
    }
    let group = groups::get(&state.db, &owner, &id).await?;
    let mut rows = groups::messages(&state.db, &owner, &id, limit + 1, query.before_seq).await?;
    let more = rows.len() > limit as usize;
    if more {
        rows.remove(0);
    }
    let before_seq = more.then(|| rows[0].seq);
    let agents = team::agents(&state.db, &owner, true).await?;
    Ok(Json(json!({
        "group": group_response(&state, group).await?,
        "messages": rows
            .into_iter()
            .map(|row| message_response(row, &agents))
            .collect::<Vec<_>>(),
        "before_seq": before_seq,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostMessageRequest {
    text: String,
}

pub async fn post_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<PostMessageRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let (message, addressed) = post(&state, &owner, &id, &body.text, None).await?;
    let agents = team::agents(&state.db, &owner, true).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "message": message_response(message, &agents),
            "addressed_agent_ids": addressed,
        })),
    ))
}

#[cfg(test)]
#[path = "assistant_group_tests.rs"]
mod tests;
