//! Node-signed service calls execute with the live key of a server-issued job.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::machine_job::{COLLECTION_NAME as JOBS, MachineJob},
    mw::auth::AuthUser,
    services::{
        billing::{BillingIngress, route_inventory::BillingRoutePolicy},
        key_service, node_service,
        node_ws_manager::NodeOutboundMessage,
    },
};
use axum::{
    body::Body,
    http::Request,
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use mongodb::bson::doc;
use nyxid_machine::{
    binary::{Frame, Kind},
    signing::ReplayGuard,
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};

pub struct Session {
    uploads: Mutex<HashMap<uuid::Uuid, Upload>>,
    calls: Mutex<HashMap<uuid::Uuid, ActiveCall>>,
    closed: tokio_util::sync::CancellationToken,
    replay: Mutex<ReplayGuard>,
    sender: mpsc::Sender<NodeOutboundMessage>,
}

struct ActiveCall {
    job_id: String,
    cancel: tokio_util::sync::CancellationToken,
}

struct Upload {
    sender: mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
    sequence: u64,
}

impl Session {
    pub fn new(sender: mpsc::Sender<NodeOutboundMessage>) -> Arc<Self> {
        Arc::new(Self {
            uploads: Mutex::new(HashMap::new()),
            calls: Mutex::new(HashMap::new()),
            closed: tokio_util::sync::CancellationToken::new(),
            replay: Mutex::new(ReplayGuard::default()),
            sender,
        })
    }
    pub async fn receive(&self, frame: Frame<'_>) {
        if frame.kind == Kind::GatewayCancel {
            if let Some(call) = self.calls.lock().await.get(&frame.id) {
                call.cancel.cancel();
            }
            return;
        }
        if !matches!(frame.kind, Kind::GatewayUpload | Kind::GatewayUploadAbort) {
            return;
        }
        let (sender, valid) = {
            let mut uploads = self.uploads.lock().await;
            let Some(upload) = uploads.get_mut(&frame.id) else {
                return;
            };
            let valid = upload.sequence == frame.sequence && frame.kind != Kind::GatewayUploadAbort;
            upload.sequence += 1;
            let sender = upload.sender.clone();
            if frame.end || !valid {
                uploads.remove(&frame.id);
            }
            (sender, valid)
        };
        if !valid {
            let _ = sender.try_send(Err(std::io::Error::other("machine upload interrupted")));
            return;
        }
        if !frame.bytes.is_empty()
            && !tokio::time::timeout(
                Duration::from_secs(1),
                sender.send(Ok(frame.bytes.to_vec())),
            )
            .await
            .is_ok_and(|r| r.is_ok())
        {
            self.uploads.lock().await.remove(&frame.id);
            if let Some(call) = self.calls.lock().await.get(&frame.id) {
                call.cancel.cancel();
            }
            return;
        }
        if frame.end {
            let _ = tokio::time::timeout(Duration::from_secs(1), sender.send(Ok(Vec::new()))).await;
        }
    }
    pub async fn start(
        self: &Arc<Self>,
        state: AppState,
        node_id: &str,
        request: nyxid_machine::Request,
    ) {
        if request.operation == nyxid_machine::Operation::JobFinished {
            if verify(state.clone(), self, node_id, &request).await.is_ok() {
                let job_id = request.parameters["job_id"].as_str().unwrap_or_default();
                let result = state.db.collection::<MachineJob>(JOBS).update_one(
                    doc! {
                        "_id": job_id,
                         "node_id": node_id,
                         "runtime_id": request.parameters["runtime_id"].as_str().unwrap_or_default(),
                         "conversation_id": request.parameters["conversation_id"].as_str().unwrap_or_default()
                    },
                    doc! {
                        "$set":{
                            "state":"finished",
                            "finished_at":mongodb::bson::DateTime::now()
                        }
                    },
                ).await;
                // Acknowledge only a durable update. A lost acknowledgement is
                // retried with a fresh nonce, so this transition is idempotent.
                if result.is_ok_and(|update| update.matched_count == 1) {
                    for call in self
                        .calls
                        .lock()
                        .await
                        .values()
                        .filter(|call| call.job_id == job_id)
                    {
                        call.cancel.cancel();
                    }
                    let _ = self
                        .sender
                        .send(NodeOutboundMessage::Text(
                            json!({
                                "type":"machine_job_finished_ack", "request_id":request.request_id
                            })
                            .to_string(),
                        ))
                        .await;
                }
            }
            return;
        }
        let session = self.clone();
        let node_id = node_id.to_owned();
        let id = request.request_id.clone();
        let header_timeout = if request.parameters["path"]
            .as_str()
            .is_some_and(|p| p.starts_with("/git/"))
        {
            nyxid_machine::GIT_UPLOAD_TIMEOUT_SECS
        } else {
            120
        };
        let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
            return;
        };
        let cancel = self.closed.child_token();
        {
            let mut calls = self.calls.lock().await;
            if calls.contains_key(&uuid) {
                return;
            }
            if calls.len() >= 32 {
                drop(calls);
                // A node must get a bounded HTTP failure, not wait for the
                // request timeout when its gateway concurrency is exhausted.
                let response = AppError::RateLimited.into_response();
                let status = response.status().as_u16();
                let bytes = axum::body::to_bytes(response.into_body(), 65536)
                    .await
                    .unwrap_or_default();
                let reply = async {
                    self.sender
                        .send(NodeOutboundMessage::Text(
                            json!({
                                "type":"machine_service_response",
                                "request_id":id,
                                "status":status,
                                "headers":[["content-type","application/json"]]
                            })
                            .to_string(),
                        ))
                        .await
                        .ok()?;
                    let frame = (Frame {
                        kind: Kind::GatewayDownload,
                        end: true,
                        id: uuid,
                        sequence: 0,
                        bytes: &bytes,
                    })
                    .encode()
                    .ok()?;
                    self.sender
                        .send(NodeOutboundMessage::Binary(frame))
                        .await
                        .ok()
                };
                let _ = tokio::time::timeout(Duration::from_secs(1), reply).await;
                return;
            }
            calls.insert(
                uuid,
                ActiveCall {
                    job_id: request.parameters["job_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    cancel: cancel.clone(),
                },
            );
        }
        let (tx, rx) = mpsc::channel(16);
        {
            let mut uploads = self.uploads.lock().await;
            uploads.insert(
                uuid,
                Upload {
                    sender: tx,
                    sequence: 0,
                },
            );
        }
        #[cfg(test)]
        let target_client = crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
            .try_with(Clone::clone)
            .ok();
        tokio::spawn(async move {
            let execute = async {
                let response = tokio::time::timeout(
                    Duration::from_secs(header_timeout),
                    authorize_and_execute(&state, &session, &node_id, request, rx),
                )
                .await;
                let response = match response {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => error.into_response(),
                    Err(_) => AppError::NodeProxyTimeout.into_response(),
                };
                let status = response.status().as_u16();
                let headers: Vec<_> = response
                    .headers()
                    .iter()
                    .filter(|(k, _)| {
                        matches!(
                            k.as_str(),
                            "content-type"
                                | "content-encoding"
                                | "content-length"
                                | "cache-control"
                                | "retry-after"
                                | "content-disposition"
                        )
                    })
                    .filter_map(|(k, v)| {
                        v.to_str()
                            .ok()
                            .map(|v| (k.as_str().to_owned(), v.to_owned()))
                    })
                    .collect();
                if session
                    .sender
                    .send(NodeOutboundMessage::Text(
                        json!({
                            "type":"machine_service_response",
                            "request_id":id,
                            "status":status,
                            "headers":headers
                        })
                        .to_string(),
                    ))
                    .await
                    .is_ok()
                {
                    let mut body = response.into_body().into_data_stream();
                    let mut sequence = 0;
                    let mut aborted = false;
                    while let Some(result) = body.next().await {
                        let Ok(bytes) = result else {
                            aborted = true;
                            break;
                        };
                        for chunk in bytes.chunks(nyxid_machine::STREAM_CHUNK_BYTES) {
                            let Ok(frame) = (Frame {
                                kind: Kind::GatewayDownload,
                                end: false,
                                id: uuid,
                                sequence,
                                bytes: chunk,
                            })
                            .encode() else {
                                return;
                            };
                            if session
                                .sender
                                .send(NodeOutboundMessage::Binary(frame))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            sequence += 1;
                        }
                    }
                    if let Ok(frame) = (Frame {
                        kind: if aborted {
                            Kind::GatewayDownloadAbort
                        } else {
                            Kind::GatewayDownload
                        },
                        end: true,
                        id: uuid,
                        sequence,
                        bytes: &[],
                    })
                    .encode()
                    {
                        let _ = session
                            .sender
                            .send(NodeOutboundMessage::Binary(frame))
                            .await;
                    }
                }
            };
            #[cfg(test)]
            let execute = async {
                if let Some(builder) = target_client {
                    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
                        .scope(builder, execute)
                        .await;
                } else {
                    execute.await;
                }
            };
            tokio::select! {
                _ = execute => {},
                _ = cancel.cancelled() => {
                    // Abort also releases a node still waiting for response
                    // headers: removing its pending row drops that oneshot.
                    if let Ok(frame) = (Frame {
                        kind: Kind::GatewayDownloadAbort,
                        end: true,
                        id: uuid,
                        sequence: 0,
                        bytes: &[],
                    }).encode() {
                        let _ = tokio::time::timeout(
                            Duration::from_secs(1),
                            session.sender.send(NodeOutboundMessage::Binary(frame)),
                        ).await;
                    }
                }
            }
            session.uploads.lock().await.remove(&uuid);
            session.calls.lock().await.remove(&uuid);
        });
    }
    pub async fn close(&self) {
        self.closed.cancel();
        self.uploads.lock().await.clear();
    }
}

async fn authorize_and_execute(
    state: &AppState,
    session: &Session,
    node_id: &str,
    request: nyxid_machine::Request,
    receiver: mpsc::Receiver<Result<Vec<u8>, std::io::Error>>,
) -> AppResult<Response> {
    if request.operation != nyxid_machine::Operation::ServiceCall || request.node_id != node_id {
        return Err(AppError::Forbidden("Invalid machine service call".into()));
    }
    verify(state.clone(), session, node_id, &request).await?;
    let p = &request.parameters;
    let id = p["job_id"].as_str().unwrap_or_default();
    // The unique _id index handles the one job-binding lookup. Authority never
    // comes from identity or key IDs supplied by the node.
    let job = crate::services::machine_service::gateway_job(
        &state.db,
        node_id,
        p["runtime_id"].as_str().unwrap_or_default(),
        p["conversation_id"].as_str().unwrap_or_default(),
        id,
    )
    .await?;
    let auth = job_auth(state, &job).await?;
    crate::services::machine_desktop_service::agent_allowed(&state.db, node_id).await?;
    let raw_path = p["path"]
        .as_str()
        .ok_or_else(|| AppError::ValidationError("Invalid gateway path".into()))?;
    if raw_path.len() > 8192 {
        return Err(AppError::ValidationError("Gateway path too long".into()));
    }
    let method = p["method"].as_str().unwrap_or("GET");
    let available = crate::services::machine_gateway_service::services(
        &state.db,
        &job.user_id,
        &job.api_key_id,
    )
    .await?;
    let is_declared = |row: &&crate::services::machine_gateway_service::AvailableService| {
        job.services
            .iter()
            .any(|grant| grant.id == row.id && grant.slug == row.slug)
    };
    let selected;
    let git;
    let (slug, path) = if raw_path.starts_with("/git/") {
        let (host, path) = crate::services::machine_gateway_service::git_path(raw_path, method)?;
        selected = available.iter().filter(is_declared).find(|row| {
            row.git.as_ref().is_some_and(|git| {
                crate::services::machine_gateway_service::git_host(git).ok().as_deref() == Some(host)
            })
        }).ok_or_else(|| AppError::ApiKeyScopeForbidden(
            format!("Declare the connected git host service for {host} in services on nyx__machine_exec"),
        ))?;
        git = selected.git.clone();
        (selected.slug.as_str(), path)
    } else {
        let (slug, path) = raw_path
            .strip_prefix("/s/")
            .and_then(|p| p.split_once('/'))
            .ok_or_else(|| {
                AppError::ValidationError("Use /s/{slug}/{path} or /git/{host}/{repository}".into())
            })?;
        selected = available.iter().filter(is_declared).find(|row| row.slug == slug)
            .ok_or_else(|| AppError::ApiKeyScopeForbidden(format!(
                "Declare {slug} in services on nyx__machine_exec; this job may only call its declared, still-accessible services"
            )))?;
        git = None;
        (slug, path)
    };
    if slug.is_empty()
        || !slug
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(AppError::ValidationError("Invalid service slug".into()));
    }
    if !matches!(
        method,
        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
    ) {
        return Err(AppError::ValidationError(
            "HTTP method is not supported".into(),
        ));
    }
    let stream = futures::stream::unfold((receiver, false), |(mut receiver, ended)| async move {
        if ended {
            return None;
        }
        match tokio::time::timeout(Duration::from_secs(60), receiver.recv()).await {
            Ok(Some(Ok(bytes))) if bytes.is_empty() => None,
            Ok(Some(value)) => {
                let ended = value.is_err();
                Some((value, (receiver, ended)))
            }
            Ok(None) | Err(_) => Some((
                Err(std::io::Error::other(
                    "machine disconnected or idle before upload completion",
                )),
                (receiver, true),
            )),
        }
    });
    // A process may not override the server-bound credential selection through
    // the ordinary proxy's routing query parameter.
    let (resource, query) = path.split_once('?').unwrap_or((path, ""));
    let query = {
        let mut query_builder = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if key != "_nyxid_via" {
                query_builder.append_pair(&key, &value);
            }
        }
        if selected.user_service {
            query_builder.append_pair("_nyxid_via", &selected.id);
        }
        query_builder.finish()
    };
    let suffix = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("/api/v1/proxy/s/{slug}/{resource}{suffix}"));
    if let Some(headers) = p["headers"].as_array() {
        if headers.len() > 64 {
            return Err(AppError::ValidationError("Too many gateway headers".into()));
        }
        for pair in headers {
            if let (Some(key), Some(value)) = (pair[0].as_str(), pair[1].as_str()) {
                let lower = key.to_ascii_lowercase();
                if lower.starts_with("x-nyxid-")
                    || lower.starts_with("x-forwarded-")
                    || lower.starts_with("proxy-")
                    || lower == "forwarded"
                {
                    continue;
                }
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "authorization"
                        | "x-api-key"
                        | "host"
                        | "cookie"
                        | "connection"
                        | "transfer-encoding"
                        | "proxy-authorization"
                        | "upgrade"
                ) {
                    continue;
                }
                if key.len() + value.len() > 8192 {
                    return Err(AppError::ValidationError("Gateway header too long".into()));
                }
                builder = builder.header(key, value);
            }
        }
    }
    let mut request = builder
        .body(Body::from_stream(stream))
        .map_err(|_| AppError::ValidationError("Invalid gateway HTTP request".into()))?;
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
    request
        .extensions_mut()
        .insert(crate::services::machine_gateway_service::Ingress {
            declared_id: selected.id.clone(),
            git,
        });
    let path_only = path.split('?').next().unwrap_or_default();
    super::proxy::proxy_request_by_slug_inner(
        state,
        &auth,
        slug,
        path_only,
        request,
        &mut String::new(),
    )
    .await
}

