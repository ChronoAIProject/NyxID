//! Credential-bound bridge to IFTTT's OAuth MCP server.

use std::{sync::LazyLock, time::Duration};

use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

pub const AUTH_METHOD: &str = "ifttt_mcp";
pub const BASE_URL: &str = "https://ifttt.com/mcp";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IFTTT OAuth requires the fixed https://ifttt.com/mcp destination")]
    Destination,
    #[error(
        "Use GET /tools with an optional cursor, or POST /tools/TOOL_NAME with a JSON object of arguments"
    )]
    Request,
    #[error("IFTTT returned an invalid or oversized MCP response")]
    Protocol,
    #[error("IFTTT rejected the MCP request (code {0})")]
    Rpc(i64),
    #[error("IFTTT request outcome is unknown; check IFTTT before retrying")]
    Transport(#[source] reqwest::Error),
}

pub fn validate_destination(base_url: &str) -> Result<(), Error> {
    if base_url != BASE_URL {
        return Err(Error::Destination);
    }
    Ok(())
}

pub fn validate_request(
    base_url: &str,
    method: &Method,
    path: &str,
    query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<(), Error> {
    operation(base_url, method, path, query, body).map(|_| ())
}

fn operation(
    base_url: &str,
    method: &Method,
    path: &str,
    query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<(&'static str, Value), Error> {
    validate_destination(base_url)?;
    let path = path.strip_prefix('/').unwrap_or(path);
    if method == Method::GET && path == "tools" && body.is_none_or(|b| b.is_empty()) {
        let mut params = json!({});
        if let Some(query) = query.filter(|q| !q.is_empty()) {
            if query.len() > 8192 {
                return Err(Error::Request);
            }
            let url =
                reqwest::Url::parse(&format!("{BASE_URL}?{query}")).map_err(|_| Error::Request)?;
            let pairs: Vec<_> = url.query_pairs().collect();
            if pairs.len() != 1 || pairs[0].0 != "cursor" || pairs[0].1.is_empty() {
                return Err(Error::Request);
            }
            params["cursor"] = Value::String(pairs[0].1.to_string());
        }
        return Ok(("tools/list", params));
    }
    if method == Method::POST && query.is_none_or(str::is_empty) {
        let name = path.strip_prefix("tools/").ok_or(Error::Request)?;
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return Err(Error::Request);
        }
        let args: Value =
            serde_json::from_slice(body.unwrap_or(b"{}")).map_err(|_| Error::Request)?;
        if !args.is_object() {
            return Err(Error::Request);
        }
        return Ok(("tools/call", json!({"name": name, "arguments": args})));
    }
    Err(Error::Request)
}

pub struct Client(reqwest::Client);

impl Client {
    pub fn new(builder: reqwest::ClientBuilder) -> Result<Self, reqwest::Error> {
        builder
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map(Self)
    }

    fn request(
        &self,
        token: &str,
        session: Option<&str>,
        version: &str,
        body: Value,
    ) -> reqwest::RequestBuilder {
        let mut request = self
            .0
            .post(BASE_URL)
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", version)
            .body(body.to_string());
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        request
    }

    pub async fn forward(
        &self,
        base_url: &str,
        method: &Method,
        path: &str,
        query: Option<&str>,
        token: &str,
        body: Option<&[u8]>,
    ) -> Result<Response, Error> {
        let (rpc_method, params) = operation(base_url, method, path, query, body)?;
        let response = self
            .request(
                token,
                None,
                PROTOCOL_VERSION,
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                        "clientInfo": {"name": "NyxID", "version": env!("CARGO_PKG_VERSION")}}
                }),
            )
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Ok(rejection(response.status()));
        }
        let session = response
            .headers()
            .get("Mcp-Session-Id")
            .map(|v| v.to_str().map(str::to_owned).map_err(|_| Error::Protocol))
            .transpose()?;
        if session.as_ref().is_some_and(|s| {
            s.is_empty() || s.len() > 1024 || !s.bytes().all(|b| (0x21..=0x7e).contains(&b))
        }) {
            return Err(Error::Protocol);
        }
        let mut version = PROTOCOL_VERSION.to_string();
        let result = async {
            let initialized = rpc_result(response, 1).await?;
            let negotiated = initialized["protocolVersion"]
                .as_str()
                .ok_or(Error::Protocol)?;
            if !matches!(negotiated, "2025-03-26" | "2025-06-18" | "2025-11-25")
                || !initialized["capabilities"]["tools"].is_object()
            {
                return Err(Error::Protocol);
            }
            version = negotiated.to_owned();
            let notification = self
                .request(
                    token,
                    session.as_deref(),
                    &version,
                    json!({
                        "jsonrpc": "2.0", "method": "notifications/initialized"
                    }),
                )
                .send()
                .await
                .map_err(transport_error)?;
            if !notification.status().is_success() {
                return Ok(rejection(notification.status()));
            }
            let response = self
                .request(
                    token,
                    session.as_deref(),
                    &version,
                    json!({
                        "jsonrpc": "2.0", "id": 2, "method": rpc_method, "params": params
                    }),
                )
                .send()
                .await
                .map_err(transport_error)?;
            if response.status().is_success() {
                rpc_result(response, 2).await.map(|result| {
                    let status = if result["isError"] == true {
                        StatusCode::UNPROCESSABLE_ENTITY
                    } else {
                        StatusCode::OK
                    };
                    json_response(status, result)
                })
            } else {
                Ok(rejection(response.status()))
            }
        }
        .await;
        // Each invocation owns its session; no credentials or session IDs are cached.
        if let Some(session) = session {
            let _ = self
                .0
                .delete(BASE_URL)
                .bearer_auth(token)
                .header("Mcp-Session-Id", session)
                .header("MCP-Protocol-Version", version)
                .timeout(Duration::from_secs(2))
                .send()
                .await;
        }
        result
    }
}

