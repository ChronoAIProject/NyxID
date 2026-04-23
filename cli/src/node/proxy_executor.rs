use base64::Engine;
use futures::StreamExt;
use reqwest::Client;
use tokio::sync::mpsc;

use super::credential_store::CredentialStore;
use super::error::Result;
use super::metrics::NodeMetrics;
use super::signing::{self, ReplayGuard};
use super::ws_client::NodeWsMessage;

/// Maximum chunk size for streaming responses (64 KB raw bytes).
const MAX_CHUNK_SIZE: usize = 64 * 1024;

/// Content types that should be streamed regardless of size.
const STREAMING_CONTENT_TYPES: &[&str] = &[
    "text/event-stream",
    "video/",
    "audio/",
    "application/octet-stream",
    "image/",
    "application/pdf",
];

/// Threshold above which responses are streamed rather than buffered (256 KB).
const STREAM_SIZE_THRESHOLD: u64 = 256 * 1024;

/// Execute a proxy request and return the WS response message(s).
///
/// For non-streaming responses, returns a single `proxy_response` JSON string.
/// For streaming responses, sends `proxy_response_start`, one or more chunk
/// messages, and `proxy_response_end` through the provided channel. The chunk
/// encoding is negotiated during the node `auth_ok` handshake.
#[allow(clippy::too_many_arguments)]
pub async fn execute_proxy_request(
    request: &serde_json::Value,
    credentials: &CredentialStore,
    signing_secret: Option<&str>,
    replay_guard: &tokio::sync::Mutex<ReplayGuard>,
    metrics: &NodeMetrics,
    tx: &mpsc::Sender<NodeWsMessage>,
    use_binary_proxy_chunks: bool,
    http_client: &Client,
) {
    let request_id = request["request_id"].as_str().unwrap_or("");
    let service_slug = request["service_slug"].as_str().unwrap_or("");

    // 1. Verify HMAC signature if signing is enabled
    if let Some(secret) = signing_secret {
        let timestamp = request["timestamp"].as_str();
        let nonce = request["nonce"].as_str();
        let signature = request["signature"].as_str();

        if timestamp.is_some() || nonce.is_some() || signature.is_some() {
            let Some(signature) = signature else {
                metrics.record_error();
                let _ = send_ws_message(
                    tx,
                    proxy_error_response(request_id, "Missing HMAC signature", 403, false),
                )
                .await;
                return;
            };

            if !signing::verify_request_signature(request, secret, signature) {
                metrics.record_error();
                let _ = send_ws_message(
                    tx,
                    proxy_error_response(
                        request_id,
                        "HMAC signature verification failed",
                        403,
                        false,
                    ),
                )
                .await;
                return;
            }

            // Replay protection
            let timestamp = timestamp.unwrap_or("");
            let nonce = nonce.unwrap_or("");
            let mut guard = replay_guard.lock().await;
            if !guard.check(timestamp, nonce) {
                metrics.record_error();
                let _ = send_ws_message(
                    tx,
                    proxy_error_response(
                        request_id,
                        "Request rejected: replay or expired timestamp",
                        403,
                        false,
                    ),
                )
                .await;
                return;
            }
        }
    }

    // 2. Look up credentials for this service
    let cred = match credentials.get(service_slug) {
        Some(c) => c,
        None => {
            metrics.record_error();
            let _ = send_ws_message(
                tx,
                proxy_error_response_with_reason(
                    request_id,
                    &format!("No credentials configured for service '{service_slug}'"),
                    502,
                    true,
                    Some("credential_missing"),
                ),
            )
            .await;
            return;
        }
    };

    // 3. Build the downstream HTTP request
    let method_str = request["method"].as_str().unwrap_or("GET");
    let path = request["path"].as_str().unwrap_or("/");
    let query = request["query"].as_str();
    let base_url = request["base_url"].as_str().unwrap_or("");

    // If NyxID sent an empty base_url, resolve from local credential config
    let effective_base_url = if base_url.is_empty() {
        match cred.target_url() {
            Some(url) => url,
            None => {
                metrics.record_error();
                let _ = send_ws_message(
                    tx,
                    proxy_error_response(
                        request_id,
                        &format!(
                            "No target URL configured for service '{service_slug}'. \
                             Run: nyxid node credentials add --service {service_slug} --url <URL> ..."
                        ),
                        502,
                        false,
                    ),
                )
                .await;
                return;
            }
        }
    } else {
        base_url
    };

    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    // Path-prefix injection: prepend /{prefix}{credential} to the URL path
    let final_path = if let Some((prefix, credential)) = cred.path_prefix() {
        format!("/{prefix}{credential}{normalized_path}")
    } else {
        normalized_path
    };

    let mut url = format!("{}{}", effective_base_url.trim_end_matches('/'), final_path);
    if let Some(q) = query
        && !q.is_empty()
    {
        url = format!("{url}?{q}");
    }

    // Handle query_param injection by appending to URL
    if let Some((param_name, param_value)) = cred.query_param() {
        url = append_query_param(&url, param_name, param_value);
    }

    let method = reqwest::Method::from_bytes(method_str.as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut req_builder = http_client.request(method, &url);

    // 4. Forward headers from the proxy_request
    if let Some(headers) = request["headers"].as_object() {
        for (name, value) in headers {
            if let Some(v) = value.as_str() {
                req_builder = req_builder.header(name.as_str(), v);
            }
        }
    }

    // 5. Inject header credentials
    if let Some((hdr_name, hdr_value)) = cred.header() {
        req_builder = req_builder.header(hdr_name, hdr_value);
    }

    // 6. Attach body
    if let Some(body_b64) = request["body"].as_str()
        && let Ok(body_bytes) = base64::engine::general_purpose::STANDARD.decode(body_b64)
    {
        req_builder = req_builder.body(body_bytes);
    }

    // 7. Execute request
    match req_builder.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let is_streaming = should_stream_response(&response, status);

            if is_streaming {
                stream_proxy_response(
                    request_id,
                    status,
                    response,
                    tx,
                    use_binary_proxy_chunks,
                    metrics,
                )
                .await;
            } else {
                let headers = extract_response_headers(&response);
                match response.bytes().await {
                    Ok(body) => {
                        let body_b64 = base64::engine::general_purpose::STANDARD.encode(&body);
                        metrics.record_success();
                        let _ = send_ws_message(
                            tx,
                            serde_json::json!({
                                "type": "proxy_response",
                                "request_id": request_id,
                                "status": status,
                                "headers": headers,
                                "body": body_b64,
                            })
                            .to_string(),
                        )
                        .await;
                    }
                    Err(e) => {
                        metrics.record_error();
                        let _ = send_ws_message(
                            tx,
                            proxy_error_response(
                                request_id,
                                &format!("Failed to read response body: {e}"),
                                502,
                                false,
                            ),
                        )
                        .await;
                    }
                }
            }
        }
        Err(e) => {
            metrics.record_error();
            let _ = send_ws_message(
                tx,
                proxy_error_response(
                    request_id,
                    &format!("Downstream request failed: {e}"),
                    502,
                    false,
                ),
            )
            .await;
        }
    }
}