pub(crate) async fn job_auth(state: &AppState, job: &MachineJob) -> AppResult<AuthUser> {
    let key = key_service::get_api_key(&state.db, &job.user_id, &job.api_key_id).await?;
    if key.expires_at.is_some_and(|at| at <= chrono::Utc::now()) {
        return Err(AppError::Forbidden("The job's chat key expired".into()));
    }
    let bound = state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
        )
        .find_one(doc! {
            "user_id":&job.user_id,
            "conversation_id":&job.conversation_id,
            "api_key_id":&job.api_key_id
        })
        .await?;
    if bound.is_none() {
        return Err(AppError::Forbidden(
            "The job's chat key is no longer bound to its conversation".into(),
        ));
    }
    let node = node_service::get_node_by_id(&state.db, &job.node_id)
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Machine unavailable".into()))?;
    if !node.is_active {
        return Err(AppError::MachineNotAllowed);
    }
    crate::services::machine_service::capable(&node, nyxid_machine::Operation::ServiceCall)?;
    if job.runtime_id.is_empty()
        || node
            .machine
            .as_ref()
            .is_none_or(|profile| profile.runtime_id != job.runtime_id)
    {
        return Err(AppError::Forbidden(
            "The machine restarted; start a new job".into(),
        ));
    }
    if !crate::services::org_service::resolve_owner_access(&state.db, &job.user_id, &node.user_id)
        .await?
        .can_write()
    {
        return Err(AppError::Forbidden("Machine ownership changed".into()));
    }
    let agent =
        crate::services::assistant_team_service::agent(&state.db, &job.user_id, &job.agent_id)
            .await?;
    if agent.destroyed_at.is_some()
        || (!agent.is_nyxbot() && !agent.machine_node_ids.contains(&job.node_id))
    {
        return Err(AppError::Forbidden("Machine grant was removed".into()));
    }
    crate::mw::auth::api_key_auth_user(&state.db, &key, None, None, None).await
}

async fn verify(
    state: AppState,
    session: &Session,
    node_id: &str,
    request: &nyxid_machine::Request,
) -> AppResult<()> {
    let secret =
        node_service::get_node_signing_secret(&state.db, &state.encryption_keys, node_id).await?;
    session
        .replay
        .lock()
        .await
        .verify(request, node_id, &secret, chrono::Utc::now().timestamp())
        .map_err(|_| AppError::Forbidden("Machine service signature refused".into()))
}
