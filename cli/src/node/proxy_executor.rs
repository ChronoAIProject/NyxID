use std::sync::OnceLock;

use base64::Engine;
use futures::StreamExt;
use nyxid_cloud_auth::aws_sigv4::{self, AwsCredentials};
use nyxid_cloud_auth::gcp_oauth::{DEFAULT_GCP_SCOPES, GcpTokenCache};
use reqwest::Client;
use tokio::sync::mpsc;

use super::credential_store::CredentialStore;
use super::error::Result;
use super::metrics::NodeMetrics;
use super::signing::{self, ReplayGuard};
use super::ws_client::NodeWsMessage;

/// Process-wide GCP access-token cache for the `gcp_service_account`
/// auth method. Mirrors `AppState.gcp_token_cache` on the backend so a
/// node serving multiple proxy requests for the same SA reuses the
/// minted token until natural expiry. Initialized on first use.
fn gcp_token_cache() -> &'static GcpTokenCache {
    static CACHE: OnceLock<GcpTokenCache> = OnceLock::new();
    CACHE.get_or_init(GcpTokenCache::new)
}

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
    let mut req_builder = http_client.request(method.clone(), &url);

    // 4. Collect forwarded headers. We accumulate them in `forwarded_headers`
    //    as well as applying them to the builder so SigV4 can sign over the
    //    exact set that will be sent.
    //
    //    For aws_sigv4 we strip caller-supplied managed headers
    //    (Authorization, X-Amz-Date, X-Amz-Content-Sha256, X-Amz-Security-Token)
    //    before attaching — the signer step below adds canonical values
    //    and reqwest's `.header()` appends rather than replaces, so
    //    keeping caller values would produce duplicate headers on the
    //    wire (Codex review BLOCKER 8).
    let is_aws_sigv4 = cred.aws_sigv4_credential().is_some();
    let mut forwarded_headers: Vec<(String, String)> = Vec::new();
    if let Some(headers) = request["headers"].as_object() {
        for (name, value) in headers {
            if let Some(v) = value.as_str() {
                if is_aws_sigv4 {
                    let lower = name.to_ascii_lowercase();
                    if matches!(
                        lower.as_str(),
                        "authorization"
                            | "x-amz-date"
                            | "x-amz-content-sha256"
                            | "x-amz-security-token"
                    ) {
                        continue;
                    }
                }
                req_builder = req_builder.header(name.as_str(), v);
                forwarded_headers.push((name.clone(), v.to_string()));
            }
        }
    }

    // 5. Inject header credentials (legacy header/bearer path).
    if let Some((hdr_name, hdr_value)) = cred.header() {
        req_builder = req_builder.header(hdr_name, hdr_value);
    }

    // 6. Decode the body up front so the SigV4 path can hash it before
    //    it's attached. Skipping decode errors keeps behavior identical
    //    to the pre-#716 code for non-SigV4 paths.
    let body_bytes: Option<Vec<u8>> = request["body"]
        .as_str()
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok());

    // 6a. AWS SigV4: compute the signature over the final URL + forwarded
    //     headers + body, then append the SigV4 headers (Authorization,
    //     X-Amz-Date, X-Amz-Content-Sha256, optional X-Amz-Security-Token).
    if let Some(creds_json) = cred.aws_sigv4_credential() {
        let creds = match AwsCredentials::from_json(creds_json) {
            Ok(c) => c,
            Err(e) => {
                metrics.record_error();
                let _ = send_ws_message(
                    tx,
                    proxy_error_response(
                        request_id,
                        &format!("aws_sigv4 credential is malformed: {e}"),
                        500,
                        false,
                    ),
                )
                .await;
                return;
            }
        };
        let body_for_sig: &[u8] = body_bytes.as_deref().unwrap_or(&[]);
        let signed = match aws_sigv4::sign_request(
            method.as_str(),
            &url,
            &forwarded_headers,
            body_for_sig,
            &creds,
        ) {
            Ok(s) => s,
            Err(e) => {
                metrics.record_error();
                let _ = send_ws_message(
                    tx,
                    proxy_error_response(
                        request_id,
                        &format!("aws_sigv4 signing failed: {e}"),
                        500,
                        false,
                    ),
                )
                .await;
                return;
            }
        };
        for header in signed {
            req_builder = req_builder.header(&header.name, &header.value);
        }
    }

    // 6b. GCP service-account: mint + cache an access token from the SA
    //     JSON and inject as a Bearer token.
    if let Some(sa_json) = cred.gcp_service_account_credential() {
        let token = match gcp_token_cache()
            .access_token(http_client, sa_json, DEFAULT_GCP_SCOPES)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                metrics.record_error();
                let _ = send_ws_message(
                    tx,
                    proxy_error_response(
                        request_id,
                        &format!("gcp_service_account token mint failed: {e}"),
                        502,
                        false,
                    ),
                )
                .await;
                return;
            }
        };
        req_builder = req_builder.bearer_auth(token.as_ref());
    }

    // 6c. Attach the body now that any signing pass that needed to read
    //     it has run.
    if let Some(bytes) = body_bytes {
        req_builder = req_builder.body(bytes);
    }

    // 7. Execute request
    let gcp_credential_for_invalidation: Option<String> =
        cred.gcp_service_account_credential().map(|s| s.to_string());
    match req_builder.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            // GCP-specific: drop the cached access token on 401/403 so
            // the next request re-mints. Codex review REC 8.
            if let Some(creds_json) = gcp_credential_for_invalidation.as_deref()
                && matches!(status, 401 | 403)
            {
                gcp_token_cache().invalidate(creds_json, DEFAULT_GCP_SCOPES);
                tracing::warn!(
                    service_slug = %service_slug,
                    status,
                    "gcp_service_account: upstream rejected token, invalidated cached access token"
                );
            }
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
