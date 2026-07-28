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
use crate::models::service_billing::{BillingMetric, PlatformUsage, ResaleUsage};
use crate::models::usage_meter::CredentialClass;
use crate::mw::auth::AuthUser;
use crate::services::{
    approval_service, audit_service, chatgpt_translator, delegation_service, llm_gateway_service,
    llm_usage_service, notification_service, operation_descriptor, proxy_service, sse_parser,
};

fn llm_credential_class(
    resolved_via_user_service: bool,
    master_credential: bool,
    target: &proxy_service::ProxyTarget,
) -> CredentialClass {
    if target.auth_method == "none" && target.credential.is_empty() {
        CredentialClass::NoAuth
    } else if resolved_via_user_service {
        // Auto-provisioned UserServices with no user key inject the
        // catalog master credential; classify by whose key was used.
        if master_credential {
            CredentialClass::NyxidManagedMaster
        } else {
            CredentialClass::UserOwned
        }
    } else if !target.service.requires_user_credential && !target.credential.is_empty() {
        CredentialClass::NyxidManagedMaster
    } else {
        CredentialClass::UserOwned
    }
}

fn resale_usage_from_optional_reported(
    metric: BillingMetric,
    usage: Option<&llm_usage_service::ReportedLlmUsage>,
    fallback_bytes: i64,
) -> Option<ResaleUsage> {
    match metric {
        BillingMetric::Tokens => Some(ResaleUsage {
            metric,
            quantity: llm_usage_service::token_quantity_or_estimate(usage, fallback_bytes),
        }),
        BillingMetric::Requests => Some(ResaleUsage {
            metric,
            quantity: 1,
        }),
        BillingMetric::Bytes => Some(ResaleUsage {
            metric,
            quantity: fallback_bytes.max(0),
        }),
    }
}

pub(crate) fn llm_platform_usage(
    usage: Option<&llm_usage_service::ReportedLlmUsage>,
    fallback_bytes: i64,
) -> PlatformUsage {
    PlatformUsage::llm_completion(
        fallback_bytes,
        llm_usage_service::token_quantity_or_estimate(usage, fallback_bytes),
    )
}

pub(crate) fn enforce_llm_billing_classification(
    request: &Request<Body>,
    expected: crate::services::billing::BillingIngress,
) -> AppResult<crate::services::billing::route_inventory::BillingEgressPermit> {
    crate::services::billing::route_inventory::enforce_billing_egress_classification(
        request
            .extensions()
            .get::<crate::services::billing::route_inventory::BillingRoutePolicy>()
            .copied(),
        expected,
    )
}

fn supports_stream_options_include_usage(provider_slug: &str) -> bool {
    matches!(provider_slug, "openai" | "deepseek")
}

fn should_force_stream_usage(provider_slug: &str, path: &str, body: &serde_json::Value) -> bool {
    supports_stream_options_include_usage(provider_slug)
        && path.contains("chat/completions")
        && body.get("stream").and_then(|value| value.as_bool()) == Some(true)
}

fn force_stream_usage_for_provider(
    provider_slug: &str,
    path: &str,
    mut body: serde_json::Value,
) -> serde_json::Value {
    if should_force_stream_usage(provider_slug, path, &body) {
        llm_usage_service::force_stream_options_include_usage(&mut body);
    }
    body
}

fn force_stream_usage_bytes_for_provider(
    provider_slug: &str,
    path: &str,
    body_bytes: bytes::Bytes,
) -> bytes::Bytes {
    if body_bytes.is_empty() || !supports_stream_options_include_usage(provider_slug) {
        return body_bytes;
    }

    let Ok(mut body) = serde_json::from_slice::<serde_json::Value>(&body_bytes) else {
        return body_bytes;
    };
    if !should_force_stream_usage(provider_slug, path, &body) {
        return body_bytes;
    }
    if !llm_usage_service::force_stream_options_include_usage(&mut body) {
        return body_bytes;
    }
    serde_json::to_vec(&body)
        .map(bytes::Bytes::from)
        .unwrap_or(body_bytes)
}

fn chatgpt_usage_callback(
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    metered: crate::services::billing::MeteredProxyContext,
    request_len: i64,
    resale_metric: Option<BillingMetric>,
    model: Option<String>,
) -> chatgpt_translator::UsageCompleteCallback {
    std::sync::Arc::new(move |usage, response_len| {
        let total_bytes = request_len + response_len;
        let resale = resale_metric.and_then(|metric| {
            resale_usage_from_optional_reported(metric, usage.as_ref(), total_bytes)
        });
        Box::pin(settle_meter_async(
            billing.clone(),
            metered.clone(),
            llm_platform_usage(usage.as_ref(), total_bytes),
            resale,
            model.clone(),
        ))
    })
}

