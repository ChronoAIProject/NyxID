use axum::{
    body::Body,
    extract::{Path, State},
    http::{Method, Request, StatusCode},
    response::Response,
    Json,
};
use futures::StreamExt;
use mongodb::bson::doc;
use tokio_stream::wrappers::ReceiverStream;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, chatgpt_translator, delegation_service, llm_gateway_service,
    proxy_service,
};
use crate::AppState;

/// Maximum size for upstream response bodies (50 MB).
const MAX_RESPONSE_BODY_SIZE: usize = 50 * 1024 * 1024;

/// Response headers that are safe to forward back to the client.
const ALLOWED_RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-disposition",
    "cache-control",
    "etag",
    "last-modified",
    "x-request-id",
    "x-correlation-id",
    "vary",
    "access-control-allow-origin",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-expose-headers",
];

/// GET /api/v1/llm/status
///
/// Return which LLM providers the user can use and their proxy URLs.
pub async fn llm_status(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<llm_gateway_service::LlmStatusResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let status = llm_gateway_service::get_llm_status(
        &state.db,
        &user_id_str,
        &state.config.base_url,
    )
    .await?;

    Ok(Json(status))
}

/// ANY /api/v1/llm/{provider_slug}/v1/{*path}
///
/// Forward the request to the provider's API using the user's stored credential.
/// This is a passthrough proxy -- no request/response translation.
pub async fn llm_proxy_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((provider_slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let user_id_str = auth_user.user_id.to_string();

    // Resolve the downstream service for this provider slug
    let (service, _provider) =
        llm_gateway_service::resolve_llm_service_by_slug(&state.db, &provider_slug)
            .await?;

    let service_id = service.id.clone();

    // Use existing proxy_service to resolve the proxy target
    let target = proxy_service::resolve_proxy_target(
        &state.db,
        &encryption_key,
        &user_id_str,
        &service_id,
    )
    .await?;

    // Resolve delegated credentials (provider tokens)
    let delegated = delegation_service::resolve_delegated_credentials(
        &state.db,
        &encryption_key,
        &user_id_str,
        &service_id,
    )
    .await
    .map_err(|e| {
        AppError::BadRequest(format!(
            "Provider credentials not available: {e}. Please connect the provider first."
        ))
    })?;

    let method = request.method().clone();
    let query = request.uri().query().map(String::from);
    let headers = request.headers().clone();

    // Read the request body (10MB limit)
    let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read request body: {e}")))?;

    // OpenAI Codex: use WebSocket transport with Responses API translation
    // (Cloudflare blocks regular HTTP to chatgpt.com, but allows WebSocket)
    let response = if provider_slug == "openai-codex" && !body_bytes.is_empty() {
        let body_json: serde_json::Value =
            serde_json::from_slice(&body_bytes).map_err(|e| {
                AppError::BadRequest(format!("Invalid JSON body: {e}"))
            })?;

        let translator = llm_gateway_service::get_translator(&provider_slug);
        let translated = translator.translate_request(&path, &body_json)?;

        let bearer_token = extract_bearer_token(&delegated)?;
        let is_streaming = body_json
            .get("stream")
            .and_then(|s| s.as_bool())
            .unwrap_or(false);

        chatgpt_translator::send_to_chatgpt(
            &translated.body,
            &bearer_token,
            is_streaming,
        )
        .await?
    } else {
        let body = if body_bytes.is_empty() {
            None
        } else {
            Some(body_bytes)
        };

        let reqwest_method = convert_method(&method)?;
        let reqwest_headers = convert_headers(&headers);

        let downstream_response = proxy_service::forward_request(
            &state.http_client,
            &target,
            reqwest_method,
            &path,
            query.as_deref(),
            reqwest_headers,
            body,
            vec![], // no identity headers for LLM proxy
            delegated,
        )
        .await?;

        build_filtered_response(downstream_response).await?
    };

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "llm_proxy_request".to_string(),
        Some(serde_json::json!({
            "provider_slug": &provider_slug,
            "method": method.as_str(),
            "path": &path,
        })),
        None,
        None,
    );

    Ok(response)
}

