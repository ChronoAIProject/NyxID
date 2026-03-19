use base64::Engine;
use futures::StreamExt;
use reqwest::Client;
use tokio::sync::mpsc;

use crate::credential_store::CredentialStore;
use crate::error::Result;
use crate::metrics::NodeMetrics;
use crate::signing::{self, ReplayGuard};

/// Maximum base64 chunk size for streaming responses (64KB decoded -> ~87KB base64).
const MAX_CHUNK_SIZE: usize = 64 * 1024;

/// Execute a proxy request and return the WS response message(s).
///
/// For non-streaming responses, returns a single `proxy_response` JSON string.
/// For streaming (SSE) responses, sends `proxy_response_start`, `proxy_response_chunk`(s),
/// and `proxy_response_end` through the provided channel.
pub async fn execute_proxy_request(
    request: &serde_json::Value,
    credentials: &CredentialStore,
    signing_secret: Option<&str>,
    replay_guard: &tokio::sync::Mutex<ReplayGuard>,
    metrics: &NodeMetrics,
    tx: &mpsc::Sender<String>,
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
                proxy_error_response(
                    request_id,
                    &format!("No credentials configured for service '{service_slug}'"),
                    502,
                    true,
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

    if base_url.is_empty() {
        metrics.record_error();
        let _ = send_ws_message(
            tx,
            proxy_error_response(
                request_id,
                "Missing downstream base URL in proxy request",
                502,
                false,
            ),
        )
        .await;
        return;
    }

    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let mut url = format!("{}{}", base_url.trim_end_matches('/'), normalized_path);
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
            let is_streaming = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.contains("text/event-stream"));

            if is_streaming {
                stream_proxy_response(request_id, status, response, tx, metrics).await;
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
async fn stream_proxy_response(
    request_id: &str,
    status: u16,
    response: reqwest::Response,
    tx: &mpsc::Sender<String>,
    metrics: &NodeMetrics,
) {
    let headers = extract_response_headers(&response);

    // Send start
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

    // Stream chunks
    let mut stream = response.bytes_stream();
    let mut had_error = false;

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                // Split large chunks to respect the size limit
                for sub_chunk in bytes.chunks(MAX_CHUNK_SIZE) {
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

    // Send end
    let end_msg = serde_json::json!({
        "type": "proxy_response_end",
        "request_id": request_id,
    });
    let _ = send_ws_message(tx, end_msg.to_string()).await;
    metrics.record_success();
}

async fn send_ws_message(tx: &mpsc::Sender<String>, message: String) -> bool {
    tx.send(message).await.is_ok()
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
    serde_json::json!({
        "type": "proxy_error",
        "request_id": request_id,
        "error": error,
        "status": status,
        "retryable": retryable,
    })
    .to_string()
}

fn append_query_param(url: &str, param_name: &str, param_value: &str) -> String {
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