async fn settle_meter_async(
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    metered: crate::services::billing::MeteredProxyContext,
    platform: PlatformUsage,
    resale: Option<ResaleUsage>,
    model: Option<String>,
) {
    if !metered.is_enabled() {
        return;
    }

    if billing
        .settle_deferred(&metered, platform, resale, model)
        .await
        .is_err()
    {
        let billing_request_id = metered
            .route
            .as_ref()
            .map(|route| route.billing_request_id.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            billing_request_id,
            "Failed to persist LLM usage settlement intent"
        );
    }
}

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
    let billing_egress_permit = enforce_llm_billing_classification(
        &request,
        crate::services::billing::BillingIngress::LlmProvider,
    )?;

    // Per-agent rate limit check
    crate::mw::rate_limit::check_agent_rate_limit(&state.per_agent_limiter, &auth_user)?;

    let user_id_str = auth_user.user_id.to_string();

    // Resolve the downstream service for this provider slug
    let (service, _provider) =
        llm_gateway_service::resolve_llm_service_by_slug(&state.db, &provider_slug).await?;

    let service_id = service.id.clone();

    // Read request parts and body before credential resolution so Deny
    // policies can short-circuit without decrypting downstream credentials.
    let request_method_str = request.method().as_str().to_string();
    let method = request.method().clone();
    let query = request.uri().query().map(String::from);
    let headers = request.headers().clone();
    let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read request body: {e}")))?;

    preflight_llm_deny_before_resolution(
        &state,
        &auth_user,
        &service_id,
        &path,
        &request_method_str,
        if body_bytes.is_empty() {
            None
        } else {
            Some(&body_bytes)
        },
    )
    .await?;

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
    let (target, resolved_via_user_service, master_credential, owner_for_approval) =
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
                let effective_owner = resolution
                    .org_routing
                    .as_ref()
                    .map(|routing| routing.org_user_id.clone())
                    .unwrap_or_else(|| user_id_str.clone());
                (
                    resolution.target,
                    true,
                    resolution.master_credential,
                    Some(effective_owner),
                )
            }
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
                (legacy, false, false, None)
            }
        };
    // Check approval against the owner selected by credential resolution.
    // Legacy credentials are personal, so `None` retains the actor fallback.
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

    let billing_resolution_user_id = auth_user.proxy_resolution_user_id();
    let billing_resource_owner_id = owner_for_approval
        .as_deref()
        .unwrap_or(&billing_resolution_user_id);
    let billing_owner = state
        .billing
        .owner_resolver()
        .resolve_for_resource(&billing_resolution_user_id, billing_resource_owner_id)
        .await?;
    let billing_ctx = crate::services::billing::BillingRouteContext::new(
        crate::services::billing::BillingIngress::LlmProvider,
        uuid::Uuid::new_v4().to_string(),
        billing_owner.owner_id,
        user_id_str.clone(),
        auth_user.api_key_id.clone(),
        None,
        Some(service_id.clone()),
        Some(service.slug.clone()),
        crate::services::billing::NodeIntent::Direct,
        target.auth_method.clone(),
        llm_credential_class(resolved_via_user_service, master_credential, &target),
        BillingMetric::Tokens,
        target.service.billing.as_ref().or(service.billing.as_ref()),
        state.billing.resale_enabled(),
    );
    let metered = state.billing.open(&billing_ctx).await?;

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
        let request_len = serde_json::to_vec(&translated.body)
            .map(|bytes| bytes.len() as i64)
            .unwrap_or(body_bytes.len() as i64);

        let bearer_token = extract_bearer_token(&delegated)?;
        let is_streaming = body_json
            .get("stream")
            .and_then(|s| s.as_bool())
            .unwrap_or(false);
        let usage_complete = chatgpt_usage_callback(
            state.billing.clone(),
            metered.clone(),
            request_len,
            metered
                .route
                .as_ref()
                .and_then(|ctx| ctx.resale.as_ref().map(|spec| spec.metric)),
            body_json
                .get("model")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        );

        state.billing.mark_forwarded(&metered).await?;
        chatgpt_translator::send_to_chatgpt(
            &translated.body,
            &bearer_token,
            is_streaming,
            is_chat_completions_path,
            query.as_deref(),
            Some(usage_context),
            Some(usage_complete),
            billing_egress_permit,
        )
        .await?
    } else {
        let body = if body_bytes.is_empty() {
            None
        } else {
            Some(force_stream_usage_bytes_for_provider(
                &provider_slug,
                &path,
                body_bytes,
            ))
        };
        let request_len = body.as_ref().map(|bytes| bytes.len() as i64).unwrap_or(0);

        let reqwest_method = convert_method(&method)?;
        let reqwest_headers = convert_headers(&headers);

        state.billing.mark_forwarded(&metered).await?;
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
            &state.cloud_response_cache,
            billing_egress_permit,
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
            metered.clone(),
            state.billing.clone(),
            request_len,
        )
        .await?
    };

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "llm_proxy_request",
        Some(serde_json::json!({
            "provider_slug": &provider_slug,
            "method": method.as_str(),
            "path": &path,
            "response_status": response.status().as_u16(),
            "api_key_id": &auth_user.api_key_id,
            "api_key_name": &auth_user.api_key_name,
        })),
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
    let billing_egress_permit = enforce_llm_billing_classification(
        &request,
        crate::services::billing::BillingIngress::LlmGateway,
    )?;

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
    let mut body_json: serde_json::Value = if body_bytes.is_empty() {
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
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::ValidationError("'model' field is required in request body".to_string())
        })?;

    // Resolve provider slug from model name
    let primary_slug =
        llm_gateway_service::resolve_provider_for_model(&model).ok_or_else(|| {
            AppError::BadRequest(format!(
                "Unknown model: '{model}'. Cannot determine provider."
            ))
        })?;

    // Try to find the user's active token for the resolved provider.
    // For OpenAI models, fall back to openai-codex if openai is not connected.
    let provider_slug =
        resolve_provider_slug_with_fallback(&state.db, &user_id_str, primary_slug).await?;
    body_json = force_stream_usage_for_provider(&provider_slug, &path, body_json);

    // Resolve the downstream service
    let (service, _provider) =
        llm_gateway_service::resolve_llm_service_by_slug(&state.db, &provider_slug).await?;

    let service_id = service.id.clone();

    // Get the translator
    let translator = llm_gateway_service::get_translator(&provider_slug);

    preflight_llm_deny_before_resolution(
        &state,
        &auth_user,
        &service_id,
        &path,
        "POST",
        if body_bytes.is_empty() {
            None
        } else {
            Some(&body_bytes)
        },
    )
    .await?;

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
    let (target, resolved_via_user_service, master_credential) =
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
                let pool_metadata = resolution.pool_selection.as_ref().map(|selection| {
                    serde_json::json!({
                        "pool_id": selection.pool_id,
                        "pool_slug": selection.pool_slug,
                        "chosen_user_service_id": selection.selected_member_id,
                        "pool_strategy": selection.strategy.as_str(),
                    })
                });
                // Audit org-routed LLM gateway calls so the org's owner can
                // see who is using shared credentials. Mirrors the pattern
                // in handlers/proxy.rs.
                if let Some(routing) = &resolution.org_routing {
                    let mut event_data = serde_json::json!({
                        "routed_via": "org",
                        "service_id": service_id,
                        "user_service_id": resolution.user_service_id,
                        "org_user_id": routing.org_user_id,
                        "member_user_id": routing.member_user_id,
                        "membership_id": routing.membership_id,
                    });
                    if let Some(metadata) = pool_metadata.clone()
                        && let (Some(dst), Some(src)) =
                            (event_data.as_object_mut(), metadata.as_object())
                    {
                        dst.extend(src.clone());
                    }
                    audit_service::log_for_user(
                        state.db.clone(),
                        &auth_user,
                        "llm_gateway_routed_via_org",
                        Some(event_data),
                    );
                } else {
                    let mut event_data = serde_json::json!({
                        "routed_via": "personal",
                        "service_id": service_id,
                        "user_service_id": resolution.user_service_id,
                    });
                    if let Some(metadata) = pool_metadata
                        && let (Some(dst), Some(src)) =
                            (event_data.as_object_mut(), metadata.as_object())
                    {
                        dst.extend(src.clone());
                    }
                    audit_service::log_for_user(
                        state.db.clone(),
                        &auth_user,
                        "llm_gateway_routed_via_personal",
                        Some(event_data),
                    );
                }
                (resolution.target, true, resolution.master_credential)
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
                audit_service::log_for_user(
                    state.db.clone(),
                    &auth_user,
                    "llm_gateway_routed_via_personal",
                    Some(serde_json::json!({
                        "routed_via": "personal",
                        "service_id": service_id,
                        "user_service_id": serde_json::Value::Null,
                    })),
                );
                (legacy, false, false)
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

    let billing_resolution_user_id = auth_user.proxy_resolution_user_id();
    let billing_resource_owner_id = effective_owner_for_approval
        .as_deref()
        .unwrap_or(&billing_resolution_user_id);
    let billing_owner = state
        .billing
        .owner_resolver()
        .resolve_for_resource(&billing_resolution_user_id, billing_resource_owner_id)
        .await?;
    let billing_ctx = crate::services::billing::BillingRouteContext::new(
        crate::services::billing::BillingIngress::LlmGateway,
        uuid::Uuid::new_v4().to_string(),
        billing_owner.owner_id,
        user_id_str.clone(),
        auth_user.api_key_id.clone(),
        None,
        Some(service_id.clone()),
        Some(service.slug.clone()),
        crate::services::billing::NodeIntent::Direct,
        target.auth_method.clone(),
        llm_credential_class(resolved_via_user_service, master_credential, &target),
        BillingMetric::Tokens,
        target.service.billing.as_ref().or(service.billing.as_ref()),
        state.billing.resale_enabled(),
    );
    let metered = state.billing.open(&billing_ctx).await?;

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
            connection_id: target.connection_id,
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
    let request_len = final_body_bytes
        .as_ref()
        .map(|body| body.len() as i64)
        .unwrap_or(0);

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
        let usage_complete = chatgpt_usage_callback(
            state.billing.clone(),
            metered.clone(),
            request_len,
            metered
                .route
                .as_ref()
                .and_then(|ctx| ctx.resale.as_ref().map(|spec| spec.metric)),
            Some(model.to_string()),
        );

        state.billing.mark_forwarded(&metered).await?;
        chatgpt_translator::send_to_chatgpt(
            &translated_body,
            &bearer_token,
            is_streaming,
            is_chat_completions_path,
            query.as_deref(),
            Some(usage_context),
            Some(usage_complete),
            billing_egress_permit,
        )
        .await?
    } else {
        state.billing.mark_forwarded(&metered).await?;
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
            &state.cloud_response_cache,
            billing_egress_permit,
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
                    metered.clone(),
                    state.billing.clone(),
                    request_len,
                )
                .await?
            } else {
                // Non-streaming: buffer and translate the full response
                build_translated_json_response(
                    downstream_response,
                    translator.as_ref(),
                    Some(usage_context),
                    metered.clone(),
                    state.billing.clone(),
                    request_len,
                )
                .await?
            }
        } else {
            build_filtered_response(
                downstream_response,
                Some(usage_context),
                idle_timeout_secs,
                metered.clone(),
                state.billing.clone(),
                request_len,
            )
            .await?
        }
    };

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "llm_gateway_request",
        Some(serde_json::json!({
            "model": model,
            "provider_slug": &provider_slug,
            "method": method.as_str(),
            "path": &path,
            "response_status": response.status().as_u16(),
            "api_key_id": &auth_user.api_key_id,
            "api_key_name": &auth_user.api_key_name,
        })),
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
    metered: crate::services::billing::MeteredProxyContext,
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    request_len: i64,
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
            let stream_metered = metered.clone();
            let stream_billing = billing.clone();
            let resale_metric = stream_metered
                .route
                .as_ref()
                .and_then(|ctx| ctx.resale.as_ref().map(|spec| spec.metric));

            tokio::spawn(async move {
                let mut buffer = String::new();
                let mut stream = downstream_response.bytes_stream();
                let mut accumulator = llm_usage_service::ReportedLlmUsageAccumulator::default();
                let mut response_len: i64 = 0;

                loop {
                    match tokio::time::timeout(idle_timeout, stream.next()).await {
                        Ok(Some(Ok(bytes))) => {
                            response_len += bytes.len() as i64;
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

                let usage = accumulator.finalize();
                if let Some(usage) = usage.clone() {
                    llm_usage_service::log_reported_usage_async(context.clone(), usage);
                }
                let resale = resale_metric.and_then(|metric| {
                    resale_usage_from_optional_reported(
                        metric,
                        usage.as_ref(),
                        request_len + response_len,
                    )
                });
                settle_meter_async(
                    stream_billing,
                    stream_metered,
                    llm_platform_usage(usage.as_ref(), request_len + response_len),
                    resale,
                    context.model,
                )
                .await;
            });

            let body = Body::from_stream(ReceiverStream::new(rx));
            return response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")));
        }

        // Stream SSE responses directly without buffering
        let mut stream = downstream_response.bytes_stream();
        let stream_metered = metered.clone();
        let stream_billing = billing.clone();
        let body_stream = async_stream::stream! {
            let mut response_len: i64 = 0;
            while let Some(next) = stream.next().await {
                match next {
                    Ok(bytes) => {
                        response_len += bytes.len() as i64;
                        yield Ok::<_, reqwest::Error>(bytes);
                    }
                    Err(error) => {
                        yield Err(error);
                        break;
                    }
                }
            }
            settle_meter_async(
                stream_billing,
                stream_metered,
                llm_platform_usage(None, request_len + response_len),
                None,
                None,
            )
            .await;
        };
        let body = Body::from_stream(body_stream);
        response_builder
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    } else {
        // H-3: Buffer non-streaming responses with size limit
        let response_body = read_response_with_limit(downstream_response).await?;

        let mut reported_usage = None;
        let mut model = None;
        if let Some(context) = usage_context
            && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response_body)
            && let Some(usage) = llm_usage_service::extract_reported_usage(&json)
        {
            model = context.model.clone();
            llm_usage_service::log_reported_usage_async(context, usage.clone());
            reported_usage = Some(usage);
        }
        let response_len = response_body.len() as i64;
        let resale = metered.route.as_ref().and_then(|ctx| {
            ctx.resale.as_ref().and_then(|spec| {
                resale_usage_from_optional_reported(
                    spec.metric,
                    reported_usage.as_ref(),
                    request_len + response_len,
                )
            })
        });
        billing
            .settle(
                &metered,
                llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
                resale,
                model,
            )
            .await?;

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
    metered: crate::services::billing::MeteredProxyContext,
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    request_len: i64,
) -> AppResult<Response> {
    let status = downstream_response.status();
    let resp_headers = downstream_response.headers().clone();
    let resp_bytes = read_response_with_limit(downstream_response).await?;

    if status.is_success() {
        let resp_json: serde_json::Value = serde_json::from_slice(&resp_bytes).map_err(|e| {
            AppError::Internal(format!("Failed to parse provider response as JSON: {e}"))
        })?;

        let translated = translator.translate_response(resp_json)?;
        let mut reported_usage = None;
        let mut model = None;
        if let Some(context) = usage_context
            && let Some(usage) = llm_usage_service::extract_reported_usage(&translated)
        {
            model = context.model.clone();
            llm_usage_service::log_reported_usage_async(context, usage.clone());
            reported_usage = Some(usage);
        }
        let translated_bytes = serde_json::to_vec(&translated).map_err(|e| {
            AppError::Internal(format!("Failed to serialize translated response: {e}"))
        })?;
        let response_len = translated_bytes.len() as i64;
        let resale = metered.route.as_ref().and_then(|ctx| {
            ctx.resale.as_ref().and_then(|spec| {
                resale_usage_from_optional_reported(
                    spec.metric,
                    reported_usage.as_ref(),
                    request_len + response_len,
                )
            })
        });
        billing
            .settle(
                &metered,
                llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
                resale,
                model,
            )
            .await?;

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
        billing
            .settle(
                &metered,
                llm_platform_usage(None, request_len + error_bytes.len() as i64),
                None,
                usage_context.and_then(|context| context.model),
            )
            .await?;

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
    metered: crate::services::billing::MeteredProxyContext,
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    request_len: i64,
) -> AppResult<Response> {
    let status = downstream_response.status();

    // If the upstream returned an error, buffer and return as translated JSON error
    if !status.is_success() {
        return build_translated_json_response(
            downstream_response,
            translator.as_ref(),
            usage_context,
            metered,
            billing,
            request_len,
        )
        .await;
    }

    let axum_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);
    let idle_timeout = std::time::Duration::from_secs(idle_timeout_secs);

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);
    let stream_metered = metered.clone();
    let stream_billing = billing.clone();
    let resale_metric = stream_metered
        .route
        .as_ref()
        .and_then(|ctx| ctx.resale.as_ref().map(|spec| spec.metric));

    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut state = llm_gateway_service::StreamTranslationState::default();
        let mut stream = downstream_response.bytes_stream();
        let mut accumulator = llm_usage_service::ReportedLlmUsageAccumulator::default();
        let mut response_len: i64 = 0;

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
                            && {
                                response_len += translated.len() as i64;
                                tx.send(Ok(bytes::Bytes::from(translated))).await.is_err()
                            }
                        {
                            if let Some(context) = usage_context.clone()
                                && let Some(usage) = accumulator.clone().finalize()
                            {
                                llm_usage_service::log_reported_usage_async(context, usage);
                            }
                            let usage = accumulator.clone().finalize();
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                usage_context.and_then(|context| context.model),
                            )
                            .await;
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
            && let Some(usage) = accumulator.clone().finalize()
        {
            llm_usage_service::log_reported_usage_async(context, usage);
        }
        let usage = accumulator.finalize();
        let resale = resale_metric.and_then(|metric| {
            resale_usage_from_optional_reported(metric, usage.as_ref(), request_len + response_len)
        });
        settle_meter_async(
            stream_billing,
            stream_metered,
            llm_platform_usage(usage.as_ref(), request_len + response_len),
            resale,
            None,
        )
        .await;
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
async fn preflight_llm_deny_before_resolution(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    method_str: &str,
    body: Option<&[u8]>,
) -> AppResult<()> {
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();
    let hint = proxy_service::find_approval_resolution_hint(
        &state.db,
        &approval_owner_user_id,
        None,
        Some(service_id),
    )
    .await?
    .unwrap_or_else(|| proxy_service::ApprovalResolutionHint {
        service_id: service_id.to_string(),
        service_owner_id: approval_owner_user_id.clone(),
    });

    let operation = operation_descriptor::build_llm_descriptor(method_str, path, body);
    let denied = approval_service::evaluate_deny_only(
        &state.db,
        &approval_owner_user_id,
        &hint.service_owner_id,
        &hint.service_id,
        &operation,
    )
    .await?;

    if denied {
        return Err(AppError::Forbidden(
            "Operation denied by approval policy".to_string(),
        ));
    }

    Ok(())
}

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
    let operation = operation_descriptor::build_llm_descriptor(method_str, path, body);
    let approval_outcome = approval_service::evaluate_and_check(
        &state.db,
        &approval_owner_user_id,
        owner_for_resolution,
        service_id,
        &operation,
        auth_user.approval_requester_type(),
        &auth_user.approval_requester_id(),
        auth_user.auth_method == crate::mw::auth::AuthMethod::Session,
    )
    .await?;

    let pending = match approval_outcome {
        approval_service::ApprovalOutcome::Allowed { .. } => return Ok(()),
        approval_service::ApprovalOutcome::Denied => {
            return Err(AppError::Forbidden(
                "Operation denied by approval policy".to_string(),
            ));
        }
        approval_service::ApprovalOutcome::NeedsApproval(pending) => pending,
    };

    let notify_user_ids = approval_service::approval_notification_recipients(
        &state.db,
        &approval_owner_user_id,
        &pending,
    )
    .await?;
    let timeout_recipient = notify_user_ids.first().cloned().ok_or_else(|| {
        AppError::Internal("approval recipient list unexpectedly empty".to_string())
    })?;
    let channel =
        notification_service::get_or_create_channel(&state.db, &timeout_recipient).await?;

    let timeout_secs = channel.approval_timeout_secs;
    let request_operation = approval_service::ApprovalRequestOperation::from_descriptor(
        &operation,
        pending.resolution.grant_scope.clone(),
    );
    let approval_request = approval_service::create_approval_request(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &pending.primary_owner_user_id,
        service_id,
        &service.name,
        &service.slug,
        &pending.requester_type,
        &pending.requester_id,
        None,
        request_operation,
        pending.resolution.mode.clone(),
        timeout_secs,
        notify_user_ids,
        pending.resolution.from_org_policy,
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

#[cfg(test)]
fn should_bypass_approval_flow(
    requires_approval: bool,
    auth_method: &crate::mw::auth::AuthMethod,
) -> bool {
    !requires_approval || *auth_method == crate::mw::auth::AuthMethod::Session
}

#[cfg(test)]
mod tests {
    use super::{
        force_stream_usage_bytes_for_provider, llm_platform_usage,
        resale_usage_from_optional_reported, should_bypass_approval_flow,
    };
    use crate::models::service_billing::BillingMetric;
    use crate::mw::auth::AuthMethod;
    use crate::services::llm_usage_service::ReportedLlmUsage;

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

    #[test]
    fn llm_platform_usage_uses_reported_tokens() {
        let usage = ReportedLlmUsage {
            prompt_tokens: 8,
            completion_tokens: 12,
            total_tokens: 20,
            reported_cost: None,
        };

        let platform = llm_platform_usage(Some(&usage), 10_000);

        assert_eq!(platform.requests, 1);
        assert_eq!(platform.bytes, 10_000);
        assert_eq!(platform.tokens, 20);
    }

    #[test]
    fn llm_platform_usage_estimates_when_usage_is_omitted() {
        let platform = llm_platform_usage(None, 17);

        assert_eq!(platform.requests, 1);
        assert_eq!(platform.bytes, 17);
        assert_eq!(platform.tokens, 5);
    }

    #[test]
    fn token_resale_usage_estimates_when_usage_is_omitted() {
        let resale = resale_usage_from_optional_reported(BillingMetric::Tokens, None, 9)
            .expect("token resale usage");

        assert_eq!(resale.metric, BillingMetric::Tokens);
        assert_eq!(resale.quantity, 3);
    }

    #[test]
    fn force_stream_usage_bytes_only_for_supported_provider() {
        let body = bytes::Bytes::from_static(br#"{"model":"gpt-4o-mini","stream":true}"#);
        let forced =
            force_stream_usage_bytes_for_provider("openai", "chat/completions", body.clone());
        let forced_json: serde_json::Value = serde_json::from_slice(&forced).unwrap();

        assert_eq!(forced_json["stream_options"]["include_usage"], true);

        let unchanged =
            force_stream_usage_bytes_for_provider("anthropic", "chat/completions", body);
        let unchanged_json: serde_json::Value = serde_json::from_slice(&unchanged).unwrap();
        assert!(unchanged_json.get("stream_options").is_none());
    }

    // -----------------------------------------------------------------------
    // convert_method tests
    // -----------------------------------------------------------------------

    use super::ALLOWED_RESPONSE_HEADERS;
    use super::MAX_RESPONSE_BODY_SIZE;
    use super::{convert_headers, convert_method, extract_bearer_token, parse_next_sse_event};
    use crate::services::delegation_service::DelegatedCredential;
    use axum::http::Method;

    #[test]
    fn convert_method_get() {
        let result = convert_method(&Method::GET).unwrap();
        assert_eq!(result, reqwest::Method::GET);
    }

    #[test]
    fn convert_method_post() {
        let result = convert_method(&Method::POST).unwrap();
        assert_eq!(result, reqwest::Method::POST);
    }

    #[test]
    fn convert_method_put() {
        let result = convert_method(&Method::PUT).unwrap();
        assert_eq!(result, reqwest::Method::PUT);
    }

    #[test]
    fn convert_method_delete() {
        let result = convert_method(&Method::DELETE).unwrap();
        assert_eq!(result, reqwest::Method::DELETE);
    }

    #[test]
    fn convert_method_patch() {
        let result = convert_method(&Method::PATCH).unwrap();
        assert_eq!(result, reqwest::Method::PATCH);
    }

    #[test]
    fn convert_method_head() {
        let result = convert_method(&Method::HEAD).unwrap();
        assert_eq!(result, reqwest::Method::HEAD);
    }

    #[test]
    fn convert_method_options() {
        let result = convert_method(&Method::OPTIONS).unwrap();
        assert_eq!(result, reqwest::Method::OPTIONS);
    }

    #[test]
    fn convert_method_trace_is_unsupported() {
        let result = convert_method(&Method::TRACE);
        assert!(result.is_err());
    }

    #[test]
    fn convert_method_connect_is_unsupported() {
        let result = convert_method(&Method::CONNECT);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // convert_headers tests
    // -----------------------------------------------------------------------

    #[test]
    fn convert_headers_copies_standard_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("authorization", "Bearer tok123".parse().unwrap());

        let converted = convert_headers(&headers);
        assert_eq!(
            converted.get("content-type").map(|v| v.to_str().unwrap()),
            Some("application/json")
        );
        assert_eq!(
            converted.get("authorization").map(|v| v.to_str().unwrap()),
            Some("Bearer tok123")
        );
    }

    #[test]
    fn convert_headers_handles_empty_map() {
        let headers = axum::http::HeaderMap::new();
        let converted = convert_headers(&headers);
        assert!(converted.is_empty());
    }

    #[test]
    fn convert_headers_preserves_multiple_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-custom", "value1".parse().unwrap());
        headers.insert("x-another", "value2".parse().unwrap());
        headers.insert("accept", "text/plain".parse().unwrap());

        let converted = convert_headers(&headers);
        assert_eq!(converted.len(), 3);
    }

    // -----------------------------------------------------------------------
    // extract_bearer_token tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_bearer_token_finds_bearer_credential() {
        let creds = vec![
            DelegatedCredential {
                provider_slug: "openai".into(),
                injection_method: "header".into(),
                injection_key: "X-Custom".into(),
                credential: "custom-val".into(),
            },
            DelegatedCredential {
                provider_slug: "openai".into(),
                injection_method: "bearer".into(),
                injection_key: "Authorization".into(),
                credential: "sk-test-12345".into(),
            },
        ];

        let token = extract_bearer_token(&creds).unwrap();
        assert_eq!(token, "sk-test-12345");
    }

    #[test]
    fn extract_bearer_token_returns_first_bearer() {
        let creds = vec![
            DelegatedCredential {
                provider_slug: "openai".into(),
                injection_method: "bearer".into(),
                injection_key: "Authorization".into(),
                credential: "first-token".into(),
            },
            DelegatedCredential {
                provider_slug: "openai".into(),
                injection_method: "bearer".into(),
                injection_key: "Authorization".into(),
                credential: "second-token".into(),
            },
        ];

        let token = extract_bearer_token(&creds).unwrap();
        assert_eq!(token, "first-token");
    }

    #[test]
    fn extract_bearer_token_fails_when_no_bearer() {
        let creds = vec![DelegatedCredential {
            provider_slug: "openai".into(),
            injection_method: "header".into(),
            injection_key: "X-Key".into(),
            credential: "not-a-bearer".into(),
        }];

        let result = extract_bearer_token(&creds);
        assert!(result.is_err());
    }

    #[test]
    fn extract_bearer_token_fails_on_empty_slice() {
        let result = extract_bearer_token(&[]);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // parse_next_sse_event tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_next_sse_event_extracts_data_event() {
        let mut buffer = "data: {\"id\":\"1\"}\n\n".to_string();
        let event = parse_next_sse_event(&mut buffer).expect("should parse one event");
        assert_eq!(event.data, "{\"id\":\"1\"}");
        assert!(buffer.is_empty());
    }

    #[test]
    fn parse_next_sse_event_extracts_typed_event() {
        let mut buffer = "event: message\ndata: hello\n\n".to_string();
        let event = parse_next_sse_event(&mut buffer).expect("should parse one event");
        assert_eq!(event.event_type.as_deref(), Some("message"));
        assert_eq!(event.data, "hello");
    }

    #[test]
    fn parse_next_sse_event_returns_none_for_incomplete() {
        let mut buffer = "data: partial".to_string();
        assert!(parse_next_sse_event(&mut buffer).is_none());
        // Buffer should remain intact
        assert_eq!(buffer, "data: partial");
    }

    #[test]
    fn parse_next_sse_event_returns_none_for_empty() {
        let mut buffer = String::new();
        assert!(parse_next_sse_event(&mut buffer).is_none());
    }

    #[test]
    fn parse_next_sse_event_handles_multiple_data_lines() {
        let mut buffer = "data: line1\ndata: line2\n\n".to_string();
        let event = parse_next_sse_event(&mut buffer).expect("should parse");
        assert_eq!(event.data, "line1\nline2");
    }

    #[test]
    fn parse_next_sse_event_consumes_first_leaves_second() {
        let mut buffer = "data: first\n\ndata: second\n\n".to_string();
        let event1 = parse_next_sse_event(&mut buffer).expect("should parse first");
        assert_eq!(event1.data, "first");

        let event2 = parse_next_sse_event(&mut buffer).expect("should parse second");
        assert_eq!(event2.data, "second");

        assert!(buffer.is_empty());
    }

    #[test]
    fn parse_next_sse_event_done_marker() {
        let mut buffer = "data: [DONE]\n\n".to_string();
        let event = parse_next_sse_event(&mut buffer).expect("should parse [DONE]");
        assert_eq!(event.data, "[DONE]");
    }

    // -----------------------------------------------------------------------
    // ALLOWED_RESPONSE_HEADERS constant tests
    // -----------------------------------------------------------------------

    #[test]
    fn allowed_response_headers_contains_essentials() {
        assert!(ALLOWED_RESPONSE_HEADERS.contains(&"content-type"));
        assert!(ALLOWED_RESPONSE_HEADERS.contains(&"content-length"));
        assert!(ALLOWED_RESPONSE_HEADERS.contains(&"cache-control"));
        assert!(ALLOWED_RESPONSE_HEADERS.contains(&"etag"));
    }

    #[test]
    fn allowed_response_headers_excludes_security_sensitive() {
        // Set-Cookie and Authorization should not be forwarded
        assert!(!ALLOWED_RESPONSE_HEADERS.contains(&"set-cookie"));
        assert!(!ALLOWED_RESPONSE_HEADERS.contains(&"authorization"));
        assert!(!ALLOWED_RESPONSE_HEADERS.contains(&"cookie"));
    }

    // -----------------------------------------------------------------------
    // MAX_RESPONSE_BODY_SIZE constant test
    // -----------------------------------------------------------------------

    #[test]
    fn max_response_body_size_is_50mb() {
        assert_eq!(MAX_RESPONSE_BODY_SIZE, 50 * 1024 * 1024);
    }
}
