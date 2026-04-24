use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{Method, Request, StatusCode},
    response::Response,
};
use futures::StreamExt;
use mongodb::bson::doc;
use tokio_stream::wrappers::ReceiverStream;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{
    action_description, approval_service, audit_service, chatgpt_translator, delegation_service,
    llm_gateway_service, llm_usage_service, notification_service, proxy_service, sse_parser,
};

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
    auth_user.ensure_llm_proxy_access()?;

    let user_id_str = auth_user.user_id.to_string();

    let status =
        llm_gateway_service::get_llm_status(&state.db, &user_id_str, &state.config.base_url)
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
    auth_user.ensure_llm_proxy_access()?;

    // Per-agent rate limit check
    crate::mw::rate_limit::check_agent_rate_limit(&state.per_agent_limiter, &auth_user)?;

    let user_id_str = auth_user.user_id.to_string();

    // Resolve the downstream service for this provider slug
    let (service, _provider) =
        llm_gateway_service::resolve_llm_service_by_slug(&state.db, &provider_slug).await?;

    let service_id = service.id.clone();

    // Two-tier credential resolution:
    //   1. Prefer the new UserService / UserApiKey model (created via
    //      `nyxid service add` / POST /api/v1/keys). Its target has the
    //      credential baked into `auth_method` + `credential`, so no legacy
    //      delegation lookup is needed.
    //   2. Fall back to the legacy `DownstreamService` + `UserProviderToken`
    //      path (`resolve_proxy_target` + `resolve_delegated_credentials`)
    //      for users who still have legacy provider tokens.
    // Resolve against the UserService by catalog_service_id only. The URL's
    // `provider_slug` (e.g. "anthropic", "deepseek") is the ProviderConfig
    // slug and has no relation to UserService.slug, which is user-chosen
    // when the service is added via AI Services. Passing it here would make
    // `lookup_user_service` run `find_by_slug(user_id, "anthropic")` against
    // user-picked slugs like "llm-anthropic" or "my-anthropic-prod" --
    // always None -- and then fall through to the legacy delegation path
    // with the "Provider ... connection required" error, even though the
    // user has a perfectly valid UserService linked by catalog_service_id.
    let (target, resolved_via_user_service) =
        match proxy_service::resolve_proxy_target_from_user_service(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &user_id_str,
            None,
            Some(&service_id),
        )
        .await?
        {
            Some(resolution) => (resolution.target, true),
            None => {
                // Before the legacy fallback, block org viewers whose
                // org has any presence for this service so they cannot
                // slip into the LLM gateway approval flow via the
                // legacy path (see ChronoAIProject/NyxID#375).
                proxy_service::guard_slug_against_viewer_orgs(
                    &state.db,
                    &user_id_str,
                    None,
                    Some(&service_id),
                )
                .await?;
                let legacy = proxy_service::resolve_proxy_target(
                    &state.db,
                    &state.encryption_keys,
                    &user_id_str,
                    &service_id,
                )
                .await?;
                (legacy, false)
            }
        };

    // Read request parts before approval check so we can build action descriptions
    let request_method_str = request.method().as_str().to_string();
    let method = request.method().clone();
    let query = request.uri().query().map(String::from);
    let headers = request.headers().clone();

    // Read the request body (10MB limit)
    let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read request body: {e}")))?;

    // Check approval if user has it enabled. The two-tier resolver above
    // doesn't surface its `org_routing` back to this scope, so we look up
    // the effective owner separately via `find_effective_service_owner`,
    // which mirrors the same personal-then-org cascade and returns the
    // identity the proxy would actually pick. Cheap second lookup; the
    // alternative is rewiring the resolver to return the org context.
    let owner_for_approval = proxy_service::find_effective_service_owner(
        &state.db,
        &user_id_str,
        None,
        Some(&service_id),
    )
    .await?;
    check_llm_approval(
        &state,
        &auth_user,
        &service_id,
        &service,
        &path,
        &request_method_str,
        if body_bytes.is_empty() {
            None
        } else {
            Some(&body_bytes)
        },
        owner_for_approval.as_deref(),
    )
    .await?;

    // Resolve credentials for injection. The new UserService path bakes the
    // credential into `target` (via auth_method / credential), so we only need
    // to synthesize a bearer DelegatedCredential for the openai-codex HTTP
    // transport branch. The legacy path still goes through
    // `resolve_delegated_credentials` to fetch `UserProviderToken` records.
    let delegated = if resolved_via_user_service {
        // New path: target already carries the credential. For openai-codex,
        // which reads the token via `extract_bearer_token`, synthesize a
        // bearer DelegatedCredential from the resolved target when possible.
        if provider_slug == "openai-codex" && target.auth_method == "bearer" {
            vec![delegation_service::DelegatedCredential {
                provider_slug: provider_slug.clone(),
                injection_method: "bearer".to_string(),
                injection_key: "Authorization".to_string(),
                credential: target.credential.clone(),
            }]
        } else {
            Vec::new()
        }
    } else {
        delegation_service::resolve_delegated_credentials(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            &service_id,
        )
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "Provider credentials not available: {e}. Please connect the provider first."
            ))
        })?
    };

    // OpenAI Codex: use the specialized HTTP SSE transport with Responses API
    // translation and Codex-specific headers.
    let response = if provider_slug == "openai-codex" && !body_bytes.is_empty() {
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes)
            .map_err(|e| AppError::BadRequest(format!("Invalid JSON body: {e}")))?;
        let usage_context = llm_usage_service::UsageAuditContext {
            db: state.db.clone(),
            user_id: user_id_str.clone(),
            provider_slug: Some(provider_slug.clone()),
            service_id: Some(service_id.clone()),
            model: body_json
                .get("model")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            path: path.clone(),
            api_key_id: auth_user.api_key_id.clone(),
            api_key_name: auth_user.api_key_name.clone(),
        };

        // Path determines response format: chat/completions → Chat Completions,
        // responses → Responses API passthrough
        let is_chat_completions_path = path.contains("chat/completions");

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
            is_chat_completions_path,
            query.as_deref(),
            Some(usage_context),
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
            proxy_service::ProxyBody::Buffered(body),
            vec![], // no identity headers for LLM proxy
            delegated,
            None,
            &state.token_exchange_cache,
        )
        .await?;

        let usage_context = llm_usage_service::UsageAuditContext {
            db: state.db.clone(),
            user_id: user_id_str.clone(),
            provider_slug: Some(provider_slug.clone()),
            service_id: Some(service_id.clone()),
            model: None,
            path: path.clone(),
            api_key_id: auth_user.api_key_id.clone(),
            api_key_name: auth_user.api_key_name.clone(),
        };

        build_filtered_response(
            downstream_response,
            Some(usage_context),
            state.config.proxy_stream_idle_timeout_secs,
        )
        .await?
    };

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "llm_proxy_request".to_string(),
        Some(serde_json::json!({
            "provider_slug": &provider_slug,
            "method": method.as_str(),
            "path": &path,
            "response_status": response.status().as_u16(),
            "api_key_id": &auth_user.api_key_id,
            "api_key_name": &auth_user.api_key_name,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
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
    auth_user.ensure_llm_proxy_access()?;

    // Per-agent rate limit check
    crate::mw::rate_limit::check_agent_rate_limit(&state.per_agent_limiter, &auth_user)?;

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
        serde_json::from_slice(&body_bytes)
            .map_err(|e| AppError::BadRequest(format!("Invalid JSON body: {e}")))?
    };

    let is_streaming = body_json.get("stream").and_then(|v| v.as_bool()) == Some(true);

    let model = body_json
        .get("model")
        .and_then(|m| m.as_str())
        .ok_or_else(|| {
            AppError::ValidationError("'model' field is required in request body".to_string())
        })?;

    // Resolve provider slug from model name
    let primary_slug = llm_gateway_service::resolve_provider_for_model(model).ok_or_else(|| {
        AppError::BadRequest(format!(
            "Unknown model: '{model}'. Cannot determine provider."
        ))
    })?;

    // Try to find the user's active token for the resolved provider.
    // For OpenAI models, fall back to openai-codex if openai is not connected.
    let provider_slug =
        resolve_provider_slug_with_fallback(&state.db, &user_id_str, primary_slug).await?;

    // Resolve the downstream service
    let (service, _provider) =
        llm_gateway_service::resolve_llm_service_by_slug(&state.db, &provider_slug).await?;

    let service_id = service.id.clone();

    // Get the translator
    let translator = llm_gateway_service::get_translator(&provider_slug);

    // Two-tier proxy target resolution (mirrors `llm_proxy_request`):
    //   1. Prefer the new UserService / UserApiKey model, which bakes the
    //      credential into `target` (via auth_method + credential).
    //   2. Fall back to the legacy `DownstreamService` path for users who
    //      still have `UserProviderToken` records.
    //
    // Capture `effective_owner_for_approval` along the way so the approval
    // check below can apply the org-aware cascade -- for org-routed
    // resolutions the owner is the org's user_id, otherwise it falls
    // through to the actor.
    let mut effective_owner_for_approval: Option<String> = None;
    // See `llm_proxy_request` for why we pass `None` as the slug here
    // instead of `provider_slug` -- the URL's provider slug does not
    // match UserService.slug, which is user-chosen at provision time.
    let (target, resolved_via_user_service) =
        match proxy_service::resolve_proxy_target_from_user_service(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &user_id_str,
            None,
            Some(&service_id),
        )
        .await?
        {
            Some(resolution) => {
                effective_owner_for_approval = Some(
                    resolution
                        .org_routing
                        .as_ref()
                        .map(|r| r.org_user_id.clone())
                        .unwrap_or_else(|| user_id_str.clone()),
                );
                // Audit org-routed LLM gateway calls so the org's owner can
                // see who is using shared credentials. Mirrors the pattern
                // in handlers/proxy.rs.
                if let Some(routing) = &resolution.org_routing {
                    audit_service::log_async(
                        state.db.clone(),
                        Some(auth_user.user_id.to_string()),
                        "llm_gateway_routed_via_org".to_string(),
                        Some(serde_json::json!({
                            "routed_via": "org",
                            "service_id": service_id,
                            "user_service_id": resolution.user_service_id,
                            "org_user_id": routing.org_user_id,
                            "member_user_id": routing.member_user_id,
                            "membership_id": routing.membership_id,
                        })),
                        None,
                        None,
                        auth_user.api_key_id.clone(),
                        auth_user.api_key_name.clone(),
                    );
                } else {
                    audit_service::log_async(
                        state.db.clone(),
                        Some(auth_user.user_id.to_string()),
                        "llm_gateway_routed_via_personal".to_string(),
                        Some(serde_json::json!({
                            "routed_via": "personal",
                            "service_id": service_id,
                            "user_service_id": resolution.user_service_id,
                        })),
                        None,
                        None,
                        auth_user.api_key_id.clone(),
                        auth_user.api_key_name.clone(),
                    );
                }
                (resolution.target, true)
            }
            None => {
                let legacy = proxy_service::resolve_proxy_target(
                    &state.db,
                    &state.encryption_keys,
                    &user_id_str,
                    &service_id,
                )
                .await?;
                // Still personal routing — attribute it so unmigrated users
                // are present in the audit trail just like migrated ones.
                // `user_service_id` is null because the legacy path resolves
                // straight from the catalog + provider-token store, with no
                // `UserService` record to point at. See NyxID#423.
                audit_service::log_async(
                    state.db.clone(),
                    Some(auth_user.user_id.to_string()),
                    "llm_gateway_routed_via_personal".to_string(),
                    Some(serde_json::json!({
                        "routed_via": "personal",
                        "service_id": service_id,
                        "user_service_id": serde_json::Value::Null,
                    })),
                    None,
                    None,
                    auth_user.api_key_id.clone(),
                    auth_user.api_key_name.clone(),
                );
                (legacy, false)
            }
        };

    // Check approval if user has it enabled (uses cascade if the service
    // turned out to be org-owned).
    check_llm_approval(
        &state,
        &auth_user,
        &service_id,
        &service,
        &path,
        "POST",
        if body_bytes.is_empty() {
            None
        } else {
            Some(&body_bytes)
        },
        effective_owner_for_approval.as_deref(),
    )
    .await?;

    // Resolve delegated credentials. When the target came from the new
    // UserService path, the credential is already baked into `target`; we only
    // synthesize a bearer DelegatedCredential for the openai-codex branch,
    // which reads the token via `extract_bearer_token`. The legacy path still
    // fetches `UserProviderToken` records via `resolve_delegated_credentials`.
    let delegated = if resolved_via_user_service {
        if provider_slug == "openai-codex" && target.auth_method == "bearer" {
            vec![delegation_service::DelegatedCredential {
                provider_slug: provider_slug.clone(),
                injection_method: "bearer".to_string(),
                injection_key: "Authorization".to_string(),
                credential: target.credential.clone(),
            }]
        } else {
            Vec::new()
        }
    } else {
        delegation_service::resolve_delegated_credentials(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            &service_id,
        )
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "Provider '{}' not connected. Connect at /providers. ({})",
                provider_slug, e
            ))
        })?
    };

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
            catalog_default_headers: target.catalog_default_headers,
            user_service_default_headers: target.user_service_default_headers,
            ws_frame_injections: target.ws_frame_injections,
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
    let delegated: Vec<_> =
        delegated
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

    // Construct usage context once -- all branches share the same fields.
    let usage_context = llm_usage_service::UsageAuditContext {
        db: state.db.clone(),
        user_id: user_id_str.clone(),
        provider_slug: Some(provider_slug.clone()),
        service_id: Some(service_id.clone()),
        model: Some(model.to_string()),
        path: path.clone(),
        api_key_id: auth_user.api_key_id.clone(),
        api_key_name: auth_user.api_key_name.clone(),
    };
    let idle_timeout_secs = state.config.proxy_stream_idle_timeout_secs;

    // OpenAI Codex: use the specialized HTTP SSE transport and preserve query
    // parameters on the translated request.
    let response = if provider_slug == "openai-codex" {
        let bearer_token = extract_bearer_token(&delegated)?;
        // final_body_bytes is already the translated Responses API body
        let translated_body: serde_json::Value =
            serde_json::from_slice(final_body_bytes.as_deref().unwrap_or(&[]))
                .map_err(|e| AppError::Internal(format!("Failed to parse translated body: {e}")))?;

        // Path determines response format: chat/completions → translate back
        // to Chat Completions, responses → return Responses API as-is
        let is_chat_completions_path = path.contains("chat/completions");

        chatgpt_translator::send_to_chatgpt(
            &translated_body,
            &bearer_token,
            is_streaming,
            is_chat_completions_path,
            query.as_deref(),
            Some(usage_context),
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
            proxy_service::ProxyBody::Buffered(final_body_bytes),
            vec![],
            delegated,
            None,
            &state.token_exchange_cache,
        )
        .await?;

        // If translator needs translation, parse and translate the response
        if translator.needs_translation() {
            if is_streaming {
                // Streaming: translate SSE events on the fly
                build_translated_sse_response(
                    downstream_response,
                    translator,
                    Some(usage_context),
                    idle_timeout_secs,
                )
                .await?
            } else {
                // Non-streaming: buffer and translate the full response
                build_translated_json_response(
                    downstream_response,
                    translator.as_ref(),
                    Some(usage_context),
                )
                .await?
            }
        } else {
            build_filtered_response(downstream_response, Some(usage_context), idle_timeout_secs)
                .await?
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
            "response_status": response.status().as_u16(),
            "api_key_id": &auth_user.api_key_id,
            "api_key_name": &auth_user.api_key_name,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(response)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try primary slug, then fall back to openai-codex for OpenAI models.
async fn resolve_provider_slug_with_fallback(
    db: &mongodb::Database,
    user_id: &str,
    primary_slug: &str,
) -> AppResult<String> {
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use crate::models::user_provider_token::{
        COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
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
        if let Ok(reqwest_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Ok(reqwest_value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
        {
            reqwest_headers.insert(reqwest_name, reqwest_value);
        }
    }
    reqwest_headers
}

/// Read a reqwest response body with a size limit.
async fn read_response_with_limit(response: reqwest::Response) -> AppResult<bytes::Bytes> {
    let resp_bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read downstream response: {e}")))?;

    if resp_bytes.len() > MAX_RESPONSE_BODY_SIZE {
        return Err(AppError::Internal(
            "Upstream response too large".to_string(),
        ));
    }

    Ok(resp_bytes)
}

async fn build_filtered_response(
    downstream_response: reqwest::Response,
    usage_context: Option<llm_usage_service::UsageAuditContext>,
    idle_timeout_secs: u64,
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
        if ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str())
            && let Ok(header_name) =
                axum::http::header::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Ok(header_value) = axum::http::header::HeaderValue::from_bytes(value.as_bytes())
        {
            response_builder = response_builder.header(header_name, header_value);
        }
    }

    if is_sse {
        if let Some(context) = usage_context {
            let idle_timeout = std::time::Duration::from_secs(idle_timeout_secs);
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);

            tokio::spawn(async move {
                let mut buffer = String::new();
                let mut stream = downstream_response.bytes_stream();
                let mut accumulator = llm_usage_service::ReportedLlmUsageAccumulator::default();

                loop {
                    match tokio::time::timeout(idle_timeout, stream.next()).await {
                        Ok(Some(Ok(bytes))) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));

                            while let Some(event) = parse_next_sse_event(&mut buffer) {
                                if let Some((usage, mode)) =
                                    llm_usage_service::extract_reported_usage_from_sse_event(
                                        event.event_type.as_deref(),
                                        &event.data,
                                    )
                                {
                                    accumulator.observe(usage, mode);
                                }
                            }

                            if tx.send(Ok(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Ok(Some(Err(error))) => {
                            let _ = tx.send(Err(std::io::Error::other(error))).await;
                            break;
                        }
                        Ok(None) => break,
                        Err(_) => {
                            tracing::warn!(
                                idle_timeout_secs,
                                "LLM gateway SSE stream idle timeout reached"
                            );
                            break;
                        }
                    }
                }

                if let Some(usage) = accumulator.finalize() {
                    llm_usage_service::log_reported_usage_async(context, usage);
                }
            });

            let body = Body::from_stream(ReceiverStream::new(rx));
            return response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")));
        }

        // Stream SSE responses directly without buffering
        let body = Body::from_stream(downstream_response.bytes_stream());
        response_builder
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    } else {
        // H-3: Buffer non-streaming responses with size limit
        let response_body = read_response_with_limit(downstream_response).await?;

        if let Some(context) = usage_context
            && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response_body)
            && let Some(usage) = llm_usage_service::extract_reported_usage(&json)
        {
            llm_usage_service::log_reported_usage_async(context, usage);
        }

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
    usage_context: Option<llm_usage_service::UsageAuditContext>,
) -> AppResult<Response> {
    let status = downstream_response.status();
    let resp_headers = downstream_response.headers().clone();
    let resp_bytes = read_response_with_limit(downstream_response).await?;

    if status.is_success() {
        let resp_json: serde_json::Value = serde_json::from_slice(&resp_bytes).map_err(|e| {
            AppError::Internal(format!("Failed to parse provider response as JSON: {e}"))
        })?;

        let translated = translator.translate_response(resp_json)?;
        if let Some(context) = usage_context
            && let Some(usage) = llm_usage_service::extract_reported_usage(&translated)
        {
            llm_usage_service::log_reported_usage_async(context, usage);
        }
        let translated_bytes = serde_json::to_vec(&translated).map_err(|e| {
            AppError::Internal(format!("Failed to serialize translated response: {e}"))
        })?;

        let axum_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

        let mut response_builder = Response::builder()
            .status(axum_status)
            .header("content-type", "application/json");

        for (name, value) in resp_headers.iter() {
            let name_lower = name.as_str().to_lowercase();
            if name_lower != "content-type"
                && name_lower != "content-length"
                && ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str())
                && let Ok(header_name) =
                    axum::http::header::HeaderName::from_bytes(name.as_str().as_bytes())
                && let Ok(header_value) =
                    axum::http::header::HeaderValue::from_bytes(value.as_bytes())
            {
                response_builder = response_builder.header(header_name, header_value);
            }
        }

        response_builder
            .body(Body::from(translated_bytes))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    } else {
        // M-6: Translate error responses to OpenAI error format
        let axum_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

        let error_message = serde_json::from_slice::<serde_json::Value>(&resp_bytes)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| format!("Upstream provider error (HTTP {})", status.as_u16()));

        let error_body = serde_json::json!({
            "error": {
                "message": error_message,
                "type": "gateway_error",
                "code": status.as_u16(),
            }
        });

        let error_bytes = serde_json::to_vec(&error_body)
            .map_err(|e| AppError::Internal(format!("Failed to serialize error response: {e}")))?;

        Response::builder()
            .status(axum_status)
            .header("content-type", "application/json")
            .body(Body::from(error_bytes))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    }
}