pub fn build_http_client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .build()?)
}

/// Stream a proxy response back through the WebSocket channel.
///
/// Control messages (start, end, error) are sent as JSON text frames.
/// When the server advertises `proxy_binary_chunks=true`, data chunks are sent
/// as binary frames with the format:
///   `[36 bytes: request_id as ASCII UUID][remaining: raw data]`
/// Otherwise the agent falls back to legacy JSON `proxy_response_chunk`
/// messages with base64-encoded payloads.
async fn stream_proxy_response(
    request_id: &str,
    status: u16,
    response: reqwest::Response,
    tx: &mpsc::Sender<NodeWsMessage>,
    use_binary_proxy_chunks: bool,
    metrics: &NodeMetrics,
) {
    let headers = extract_response_headers(&response);

    // Send start (text frame -- JSON control message)
    let start_msg = serde_json::json!({
        "type": "proxy_response_start",
        "request_id": request_id,
        "status": status,
        "headers": headers,
    });
    if !send_ws_message(tx, start_msg.to_string()).await {
        metrics.record_error();
        return;
    }

    // Pre-compute the request_id prefix for binary frames (36 bytes ASCII UUID)
    let id_bytes = request_id.as_bytes();

    // Stream chunks using the negotiated encoding.
    let mut stream = response.bytes_stream();
    let mut had_error = false;

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                for sub_chunk in bytes.chunks(MAX_CHUNK_SIZE) {
                    if use_binary_proxy_chunks {
                        // Binary frame: [36-byte request_id][raw data]
                        let mut frame = Vec::with_capacity(id_bytes.len() + sub_chunk.len());
                        frame.extend_from_slice(id_bytes);
                        frame.extend_from_slice(sub_chunk);
                        if tx.send(NodeWsMessage::Binary(frame)).await.is_err() {
                            had_error = true;
                            break;
                        }
                    } else {
                        let chunk_msg = serde_json::json!({
                            "type": "proxy_response_chunk",
                            "request_id": request_id,
                            "data": base64::engine::general_purpose::STANDARD.encode(sub_chunk),
                        });
                        if !send_ws_message(tx, chunk_msg.to_string()).await {
                            had_error = true;
                            break;
                        }
                    }
                }
                if had_error {
                    break;
                }
            }
            Err(e) => {
                let err_msg = serde_json::json!({
                    "type": "proxy_error",
                    "request_id": request_id,
                    "error": format!("Stream error: {e}"),
                    "status": 502,
                });
                let _ = send_ws_message(tx, err_msg.to_string()).await;
                metrics.record_error();
                return;
            }
        }
    }

    if had_error {
        metrics.record_error();
        return;
    }

    // Send end (text frame -- JSON control message)
    let end_msg = serde_json::json!({
        "type": "proxy_response_end",
        "request_id": request_id,
    });
    let _ = send_ws_message(tx, end_msg.to_string()).await;
    metrics.record_success();
}