/// ANY /api/v1/llm/gateway/v1/{*path}
///
/// OpenAI-compatible gateway. Accepts OpenAI-format requests, routes to the
/// correct provider based on the `model` field, translates request/response
/// formats as needed.
pub async fn gateway_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(path): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let user_id_str = auth_user.user_id.to_string();

    let method = request.method().clone();
    let query = request.uri().query().map(String::from);
    let headers = request.headers().clone();

    // Read the full request body to extract the model field
    let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read request body: {e}")))?;

    // Parse body as JSON to extract model
    let body_json: serde_json::Value = if body_bytes.is_empty() {
        return Err(AppError::ValidationError(
            "Request body is required with a 'model' field".to_string(),
        ));
    } else {
        serde_json::from_slice(&body_bytes).map_err(|e| {
            AppError::BadRequest(format!("Invalid JSON body: {e}"))
        })?
    };

    let is_streaming = body_json
        .get("stream")
        .and_then(|v| v.as_bool())
        == Some(true);

    let model = body_json
        .get("model")
        .and_then(|m| m.as_str())
        .ok_or_else(|| {
            AppError::ValidationError("'model' field is required in request body".to_string())
        })?;

    // Resolve provider slug from model name
    let primary_slug = llm_gateway_service::resolve_provider_for_model(model)
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "Unknown model: '{model}'. Cannot determine provider."
            ))
        })?;

    // Try to find the user's active token for the resolved provider.
    // For OpenAI models, fall back to openai-codex if openai is not connected.
    let provider_slug = resolve_provider_slug_with_fallback(
        &state.db,
        &user_id_str,
        primary_slug,
    )
    .await?;

    // Resolve the downstream service
    let (service, _provider) =
        llm_gateway_service::resolve_llm_service_by_slug(&state.db, &provider_slug)
            .await?;

    let service_id = service.id.clone();

    // Get the translator
    let translator = llm_gateway_service::get_translator(&provider_slug);

    // Resolve proxy target
    let target = proxy_service::resolve_proxy_target(
        &state.db,
        &encryption_key,
        &user_id_str,
        &service_id,
    )
    .await?;

    // Resolve delegated credentials
    let delegated = delegation_service::resolve_delegated_credentials(
        &state.db,
        &encryption_key,
        &user_id_str,
        &service_id,
    )
    .await
    .map_err(|e| {
        AppError::BadRequest(format!(
            "Provider '{}' not connected. Connect at /providers. ({})",
            provider_slug, e
        ))
    })?;

    // Apply translation if needed
    let (final_path, final_body_bytes, extra_headers) = if translator.needs_translation() {
        let translated = translator.translate_request(&path, &body_json)?;

        let translated_bytes = serde_json::to_vec(&translated.body).map_err(|e| {
            AppError::Internal(format!("Failed to serialize translated request: {e}"))
        })?;

        (
            translated.path,
            Some(bytes::Bytes::from(translated_bytes)),
            translated.extra_headers,
        )
    } else {
        // M-2: body_bytes guaranteed non-empty (validated above), use directly
        (path.clone(), Some(body_bytes), vec![])
    };

    // L-4: Override base URL immutably via shadow binding
    let target = match translator.gateway_base_url() {
        // M-5: Google AI uses OpenAI-compatible format but at a different base URL.
        // No body translation needed, but the base URL must be overridden.
        Some(base) => proxy_service::ProxyTarget {
            base_url: base.to_string(),
            auth_method: target.auth_method,
            auth_key_name: target.auth_key_name,
            credential: target.credential,
            service: target.service,
        },
        None => target,
    };

    let reqwest_method = convert_method(&method)?;
    let mut reqwest_headers = convert_headers(&headers);

    // Remove forwarded headers that the translator wants to override, so
    // the translator's version takes precedence (reqwest appends, not replaces).
    for (key, _) in &extra_headers {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
            reqwest_headers.remove(&name);
        }
    }

    // L-4: Extend delegated credentials immutably via iterator chaining
    let delegated: Vec<_> = delegated
        .into_iter()
        .chain(extra_headers.iter().map(|(key, value)| {
            delegation_service::DelegatedCredential {
                provider_slug: provider_slug.clone(),
                injection_method: "header".to_string(),
                injection_key: key.clone(),
                credential: value.clone(),
            }
        }))
        .collect();

    // OpenAI Codex: use WebSocket transport (Cloudflare blocks HTTP to chatgpt.com)
    let response = if provider_slug == "openai-codex" {
        let bearer_token = extract_bearer_token(&delegated)?;
        // final_body_bytes is already the translated Responses API body
        let translated_body: serde_json::Value = serde_json::from_slice(
            final_body_bytes.as_deref().unwrap_or(&[]),
        )
        .map_err(|e| {
            AppError::Internal(format!("Failed to parse translated body: {e}"))
        })?;

        chatgpt_translator::send_to_chatgpt(
            &translated_body,
            &bearer_token,
            is_streaming,
        )
        .await?
    } else {
        let downstream_response = proxy_service::forward_request(
            &state.http_client,
            &target,
            reqwest_method,
            &final_path,
            query.as_deref(),
            reqwest_headers,
            final_body_bytes,
            vec![],
            delegated,
        )
        .await?;

        // If translator needs translation, parse and translate the response
        if translator.needs_translation() {
            if is_streaming {
                // Streaming: translate SSE events on the fly
                build_translated_sse_response(downstream_response, translator).await?
            } else {
                // Non-streaming: buffer and translate the full response
                build_translated_json_response(downstream_response, translator.as_ref())
                    .await?
            }
        } else {
            build_filtered_response(downstream_response).await?
        }
    };

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "llm_gateway_request".to_string(),
        Some(serde_json::json!({
            "model": model,
            "provider_slug": &provider_slug,
            "method": method.as_str(),
            "path": &path,
        })),
        None,
        None,
    );

    Ok(response)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try primary slug, then fall back to openai-codex for OpenAI models.