/// Build a streaming SSE response with on-the-fly event translation.
/// Parses provider SSE events, translates each to OpenAI chunk format, and
/// re-emits as SSE text without buffering the full response.
async fn build_translated_sse_response(
    downstream_response: reqwest::Response,
    translator: Box<dyn llm_gateway_service::LlmTranslator>,
    usage_context: Option<llm_usage_service::UsageAuditContext>,
    idle_timeout_secs: u64,
) -> AppResult<Response> {
    let status = downstream_response.status();

    // If the upstream returned an error, buffer and return as translated JSON error
    if !status.is_success() {
        return build_translated_json_response(
            downstream_response,
            translator.as_ref(),
            usage_context,
        )
        .await;
    }

    let axum_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);
    let idle_timeout = std::time::Duration::from_secs(idle_timeout_secs);

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);

    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut state = llm_gateway_service::StreamTranslationState::default();
        let mut stream = downstream_response.bytes_stream();
        let mut accumulator = llm_usage_service::ReportedLlmUsageAccumulator::default();

        loop {
            match tokio::time::timeout(idle_timeout, stream.next()).await {
                Ok(Some(Ok(bytes))) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    while let Some(event) = parse_next_sse_event(&mut buffer) {
                        if let Some((usage, mode)) =
                            llm_usage_service::extract_reported_usage_from_sse_event(
                                event.event_type.as_deref(),
                                &event.data,
                            )
                        {
                            accumulator.observe(usage, mode);
                        }

                        if let Some(translated) =
                            translator.translate_stream_event(&event, &mut state)
                            && tx.send(Ok(bytes::Bytes::from(translated))).await.is_err()
                        {
                            if let Some(context) = usage_context.clone()
                                && let Some(usage) = accumulator.clone().finalize()
                            {
                                llm_usage_service::log_reported_usage_async(context, usage);
                            }
                            return; // client disconnected
                        }
                    }
                }
                Ok(Some(Err(e))) => {
                    if let Some(context) = usage_context.clone()
                        && let Some(usage) = accumulator.clone().finalize()
                    {
                        llm_usage_service::log_reported_usage_async(context, usage);
                    }
                    let _ = tx.send(Err(std::io::Error::other(e))).await;
                    return;
                }
                Ok(None) => break,
                Err(_) => {
                    tracing::warn!(
                        idle_timeout_secs,
                        "LLM gateway translated SSE stream idle timeout reached"
                    );
                    break;
                }
            }
        }

        if let Some(context) = usage_context
            && let Some(usage) = accumulator.finalize()
        {
            llm_usage_service::log_reported_usage_async(context, usage);
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

/// Convenience wrapper around the shared SSE parser.
fn parse_next_sse_event(buffer: &mut String) -> Option<sse_parser::SseEvent> {
    sse_parser::parse_next_event(buffer)
}

/// Check approval for LLM proxy request.
///
/// `service_owner_user_id` is the user_id that owns the resolved
/// `UserService` (the actor for personal credentials, an org for
/// org-shared credentials). When `None`, the caller couldn't determine
/// the owner -- the function falls back to the actor's policy only.
#[allow(clippy::too_many_arguments)]
async fn check_llm_approval(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    service: &crate::models::downstream_service::DownstreamService,
    path: &str,
    method_str: &str,
    body: Option<&[u8]>,
    service_owner_user_id: Option<&str>,
) -> AppResult<()> {
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();
    let owner_for_resolution = service_owner_user_id.unwrap_or(&approval_owner_user_id);
    let approval_resolution = approval_service::resolve_org_aware_approval(
        &state.db,
        &approval_owner_user_id,
        owner_for_resolution,
        service_id,
    )
    .await?;

    if should_bypass_approval_flow(approval_resolution.required, &auth_user.auth_method) {
        return Ok(());
    }

    let requester_type = auth_user
        .approval_requester_type()
        .ok_or_else(|| AppError::Forbidden("Session auth does not require approval".to_string()))?;
    let requester_id = auth_user.approval_requester_id();

    let primary_owner = &approval_resolution.primary_owner_user_id;

    // In grant mode, check for existing grant first.
    // In per_request mode, skip grant check -- every request needs fresh approval.
    let has_grant = if approval_resolution.mode
        == crate::models::service_approval_config::ApprovalMode::Grant
    {
        // Org-policy grants are org-scoped (see ChronoAIProject/NyxID#364) --
        // pass the flag through so members of the same org reuse the grant.
        approval_service::check_approval(
            &state.db,
            primary_owner,
            service_id,
            requester_type,
            &requester_id,
            approval_resolution.from_org_policy,
        )
        .await?
    } else {
        false
    };

    if has_grant {
        return Ok(());
    }

    // Compute notification recipients (see proxy.rs for the same pattern).
    // Org policy with no admins MUST fail closed -- otherwise the
    // requesting member would end up in `notify_user_ids` and could
    // self-approve their own org-gated request.
    let notify_user_ids: Vec<String> = if approval_resolution.from_org_policy {
        let mut admins =
            crate::services::org_service::list_admin_user_ids(&state.db, primary_owner).await?;
        admins.sort();
        admins.dedup();
        if admins.is_empty() {
            return Err(AppError::OrgApprovalNoAdmin(format!(
                "Org {primary_owner} has an approval policy on this service but no active admins to decide. Add an admin to the org and retry."
            )));
        }
        admins
    } else {
        vec![approval_owner_user_id.clone()]
    };

    let timeout_recipient = notify_user_ids.first().cloned().ok_or_else(|| {
        AppError::Internal("approval recipient list unexpectedly empty".to_string())
    })?;
    let channel =
        notification_service::get_or_create_channel(&state.db, &timeout_recipient).await?;

    let action_desc = action_description::build_action_description(method_str, path, body);

    let timeout_secs = channel.approval_timeout_secs;
    let approval_request = approval_service::create_approval_request(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        primary_owner,
        service_id,
        &service.name,
        &service.slug,
        requester_type,
        &requester_id,
        None,
        &format!("llm:{} {}", method_str, path),
        Some(&action_desc),
        approval_resolution.mode.clone(),
        timeout_secs,
        notify_user_ids,
        approval_resolution.from_org_policy,
    )
    .await?;

    // Block until the user approves/rejects or timeout expires
    let req_id = approval_request.id.clone();
    approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
        .await
        .map_err(|error| {
            approval_service::map_wait_for_decision_error(
                error,
                &req_id,
                &state.config.frontend_url,
            )
        })
}

fn should_bypass_approval_flow(
    requires_approval: bool,
    auth_method: &crate::mw::auth::AuthMethod,
) -> bool {
    !requires_approval || *auth_method == crate::mw::auth::AuthMethod::Session
}

#[cfg(test)]
mod tests {
    use super::should_bypass_approval_flow;
    use crate::mw::auth::AuthMethod;

    #[test]
    fn bypasses_when_approval_is_disabled() {
        assert!(should_bypass_approval_flow(false, &AuthMethod::Session));
        assert!(should_bypass_approval_flow(false, &AuthMethod::ApiKey));
    }

    #[test]
    fn bypasses_for_session_when_approval_is_required() {
        assert!(should_bypass_approval_flow(true, &AuthMethod::Session));
    }

    #[test]
    fn relay_does_not_bypass_when_approval_is_required() {
        assert!(!should_bypass_approval_flow(true, &AuthMethod::Relay));
    }

    #[test]
    fn does_not_bypass_for_programmatic_auth_when_required() {
        assert!(!should_bypass_approval_flow(true, &AuthMethod::ApiKey));
        assert!(!should_bypass_approval_flow(true, &AuthMethod::AccessToken));
        assert!(!should_bypass_approval_flow(true, &AuthMethod::Delegated));
        assert!(!should_bypass_approval_flow(
            true,
            &AuthMethod::ServiceAccount
        ));
    }
}
