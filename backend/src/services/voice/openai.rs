//! GPT-Live's own wire protocol, deliberately separate from Realtime.
use crate::errors::{
    AppError, AppResult,
    voice_start::{Provider, ProviderFailure, Stage},
};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
use zeroize::Zeroizing;

pub const MAX_FRAME: usize = 256 * 1024;
pub type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub struct OpenAi {
    pub(super) delegation_model: Option<String>,
    client: reqwest::Client,
    key: Zeroizing<String>,
    http_origin: String,
    ws_origin: String,
}
pub struct Created {
    pub provider_id: String,
    pub sdp: String,
    pub expires_at: Option<i64>,
}

pub struct CreateError {
    pub provider_id: Option<String>,
    pub not_created: bool,
    pub error: AppError,
}
impl std::fmt::Debug for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CreateError { [REDACTED] }")
    }
}
impl From<AppError> for CreateError {
    fn from(error: AppError) -> Self {
        Self {
            provider_id: None,
            not_created: false,
            error: Stage::ProviderAnswer.error(error),
        }
    }
}

impl OpenAi {
    pub fn new(key: Zeroizing<String>) -> AppResult<Self> {
        // Pool only transport, never credentials or per-call state. Calls still
        // resolve their own authorized credential and set Bearer on each request.
        static CLIENT: std::sync::OnceLock<Result<reqwest::Client, reqwest::Error>> =
            std::sync::OnceLock::new();
        let client = CLIENT
            .get_or_init(|| {
                reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(std::time::Duration::from_secs(15))
                    .build()
            })
            .as_ref()
            .cloned()
            .map_err(|_| AppError::VoiceProviderUnavailable)?;
        Ok(Self {
            delegation_model: None,
            client,
            key,
            http_origin: "https://api.openai.com".into(),
            ws_origin: "wss://api.openai.com".into(),
        })
    }

    pub(super) fn with_delegation_model(mut self, model: Option<String>) -> Self {
        self.delegation_model = model;
        self
    }

