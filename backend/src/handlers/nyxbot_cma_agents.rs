//! Human execution views and bounded, exact-resource CMA SSE/RFB relays.
use super::nyxbot_cma_sandboxes::{PageQuery, owner};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::nyxbot_cma_agent::NyxbotCmaAgent,
    mw::auth::{AuthMethod, AuthUser},
    services::{
        nyxbot_cma_agent_service as agents,
        nyxbot_cma_gateway::{
            CatalogCmaGateway, CmaDesktopSocket, CmaGateway, MAX_CURSOR_BYTES,
            MAX_DESKTOP_FRAME_BYTES, MAX_STREAM_SECONDS,
        },
    },
};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use mongodb::bson::{self, doc};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use tokio::time::Instant;

const AUTHORITY_INTERVAL: Duration = Duration::from_secs(10);
const AUTHORITY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;

async fn reference(
    state: &AppState,
    auth: &AuthUser,
    id: &str,
) -> AppResult<(String, NyxbotCmaAgent, CatalogCmaGateway)> {
    let owner = owner(state, auth).await?;
    let row = agents::get_owned(&state.db, &owner, id).await?;
    let gateway = CatalogCmaGateway::for_service(state, &row.cma_service_id);
    Ok((owner, row, gateway))
}

pub async fn detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<agents::AgentDto>> {
    let owner = owner(&state, &auth).await?;
    let row = agents::get_owned(&state.db, &owner, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    Ok(Json(
        agents::detail(&state.db, &gateway, &owner, &id).await?,
    ))
}
pub async fn transcript(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<crate::services::nyxbot_cma_gateway::CmaTranscript>> {
    let (owner, row, gateway) = reference(&state, &auth, &id).await?;
    agents::revalidate(&state.db, &gateway, &owner, &row).await?;
    Ok(Json(
        gateway
            .get_transcript(&owner, &row.cma_agent_id, query.cursor.as_deref())
            .await?,
    ))
}
pub async fn snapshot(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<crate::services::nyxbot_cma_gateway::CmaEventSnapshot>> {
    let (owner, row, gateway) = reference(&state, &auth, &id).await?;
    agents::revalidate(&state.db, &gateway, &owner, &row).await?;
    Ok(Json(
        gateway
            .get_event_snapshot(&owner, &row.cma_agent_id, query.cursor.as_deref())
            .await?,
    ))
}
pub async fn response(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, response_id)): Path<(String, String)>,
) -> AppResult<Json<crate::services::nyxbot_cma_gateway::CmaResponse>> {
    let (owner, row, gateway) = reference(&state, &auth, &id).await?;
    agents::revalidate(&state.db, &gateway, &owner, &row).await?;
    Ok(Json(
        gateway
            .get_response(&owner, &row.cma_agent_id, &response_id)
            .await?,
    ))
}
pub async fn desktop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<crate::services::nyxbot_cma_gateway::CmaDesktopDescriptor>> {
    let owner = owner(&state, &auth).await?;
    let row = agents::get_owned(&state.db, &owner, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    Ok(Json(
        agents::desktop(&state.db, &gateway, &owner, &id).await?,
    ))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamQuery {
    pub after_cursor: Option<String>,
}

fn resume_cursor<'a>(
    query: &'a StreamQuery,
    headers: &'a HeaderMap,
    response: bool,
) -> AppResult<Option<&'a str>> {
    let invalid = || AppError::ValidationError("Invalid CMA event cursor".into());
    let mut values = headers.get_all("last-event-id").iter();
    let header = values
        .next()
        .map(|value| value.to_str().map_err(|_| invalid()))
        .transpose()?;
    if values.next().is_some()
        || query
            .after_cursor
            .as_deref()
            .zip(header)
            .is_some_and(|(a, b)| a != b)
    {
        return Err(invalid());
    }
    let cursor = query.after_cursor.as_deref().or(header);
    if cursor.is_some_and(|cursor| {
        cursor.is_empty()
            || cursor.len() > MAX_CURSOR_BYTES
            || cursor.chars().any(char::is_control)
            || (response && cursor.parse::<u64>().is_err())
    }) {
        return Err(invalid());
    }
    Ok(cursor)
}

async fn session_live(state: &AppState, owner: &str, session: Option<uuid::Uuid>) -> AppResult<()> {
    use crate::models::{
        session::{COLLECTION_NAME as SESSIONS, Session},
        user::{COLLECTION_NAME as USERS, User},
    };
    let session =
        session.ok_or_else(|| AppError::Forbidden("A current human session is required".into()))?;
    let (user, session) = tokio::try_join!(
        async {
            state
                .db
                .collection::<User>(USERS)
                .find_one(doc! {"_id":owner,"is_active":true})
                .await
        },
        async {
            state
                .db
                .collection::<Session>(SESSIONS)
                .find_one(crate::services::session_authority::authenticated(doc! {
                    "_id":session.to_string(),"user_id":owner,"revoked":false,
                    "expires_at":{"$gt":bson::DateTime::from_chrono(chrono::Utc::now())}
                }))
                .await
        },
    )?;
    let user = user
        .filter(|user| user.user_type.is_person())
        .ok_or_else(|| AppError::Forbidden("Human session ended".into()))?;
    let session = session.ok_or_else(|| AppError::Forbidden("Human session ended".into()))?;
    if user.mfa_enabled && !crate::services::session_authority::satisfies_mfa(&session) {
        return Err(AppError::Forbidden(
            "Human session verification changed".into(),
        ));
    }
    Ok(())
}

async fn stream_authority(
    state: &AppState,
    gateway: &dyn CmaGateway,
    owner: &str,
    session: Option<uuid::Uuid>,
    row: &NyxbotCmaAgent,
    response: Option<&str>,
    desktop: bool,
) -> AppResult<()> {
    tokio::time::timeout(AUTHORITY_TIMEOUT, async {
        tokio::try_join!(
            session_live(state, owner, session),
            agents::revalidate_stream(&state.db, gateway, owner, row, response, desktop),
        )?;
        Ok(())
    })
    .await
    .map_err(|_| AppError::Forbidden("CMA observation authority unavailable".into()))?
}

fn authority_interval(checked: Instant) -> tokio::time::Interval {
    // Opening the upstream counts against the check's age. A slow open makes
    // the first tick immediately ready; a fast open reuses the admission check.
    let mut interval = tokio::time::interval_at(checked + AUTHORITY_INTERVAL, AUTHORITY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

fn authority_check_completed(checked: &mut Instant, interval: &mut tokio::time::Interval) {
    *checked = Instant::now();
    // A successful check replaces any pending tick, including one missed
    // during downstream backpressure. It never renews the stream lifetime.
    interval.reset_at(*checked + AUTHORITY_INTERVAL);
}

pub async fn events(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> AppResult<Response> {
    relay_events(state, auth, id, None, query, headers).await
}
pub async fn response_events(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, response_id)): Path<(String, String)>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> AppResult<Response> {
    relay_events(state, auth, id, Some(response_id), query, headers).await
}

fn observation_error() -> Bytes {
    Bytes::from_static(b"event: cma.observation.error\ndata: {\"type\":\"error\",\"code\":\"cma_observation_unavailable\",\"message\":\"CMA observation ended. Reconnect with current authorization.\"}\n\n")
}

fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    (0..buffer.len()).find_map(|i| {
        if buffer[i..].starts_with(b"\r\n\r\n") {
            Some((i, 4))
        } else if buffer[i..].starts_with(b"\n\n") || buffer[i..].starts_with(b"\r\r") {
            Some((i, 2))
        } else {
            None
        }
    })
}

fn validate_frame(frame: &[u8], agent: &str, response: Option<&str>) -> Result<bool, ()> {
    if frame.len() > MAX_EVENT_BYTES {
        return Err(());
    }
    let text = std::str::from_utf8(frame).map_err(|_| ())?;
    let mut data = Vec::new();
    let mut event = None;
    let mut cursor = None;
    for line in text.split(['\r', '\n']).filter(|line| !line.is_empty()) {
        if line.starts_with(':') {
            continue;
        }
        let (key, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match key {
            "data" => data.push(value),
            "event" => {
                if event.replace(value).is_some() {
                    return Err(());
                }
            }
            "id" => {
                if cursor.replace(value).is_some() {
                    return Err(());
                }
            }
            "retry" => {}
            _ => return Err(()),
        }
    }
    if data.is_empty() {
        return Ok(true);
    }
    let body: Value = serde_json::from_str(&data.join("\n")).map_err(|_| ())?;
    if event.is_some_and(|name| name.ends_with("error"))
        || body.get("type").and_then(Value::as_str) == Some("error")
    {
        return Ok(false);
    }
    if body
        .get("agent_id")
        .is_some_and(|value| value.as_str() != Some(agent))
    {
        return Err(());
    }
    if let Some(response) = response {
        let kind = body.get("type").and_then(Value::as_str).ok_or(())?;
        if !kind.starts_with("response.") || event.is_some_and(|event| event != kind) {
            return Err(());
        }
        if body
            .get("response_id")
            .is_some_and(|value| value.as_str() != Some(response))
            || body
                .pointer("/response/id")
                .is_some_and(|value| value.as_str() != Some(response))
            || body
                .pointer("/response/conversation/id")
                .is_some_and(|value| value.as_str() != Some(agent))
        {
            return Err(());
        }
        let sequence = body
            .get("sequence_number")
            .and_then(Value::as_u64)
            .ok_or(())?;
        if cursor.and_then(|cursor| cursor.parse::<u64>().ok()) != Some(sequence) {
            return Err(());
        }
    } else {
        if body.get("agent_id").and_then(Value::as_str) != Some(agent) {
            return Err(());
        }
        match event {
            Some("cma.agent.event") => {
                let native = body.get("cursor").and_then(Value::as_str).ok_or(())?;
                if cursor != Some(native) || native.len() > MAX_CURSOR_BYTES {
                    return Err(());
                }
            }
            Some("cma.agent.live" | "cma.agent.live_gap") if cursor.is_none() => {}
            _ => return Err(()),
        }
    }
    Ok(true)
}

async fn relay_events(
    state: AppState,
    auth: AuthUser,
    id: String,
    response_id: Option<String>,
    query: StreamQuery,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (owner, row, gateway) = reference(&state, &auth, &id).await?;
    let cursor = resume_cursor(&query, &headers, response_id.is_some())?;
    stream_authority(
        &state,
        &gateway,
        &owner,
        auth.session_id,
        &row,
        response_id.as_deref(),
        false,
    )
    .await?;
    let checked = Instant::now();
    let mut upstream = gateway
        .open_event_stream(&owner, &row.cma_agent_id, response_id.as_deref(), cursor)
        .await?;
    let stream = async_stream::stream! {
        let started=Instant::now();let mut checked=checked;let mut buffer=Vec::new();let mut received=0usize;
        let mut interval=authority_interval(checked);
        let lifetime=tokio::time::sleep(Duration::from_secs(MAX_STREAM_SECONDS));tokio::pin!(lifetime);
        'relay: loop {
            tokio::select! {
                biased;
                _=&mut lifetime=>break,
                _=interval.tick()=>{
                    if stream_authority(&state,&gateway,&owner,auth.session_id,&row,response_id.as_deref(),false).await.is_err() {yield Ok::<Bytes,std::io::Error>(observation_error());break;}
                    authority_check_completed(&mut checked,&mut interval);
                }
                chunk=upstream.chunk()=>{
                    let chunk=match chunk {Ok(Some(chunk))=>chunk,Ok(None)=>break,Err(_)=>{yield Ok(observation_error());break;}};
                    received=received.saturating_add(chunk.len());
                    if received>MAX_STREAM_BYTES || buffer.len().saturating_add(chunk.len())>2*1024*1024 {yield Ok(observation_error());break;}
                    buffer.extend_from_slice(&chunk);
                    while let Some((end,separator))=frame_end(&buffer) {
                        let frame:Vec<u8>=buffer.drain(..end+separator).collect();
                        if !matches!(validate_frame(&frame[..end],&row.cma_agent_id,response_id.as_deref()),Ok(true)) {yield Ok(observation_error());break 'relay;}
                        // Recheck after downstream backpressure, before releasing
                        // any buffered frame; no observation renews the lifetime.
                        if started.elapsed()>=Duration::from_secs(MAX_STREAM_SECONDS) {break 'relay;}
                        if checked.elapsed()>=AUTHORITY_INTERVAL {
                            if stream_authority(&state,&gateway,&owner,auth.session_id,&row,response_id.as_deref(),false).await.is_err() {yield Ok(observation_error());break 'relay;}
                            authority_check_completed(&mut checked,&mut interval);
                        }
                        yield Ok(Bytes::from(frame));
                    }
                    if buffer.len()>MAX_EVENT_BYTES {yield Ok(observation_error());break;}
                }
            }
        }
    };
    Ok((
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "private, no-store"),
            ("x-accel-buffering", "no"),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

fn admit_desktop(frontend: &str, api: &str, auth: &AuthUser, headers: &HeaderMap) -> AppResult<()> {
    super::login_client_context::require_first_party_human(auth)?;
    let denied = || AppError::Forbidden("Desktop authentication or origin refused".into());
    if auth.mfa_enabled.is_none() || auth.session_id.is_none() {
        return Err(denied());
    }
    let mut authorizations = headers.get_all("authorization").iter();
    let bearer = if let Some(header) = authorizations.next() {
        if authorizations.next().is_some()
            || !header
                .to_str()
                .ok()
                .and_then(|s| s.strip_prefix("Bearer "))
                .is_some_and(|token| {
                    !token.is_empty() && !token.bytes().any(|b| b.is_ascii_whitespace())
                })
            || auth.auth_method != AuthMethod::AccessToken
            || !auth.allow_all_services
            || !auth.allow_all_nodes
            || !auth.allowed_service_ids.is_empty()
            || !auth.allowed_node_ids.is_empty()
            || auth.resource_uris.is_some()
        {
            return Err(denied());
        }
        true
    } else {
        if auth.auth_method != AuthMethod::Session {
            return Err(denied());
        }
        false
    };
    let mut origins = headers.get_all("origin").iter();
    let origin = origins.next();
    if origins.next().is_some() {
        return Err(denied());
    }
    let Some(origin) = origin else {
        return if bearer { Ok(()) } else { Err(denied()) };
    };
    let origin = origin.to_str().map_err(|_| denied())?;
    let frontend = url::Url::parse(frontend)
        .map_err(|_| denied())?
        .origin()
        .ascii_serialization();
    let api = url::Url::parse(api)
        .map_err(|_| denied())?
        .origin()
        .ascii_serialization();
    if origin == frontend || (bearer && origin == api) {
        Ok(())
    } else {
        Err(denied())
    }
}

pub async fn desktop_ws(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    admit_desktop(
        &state.config.frontend_url,
        &state.config.base_url,
        &auth,
        &headers,
    )?;
    let (owner, row, gateway) = reference(&state, &auth, &id).await?;
    stream_authority(&state, &gateway, &owner, auth.session_id, &row, None, true).await?;
    let checked = Instant::now();
    let upstream = gateway.open_desktop(&owner, &row.cma_agent_id).await?;
    // CMA's open enforces one viewer per shared Sandbox before our browser
    // upgrade succeeds. Neither endpoint mints a replacement Agent.
    Ok(ws
        .max_message_size(MAX_DESKTOP_FRAME_BYTES)
        .max_frame_size(MAX_DESKTOP_FRAME_BYTES)
        .on_upgrade(move |socket| {
            desktop_relay(state, auth, row, gateway, upstream, socket, checked)
        }))
}

async fn desktop_relay(
    state: AppState,
    auth: AuthUser,
    row: NyxbotCmaAgent,
    gateway: CatalogCmaGateway,
    upstream: CmaDesktopSocket,
    socket: WebSocket,
    checked: Instant,
) {
    use tokio_tungstenite::tungstenite::Message as Remote;
    let (mut remote_sink, mut remote_source) = upstream.split();
    let (mut browser_sink, mut browser_source) = socket.split();
    let mut checked = checked;
    let mut interval = authority_interval(checked);
    let lifetime = tokio::time::sleep(Duration::from_secs(MAX_STREAM_SECONDS));
    tokio::pin!(lifetime);
    loop {
        tokio::select! {
            biased;
            _=&mut lifetime=>break,
            _=interval.tick()=>{
                if stream_authority(&state,&gateway,&row.user_id,auth.session_id,&row,None,true).await.is_err(){break;}
                authority_check_completed(&mut checked,&mut interval);
            },
            outgoing=remote_source.next()=>{
                let message=match outgoing {Some(Ok(Remote::Binary(bytes)))=>Message::Binary(bytes),Some(Ok(Remote::Ping(bytes)))=>Message::Ping(bytes),Some(Ok(Remote::Pong(bytes)))=>Message::Pong(bytes),_=>break};
                if !matches!(tokio::time::timeout(Duration::from_secs(3),browser_sink.send(message)).await,Ok(Ok(()))){break;}
            },
            incoming=browser_source.next()=>{
                let message=match incoming {Some(Ok(Message::Binary(bytes)))=>Remote::Binary(bytes),Some(Ok(Message::Ping(bytes)))=>Remote::Ping(bytes),Some(Ok(Message::Pong(bytes)))=>Remote::Pong(bytes),_=>break};
                if !matches!(tokio::time::timeout(Duration::from_secs(3),remote_sink.send(message)).await,Ok(Ok(()))){break;}
            },
        }
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        remote_sink.send(Remote::Close(None)),
    )
    .await;
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        browser_sink.send(Message::Close(None)),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn authority_interval_reuses_the_initial_check_until_its_deadline() {
        let checked = tokio::time::Instant::now();
        let mut interval = authority_interval(checked);
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_pending());
        tokio::time::advance(AUTHORITY_INTERVAL - Duration::from_millis(1)).await;
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_pending());
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_ready());
    }

    #[tokio::test(start_paused = true)]
    async fn slow_upstream_open_does_not_postpone_the_next_authority_check() {
        let checked = tokio::time::Instant::now();
        tokio::time::advance(AUTHORITY_INTERVAL + Duration::from_secs(1)).await;
        let mut interval = authority_interval(checked);
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_ready());
    }

    #[tokio::test(start_paused = true)]
    async fn successful_authority_check_starts_the_next_interval_at_completion() {
        let mut checked = Instant::now();
        let mut interval = authority_interval(checked);
        tokio::time::advance(AUTHORITY_INTERVAL).await;
        interval.tick().await;
        // The check starts at t=10 and completes at t=14.
        tokio::time::advance(Duration::from_secs(4)).await;
        authority_check_completed(&mut checked, &mut interval);
        assert_eq!(checked, Instant::now());
        tokio::time::advance(Duration::from_secs(6)).await;
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_pending());
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_ready());
    }

    #[tokio::test(start_paused = true)]
    async fn buffered_frame_authority_check_clears_an_unconsumed_interval_tick() {
        let mut checked = Instant::now();
        let mut interval = authority_interval(checked);
        // Backpressure resumes at t=12, before the relay consumes its t=10
        // tick. The buffered-frame check then completes at t=16.
        tokio::time::advance(Duration::from_secs(12)).await;
        tokio::time::advance(Duration::from_secs(4)).await;
        authority_check_completed(&mut checked, &mut interval);
        assert_eq!(checked, Instant::now());
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_pending());
        tokio::time::advance(AUTHORITY_INTERVAL).await;
        assert!(futures::poll!(std::pin::pin!(interval.tick())).is_ready());
    }

    #[test]
    fn stream_cursors_are_bounded_and_headers_must_agree() {
        let mut headers = HeaderMap::new();
        let query = StreamQuery {
            after_cursor: Some("opaque-agent-cursor".into()),
        };
        assert_eq!(
            resume_cursor(&query, &headers, false).unwrap(),
            Some("opaque-agent-cursor")
        );
        assert!(resume_cursor(&query, &headers, true).is_err());
        headers.insert("last-event-id", "different".parse().unwrap());
        assert!(resume_cursor(&query, &headers, false).is_err());
        let query = StreamQuery {
            after_cursor: Some("4".into()),
        };
        headers.insert("last-event-id", "4".parse().unwrap());
        assert_eq!(resume_cursor(&query, &headers, true).unwrap(), Some("4"));
        headers.append("last-event-id", "4".parse().unwrap());
        assert!(resume_cursor(&query, &headers, true).is_err());
    }
    #[test]
    fn frames_cannot_substitute_agent_response_or_durable_cursor() {
        let event=b"event: cma.agent.event\nid: cursor\ndata: {\"agent_id\":\"agent\",\"cursor\":\"cursor\",\"sequence\":1,\"kind\":{\"type\":\"message\"}}";
        assert_eq!(validate_frame(event, "agent", None), Ok(true));
        assert!(validate_frame(event, "other", None).is_err());
        let response=b"event: response.completed\nid: 3\ndata: {\"type\":\"response.completed\",\"sequence_number\":3,\"response\":{\"id\":\"r1\",\"conversation\":{\"id\":\"agent\"}}}";
        assert_eq!(validate_frame(response, "agent", Some("r1")), Ok(true));
        assert!(validate_frame(response, "agent", Some("r2")).is_err());
        assert!(validate_frame(response, "other", Some("r1")).is_err());
        assert!(validate_frame(&vec![b'x'; MAX_EVENT_BYTES + 1], "agent", None).is_err());
        assert_eq!(
            validate_frame(
                b"event: cma.agent.error\ndata: {\"message\":\"internal detail\"}",
                "agent",
                None
            ),
            Ok(false)
        );
    }
    #[test]
    fn browser_desktop_requires_exact_origin_and_native_bearer_keeps_session_authority() {
        let mut auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        auth.mfa_enabled = Some(false);
        auth.session_id = Some(uuid::Uuid::new_v4());
        let mut headers = HeaderMap::new();
        let frontend = "https://web.example";
        let api = "https://api.example";
        assert!(admit_desktop(frontend, api, &auth, &headers).is_err());
        headers.insert("origin", frontend.parse().unwrap());
        assert!(admit_desktop(frontend, api, &auth, &headers).is_ok());
        headers.insert("origin", "https://other.example".parse().unwrap());
        assert!(admit_desktop(frontend, api, &auth, &headers).is_err());
        headers.remove("origin");
        headers.insert("authorization", "Bearer explicit-token".parse().unwrap());
        auth.auth_method = AuthMethod::AccessToken;
        assert!(admit_desktop(frontend, api, &auth, &headers).is_ok());
        auth.session_id = None;
        assert!(admit_desktop(frontend, api, &auth, &headers).is_err());
    }
}