async fn send_ws_message(tx: &mpsc::Sender<NodeWsMessage>, message: String) -> bool {
    tx.send(NodeWsMessage::Text(message)).await.is_ok()
}

/// Decide whether a downstream response should be streamed rather than buffered.
fn should_stream_response(response: &reqwest::Response, status: u16) -> bool {
    if status == reqwest::StatusCode::PARTIAL_CONTENT.as_u16() {
        return true;
    }

    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let ct_lower = ct.to_lowercase();
    if STREAMING_CONTENT_TYPES
        .iter()
        .any(|prefix| ct_lower.starts_with(prefix))
    {
        return true;
    }

    // Stream when content-length is absent (unknown size) or exceeds the threshold
    match response.content_length() {
        None => true,
        Some(len) => len > STREAM_SIZE_THRESHOLD,
    }
}

/// Extract response headers as a JSON object.
fn extract_response_headers(response: &reqwest::Response) -> serde_json::Value {
    let mut headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        if let Ok(v) = value.to_str() {
            headers.insert(
                name.as_str().to_string(),
                serde_json::Value::String(v.to_string()),
            );
        }
    }
    serde_json::Value::Object(headers)
}

fn proxy_error_response(request_id: &str, error: &str, status: u16, retryable: bool) -> String {
    proxy_error_response_with_reason(request_id, error, status, retryable, None)
}

/// Like `proxy_error_response` but also emits a machine-readable
/// `reason` tag so the backend can distinguish specific classes of
/// failure (e.g. a locally-missing credential) from generic node/proxy
/// errors without string-matching. Older backends that don't know
/// about the tag simply ignore the extra field.
fn proxy_error_response_with_reason(
    request_id: &str,
    error: &str,
    status: u16,
    retryable: bool,
    reason: Option<&str>,
) -> String {
    let mut payload = serde_json::json!({
        "type": "proxy_error",
        "request_id": request_id,
        "error": error,
        "status": status,
        "retryable": retryable,
    });
    if let Some(reason) = reason
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(
            "reason".to_string(),
            serde_json::Value::String(reason.to_string()),
        );
    }
    payload.to_string()
}

pub fn append_query_param(url: &str, param_name: &str, param_value: &str) -> String {
    let separator = if url.contains('?') { "&" } else { "?" };
    let encoded_name = urlencoding::encode(param_name);
    let encoded_value = urlencoding::encode(param_value);
    format!("{url}{separator}{encoded_name}={encoded_value}")
}

#[cfg(test)]
mod tests {
    use super::append_query_param;

    #[test]
    fn append_query_param_url_encodes_name_and_value() {
        let url = append_query_param("https://example.com/api", "api key", "a=b&c d#fragment");

        assert_eq!(
            url,
            "https://example.com/api?api%20key=a%3Db%26c%20d%23fragment"
        );
    }

    #[test]
    fn append_query_param_preserves_existing_query_string() {
        let url = append_query_param("https://example.com/api?x=1", "token", "abc");
        assert_eq!(url, "https://example.com/api?x=1&token=abc");
    }
}
