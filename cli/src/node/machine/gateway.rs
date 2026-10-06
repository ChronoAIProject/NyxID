//! Per-job loopback HTTP gateway. Tokens are local capabilities, never NyxID keys.
use super::jobs::Jobs;
use crate::node::ws_client::NodeWsMessage;
use anyhow::{Context, Result, bail};
use axum::{
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use futures::StreamExt;
use nyxid_machine::{
    Operation,
    binary::{Frame, Kind},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct Gateway {
    url: String,
    jobs: Weak<Jobs>,
    node_id: String,
    runtime_id: String,
    server_task: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
    tokens: Mutex<HashMap<[u8; 32], Binding>>,
    sender: RwLock<Option<mpsc::Sender<NodeWsMessage>>>,
    signing: Zeroizing<Vec<u8>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
}
struct Binding {
    job_id: String,
    conversation_id: String,
    report: Option<(String, Instant)>,
    authority_binding: Value,
}
struct Pending {
    job_id: String,
    start: Option<oneshot::Sender<Value>>,
    body: mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
    sequence: u64,
}

impl Gateway {
    pub async fn start(
        jobs: Weak<Jobs>,
        node_id: String,
        runtime_id: String,
        signing: Zeroizing<Vec<u8>>,
    ) -> Result<Arc<Self>> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let gateway = Arc::new(Self {
            url: format!("http://127.0.0.1:{}", listener.local_addr()?.port()),
            jobs,
            node_id,
            runtime_id,
            server_task: std::sync::Mutex::new(None),
            tokens: Mutex::new(HashMap::new()),
            sender: RwLock::new(None),
            signing,
            pending: Mutex::new(HashMap::new()),
        });
        let weak = Arc::downgrade(&gateway);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            loop {
                interval.tick().await;
                let Some(gateway) = weak.upgrade() else {
                    break;
                };
                gateway.report_finished().await;
            }
        });
        let router = axum::Router::new()
            .fallback(forward)
            .with_state(Arc::downgrade(&gateway));
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        *gateway
            .server_task
            .lock()
            .expect("gateway server task lock") = Some(server.abort_handle());
        Ok(gateway)
    }
    async fn report_finished(&self) {
        let Some(sender) = self.sender.read().await.clone() else {
            return;
        };
        let Some(jobs) = self.jobs.upgrade() else {
            return;
        };
        let mut tokens = self.tokens.lock().await;
        for binding in tokens.values_mut() {
            if !jobs.finished(&binding.job_id).await
                || binding
                    .report
                    .as_ref()
                    .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(1))
            {
                continue;
            }
            let mut request = nyxid_machine::Request {
                version: 1,
                authority: None,
                // Retain the correlation ID until acknowledged. A slow server
                // may acknowledge an earlier attempt after this retry is sent.
                request_id: binding
                    .report
                    .as_ref()
                    .map(|(id, _)| id.clone())
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                node_id: self.node_id.clone(),
                operation: Operation::JobFinished,
                parameters: json!({"job_id":binding.job_id,"conversation_id":binding.conversation_id,"runtime_id":self.runtime_id}),
                timestamp: chrono::Utc::now().timestamp(),
                nonce: Uuid::new_v4().to_string(),
                signature: String::new(),
            };
            request.signature = nyxid_machine::signing::sign(&request, &self.signing);
            let request_id = request.request_id.clone();
            if let Ok(mut message) = serde_json::to_value(request) {
                message["type"] = json!("machine_service_call");
                if sender
                    .try_send(NodeWsMessage::Text(message.to_string()))
                    .is_ok()
                {
                    binding.report = Some((request_id, Instant::now()));
                }
            }
        }
    }
    pub async fn finished_ack(&self, request_id: &str) {
        self.tokens.lock().await.retain(|_, binding| {
            binding
                .report
                .as_ref()
                .is_none_or(|(id, _)| id != request_id)
        });
    }
    pub async fn remove_job(&self, job_id: &str) {
        self.tokens
            .lock()
            .await
            .retain(|_, binding| binding.job_id != job_id);
    }
    pub async fn cancel_jobs(&self, jobs: &[String]) {
        self.tokens
            .lock()
            .await
            .retain(|_, binding| !jobs.contains(&binding.job_id));
        let cancelled = {
            let mut pending = self.pending.lock().await;
            let ids: Vec<_> = pending
                .iter()
                .filter(|(_, call)| jobs.contains(&call.job_id))
                .map(|(id, _)| *id)
                .collect();
            for id in &ids {
                pending.remove(id);
            }
            ids
        };
        if let Some(sender) = self.sender.read().await.as_ref() {
            // One bound for the entire batch, independent of the number of
            // streams. Dropped response channels already wake local clients.
            let _ = tokio::time::timeout(Duration::from_millis(250), async {
                for id in cancelled {
                    if let Ok(frame) = (Frame {
                        kind: Kind::GatewayCancel,
                        end: true,
                        id,
                        sequence: 0,
                        bytes: &[],
                    })
                    .encode()
                        && sender.send(NodeWsMessage::Binary(frame)).await.is_err()
                    {
                        break;
                    }
                }
            })
            .await;
        }
    }

    pub async fn connect(&self, sender: mpsc::Sender<NodeWsMessage>) {
        *self.sender.write().await = Some(sender);
    }
    pub async fn disconnect(&self) {
        *self.sender.write().await = None;
        self.pending.lock().await.clear();
        for binding in self.tokens.lock().await.values_mut() {
            binding.report = None;
        }
    }
    pub async fn environment(
        &self,
        job_id: &str,
        conversation_id: &str,
        spec: &nyxid_machine::gateway::Environment,
    ) -> Result<BTreeMap<String, String>> {
        let token = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
        self.jobs
            .upgrade()
            .context("machine stopped")?
            .register_secret(&token)
            .await?;
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut tokens = self.tokens.lock().await;
        if tokens.len() >= 64 {
            bail!("gateway job limit exceeded");
        }
        tokens.insert(
            hash,
            Binding {
                job_id: job_id.into(),
                conversation_id: conversation_id.into(),
                report: None,
                authority_binding: Value::Null,
            },
        );
        let mut env = BTreeMap::from([
            ("NYXID_GATEWAY_URL".into(), self.url.clone()),
            ("NYXID_GATEWAY_TOKEN".into(), token.to_string()),
        ]);
        for (name, value) in &spec.variables {
            if name.starts_with("NYXID_")
                || name.starts_with("GIT_")
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                || !(name.ends_with("_BASE_URL") || name.ends_with("_API_KEY"))
            {
                bail!("invalid gateway environment variable");
            }
            let value = match value {
                nyxid_machine::gateway::Variable::GatewayPath(path) => {
                    if !path.starts_with("/s/")
                        || path.contains(['?', '#', '\\'])
                        || path.contains("..")
                    {
                        bail!("invalid gateway environment path");
                    }
                    format!("{}{path}", self.url)
                }
                nyxid_machine::gateway::Variable::GatewayToken => token.to_string(),
            };
            env.insert(name.clone(), value);
        }
        let mut git = Vec::new();
        for rewrite in &spec.git {
            let origin = url::Url::parse(&rewrite.origin)?;
            if origin.scheme() != "https"
                || origin.origin().ascii_serialization() != rewrite.origin
                || !rewrite.path.starts_with("/git/")
                || rewrite.path.contains("..")
            {
                bail!("invalid gateway git origin");
            }
            let prefix = format!("{}{}", self.url, rewrite.path);
            git.push((
                format!("url.{prefix}.insteadOf"),
                format!("{}/", rewrite.origin),
            ));
            git.push((
                format!("url.{prefix}.insteadOf"),
                format!("git@{}:", origin.host_str().context("git host missing")?),
            ));
            git.push((
                format!("http.{prefix}.extraHeader"),
                format!("Authorization: Bearer {}", token.as_str()),
            ));
        }
        if !git.is_empty() {
            env.insert("GIT_CONFIG_COUNT".into(), git.len().to_string());
            for (index, (key, value)) in git.into_iter().enumerate() {
                env.insert(format!("GIT_CONFIG_KEY_{index}"), key);
                env.insert(format!("GIT_CONFIG_VALUE_{index}"), value);
            }
        }
        Ok(env)
    }
    pub async fn bind_authority(&self, job_id: &str, authority: &Value) {
        let mut tokens = self.tokens.lock().await;
        for binding in tokens.values_mut().filter(|b| b.job_id == job_id) {
            binding.authority_binding = if authority.is_object() {
                json!({"context_id":authority["context_id"],"revision":authority["revision"],"lease_id":authority["lease_id"]})
            } else {
                Value::Null
            };
        }
    }
    pub async fn response(&self, id: &str, metadata: Value) {
        if let Ok(id) = Uuid::parse_str(id)
            && let Some(pending) = self.pending.lock().await.get_mut(&id)
            && let Some(start) = pending.start.take()
        {
            let _ = start.send(metadata);
        }
    }
    async fn cancel(&self, id: Uuid) {
        if let Some(sender) = self.sender.read().await.as_ref()
            && let Ok(frame) = (Frame {
                kind: Kind::GatewayCancel,
                end: true,
                id,
                sequence: 0,
                bytes: &[],
            })
            .encode()
        {
            let _ = sender.try_send(NodeWsMessage::Binary(frame));
        }
    }
    pub async fn chunk(&self, frame: Frame<'_>) {
        if !matches!(
            frame.kind,
            Kind::GatewayDownload | Kind::GatewayDownloadAbort
        ) {
            return;
        }
        let (sender, valid) = {
            let mut pending = self.pending.lock().await;
            let Some(stream) = pending.get_mut(&frame.id) else {
                return;
            };
            let valid =
                stream.sequence == frame.sequence && frame.kind != Kind::GatewayDownloadAbort;
            stream.sequence += 1;
            let sender = stream.body.clone();
            if frame.end || !valid {
                pending.remove(&frame.id);
            }
            (sender, valid)
        };
        if !valid {
            let _ = sender.try_send(Err(std::io::Error::other(
                "machine gateway stream interrupted",
            )));
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
            self.pending.lock().await.remove(&frame.id);
            self.cancel(frame.id).await;
            return;
        }
        if frame.end {
            let _ = tokio::time::timeout(Duration::from_secs(1), sender.send(Ok(Vec::new()))).await;
        }
    }
}

