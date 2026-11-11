//! Small Streamable HTTP client. Only explicit session rejection permits replay.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::api::{ApiClient, ApiError};

const VERSION: &str = "2025-06-18";
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct Client {
    api: ApiClient,
    session: Option<String>,
    version: String,
    next_id: u64,
    initialized: bool,
    discovery: bool,
    call_dispatched: bool,
}

enum Reply {
    Message(Value),
    SessionExpired,
}

impl Client {
    pub(super) fn new(api: ApiClient) -> Self {
        Self {
            api,
            session: None,
            version: VERSION.into(),
            next_id: 0,
            initialized: false,
            discovery: false,
            call_dispatched: false,
        }
    }

    pub(super) fn for_discovery(api: ApiClient) -> Self {
        Self {
            discovery: true,
            ..Self::new(api)
        }
    }

    pub(super) fn call_dispatched(&self) -> bool {
        self.call_dispatched
    }

    async fn send(&mut self, method: &str, params: Value, notification: bool) -> Result<Reply> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .context("MCP request ID exhausted")?;
        let id = self.next_id;
        let mut body = json!({"jsonrpc":"2.0","method":method,"params":params});
        if !notification {
            body["id"] = json!(id);
        }
        let read = method != "tools/call"
            || (self.discovery && body["params"]["name"] == "nyx__search_tools");
        if method == "tools/call" {
            self.call_dispatched = true;
        }
        let response = self
            .api
            .mcp_request(self.session.as_deref(), &self.version, &body, read)
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND && self.session.is_some() {
            return Ok(Reply::SessionExpired);
        }
        if !status.is_success() {
            return Err(ApiError::new(
                "POST /mcp",
                status,
                response.text().await.unwrap_or_default(),
            )
            .into());
        }
        if method == "initialize" {
            self.session = response
                .headers()
                .get("Mcp-Session-Id")
                .map(|v| v.to_str().map(str::to_owned))
                .transpose()
                .context("Invalid MCP session header")?;
            if self.session.as_ref().is_some_and(|s| {
                s.is_empty() || s.len() > 1024 || !s.bytes().all(|b| (0x21..=0x7e).contains(&b))
            }) {
                bail!("Invalid MCP session header");
            }
        }
        if notification {
            return Ok(Reply::Message(Value::Null));
        }
        let value = message(response, id).await?;
        if value["error"]["code"] == -32002 && self.session.is_some() {
            return Ok(Reply::SessionExpired);
        }
        if let Some(error) = value.get("error") {
            bail!(
                "MCP JSON-RPC error {}: {}",
                error["code"],
                error["message"].as_str().unwrap_or("Request rejected")
            );
        }
        Ok(Reply::Message(
            value
                .get("result")
                .context("MCP response has no result")?
                .clone(),
        ))
    }

    async fn initialize(&mut self) -> Result<bool> {
        self.session = None;
        self.version = VERSION.into();
        let params = json!({
            "protocolVersion": VERSION,
            "capabilities": {},
            "clientInfo": {"name": "nyxid-cli", "version": env!("CARGO_PKG_VERSION")}
        });
        let reply = self.send("initialize", params, false).await?;
        let Reply::Message(result) = reply else {
            return Ok(false);
        };
        let version = result["protocolVersion"]
            .as_str()
            .context("MCP initialize did not negotiate a protocol version")?;
        if !["2025-03-26", "2025-06-18", "2025-11-25"].contains(&version)
            || !result["capabilities"]["tools"].is_object()
        {
            bail!("MCP server does not offer a supported tools protocol");
        }
        self.version = version.into();
        self.initialized = matches!(
            self.send("notifications/initialized", json!({}), true)
                .await?,
            Reply::Message(_)
        );
        Ok(self.initialized)
    }

    pub(super) async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        for attempt in 0..2 {
            if !self.initialized && !self.initialize().await? {
                if attempt == 0 {
                    continue;
                }
                bail!("MCP session was rejected after reinitialization");
            }
            match self.send(method, params.clone(), false).await? {
                Reply::Message(result) => return Ok(result),
                Reply::SessionExpired => {
                    self.session = None;
                    self.initialized = false;
                    if attempt == 1 {
                        bail!("MCP session was rejected after reinitialization");
                    }
                }
            }
        }
        bail!("MCP session was rejected after reinitialization")
    }
}

async fn message(mut response: reqwest::Response, id: u64) -> Result<Value> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let sse = content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim() == "text/event-stream");
    if !sse && !content_type.starts_with("application/json") {
        bail!("MCP response must be JSON or an SSE stream");
    }
    let mut buffer = vec![];
    let mut total = 0usize;
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read MCP response; inspect execution state before retrying a call")?
    {
        total = total
            .checked_add(chunk.len())
            .context("MCP response size overflow")?;
        if total > MAX_RESPONSE_BYTES {
            bail!("MCP response exceeds 16 MiB");
        }
        buffer.extend_from_slice(&chunk);
        if sse {
            while let Some((end, delimiter)) = frame_end(&buffer) {
                let frame = std::str::from_utf8(&buffer[..end]).context("Invalid MCP SSE text")?;
                let data = frame
                    .lines()
                    .filter_map(|line| {
                        line.strip_prefix("data:")
                            .map(|v| v.strip_prefix(' ').unwrap_or(v))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !data.is_empty() {
                    let value: Value =
                        serde_json::from_str(&data).context("Invalid MCP SSE JSON")?;
                    if matching(&value, id)? {
                        return Ok(value);
                    }
                }
                buffer.drain(..end + delimiter);
            }
        }
    }
    if sse {
        bail!(
            "MCP SSE stream ended without the requested result; inspect execution state before retrying a call"
        );
    }
    let value: Value = serde_json::from_slice(&buffer).context("Invalid MCP JSON response")?;
    if !matching(&value, id)? {
        bail!("MCP response ID did not match the request");
    }
    Ok(value)
}

fn matching(value: &Value, id: u64) -> Result<bool> {
    if value["jsonrpc"] != "2.0" {
        bail!("Invalid MCP JSON-RPC version");
    }
    if value.get("id").is_none() && value["method"].is_string() {
        return Ok(false);
    }
    if value["id"] != id {
        bail!("MCP response ID did not match the request");
    }
    if value.get("error").is_none() && value.get("result").is_none() {
        bail!("MCP response has neither result nor error");
    }
    Ok(true)
}

fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    (0..buffer.len()).find_map(|index| {
        if buffer[index..].starts_with(b"\r\n\r\n") {
            Some((index, 4))
        } else if buffer[index..].starts_with(b"\n\n") {
            Some((index, 2))
        } else {
            None
        }
    })
}
