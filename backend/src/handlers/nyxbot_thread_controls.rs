use super::*;
use crate::services::{
    channel_platform::ThreadCapabilities, channel_thread_follow_service as follow,
};

#[derive(serde::Serialize)]
pub(crate) struct ThreadResponse {
    busy_count: i64,
    dropped_message_count: i64,
    id: String,
    parent_chat_id: Option<String>,
    conversation_id: Option<String>,
    agent_id: Option<String>,
    label: String,
    state: String,
    kind: Option<crate::models::channel_thread::ThreadKind>,
    followed_at: Option<chrono::DateTime<Utc>>,
    last_admitted_at: Option<chrono::DateTime<Utc>>,
    expires_at: Option<chrono::DateTime<Utc>>,
    context_status: String,
    context_message_count: u32,
    follow_readiness: &'static str,
}
fn response(child: &NyxbotThread, ready: bool) -> ThreadResponse {
    let active = follow::follows(child);
    let state = match child.follow.follow_state.as_deref() {
        Some("active") if !active => "expired",
        Some(s @ ("active" | "opening" | "stopped" | "expired")) => s,
        _ => "unavailable",
    };
    ThreadResponse {
        busy_count: child.follow.follow_busy_count,
        dropped_message_count: child.follow.follow_drop_count,
        id: child.id.clone(),
        parent_chat_id: child.follow.parent_chat_id.clone(),
        conversation_id: child.conversation_id.clone(),
        agent_id: child.follow.bound_agent_id.clone(),
        label: format!(
            "{} · {}",
            if child.follow.thread_kind == Some(crate::models::channel_thread::ThreadKind::Topic) {
                "Topic"
            } else {
                "Thread"
            },
            child.created_at.format("%Y-%m-%d %H:%M UTC")
        ),
        state: state.into(),
        kind: child.follow.thread_kind,
        followed_at: child.follow.follow_started_at,
        last_admitted_at: child.follow.follow_last_admitted_at,
        expires_at: child.follow.follow_expires_at,
        context_status: child
            .follow
            .context_status
            .clone()
            .unwrap_or_else(|| "unavailable".into()),
        context_message_count: child.follow.context_message_count,
        follow_readiness: if ready { "ready" } else { "unavailable" },
    }
}

pub(crate) fn capabilities(
    state: &AppState,
    row: &NyxbotChannel,
    on: bool,
    group: bool,
) -> ThreadCapabilities {
    if !on || !group || row.transport != "direct" || row.status != "active" {
        return ThreadCapabilities::default();
    }
    crate::services::channel_adapters::resolve_adapter(&row.platform, &state.token_exchange_cache)
        .map(|a| a.thread_capabilities())
        .unwrap_or_default()
}

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub state: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

pub(crate) async fn list(
    state: &AppState,
    owner: &str,
    channel: &str,
    parent: Option<&str>,
    page: &Page,
) -> AppResult<Value> {
    let row = follow::access(&state.db, owner, channel).await?;
    let (mut rows, cursor) = follow::list_children(
        &state.db,
        owner,
        channel,
        parent,
        page.state.as_deref().unwrap_or("active"),
        page.cursor.as_deref(),
        page.limit.unwrap_or(25),
    )
    .await?;
    hide_reservations(state, owner, &mut rows).await?;
    let ready = capabilities(
        state,
        &row,
        thread_follow::enabled(state, owner).await?,
        true,
    )
    .thread_follow;
    Ok(
        json!({"threads":rows.iter().map(|r|response(r,ready)).collect::<Vec<_>>(),"next_cursor":cursor}),
    )
}

pub(crate) async fn stop(
    state: &AppState,
    owner: &str,
    channel: &str,
    parent: &str,
    id: &str,
) -> AppResult<Value> {
    let mut child = follow::stop(&state.db, owner, channel, parent, id).await?;
    hide_reservations(state, owner, std::slice::from_mut(&mut child)).await?;
    let row = follow::access(&state.db, owner, channel).await?;
    audit(
        state,
        owner,
        "nyxbot_thread_follow_stopped",
        json!({"channel_agent_id":channel,"chat_id":parent,"thread_id":id}),
    )
    .await;
    let ready = capabilities(
        state,
        &row,
        thread_follow::enabled(state, owner).await?,
        true,
    )
    .thread_follow;
    Ok(json!({"thread":response(&child,ready)}))
}

pub async fn list_threads(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path((channel, parent)): Path<(String, String)>,
    axum::extract::Query(page): axum::extract::Query<Page>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(
        list(&state, &owner, &channel, Some(&parent), &page).await?,
    ))
}
pub async fn stop_thread(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path((channel, parent, id)): Path<(String, String, String)>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(stop(&state, &owner, &channel, &parent, &id).await?))
}

pub(crate) async fn list_tool(state: &AppState, owner: &str, args: &Value) -> AppResult<Value> {
    let mut channels = Vec::new();
    for channel in super::list(state, owner).await? {
        if super::org_access_holds(state, &channel).await? {
            channels.push(channel);
        }
    }
    let ids: Vec<String> = channels
        .iter()
        .filter(|c| {
            c.status == "active"
                && args["channel_agent_id"]
                    .as_str()
                    .is_none_or(|id| id == c.id)
        })
        .map(|c| c.id.clone())
        .collect();
    if args["channel_agent_id"].as_str().is_some() && ids.is_empty() {
        return Err(follow::not_found());
    }
    let (mut rows, cursor) = follow::list_allowed(
        &state.db,
        owner,
        &ids,
        args["chat_id"].as_str(),
        args["state"].as_str().unwrap_or("active"),
        args["cursor"].as_str(),
        args["limit"].as_u64().unwrap_or(25).try_into().unwrap_or(0),
    )
    .await?;
    hide_reservations(state, owner, &mut rows).await?;
    let on = thread_follow::enabled(state, owner).await?;
    Ok(
        json!({"threads":rows.iter().map(|r|response(r,channels.iter().find(|c|c.id==r.channel_id)
        .is_some_and(|c|capabilities(state,c,on,true).thread_follow))).collect::<Vec<_>>(),"next_cursor":cursor}),
    )
}

async fn hide_reservations(
    state: &AppState,
    owner: &str,
    rows: &mut [NyxbotThread],
) -> AppResult<()> {
    let known = follow::materialized_conversations(&state.db, owner, rows).await?;
    for row in rows {
        if row
            .conversation_id
            .as_ref()
            .is_some_and(|id| !known.contains(id))
        {
            row.conversation_id = None;
        }
    }
    Ok(())
}