    /// Reconcile a known native inference after a sideband error omitted its
    /// terminal event. Fixed provider origin, own credential, bounded read only.
    pub(super) async fn response(&self, id: &str) -> AppResult<Value> {
        if !valid_provider_id(id) {
            return Err(AppError::VoiceProviderUnavailable);
        }
        let mut url = url::Url::parse(&format!("{}/v1/responses/", self.http_origin))
            .map_err(|_| AppError::VoiceProviderUnavailable)?;
        url.path_segments_mut()
            .map_err(|_| AppError::VoiceProviderUnavailable)?
            .pop_if_empty()
            .push(id);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut response = self
                .client
                .get(url)
                .bearer_auth(self.key.as_str())
                .send()
                .await
                .map_err(|_| AppError::VoiceProviderUnavailable)?;
            if !response.status().is_success() {
                return Err(AppError::VoiceProviderUnavailable);
            }
            let bytes = bounded_body(&mut response, MAX_FRAME).await?;
            serde_json::from_slice(&bytes).map_err(|_| AppError::VoiceProviderUnavailable)
        })
        .await
        .map_err(|_| AppError::VoiceProviderUnavailable)?
    }

    pub async fn create(
        &self,
        sdp: &str,
        instructions: &str,
        voice: &str,
        model: &str,
        delegation_instructions: Option<&str>,
    ) -> Result<Created, CreateError> {
        let mut response = self
            .client
            .post(format!("{}/v1/live/sessions", self.http_origin))
            .bearer_auth(self.key.as_str())
            .json(&start_payload(
                sdp,
                instructions,
                voice,
                model,
                self.delegation_model.as_deref(),
                delegation_instructions,
            ))
            .send()
            .await
            .map_err(|_| Stage::Transport.error(AppError::VoiceProviderUnavailable))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = bounded_body(&mut response, 16 * 1024)
                .await
                .unwrap_or_default();
            return Err(CreateError {
                provider_id: None,
                not_created: matches!(status, 400 | 401 | 403 | 404 | 422 | 429),
                error: ProviderFailure::new(Provider::Openai, Some(status), &body)
                    .error(Stage::ProviderCreate),
            });
        }
        let bytes = bounded_body(&mut response, MAX_FRAME).await?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| AppError::VoiceProviderUnavailable)?;
        let provider_id = value
            .pointer("/session/id")
            .and_then(Value::as_str)
            .filter(|s| valid_provider_id(s))
            .ok_or(AppError::VoiceProviderUnavailable)?
            .to_owned();
        let sdp = value
            .pointer("/transport/sdp")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("v=0") && s.len() <= 64 * 1024)
            .ok_or_else(|| CreateError {
                provider_id: Some(provider_id.clone()),
                not_created: false,
                error: Stage::ProviderAnswer.error(AppError::VoiceProviderUnavailable),
            })?
            .to_owned();
        // The official create response contains session.id, not an expiry.
        // Accept legacy expiry metadata when supplied; NyxID always has its own deadline.
        let expires_at = value.pointer("/session/expires_at").and_then(Value::as_i64);
        Ok(Created {
            provider_id,
            sdp,
            expires_at,
        })
    }

    pub(super) fn attach_url(&self, id: &str) -> AppResult<url::Url> {
        if !valid_provider_id(id) {
            return Err(AppError::VoiceProviderUnavailable);
        }
        let mut url =
            url::Url::parse(&self.ws_origin).map_err(|_| AppError::VoiceProviderUnavailable)?;
        url.path_segments_mut()
            .map_err(|_| AppError::VoiceProviderUnavailable)?
            .extend(["v1", "live", "sessions", id, "attach"]);
        Ok(url)
    }

    pub async fn attach(&self, id: &str) -> AppResult<Socket> {
        if !valid_provider_id(id) {
            return Err(AppError::VoiceProviderUnavailable);
        }
        let mut request = self
            .attach_url(id)?
            .as_str()
            .into_client_request()
            .map_err(|_| AppError::VoiceProviderUnavailable)?;
        let mut auth = format!("Bearer {}", self.key.as_str())
            .parse::<http::HeaderValue>()
            .map_err(|_| AppError::VoiceProviderUnavailable)?;
        auth.set_sensitive(true);
        request
            .headers_mut()
            .insert(http::header::AUTHORIZATION, auth);
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio_tungstenite::connect_async_with_config(request, Some(config), true),
        )
        .await
        .map_err(|_| AppError::VoiceProviderUnavailable)?
        .map(|(socket, _)| socket)
        .map_err(|error| {
            super::diagnostics::socket_error(error, Provider::Openai, Stage::Transport)
        })
    }
}

async fn bounded_body(response: &mut reqwest::Response, limit: usize) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::VoiceProviderUnavailable)?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(AppError::VoiceProviderUnavailable);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub fn valid_provider_id(id: &str) -> bool {
    // Opaque provider IDs are encoded as one URL path segment, never interpolated.
    // Keep a local resource bound and reject URL-normalized dot segments.
    !id.is_empty()
        && id.len() <= 1024
        && !matches!(id, "." | "..")
        && !id.chars().any(char::is_control)
}

pub fn start_payload(
    sdp: &str,
    instructions: &str,
    voice: &str,
    model: &str,
    delegation_model: Option<&str>,
    delegation_instructions: Option<&str>,
) -> Value {
    json!({"session":{"model":model,"store":false,"instructions":instructions,
        "delegation":delegation_model.map(|model| super::responses::configuration(model, delegation_instructions)).unwrap_or_else(|| json!({"type":"client"})),"audio":{"output":{"voice":voice}},
        "client":{"data_channel":{"allowed_client_events":[],"allowed_server_events":[
            {"type":"session.started"},{"type":"session.closed"},{"type":"error"}
        ]}}},"transport":{"type":"webrtc","sdp":sdp}})
}

pub async fn send(socket: &mut Socket, event: Value) -> AppResult<()> {
    let text = serde_json::to_string(&event).map_err(|_| AppError::VoiceProviderUnavailable)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| AppError::VoiceProviderUnavailable)?
    .map_err(|_| AppError::VoiceProviderUnavailable)
}