async fn resolve_provider_slug_with_fallback(
    db: &mongodb::Database,
    // M-1: removed unused _encryption_key parameter
    user_id: &str,
    primary_slug: &str,
) -> AppResult<String> {
    use crate::models::provider_config::{ProviderConfig, COLLECTION_NAME as PROVIDER_CONFIGS};
    use crate::models::user_provider_token::{
        UserProviderToken, COLLECTION_NAME as USER_PROVIDER_TOKENS,
    };

    // Find the primary provider
    let primary_provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "slug": primary_slug, "is_active": true })
        .await?;

    if let Some(ref provider) = primary_provider {
        // Check if user has an active token
        let token = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! {
                "user_id": user_id,
                "provider_config_id": &provider.id,
                "status": { "$in": ["active", "expired"] },
            })
            .await?;

        if token.is_some() {
            return Ok(primary_slug.to_string());
        }
    }

    // Fall back to openai-codex for OpenAI models
    if primary_slug == "openai" {
        let codex_provider = db
            .collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find_one(doc! { "slug": "openai-codex", "is_active": true })
            .await?;

        if let Some(ref provider) = codex_provider {
            let token = db
                .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
                .find_one(doc! {
                    "user_id": user_id,
                    "provider_config_id": &provider.id,
                    "status": { "$in": ["active", "expired"] },
                })
                .await?;

            if token.is_some() {
                return Ok("openai-codex".to_string());
            }
        }
    }

    // Neither primary nor fallback available
    Err(AppError::BadRequest(format!(
        "Provider '{primary_slug}' not connected. Connect at /providers."
    )))
}