fn transport_error(error: reqwest::Error) -> Error {
    Error::Transport(error.without_url())
}

fn json_response(status: StatusCode, result: Value) -> Response {
    http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(result.to_string())
        .expect("IFTTT MCP response")
        .into()
}

fn rejection(upstream_status: StatusCode) -> Response {
    let status = if upstream_status.is_redirection() {
        StatusCode::BAD_GATEWAY
    } else {
        upstream_status
    };
    json_response(
        status,
        json!({"error": "IFTTT MCP request rejected", "upstream_status": upstream_status.as_u16()}),
    )
}

fn message_result(value: Value, id: i64) -> Result<Option<Value>, Error> {
    if value.get("id") != Some(&json!(id)) {
        return Ok(None);
    }
    if value["jsonrpc"] != "2.0" {
        return Err(Error::Protocol);
    }
    if let Some(error) = value.get("error") {
        return Err(Error::Rpc(error["code"].as_i64().ok_or(Error::Protocol)?));
    }
    Ok(Some(
        value
            .get("result")
            .filter(|r| r.is_object())
            .ok_or(Error::Protocol)?
            .clone(),
    ))
}

async fn rpc_result(mut response: Response, id: i64) -> Result<Value, Error> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let sse = content_type
        .split(';')
        .next()
        .is_some_and(|v| v.trim() == "text/event-stream");
    if !sse && !content_type.starts_with("application/json") {
        return Err(Error::Protocol);
    }
    let mut buffer = Vec::new();
    let mut total = 0;
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        total += chunk.len();
        if total > MAX_RESPONSE_BYTES {
            return Err(Error::Protocol);
        }
        buffer.extend_from_slice(&chunk);
        if sse {
            while let Some((end, delimiter)) = frame_end(&buffer) {
                let frame = std::str::from_utf8(&buffer[..end]).map_err(|_| Error::Protocol)?;
                let data = frame
                    .lines()
                    .filter_map(|line| {
                        line.strip_prefix("data:")
                            .map(|v| v.strip_prefix(' ').unwrap_or(v))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !data.is_empty() {
                    let value = serde_json::from_str(&data).map_err(|_| Error::Protocol)?;
                    if let Some(result) = message_result(value, id)? {
                        return Ok(result);
                    }
                }
                buffer.drain(..end + delimiter);
            }
        }
    }
    if sse {
        return Err(Error::Protocol);
    }
    message_result(
        serde_json::from_slice(&buffer).map_err(|_| Error::Protocol)?,
        id,
    )?
    .ok_or(Error::Protocol)
}

fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    (0..buffer.len()).find_map(|i| {
        if buffer[i..].starts_with(b"\r\n\r\n") {
            Some((i, 4))
        } else if buffer[i..].starts_with(b"\n\n") {
            Some((i, 2))
        } else {
            None
        }
    })
}

pub fn client() -> &'static Client {
    static CLIENT: LazyLock<Client> =
        LazyLock::new(|| Client::new(reqwest::Client::builder()).expect("IFTTT MCP client"));
    &CLIENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::scripted_fixture;

    fn response(status: &str, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn initialized() -> String {
        response("200 OK", "Content-Type: application/json\r\nMcp-Session-Id: test-session\r\n", &json!({
            "jsonrpc":"2.0","id":1,"result":{"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{}}}
        }).to_string())
    }

    fn accepted() -> String {
        response("202 Accepted", "", "")
    }

    fn request_json(request: &[u8]) -> Value {
        let end = request.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
        serde_json::from_slice(&request[end + 4..]).unwrap()
    }

    #[tokio::test]
    async fn discovery_negotiates_session_and_preserves_schema_and_cursor() {
        let result = json!({"tools":[{"name":"create_applet","inputSchema":{"type":"object","properties":{"name":{"type":"string"}}}}],"nextCursor":"page-2"});
        let mut server = scripted_fixture(
            "ifttt.com",
            vec![
                initialized(),
                accepted(),
                response(
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    &json!({"jsonrpc":"2.0","id":2,"result":result}).to_string(),
                ),
                accepted(),
            ],
        )
        .await;
        let client = Client::new(server.builder.take().unwrap()).unwrap();
        let actual = client
            .forward(
                BASE_URL,
                &Method::GET,
                "/tools",
                Some("cursor=page%201"),
                "test-oauth-token",
                None,
            )
            .await
            .unwrap();
        assert_eq!(actual.status(), StatusCode::OK);
        assert!(actual.headers().get("Mcp-Session-Id").is_none());
        assert_eq!(
            serde_json::from_str::<Value>(&actual.text().await.unwrap()).unwrap(),
            result
        );
        let init = server.requests.recv().await.unwrap();
        assert!(String::from_utf8_lossy(&init).starts_with("POST /mcp "));
        assert!(String::from_utf8_lossy(&init).contains("authorization: Bearer test-oauth-token"));
        assert_eq!(request_json(&init)["method"], "initialize");
        let notification = server.requests.recv().await.unwrap();
        assert_eq!(
            request_json(&notification)["method"],
            "notifications/initialized"
        );
        assert!(String::from_utf8_lossy(&notification).contains("mcp-session-id: test-session"));
        let list = request_json(&server.requests.recv().await.unwrap());
        assert_eq!(list["method"], "tools/list");
        assert_eq!(list["params"]["cursor"], "page 1");
        assert!(
            String::from_utf8_lossy(&server.requests.recv().await.unwrap())
                .starts_with("DELETE /mcp ")
        );
    }

    #[tokio::test]
    async fn call_reads_sse_and_surfaces_tool_failure_without_retrying() {
        for newline in ["\n", "\r\n"] {
            let result = json!({"isError":true,"content":[{"type":"text","text":"Account connection required"}]});
            let data = format!(
                ": keepalive{newline}{newline}data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}}{newline}{newline}data: {}{newline}{newline}",
                json!({"jsonrpc":"2.0","id":2,"result":result})
            );
            let mut server = scripted_fixture(
                "ifttt.com",
                vec![
                    initialized(),
                    accepted(),
                    response("200 OK", "Content-Type: text/event-stream\r\n", &data),
                    accepted(),
                ],
            )
            .await;
            let client = Client::new(server.builder.take().unwrap()).unwrap();
            let args = br#"{"trigger":{"event":"desk_light_on"},"actions":[{"device":"desk"}]}"#;
            let actual = client
                .forward(
                    BASE_URL,
                    &Method::POST,
                    "tools/create_applet",
                    None,
                    "test-token",
                    Some(args),
                )
                .await
                .unwrap();
            assert_eq!(actual.status(), StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(
                serde_json::from_str::<Value>(&actual.text().await.unwrap()).unwrap(),
                result
            );
            server.requests.recv().await.unwrap();
            server.requests.recv().await.unwrap();
            let call = request_json(&server.requests.recv().await.unwrap());
            assert_eq!(call["method"], "tools/call");
            assert_eq!(call["params"]["name"], "create_applet");
            assert_eq!(
                call["params"]["arguments"],
                serde_json::from_slice::<Value>(args).unwrap()
            );
            server.requests.recv().await.unwrap();
            assert!(server.requests.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn rejects_unsafe_routes_before_any_dispatch() {
        let mut server = scripted_fixture("ifttt.com", vec![]).await;
        let client = Client::new(server.builder.take().unwrap()).unwrap();
        for (url, method, path, query, body) in [
            ("https://example.com/mcp", Method::GET, "tools", None, None),
            (BASE_URL, Method::GET, "tools/create_applet", None, None),
            (BASE_URL, Method::POST, "mcp", None, Some(b"{}".as_slice())),
            (
                BASE_URL,
                Method::POST,
                "tools/../create",
                None,
                Some(b"{}".as_slice()),
            ),
            (
                BASE_URL,
                Method::POST,
                "tools/create_applet",
                Some("key=secret"),
                Some(b"{}".as_slice()),
            ),
            (
                BASE_URL,
                Method::POST,
                "tools/create_applet",
                None,
                Some(b"[]".as_slice()),
            ),
            (
                BASE_URL,
                Method::GET,
                "tools",
                Some("cursor=a&cursor=b"),
                None,
            ),
            (BASE_URL, Method::GET, "tools", Some("unknown=value"), None),
        ] {
            assert!(
                client
                    .forward(url, &method, path, query, "token", body)
                    .await
                    .is_err()
            );
        }
        assert!(server.requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn refuses_redirects_and_does_not_expose_provider_headers_or_error_body() {
        for status in ["302 Found", "401 Unauthorized", "429 Too Many Requests"] {
            let mut server = scripted_fixture(
                "ifttt.com",
                vec![response(
                    status,
                    "Location: https://elsewhere.invalid/token\r\n",
                    "secret upstream response",
                )],
            )
            .await;
            let client = Client::new(server.builder.take().unwrap()).unwrap();
            let result = client
                .forward(BASE_URL, &Method::GET, "tools", None, "token", None)
                .await
                .unwrap();
            assert_eq!(
                result.status().as_u16(),
                if status.starts_with("302") {
                    502
                } else {
                    status[..3].parse().unwrap()
                }
            );
            assert!(result.headers().get("location").is_none());
            assert!(!result.text().await.unwrap().contains("secret"));
            server.requests.recv().await.unwrap();
            assert!(server.requests.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn bounds_responses_and_rejects_mismatched_ids() {
        for body in [
            "x".repeat(MAX_RESPONSE_BYTES + 1),
            json!({"jsonrpc":"2.0","id":99,"result":{}}).to_string(),
        ] {
            let mut server = scripted_fixture(
                "ifttt.com",
                vec![response(
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    &body,
                )],
            )
            .await;
            let client = Client::new(server.builder.take().unwrap()).unwrap();
            assert!(matches!(
                client
                    .forward(BASE_URL, &Method::GET, "tools", None, "token", None)
                    .await,
                Err(Error::Protocol)
            ));
        }
    }

    #[tokio::test]
    async fn never_repeats_a_tool_call_after_connection_loss() {
        let mut server =
            scripted_fixture("ifttt.com", vec![initialized(), accepted(), String::new()]).await;
        let client = Client::new(server.builder.take().unwrap()).unwrap();
        assert!(matches!(
            client
                .forward(
                    BASE_URL,
                    &Method::POST,
                    "tools/create_applet",
                    None,
                    "token",
                    Some(b"{}")
                )
                .await,
            Err(Error::Transport(_))
        ));
        for _ in 0..3 {
            server.requests.recv().await.unwrap();
        }
        assert!(
            String::from_utf8_lossy(&server.requests.recv().await.unwrap())
                .starts_with("DELETE /mcp ")
        );
        assert!(server.requests.try_recv().is_err());
    }
}