pub async fn receive(socket: &mut Socket) -> AppResult<Option<Value>> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                let mut event: Value =
                    serde_json::from_str(&text).map_err(|_| AppError::VoiceProviderUnavailable)?;
                if matches!(
                    event["type"].as_str(),
                    Some("session.usage.updated" | "session.closed")
                ) {
                    #[derive(serde::Deserialize)]
                    struct RawUsage {
                        seconds: Box<serde_json::value::RawValue>,
                    }
                    #[derive(serde::Deserialize)]
                    struct RawEvent {
                        usage: RawUsage,
                    }
                    let raw: RawEvent = serde_json::from_str(&text)
                        .map_err(|_| AppError::VoiceProviderUnavailable)?;
                    event["_nyx_usage_seconds"] = Value::String(raw.usage.seconds.get().into());
                }
                return Ok(Some(event));
            }
            Some(Ok(Message::Close(_))) | None => return Ok(None),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            _ => return Err(AppError::VoiceProviderUnavailable),
        }
    }
}

#[cfg(test)]
impl OpenAi {
    pub(super) fn fixture(key: Zeroizing<String>, address: std::net::SocketAddr) -> Self {
        let mut adapter = Self::new(key).unwrap();
        // Ordinary fixtures can reuse a port after its previous server drops.
        adapter.client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        adapter.http_origin = format!("http://{address}");
        adapter.ws_origin = format!("ws://{address}");
        adapter
    }
}

/// Floor a nonnegative provider JSON decimal without floating-point rounding.
/// The original spelling is retained independently for reconciliation.
pub(super) fn completed_seconds(raw: &str) -> AppResult<i64> {
    let invalid = || AppError::VoiceProviderUnavailable;
    if raw.is_empty() || raw.len() > 64 || raw.starts_with('-') {
        return Err(invalid());
    }
    let (mantissa, exponent) = raw
        .split_once(['e', 'E'])
        .map_or(Ok((raw, 0)), |(m, e)| e.parse::<i32>().map(|e| (m, e)))
        .map_err(|_| invalid())?;
    if !(-64..=64).contains(&exponent) {
        return Err(invalid());
    }
    let decimal = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let digits: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    if digits.is_empty() || digits.iter().any(|b| !b.is_ascii_digit()) {
        return Err(invalid());
    }
    let point = decimal + exponent;
    let mut whole = 0_i64;
    for index in 0..point.max(0) as usize {
        whole = whole
            .checked_mul(10)
            .and_then(|n| {
                n.checked_add(i64::from(digits.get(index).copied().unwrap_or(b'0') - b'0'))
            })
            .filter(|n| *n <= 86_400)
            .ok_or_else(invalid)?;
    }
    if whole == 86_400
        && digits
            .iter()
            .skip(point.max(0) as usize)
            .any(|d| *d != b'0')
    {
        return Err(invalid());
    }
    Ok(whole)
}

#[cfg(test)]
mod pooling_tests {
    use super::*;

    #[tokio::test]
    async fn transport_reuse_keeps_each_calls_credential() {
        use axum::{Json, Router, extract::ConnectInfo, http::HeaderMap, routing::post};
        use std::{
            net::SocketAddr,
            sync::{Arc, Mutex},
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let app = Router::new().route("/v1/live/sessions", post(move |ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap| {
            let captured = captured.clone();
            async move {
                captured.lock().unwrap().push((peer, headers["authorization"].to_str().unwrap().to_owned()));
                Json(json!({"session":{"id":"live_pool"},"transport":{"sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"}}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        for key in ["fixture-first", "fixture-second"] {
            let mut provider = OpenAi::new(Zeroizing::new(key.into())).unwrap();
            provider.http_origin = format!("http://{address}");
            provider
                .create(
                    "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
                    "fixture",
                    "marin",
                    "fixture-model",
                    None,
                )
                .await
                .unwrap();
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[0].0, seen[1].0,
            "The second call must reuse the TCP connection"
        );
        assert_eq!(seen[0].1, "Bearer fixture-first");
        assert_eq!(seen[1].1, "Bearer fixture-second");
        server.abort();
    }
}