async fn forward(
    State(gateway): State<Weak<Gateway>>,
    request: Request<Body>,
) -> axum::response::Response {
    let Some(gateway) = gateway.upgrade() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match forward_inner(gateway,request).await{Ok(response)=>response,Err(_)=>(StatusCode::BAD_GATEWAY,axum::Json(json!({"error":{"code":8001,"message":"Machine gateway unavailable or the job ended"}}))).into_response()}
}
async fn forward_inner(
    gateway: Arc<Gateway>,
    request: Request<Body>,
) -> Result<axum::response::Response> {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or_default();
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let (job_id, conversation_id, authority_binding) = {
        let tokens = gateway.tokens.lock().await;
        let Some(binding) = tokens.get(&hash) else {
            return Ok((StatusCode::UNAUTHORIZED, axum::Json(json!({"error":{"code":12401,"message":"A live job gateway token is required"}}))).into_response());
        };
        (
            binding.job_id.clone(),
            binding.conversation_id.clone(),
            binding.authority_binding.clone(),
        )
    };
    let jobs = gateway.jobs.upgrade().context("machine stopped")?;
    if !jobs.running(&job_id).await {
        return Ok((
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error":{"code":12403,"message":"job_ended"}})),
        )
            .into_response());
    }
    let path = request
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    if !path.starts_with("/s/") && !path.starts_with("/git/") {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let header_timeout = if path.starts_with("/git/") {
        nyxid_machine::GIT_UPLOAD_TIMEOUT_SECS
    } else {
        120
    };
    let headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "authorization"
                    | "x-api-key"
                    | "host"
                    | "connection"
                    | "transfer-encoding"
                    | "cookie"
                    | "proxy-authorization"
                    | "upgrade"
                    | "x-nyxid-user-token"
            )
        })
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().into(), v.into())))
        .collect();
    if headers
        .iter()
        .map(|(k, v)| k.len() + v.len())
        .sum::<usize>()
        > 16384
    {
        bail!("gateway headers too large");
    }
    let id = Uuid::new_v4();
    let mut signed = nyxid_machine::Request {
        version: 1,
        authority: None,
        request_id: id.to_string(),
        node_id: gateway.node_id.clone(),
        operation: Operation::ServiceCall,
        parameters: json!({"job_id":job_id,"conversation_id":conversation_id,"authority_binding":authority_binding,"runtime_id":gateway.runtime_id,"path":path,"method":request.method().as_str(),"headers":headers}),
        timestamp: chrono::Utc::now().timestamp(),
        nonce: Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    signed.signature = nyxid_machine::signing::sign(&signed, &gateway.signing);
    let mut metadata = serde_json::to_value(signed)?;
    metadata["type"] = json!("machine_service_call");
    let sender = gateway
        .sender
        .read()
        .await
        .clone()
        .context("machine offline")?;
    let (start_tx, start_rx) = oneshot::channel();
    let (body_tx, body_rx) = mpsc::channel(16);
    {
        let mut pending = gateway.pending.lock().await;
        // Cancellation removes tokens before locking pending. Revalidate
        // admission under this lock so it cannot miss a newly inserted call.
        if !jobs.running(&job_id).await {
            return Ok((
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({"error":{"code":12403,"message":"job_ended"}})),
            )
                .into_response());
        }
        if pending.len() >= 32 {
            bail!("gateway concurrency limit exceeded");
        }
        pending.insert(
            id,
            Pending {
                job_id: job_id.clone(),
                start: Some(start_tx),
                body: body_tx,
                sequence: 0,
            },
        );
    }
    if sender
        .send(NodeWsMessage::Text(metadata.to_string()))
        .await
        .is_err()
    {
        gateway.pending.lock().await.remove(&id);
        bail!("machine offline");
    }
    let upload = tokio::spawn(async move {
        let mut body = request.into_body().into_data_stream();
        let mut sequence = 0;
        let mut total = 0u64;
        let mut aborted = false;
        while let Some(result) = body.next().await {
            let Ok(bytes) = result else {
                aborted = true;
                break;
            };
            total += bytes.len() as u64;
            if total > 16 * 1024 * 1024 * 1024 {
                aborted = true;
                break;
            }
            for chunk in bytes.chunks(nyxid_machine::STREAM_CHUNK_BYTES) {
                let Ok(frame) = (Frame {
                    kind: Kind::GatewayUpload,
                    end: false,
                    id,
                    sequence,
                    bytes: chunk,
                })
                .encode() else {
                    return;
                };
                if sender.send(NodeWsMessage::Binary(frame)).await.is_err() {
                    return;
                }
                sequence += 1;
            }
        }
        if let Ok(frame) = (Frame {
            kind: if aborted {
                Kind::GatewayUploadAbort
            } else {
                Kind::GatewayUpload
            },
            end: true,
            id,
            sequence,
            bytes: &[],
        })
        .encode()
        {
            let _ = sender.send(NodeWsMessage::Binary(frame)).await;
        }
    });
    let cleanup = StreamCleanup {
        gateway: gateway.clone(),
        id,
        upload,
    };
    let start = match tokio::time::timeout(Duration::from_secs(header_timeout), start_rx).await {
        Ok(Ok(start)) => start,
        _ => {
            bail!("gateway response timeout");
        }
    };
    let status = start["status"]
        .as_u64()
        .filter(|s| (100..600).contains(s))
        .unwrap_or(502) as u16;
    let mut builder = Response::builder().status(status);
    if let Some(headers) = start["headers"].as_array() {
        for pair in headers {
            if let (Some(key), Some(value)) = (pair[0].as_str(), pair[1].as_str())
                && matches!(
                    key,
                    "content-type"
                        | "content-encoding"
                        | "content-length"
                        | "cache-control"
                        | "retry-after"
                        | "content-disposition"
                )
            {
                builder = builder.header(key, value);
            }
        }
    }
    let stream = futures::stream::unfold(
        (body_rx, false, cleanup),
        |(mut body_rx, ended, cleanup)| async move {
            if ended {
                return None;
            }
            match tokio::time::timeout(Duration::from_secs(60), body_rx.recv()).await {
                Ok(Some(Ok(bytes))) if bytes.is_empty() => None,
                Ok(Some(value)) => {
                    let ended = value.is_err();
                    Some((value, (body_rx, ended, cleanup)))
                }
                Ok(None) | Err(_) => Some((
                    Err(std::io::Error::other(
                        "machine gateway disconnected or idle before stream completion",
                    )),
                    (body_rx, true, cleanup),
                )),
            }
        },
    );
    Ok(builder.body(Body::from_stream(stream))?)
}

