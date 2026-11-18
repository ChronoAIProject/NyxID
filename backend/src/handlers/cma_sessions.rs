//! Minimal owner-only CMA attribution projection; never returns model/result rows.
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct ListQuery {
    after: Option<String>,
    conversation: Option<String>,
    pins: Option<String>,
    agents: Option<String>,
    #[serde(default)]
    discover: bool,
}
#[derive(Serialize)]
pub struct SessionOriginResponse {
    service_id: String,
    agent_id: String,
    conversation_id: String,
    turn_id: String,
    specialist_id: String,
    cma_url: Option<String>,
    connection_name: String,
    connection_slug: String,
}
#[derive(Serialize)]
pub struct ListResponse {
    sessions: Vec<SessionOriginResponse>,
    next_cursor: Option<String>,
    connections: Vec<ConnectionResponse>,
}
#[derive(Serialize)]
pub struct ConnectionResponse {
    id: String,
    name: String,
    slug: String,
    connected: bool,
    requires_connection: bool,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<ListResponse>> {
    let user = auth.user_id.to_string();
    crate::services::assistant_nyxagent::require_enabled(&state.db, &user).await?;
    if query.discover {
        let page = crate::services::cma_session_origin::connections_page(
            &state.db,
            &user,
            query.after.as_deref(),
            query.conversation.as_deref(),
            query.pins.as_deref(),
        )
        .await?;
        return Ok(Json(ListResponse {
            sessions: Vec::new(),
            next_cursor: page.next_cursor,
            connections: page
                .connections
                .into_iter()
                .map(|c| ConnectionResponse {
                    id: c.id,
                    name: c.name,
                    slug: c.slug,
                    connected: true,
                    requires_connection: true,
                })
                .collect(),
        }));
    }
    let page = crate::services::cma_session_origin::page_candidates(
        &state.db,
        &user,
        query.after.as_deref(),
        query.conversation.as_deref(),
        query.pins.as_deref(),
        query.agents.as_deref(),
    )
    .await?;
    let mut metadata = std::collections::HashMap::new();
    let mut sessions = Vec::new();
    for row in page.origins {
        if !metadata.contains_key(&row.service_id) {
            metadata.insert(
                row.service_id.clone(),
                crate::services::cma_session_origin::connection_metadata(
                    &state.db,
                    &user,
                    &row.service_id,
                )
                .await?,
            );
        }
        let Some(connection) = metadata[&row.service_id].as_ref() else {
            continue;
        };
        sessions.push(SessionOriginResponse {
            service_id: row.service_id,
            agent_id: row.remote_agent_id,
            conversation_id: row.conversation_id,
            turn_id: row.turn_id,
            specialist_id: row.agent_id,
            cma_url: crate::services::cma_operation::validated_callback_url(
                row.callback_url.as_deref(),
            ),
            connection_name: connection.name.clone(),
            connection_slug: connection.slug.clone(),
        });
    }
    Ok(Json(ListResponse {
        next_cursor: page.next_cursor,
        connections: Vec::new(),
        sessions,
    }))
}