/// Extract the bearer token from delegated credentials.
fn extract_bearer_token(
    delegated: &[delegation_service::DelegatedCredential],
) -> AppResult<String> {
    delegated
        .iter()
        .find(|c| c.injection_method == "bearer")
        .map(|c| c.credential.clone())
        .ok_or_else(|| {
            AppError::BadRequest(
                "No bearer token available for openai-codex. Connect the provider first."
                    .to_string(),
            )
        })
}

fn convert_method(method: &Method) -> AppResult<reqwest::Method> {
    match *method {
        Method::GET => Ok(reqwest::Method::GET),
        Method::POST => Ok(reqwest::Method::POST),
        Method::PUT => Ok(reqwest::Method::PUT),
        Method::DELETE => Ok(reqwest::Method::DELETE),
        Method::PATCH => Ok(reqwest::Method::PATCH),
        Method::HEAD => Ok(reqwest::Method::HEAD),
        Method::OPTIONS => Ok(reqwest::Method::OPTIONS),
        _ => Err(AppError::BadRequest("Unsupported HTTP method".to_string())),
    }
}

fn convert_headers(headers: &axum::http::HeaderMap) -> reqwest::header::HeaderMap {
    let mut reqwest_headers = reqwest::header::HeaderMap::new();
    for (name, value) in headers.iter() {
        if let Ok(reqwest_name) =
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes())
        {
            if let Ok(reqwest_value) =
                reqwest::header::HeaderValue::from_bytes(value.as_bytes())
            {
                reqwest_headers.insert(reqwest_name, reqwest_value);
            }
        }
    }
    reqwest_headers
}

/// Read a reqwest response body with a size limit.
async fn read_response_with_limit(
    response: reqwest::Response,
) -> AppResult<bytes::Bytes> {
    let resp_bytes = response.bytes().await.map_err(|e| {
        AppError::Internal(format!("Failed to read downstream response: {e}"))
    })?;

    if resp_bytes.len() > MAX_RESPONSE_BODY_SIZE {
        return Err(AppError::Internal(
            "Upstream response too large".to_string(),
        ));
    }

    Ok(resp_bytes)
}

async fn build_filtered_response(
    downstream_response: reqwest::Response,
) -> AppResult<Response> {
    let status = StatusCode::from_u16(downstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);

    let is_sse = downstream_response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    let mut response_builder = Response::builder().status(status);

    for (name, value) in downstream_response.headers().iter() {
        let name_lower = name.as_str().to_lowercase();
        // Skip content-length for SSE -- the body is streamed, length unknown
        if is_sse && name_lower == "content-length" {
            continue;
        }
        if ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str()) {
            if let Ok(header_name) =
                axum::http::header::HeaderName::from_bytes(name.as_str().as_bytes())
            {
                if let Ok(header_value) =
                    axum::http::header::HeaderValue::from_bytes(value.as_bytes())
                {
                    response_builder =
                        response_builder.header(header_name, header_value);
                }
            }
        }
    }

    if is_sse {
        // Stream SSE responses directly without buffering
        let body = Body::from_stream(downstream_response.bytes_stream());
        response_builder
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    } else {
        // H-3: Buffer non-streaming responses with size limit
        let response_body = read_response_with_limit(downstream_response).await?;
        response_builder
            .body(Body::from(response_body))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    }
}