struct StreamCleanup {
    gateway: Arc<Gateway>,
    id: Uuid,
    upload: tokio::task::JoinHandle<()>,
}
impl Drop for StreamCleanup {
    fn drop(&mut self) {
        self.upload.abort();
        let gateway = self.gateway.clone();
        let id = self.id;
        tokio::spawn(async move {
            if gateway.pending.lock().await.remove(&id).is_some()
                && let Some(sender) = gateway.sender.read().await.as_ref()
                && let Ok(frame) = (Frame {
                    kind: Kind::GatewayCancel,
                    end: true,
                    id,
                    sequence: 0,
                    bytes: &[],
                })
                .encode()
            {
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    sender.send(NodeWsMessage::Binary(frame)),
                )
                .await;
            }
        });
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        if let Ok(task) = self.server_task.get_mut()
            && let Some(task) = task.take()
        {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{files::Roots, jobs::Exec, process::Identity};
    use super::*;
    use nyxid_machine::text::Redactor;

    async fn live_job() -> (
        tempfile::TempDir,
        Arc<Jobs>,
        Arc<Gateway>,
        BTreeMap<String, String>,
        String,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let jobs = Arc::new(Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        ));
        let gateway = Gateway::start(
            Arc::downgrade(&jobs),
            Uuid::new_v4().to_string(),
            Uuid::new_v4().to_string(),
            Zeroizing::new(vec![7; 32]),
        )
        .await
        .unwrap();
        let id = Uuid::new_v4().to_string();
        let env = gateway
            .environment(
                &id,
                "conversation",
                &serde_json::from_value(json!({
                    "variables": {
                        "OPENAI_BASE_URL": {"kind":"gateway_path", "path":"/s/llm-openai/v1"},
                        "OPENAI_API_KEY": {"kind":"gateway_token"}
                    },
                    "git": [{"origin":"https://github.com", "path":"/git/github.com/"}]
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        jobs.start(
            Exec {
                scope: Default::default(),
                job_id: id.clone(),
                command: "sleep 30".into(),
                cwd: None,
                env: BTreeMap::new(),
                stdin: None,
                timeout_secs: Some(30),
                background: true,
            },
            &Identity::resolve(None).unwrap(),
            &Roots::new(&[temp.path().into()], &[]).unwrap(),
            &env,
        )
        .await
        .unwrap();
        (temp, jobs, gateway, env, id)
    }

    #[tokio::test]
    async fn gateway_tokens_are_job_bound_and_completion_survives_reconnect_until_ack() {
        let (_temp, jobs, gateway, env, id) = live_job().await;
        assert_eq!(env["OPENAI_API_KEY"], env["NYXID_GATEWAY_TOKEN"]);
        assert!(env["OPENAI_BASE_URL"].starts_with(&gateway.url));
        assert_eq!(env["GIT_CONFIG_COUNT"], "3");
        let client = reqwest::Client::new();
        assert_eq!(
            client
                .get(format!("{}/s/llm-openai/models", gateway.url))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        let other = gateway
            .environment("different-job", "other-conversation", &Default::default())
            .await
            .unwrap();
        assert_ne!(env["NYXID_GATEWAY_TOKEN"], other["NYXID_GATEWAY_TOKEN"]);
        assert!(!other.keys().any(|key| key.starts_with("OPENAI_")
            || key.starts_with("ANTHROPIC_")
            || key.starts_with("GIT_")));
        assert_eq!(
            client
                .get(format!("{}/s/llm-openai/models", gateway.url))
                .bearer_auth(&other["NYXID_GATEWAY_TOKEN"])
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        gateway.remove_job("different-job").await;
        let (tx, mut rx) = mpsc::channel(16);
        gateway.connect(tx).await;
        jobs.cancel(&id).await.unwrap();
        jobs.result(&id, 5, 0, 0).await.unwrap();
        gateway.report_finished().await;
        let Some(NodeWsMessage::Text(first)) = rx.recv().await else {
            panic!("completion message");
        };
        let first: nyxid_machine::Request = serde_json::from_str(&first).unwrap();
        assert_eq!(first.operation, Operation::JobFinished);
        assert_eq!(first.parameters["runtime_id"], gateway.runtime_id);
        assert_eq!(
            gateway.tokens.lock().await.len(),
            1,
            "enqueue is not a durable acknowledgement"
        );
        gateway.disconnect().await;
        let (tx, mut rx) = mpsc::channel(16);
        gateway.connect(tx).await;
        gateway.report_finished().await;
        let Some(NodeWsMessage::Text(second)) = rx.recv().await else {
            panic!("retried completion");
        };
        let second: nyxid_machine::Request = serde_json::from_str(&second).unwrap();
        assert_ne!(first.nonce, second.nonce);
        for binding in gateway.tokens.lock().await.values_mut() {
            binding.report.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(2);
        }
        gateway.report_finished().await;
        let Some(NodeWsMessage::Text(retry)) = rx.recv().await else {
            panic!("same-connection retry");
        };
        let retry: nyxid_machine::Request = serde_json::from_str(&retry).unwrap();
        assert_eq!(second.request_id, retry.request_id);
        assert_ne!(second.nonce, retry.nonce);
        gateway.finished_ack(&first.request_id).await;
        assert_eq!(
            gateway.tokens.lock().await.len(),
            1,
            "stale acknowledgements cannot release another report"
        );
        gateway.finished_ack(&second.request_id).await;
        assert!(gateway.tokens.lock().await.is_empty());
        assert_eq!(
            client
                .get(format!("{}/s/llm-openai/models", gateway.url))
                .bearer_auth(&env["NYXID_GATEWAY_TOKEN"])
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }

    #[tokio::test]
    async fn gateway_streams_lazily_and_client_disconnect_cancels_remote_work() {
        let (_temp, jobs, gateway, env, id) = live_job().await;
        let (tx, mut rx) = mpsc::channel(16);
        gateway.connect(tx).await;
        let url = format!("{}/s/llm-openai/files", gateway.url);
        let token = env["NYXID_GATEWAY_TOKEN"].clone();
        let client = tokio::spawn(async move {
            reqwest::Client::new()
                .get(url)
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
        });
        let Some(NodeWsMessage::Text(open)) = rx.recv().await else {
            panic!("service opening");
        };
        assert!(!open.contains(&env["NYXID_GATEWAY_TOKEN"]));
        let open: nyxid_machine::Request = serde_json::from_str(&open).unwrap();
        nyxid_machine::signing::ReplayGuard::default()
            .verify(
                &open,
                &gateway.node_id,
                &[7; 32],
                chrono::Utc::now().timestamp(),
            )
            .unwrap();
        assert_eq!(open.parameters["job_id"], id);
        let uuid = Uuid::parse_str(&open.request_id).unwrap();
        gateway
            .response(
                &open.request_id,
                json!({"status":200,"headers":[["content-type","application/octet-stream"]]}),
            )
            .await;
        gateway
            .chunk(Frame {
                kind: Kind::GatewayDownload,
                end: false,
                id: uuid,
                sequence: 0,
                bytes: b"first",
            })
            .await;
        let mut response = client.await.unwrap();
        assert_eq!(
            response.chunk().await.unwrap().unwrap(),
            "first",
            "first bytes arrive without waiting for the end"
        );
        drop(response);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(NodeWsMessage::Binary(bytes)) = rx.recv().await
                    && Frame::decode(&bytes).unwrap().kind == Kind::GatewayCancel
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(!gateway.pending.lock().await.contains_key(&uuid));
        jobs.cancel_all().await;
        gateway.disconnect().await;
    }
}