/// Build a non-streaming translated response (buffer, translate, return).
/// Used by `gateway_request` when `needs_translation() && !is_streaming`.
async fn build_translated_json_response(
    downstream_response: reqwest::Response,
    translator: &dyn llm_gateway_service::LlmTranslator,
) -> AppResult<Response> {
    let status = downstream_response.status();
    let resp_headers = downstream_response.headers().clone();
    let resp_bytes = read_response_with_limit(downstream_response).await?;

    if status.is_success() {
        let resp_json: serde_json::Value =
            serde_json::from_slice(&resp_bytes).map_err(|e| {
                AppError::Internal(format!(
                    "Failed to parse provider response as JSON: {e}"
                ))
            })?;

        let translated = translator.translate_response(resp_json)?;
        let translated_bytes = serde_json::to_vec(&translated).map_err(|e| {
            AppError::Internal(format!(
                "Failed to serialize translated response: {e}"
            ))
        })?;

        let axum_status = StatusCode::from_u16(status.as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);

        let mut response_builder = Response::builder()
            .status(axum_status)
            .header("content-type", "application/json");

        for (name, value) in resp_headers.iter() {
            let name_lower = name.as_str().to_lowercase();
            if name_lower != "content-type"
                && name_lower != "content-length"
                && ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str())
            {
                if let Ok(header_name) =
                    axum::http::header::HeaderName::from_bytes(
                        name.as_str().as_bytes(),
                    )
                {
                    if let Ok(header_value) =
                        axum::http::header::HeaderValue::from_bytes(
                            value.as_bytes(),
                        )
                    {
                        response_builder =
                            response_builder.header(header_name, header_value);
                    }
                }
            }
        }

        response_builder
            .body(Body::from(translated_bytes))
            .map_err(|e| {
                AppError::Internal(format!("Failed to build response: {e}"))
            })
    } else {
        // M-6: Translate error responses to OpenAI error format
        let axum_status = StatusCode::from_u16(status.as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);

        let error_message = serde_json::from_slice::<serde_json::Value>(&resp_bytes)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| {
                format!("Upstream provider error (HTTP {})", status.as_u16())
            });

        let error_body = serde_json::json!({
            "error": {
                "message": error_message,
                "type": "gateway_error",
                "code": status.as_u16(),
            }
        });

        let error_bytes = serde_json::to_vec(&error_body).map_err(|e| {
            AppError::Internal(format!("Failed to serialize error response: {e}"))
        })?;

        Response::builder()
            .status(axum_status)
            .header("content-type", "application/json")
            .body(Body::from(error_bytes))
            .map_err(|e| {
                AppError::Internal(format!("Failed to build response: {e}"))
            })
    }
}

/// Build a streaming SSE response with on-the-fly event translation.
/// Parses provider SSE events, translates each to OpenAI chunk format, and
/// re-emits as SSE text without buffering the full response.
async fn build_translated_sse_response(
    downstream_response: reqwest::Response,
    translator: Box<dyn llm_gateway_service::LlmTranslator>,
) -> AppResult<Response> {
    let status = downstream_response.status();

    // If the upstream returned an error, buffer and return as translated JSON error
    if !status.is_success() {
        return build_translated_json_response(downstream_response, translator.as_ref()).await;
    }

    let axum_status =
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);

    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);

    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut state = llm_gateway_service::StreamTranslationState::default();
        let mut stream = downstream_response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    while let Some(event) = parse_next_sse_event(&mut buffer) {
                        if let Some(translated) =
                            translator.translate_stream_event(&event, &mut state)
                        {
                            if tx
                                .send(Ok(bytes::Bytes::from(translated)))
                                .await
                                .is_err()
                            {
                                return; // client disconnected
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx
                        .send(Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            e,
                        )))
                        .await;
                    return;
                }
            }
        }
    });

    let body = Body::from_stream(ReceiverStream::new(rx));

    Response::builder()
        .status(axum_status)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build SSE response: {e}")))
}

/// Parse the next complete SSE event from a buffer.
/// Returns `None` if no complete event is available yet.
/// Consumes the parsed event text (including the `\n\n` delimiter) from the buffer.
fn parse_next_sse_event(
    buffer: &mut String,
) -> Option<llm_gateway_service::SseEvent> {
    let end = buffer.find("\n\n")?;
    let event_text = buffer[..end].to_string();
    buffer.drain(..end + 2);

    let mut event_type = None;
    let mut data_parts = Vec::new();

    for line in event_text.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = Some(rest.trim_start().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_parts.push(rest.trim_start().to_string());
        }
        // Ignore id:, retry:, and comment lines (starting with :)
    }

    if data_parts.is_empty() {
        return None;
    }

    Some(llm_gateway_service::SseEvent {
        event_type,
        data: data_parts.join("\n"),
    })
}
