//! Closed public model and NyxAgent transports. No owner connections or AuthUser.
use super::{
    api_key_mutation_service as tx,
    billing::{self, BillingIngress},
    llm_usage_service as usage,
    platform_manager_contract::*,
    platform_public_budget_service as budget, platform_public_policy as policy, proxy_service,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        downstream_service::{
            COLLECTION_NAME as SERVICES, DownstreamService, InferenceWireProtocol,
            PlatformKeyAudience,
        },
        platform_public_budget::{LOOPS, PublicBudgetLoop},
        platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
        platform_public_turn::{COLLECTION_NAME as TURNS, PlatformPublicTurn},
        service_endpoint::{COLLECTION_NAME as ENDPOINTS, ServiceEndpoint},
    },
};
use axum::{
    body::Body,
    http::{HeaderMap, Method},
    response::Response,
};
use futures::StreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

fn denied() -> AppError {
    AppError::Forbidden("Public model operation is unavailable".into())
}

/// Public-specific projection binds the complete catalog configuration, including
/// encrypted credential identity, without inheriting a UserService projection.
pub(crate) fn catalog_digest(service: &DownstreamService) -> AppResult<String> {
    let mut value = serde_json::to_value(service).map_err(|_| denied())?;
    let object = value.as_object_mut().ok_or_else(denied)?;
    object.remove("created_at");
    object.remove("updated_at");
    object.insert(
        "credential_encrypted".into(),
        json!(hex::encode(Sha256::digest(&service.credential_encrypted))),
    );
    Ok(super::mcp_service::canonical_sha256(json!([
        "nyxid-public-catalog.v1",
        value
    ])))
}
pub(crate) fn protocol(service: &DownstreamService) -> AppResult<PublicModelProtocol> {
    match service.inference.as_ref().map(|i| i.wire_protocol) {
        Some(InferenceWireProtocol::OpenaiResponses) => Ok(PublicModelProtocol::OpenAiResponses),
        Some(InferenceWireProtocol::OpenaiCompletions) => {
            Ok(PublicModelProtocol::OpenAiCompletions)
        }
        Some(InferenceWireProtocol::AnthropicMessages) => {
            Ok(PublicModelProtocol::AnthropicMessages)
        }
        None => Err(denied()),
    }
}
/// Inference metadata declares these protocol operations independently of Tool
/// publication. Durable endpoints, when selected, retain their generation fence.
pub(crate) fn declared_operation(protocol: PublicModelProtocol) -> &'static str {
    match protocol {
        PublicModelProtocol::OpenAiResponses => "responses",
        PublicModelProtocol::OpenAiCompletions => "chat_completions",
        PublicModelProtocol::AnthropicMessages => "messages",
    }
}
pub(crate) fn inference_path(
    service: &DownstreamService,
    protocol: PublicModelProtocol,
) -> AppResult<String> {
    // Catalog inference bases are API roots (including /v1 when required).
    // This matches platform_inference_service; never append another /v1.
    let base = reqwest::Url::parse(&service.base_url).map_err(|_| denied())?;
    if base.query().is_some() || base.fragment().is_some() {
        return Err(denied());
    }
    Ok(match protocol {
        PublicModelProtocol::OpenAiResponses => "responses",
        PublicModelProtocol::OpenAiCompletions => "chat/completions",
        PublicModelProtocol::AnthropicMessages => "messages",
    }
    .into())
}
pub(crate) fn require_model_catalog(
    service: &DownstreamService,
    context: &PublicExecutionContext,
    operation: &PublicModelOperation,
) -> AppResult<()> {
    require_model_catalog_for_sponsor(service, &context.sponsor_owner_user_id, operation)
}
pub(crate) fn require_model_catalog_for_sponsor(
    service: &DownstreamService,
    sponsor: &str,
    operation: &PublicModelOperation,
) -> AppResult<()> {
    if service.id != operation.service_id
        || !service.is_active
        || service.service_type != "http"
        || service.requires_user_credential
        || service.provider_config_id.is_some()
        || !service.destination_targets.is_empty()
        || !matches!(service.auth_method.as_str(), "bearer" | "header")
        || protocol(service)? != operation.protocol
        || catalog_digest(service)? != operation.execution_authority_digest
        || operation.method != Method::POST
        || operation.canonical_path != inference_path(service, operation.protocol)?
        || (operation.endpoint_generation.is_none()
            && (service.offering_kind
                != crate::models::downstream_service::OfferingKind::AiService
                || operation.operation != declared_operation(operation.protocol)))
        || operation.endpoint_generation.is_some_and(|n| n <= 0)
        || operation
            .canonical_path
            .chars()
            .any(|c| matches!(c, '?' | '#' | '%' | '\\'))
    {
        return Err(denied());
    }
    if let Some(config) = &service.platform_key {
        if !config.enabled
            || (config.audience == PlatformKeyAudience::Restricted
                && !config.allowed_owner_ids.iter().any(|id| id == sponsor))
        {
            return Err(denied());
        }
    } else if !super::platform_key_service::legacy_public_master(service) {
        return Err(denied());
    }
    Ok(())
}
async fn fence_catalog(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    operation: &PublicModelOperation,
) -> AppResult<()> {
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&operation.service_id})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    require_model_catalog(&service, context, operation)?;
    db.collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"_id":&service.id},
            doc! {"$inc":{"public_dispatch_fence":1_i64}},
        )
        .session(&mut *session)
        .await?;
    if operation.endpoint_generation.is_none() {
        return Ok(());
    }
    if db.collection::<ServiceEndpoint>(ENDPOINTS).update_one(doc! {"service_id":&service.id,"name":&operation.operation,
        "method":operation.method.as_str(),"path":format!("/{}",operation.canonical_path.trim_start_matches('/')),
        "operation_generation":operation.endpoint_generation.ok_or_else(denied)?,"is_active":true,"publication":"published",
        "target_id":bson::Bson::Null},doc! {"$inc":{"public_dispatch_fence":1_i64}}).session(session).await?.matched_count!=1 {return Err(denied());}
    Ok(())
}

fn fixed_tools(protocol: PublicModelProtocol) -> AppResult<Value> {
    let definitions = super::platform_public_tools::definitions();
    let tools = definitions["tools"].as_array().ok_or_else(denied)?;
    Ok(Value::Array(tools.iter().map(|tool|match protocol {
        PublicModelProtocol::OpenAiResponses=>json!({"type":"function","name":tool["name"],"description":tool["description"],"parameters":tool["inputSchema"]}),
        PublicModelProtocol::OpenAiCompletions=>json!({"type":"function","function":{"name":tool["name"],"description":tool["description"],"parameters":tool["inputSchema"]}}),
        PublicModelProtocol::AnthropicMessages=>json!({"name":tool["name"],"description":tool["description"],"input_schema":tool["inputSchema"]}),
    }).collect()))
}
fn text_content(value: &Value) -> bool {
    value.is_string()
        || value.as_array().is_some_and(|parts| {
            !parts.is_empty()
                && parts.iter().all(|p| {
                    p.as_object().is_some_and(|p| {
                        p.len() == 2
                            && p.get("type")
                                .and_then(Value::as_str)
                                .is_some_and(|t| matches!(t, "text" | "input_text" | "output_text"))
                            && p.get("text").is_some_and(Value::is_string)
                    })
                })
        })
}
fn fields(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.keys().all(|key| allowed.contains(&key.as_str())))
}
fn handle(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 256
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    })
}
fn fixed_call(name: &Value, arguments: Value) -> bool {
    super::platform_public_tools::parse_call(json!({"name":name,"arguments":arguments})).is_ok()
}
fn responses_input(value: &Value) -> bool {
    value.is_string()
        || value.as_array().is_some_and(|items| {
            !items.is_empty()
                && items.iter().all(|v| match v["type"].as_str() {
                    Some("reasoning") => responses_reasoning_item(v),
                    Some("function_call") => {
                        fields(v, &["type", "call_id", "name", "arguments"])
                            && handle(&v["call_id"])
                            && v["arguments"].as_str().is_some_and(|args| {
                                serde_json::from_str(args)
                                    .is_ok_and(|args| fixed_call(&v["name"], args))
                            })
                    }
                    Some("function_call_output") => {
                        fields(v, &["type", "call_id", "output"])
                            && handle(&v["call_id"])
                            && v["output"].is_string()
                    }
                    Some("message") | None => {
                        fields(v, &["type", "role", "content"])
                            && v["role"].as_str().is_some_and(|r| {
                                matches!(r, "system" | "developer" | "user" | "assistant")
                            })
                            && text_content(&v["content"])
                    }
                    _ => false,
                })
        })
}
fn responses_reasoning_item(v: &Value) -> bool {
    fields(
        v,
        &[
            "type",
            "id",
            "summary",
            "content",
            "encrypted_content",
            "status",
        ],
    ) && v
        .get("id")
        .is_none_or(|id| id.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256))
        && v.get("summary").is_none_or(|s| s == &json!([]))
        && v.get("content")
            .is_none_or(|s| s.is_null() || s == &json!([]))
        && v.get("encrypted_content").is_none_or(|s| {
            s.is_null() || s.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 8192)
        })
        && v.get("status").is_none_or(|s| s == "completed")
}
fn anthropic_content(value: &Value) -> bool {
    value.is_string()
        || value.as_array().is_some_and(|items| {
            !items.is_empty()
                && items.iter().all(|v| match v["type"].as_str() {
                    Some("text") => fields(v, &["type", "text"]) && v["text"].is_string(),
                    Some("tool_use") => {
                        fields(v, &["type", "id", "name", "input"])
                            && handle(&v["id"])
                            && fixed_call(&v["name"], v["input"].clone())
                    }
                    Some("tool_result") => {
                        fields(v, &["type", "tool_use_id", "content", "is_error"])
                            && handle(&v["tool_use_id"])
                            && text_content(&v["content"])
                            && v.get("is_error").is_none_or(Value::is_boolean)
                    }
                    _ => false,
                })
        })
}
fn messages(value: &Value, protocol: PublicModelProtocol) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty()
            && items.iter().all(|v| {
                if protocol == PublicModelProtocol::AnthropicMessages {
                    return fields(v, &["role", "content"])
                        && v["role"]
                            .as_str()
                            .is_some_and(|r| matches!(r, "user" | "assistant"))
                        && anthropic_content(&v["content"]);
                }
                match v["role"].as_str() {
                    Some("tool") => {
                        fields(v, &["role", "content", "tool_call_id"])
                            && handle(&v["tool_call_id"])
                            && v["content"].is_string()
                    }
                    Some("assistant") => {
                        fields(v, &["role", "content", "tool_calls"])
                            && (v["content"].is_null() || text_content(&v["content"]))
                            && v.get("tool_calls").is_none_or(|calls| {
                                calls.as_array().is_some_and(|calls| {
                                    !calls.is_empty()
                                        && calls.iter().all(|call| {
                                            fields(call, &["id", "type", "function"])
                                                && handle(&call["id"])
                                                && call["type"] == "function"
                                                && fields(&call["function"], &["name", "arguments"])
                                                && call["function"]["arguments"]
                                                    .as_str()
                                                    .is_some_and(|args| {
                                                        serde_json::from_str(args).is_ok_and(
                                                            |args| {
                                                                fixed_call(
                                                                    &call["function"]["name"],
                                                                    args,
                                                                )
                                                            },
                                                        )
                                                    })
                                        })
                                })
                            })
                    }
                    Some("system" | "developer" | "user") => {
                        fields(v, &["role", "content"]) && text_content(&v["content"])
                    }
                    _ => false,
                }
            })
    })
}
/// Unknown fields and all native/provider builtins are denied. This initial
/// client accepts only text and the server's exact fixed tool declarations.
pub(crate) fn validate_body(operation: &PublicModelOperation, body: &[u8]) -> AppResult<Value> {
    if body.len() > operation.max_input_bytes as usize
        || body.len() > operation.max_input_tokens as usize
    {
        return Err(denied());
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| denied())?;
    let object = value.as_object().ok_or_else(denied)?;
    let (allowed, limit, input) = match operation.protocol {
        PublicModelProtocol::OpenAiResponses => (
            vec![
                "model",
                "input",
                "instructions",
                "tools",
                "max_output_tokens",
                "stream",
                "reasoning",
                "include",
                "store",
                "parallel_tool_calls",
            ],
            "max_output_tokens",
            "input",
        ),
        PublicModelProtocol::OpenAiCompletions => (
            vec![
                "model",
                "messages",
                "tools",
                "max_completion_tokens",
                "stream",
                "stream_options",
            ],
            "max_completion_tokens",
            "messages",
        ),
        PublicModelProtocol::AnthropicMessages => (
            vec![
                "model",
                "messages",
                "system",
                "tools",
                "max_tokens",
                "stream",
            ],
            "max_tokens",
            "messages",
        ),
    };
    if object.keys().any(|k| !allowed.contains(&k.as_str()))
        || value["model"].as_str() != Some(operation.model_id.as_str())
        || value[limit]
            .as_u64()
            .is_none_or(|n| n == 0 || n > u64::from(operation.max_output_tokens))
        || object.get("stream").is_some_and(|v| !v.is_boolean())
        || object.get("tools").is_some_and(|v| {
            !(fixed_tools(operation.protocol).is_ok_and(|expected| v == &expected)
                || (operation.protocol == PublicModelProtocol::OpenAiResponses
                    && is_local_compaction(&value)))
        })
        || (value["instructions"] == super::platform_public_tools::COMPACT_INSTRUCTIONS
            && !is_local_compaction(&value))
        || object.get("instructions").is_some_and(|v| !v.is_string())
        || object.get("system").is_some_and(|v| !text_content(v))
        || object
            .get("stream_options")
            .is_some_and(|v| v != &json!({"include_usage":true}))
    {
        return Err(denied());
    }
    if operation.protocol == PublicModelProtocol::OpenAiResponses {
        if value.get("store").is_some_and(|v| v != false)
            || value.get("parallel_tool_calls").is_some_and(|v| v != false)
            || value
                .get("include")
                .is_some_and(|v| v != &json!([]) && v != &json!(["reasoning.encrypted_content"]))
        {
            return Err(denied());
        }
        match value.get("reasoning") {
            Some(reasoning)
                if fields(reasoning, &["effort", "summary"])
                    && reasoning["effort"].as_str().is_some_and(|s| {
                        matches!(s, "low" | "medium" | "high" | "xhigh")
                            && operation.reasoning_effort.as_deref() == Some(s)
                    })
                    && reasoning.get("summary").is_none_or(|s| s == "none") => {}
            None if operation.reasoning_effort.is_none() => {}
            _ => return Err(denied()),
        }
    } else if operation.reasoning_effort.is_some() {
        return Err(denied());
    }
    if !(if input == "input" {
        responses_input(&value[input])
    } else {
        messages(&value[input], operation.protocol)
    }) {
        return Err(denied());
    }
    Ok(value)
}

pub(crate) fn is_local_compaction(value: &Value) -> bool {
    value["tools"] == json!([])
        && value["instructions"] == super::platform_public_tools::COMPACT_INSTRUCTIONS
        && value["input"].as_array().and_then(|items| items.last()).is_some_and(|last| {
            last == &json!({"type":"message","role":"user","content":[{"type":"input_text","text":super::platform_public_tools::COMPACT_PROMPT}]})
        })
}

struct SecretTarget(proxy_service::ProxyTarget);
impl Drop for SecretTarget {
    fn drop(&mut self) {
        self.0.credential.zeroize();
    }
}
pub(crate) fn strip_identity(target: &mut proxy_service::ProxyTarget) {
    target.service.identity_propagation_mode = "none".into();
    target.service.forward_access_token = false;
    target.service.inject_delegation_token = false;
    target.service.custom_user_agent = None;
    target.catalog_default_headers.clear();
    target.user_service_default_headers.clear();
    target.ws_frame_injections.clear();
}

pub(crate) struct ModelRequest<'a> {
    pub key_id: &'a str,
    pub service_id: &'a str,
    pub path: &'a str,
    pub body: &'a [u8],
    pub call_id: &'a str,
}
pub(crate) async fn dispatch_model(
    state: &AppState,
    context: &PublicExecutionContext,
    request: ModelRequest<'_>,
) -> AppResult<Response> {
    let ModelRequest {
        key_id,
        service_id,
        path,
        body,
        call_id,
    } = request;
    if uuid::Uuid::parse_str(call_id).is_err() {
        return Err(denied());
    }
    let proof = policy::verified_context(&state.db, context).await?;
    let mut matches = context.model_operations.iter().filter(|o| {
        o.service_id == service_id && o.method == Method::POST && o.canonical_path == path
    });
    let operation = matches.next().ok_or_else(denied)?.clone();
    if matches.next().is_some() {
        return Err(denied());
    }
    let validated = validate_body(&operation, body)?;
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":service_id})
        .await?
        .ok_or_else(denied)?;
    require_model_catalog(&service, context, &operation)?;
    // Shared live ACL precedes decryption, without owner connection resolution.
    let mut target = SecretTarget(
        proxy_service::resolve_catalog_platform_target(
            &state.db,
            &state.encryption_keys,
            &context.sponsor_owner_user_id,
            service,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(2, 4),
        )
        .await?,
    );
    strip_identity(&mut target.0);
    let call_id = call_id.to_owned();
    let mut quote =
        budget::quote_operation(state, &proof, context, key_id, &operation, &call_id).await?;
    quote.set_continuation(is_local_compaction(&validated));
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db.clone(),context.clone(),quote.clone()),|session,(db,context,quote)|Box::pin(async move {
        let result:AppResult<_>=async {
            let row=db.collection::<PublicBudgetLoop>(LOOPS).find_one(doc! {"public_session_id":&context.public_session_id,"turn_id":&context.turn.turn_id})
                .session(&mut *session).await?.ok_or_else(denied)?;
            budget::prepare_call_in_session(db,session,context,&row.id,quote).await?;Ok(())
        }.await;tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error)?;
    let metered = match budget::fund_call(state, &quote).await {
        Ok(metered) => metered,
        Err(error) => {
            budget::cancel_unforwarded_call(&state.db, &call_id).await?;
            return Err(error);
        }
    };
    let admitted = session
        .start_transaction()
        .and_run(
            (
                db.clone(),
                context.clone(),
                operation.clone(),
                call_id.clone(),
                key_id.to_owned(),
            ),
            |session, (db, context, operation, call, key)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        policy::verified_context_in_session(db, session, context).await?;
                        super::platform_public_tools::fence_callback_in_session(
                            db, session, context, key,
                        )
                        .await?;
                        fence_catalog(db, session, context, operation).await?;
                        budget::admit_dispatch_in_session(db, session, context, call).await?;
                        Ok(())
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error);
    if let Err(error) = admitted {
        budget::cancel_unforwarded_call(&state.db, &call_id).await?;
        return Err(error);
    }
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        Some(billing::route_inventory::BillingRoutePolicy::Metered(
            BillingIngress::Proxy,
        )),
        BillingIngress::Proxy,
    )?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        http::HeaderValue::from_static("application/json"),
    );
    let remaining = (context.expires_at - chrono::Utc::now())
        .to_std()
        .map_err(|_| denied())?;
    let exchange = async {
        let response = proxy_service::forward_request_with_extra_outbound_headers(
            proxy_service::no_redirect_http_client(),
            &target.0,
            operation.method.clone(),
            &operation.canonical_path,
            None,
            headers,
            proxy_service::ProxyBody::Buffered(Some(bytes::Bytes::copy_from_slice(body))),
            vec![],
            vec![],
            None,
            &state.token_exchange_cache,
            &state.cloud_response_cache,
            vec![],
            permit,
        )
        .await
        .map_err(|e| e.into_app_error())?;
        let status = response.status();
        let content_type = response.headers().get("content-type").cloned();
        let mut stream = response.bytes_stream();
        let mut output = Zeroizing::new(Vec::new());
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| denied())?;
            if output.len().saturating_add(chunk.len()) > operation.max_response_bytes as usize {
                return Err(denied());
            }
            output.extend_from_slice(&chunk);
        }
        Ok::<_, AppError>((status, content_type, output))
    };
    // Timeout/error after DispatchAdmitted remains ambiguous and conservatively
    // metered; never fail/release an already forwarded provider effect.
    let (status, content_type, output) = tokio::time::timeout(remaining, exchange)
        .await
        .map_err(|_| denied())??;
    let reported = usage::usage_from_body(&output, &operation.canonical_path, status.is_success())
        .ok_or_else(denied)?;
    let bytes = (body.len() + output.len()) as i64;
    let resale = quote.route().resale.as_ref().and_then(|r| {
        super::proxy_http_response::resale_usage_from_optional_reported(
            r.metric,
            Some(&reported),
            bytes,
        )
    });
    state
        .billing
        .settle(
            &metered,
            usage::platform_usage(Some(&reported), bytes, false),
            resale,
            Some(operation.model_id.clone()),
        )
        .await?;
    budget::reconcile_call(&state.db, &call_id).await?;
    validate_reported_usage(&operation, &reported)?;
    if status.is_success() {
        validate_reported_model(&operation.model_id, &output)?;
    }
    let mut response = Response::builder().status(status);
    if let Some(content_type) = content_type {
        response = response.header("content-type", content_type);
    }
    response
        .body(Body::from(output.to_vec()))
        .map_err(|_| denied())
}

/// Exact models are required for JSON and streaming responses. A response with
/// missing or alias model identity settles truthfully but cannot continue a turn.
pub(crate) fn validate_reported_model(expected: &str, body: &[u8]) -> AppResult<()> {
    fn models(value: &Value) -> Vec<&str> {
        [
            value["model"].as_str(),
            value["response"]["model"].as_str(),
            value["message"]["model"].as_str(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
    let mut observed = false;
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        for model in models(&value) {
            if model != expected {
                return Err(denied());
            }
            observed = true;
        }
    } else {
        let text = std::str::from_utf8(body).map_err(|_| denied())?;
        for line in text.lines().filter_map(|l| l.strip_prefix("data:")) {
            if line.trim() == "[DONE]" {
                continue;
            }
            let value: Value = serde_json::from_str(line.trim()).map_err(|_| denied())?;
            for model in models(&value) {
                if model != expected {
                    return Err(denied());
                }
                observed = true;
            }
        }
    }
    if !observed {
        return Err(denied());
    }
    Ok(())
}

// Called only after the truthful sponsor settlement and cap reconciliation.
pub(crate) fn validate_reported_usage(
    operation: &PublicModelOperation,
    reported: &usage::ReportedLlmUsage,
) -> AppResult<()> {
    let input = if reported.cached_tokens_included_in_prompt {
        reported.prompt_tokens
    } else {
        reported
            .prompt_tokens
            .saturating_add(reported.cached_tokens)
            .saturating_add(reported.cache_creation_tokens)
    };
    // The shared extractor synthesizes Anthropic total_tokens from uncached
    // prompt+completion; the separate cache classes still count toward input.
    if input > u64::from(operation.max_input_tokens)
        || reported.completion_tokens > u64::from(operation.max_output_tokens)
        || reported.total_tokens
            > u64::from(operation.max_input_tokens) + u64::from(operation.max_output_tokens)
        || reported.total_tokens
            != reported
                .prompt_tokens
                .saturating_add(reported.completion_tokens)
        || reported.images > 0
        || reported.audio_input_tokens > 0
        || reported.audio_output_tokens > 0
        || reported.image_input_tokens > 0
        || reported.image_output_tokens > 0
    {
        return Err(denied());
    }
    Ok(())
}

pub(crate) fn require_outer(service: &DownstreamService) -> AppResult<()> {
    if !super::assistant_nyxagent::row_contract(Some(service), None).valid()
        || service.slug != super::assistant_nyxagent::SERVICE_SLUG
        || service.service_type != "http"
        || service.service_category == "provider"
        || service.provider_config_id.is_some()
        || !service.destination_targets.is_empty()
        || service.billing.is_some()
    {
        // NoAuth outer pricing is deliberately unsupported by this bounded
        // adapter. Never fabricate a paid master route or silently drop prices.
        return Err(denied());
    }
    Ok(())
}
pub(crate) async fn load_outer_catalog(db: &Database) -> AppResult<DownstreamService> {
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"slug":super::assistant_nyxagent::SERVICE_SLUG})
        .await?
        .ok_or_else(denied)?;
    require_outer(&service)?;
    Ok(service)
}

/// Called only after the live public fence, in the same final transaction as
/// catalog validation. A committed attempt is never reusable, even without a
/// response/session handle. Cleanup authority is retained outside the TTL row.
pub(crate) async fn mark_outer_attempt_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    key: &str,
    service_id: &str,
    digest: &str,
) -> AppResult<()> {
    if db.collection::<PlatformPublicTurn>(TURNS).update_one(doc! {
        "_id":&context.turn.turn_id,"public_session_id":&context.public_session_id,
        "conversation_id":&context.turn.conversation_id,"receipt_id":&context.ingress.ingress.receipt_id,
        "claim_id":&context.ingress.ingress.claim_id,"sponsor_owner_user_id":&context.sponsor_owner_user_id,
        "policy_digest":&context.policy_digest,"credential_api_key_id":key,"reply_id":bson::Bson::Null,
        "nyxagent_service_id":service_id,"nyxagent_catalog_digest":digest,"outer_dispatch_admitted":false,
        "$expr":{"$gt":["$expires_at","$$NOW"]},
    },doc! {"$set":{"outer_dispatch_admitted":true}}).session(&mut *session).await?.matched_count!=1 {
        return Err(denied());
    }
    if db.collection::<PlatformPublicSession>(SESSIONS).update_one(doc! {
        "_id":&context.public_session_id,"credential_api_key_id":key,"active_turn.turn_id":&context.turn.turn_id,
        "active_turn.receipt_id":&context.ingress.ingress.receipt_id,"active_turn.claim_id":&context.ingress.ingress.claim_id,
    },doc! {"$set":{"upstream_may_exist":true},"$inc":{"effect_revision":1_i64}})
        .session(session).await?.matched_count!=1 {return Err(denied());}
    Ok(())
}

pub(crate) async fn dispatch_nyxagent(
    state: &AppState,
    context: &PublicExecutionContext,
    credential: &RuntimeCredential,
    request: NyxAgentRequest,
) -> AppResult<Response> {
    policy::verified_context(&state.db, context).await?;
    if request.method != Method::POST
        || request.relative_path != "v1/responses"
        || request.idempotency_key != context.turn.turn_id
    {
        return Err(denied());
    }
    let (_, key, child) =
        super::key_service::validate_api_key(&state.db, &credential.raw_key).await?;
    if child.is_some()
        || key.id != credential.api_key_id
        || key.purpose != crate::models::api_key::ApiKeyPurpose::PublicNyxbot
        || key.user_id != context.sponsor_owner_user_id
    {
        return Err(denied());
    }
    let value = request.body.ok_or_else(denied)?;
    let extension = &value["public_execution"];
    if value["model"].as_str() != Some(context.nyxagent_profile.as_str())
        || extension["contract"] != "nyxid-public-execution.v1"
        || extension["purpose"] != "public_nyxbot"
        || extension["policy_digest"].as_str() != Some(context.policy_digest.as_str())
        || extension["turn_id"].as_str() != Some(context.turn.turn_id.as_str())
        || extension["public_session_id"].as_str() != Some(context.public_session_id.as_str())
    {
        return Err(denied());
    }
    let body = Zeroizing::new(serde_json::to_vec(&value).map_err(|_| denied())?);
    if body.len() > state.config.proxy_max_body_size
        || body.len() > context.limits.max_context_bytes as usize
    {
        return Err(denied());
    }
    let retained = state
        .db
        .collection::<PlatformPublicTurn>(TURNS)
        .find_one(doc! {"_id":&context.turn.turn_id,"public_session_id":&context.public_session_id})
        .await?
        .ok_or_else(denied)?;
    let mut target = SecretTarget(
        proxy_service::resolve_admin_proxy_target(
            &state.db,
            &state.encryption_keys,
            &retained.nyxagent_service_id,
        )
        .await?,
    );
    require_outer(&target.0.service)?;
    if catalog_digest(&target.0.service)? != retained.nyxagent_catalog_digest {
        return Err(denied());
    }
    strip_identity(&mut target.0);
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (
                db.clone(),
                context.clone(),
                credential.api_key_id.clone(),
                retained,
            ),
            |session, (db, context, key, retained)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        policy::verified_context_in_session(db, session, context).await?;
                        budget::fence_context(db, session, context, key).await?;
                        let catalog = db
                            .collection::<DownstreamService>(SERVICES)
                            .find_one(doc! {"_id":&retained.nyxagent_service_id})
                            .session(&mut *session)
                            .await?
                            .ok_or_else(denied)?;
                        require_outer(&catalog)?;
                        if catalog_digest(&catalog)? != retained.nyxagent_catalog_digest {
                            return Err(denied());
                        }
                        db.collection::<DownstreamService>(SERVICES)
                            .update_one(
                                doc! {"_id":&catalog.id},
                                doc! {"$inc":{"public_dispatch_fence":1_i64}},
                            )
                            .session(&mut *session)
                            .await?;
                        mark_outer_attempt_in_session(
                            db,
                            session,
                            context,
                            key,
                            &catalog.id,
                            &retained.nyxagent_catalog_digest,
                        )
                        .await
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        request.billing_policy,
        BillingIngress::Proxy,
    )?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        http::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "accept",
        http::HeaderValue::from_static("text/event-stream"),
    );
    let assertion = super::platform_public_contract_service::execution_assertion(
        state,
        context,
        credential.raw_key.as_str(),
        &body,
    )
    .await?;
    let downstream = proxy_service::forward_request_with_extra_outbound_headers(
        proxy_service::no_redirect_http_client(),
        &target.0,
        Method::POST,
        "v1/responses",
        None,
        headers,
        proxy_service::ProxyBody::Buffered(Some(bytes::Bytes::copy_from_slice(&body))),
        vec![],
        vec![],
        None,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        vec![
            (
                "authorization".into(),
                format!("Bearer {}", credential.raw_key.as_str()),
            ),
            ("idempotency-key".into(), request.idempotency_key),
            ("x-nyxid-public-authority".into(), assertion.to_string()),
        ],
        permit,
    )
    .await
    .map_err(|e| e.into_app_error())?;
    let mut response = Response::builder().status(downstream.status());
    if let Some(content_type) = downstream.headers().get("content-type") {
        response = response.header("content-type", content_type);
    }
    // Public execution owns the strict byte and absolute deadline bounds; the
    // stream passes no other upstream headers or owner settlement machinery.
    response
        .body(Body::from_stream(
            downstream
                .bytes_stream()
                .map(|result| result.map_err(|_| policy::unavailable())),
        ))
        .map_err(|_| denied())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operation(protocol: PublicModelProtocol) -> PublicModelOperation {
        PublicModelOperation {
            reasoning_effort: None,
            service_id: "model".into(),
            operation: "generate".into(),
            method: Method::POST,
            canonical_path: match protocol {
                PublicModelProtocol::OpenAiResponses => "v1/responses",
                PublicModelProtocol::OpenAiCompletions => "v1/chat/completions",
                PublicModelProtocol::AnthropicMessages => "v1/messages",
            }
            .into(),
            protocol,
            execution_authority_digest: "pin".into(),
            endpoint_generation: Some(1),
            model_id: "pinned-model".into(),
            max_input_bytes: 32768,
            max_input_tokens: 32768,
            max_output_tokens: 100,
            max_response_bytes: 32768,
        }
    }
    fn body(protocol: PublicModelProtocol) -> Value {
        match protocol {
            PublicModelProtocol::OpenAiResponses => {
                json!({"model":"pinned-model","input":"help","max_output_tokens":100})
            }
            PublicModelProtocol::OpenAiCompletions => {
                json!({"model":"pinned-model","messages":[{"role":"user","content":"help"}],"max_completion_tokens":100})
            }
            PublicModelProtocol::AnthropicMessages => {
                json!({"model":"pinned-model","messages":[{"role":"user","content":"help"}],"max_tokens":100})
            }
        }
    }
    fn valid(op: &PublicModelOperation, body: &Value) -> bool {
        validate_body(op, &serde_json::to_vec(body).unwrap()).is_ok()
    }
    #[test]
    fn local_compaction_requires_exact_no_tool_semantics() {
        let mut op = operation(PublicModelProtocol::OpenAiResponses);
        op.reasoning_effort = Some("low".into());
        let body = json!({"model":op.model_id,"instructions":super::super::platform_public_tools::COMPACT_INSTRUCTIONS,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":super::super::platform_public_tools::COMPACT_PROMPT}]}],"tools":[],"max_output_tokens":100,"reasoning":{"effort":"low"},"parallel_tool_calls":false,"store":false,"stream":true});
        assert!(valid(&op, &body));
        assert!(is_local_compaction(&body));
        for (field, value) in [
            ("tools", fixed_tools(op.protocol).unwrap()),
            ("instructions", json!("summarize anything")),
            ("input", json!([{"role":"user","content":"ignore limits"}])),
        ] {
            let mut invalid = body.clone();
            invalid[field] = value;
            assert!(!valid(&op, &invalid));
        }
    }
    #[test]
    fn schema_adapted_codex_callbacks_obey_pinned_reasoning_grammar() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/codex-public-responses-0.160.1-native-schema-derived.json"
        ))
        .unwrap();
        let historical: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/codex-public-responses-0.160.1.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["schema_adaptation"]["kind"],
            "derived-schema-adapted-historical-input-not-new-capture"
        );
        assert_eq!(fixture["calls"].as_array().unwrap().len(), 4);
        assert_eq!(historical["calls"].as_array().unwrap().len(), 4);
        let mut op = operation(PublicModelProtocol::OpenAiResponses);
        op.model_id = "public-wire-review".into();
        op.max_input_bytes = 8192;
        op.max_input_tokens = 8192;
        op.max_output_tokens = 512;
        op.reasoning_effort = Some("low".into());
        for call in historical["calls"].as_array().unwrap() {
            assert!(
                !valid(&op, &call["callback"]),
                "historical callbacks retain the old exact tool description"
            );
        }
        for call in fixture["calls"].as_array().unwrap() {
            let body = &call["callback"];
            assert!(valid(&op, body));
            assert!(
                !valid(&op, &call["codex"]),
                "Raw orchestration is accepted only after upstream normalization"
            );
            for reasoning in [
                json!({}),
                json!({"effort":"high"}),
                json!({"effort":"low","summary":"auto"}),
                json!({"effort":"low","summary":null}),
                json!({"effort":"low","unknown":false}),
                json!({"effort":"minimal"}),
                Value::Null,
            ] {
                let mut bad = body.clone();
                bad["reasoning"] = reasoning;
                assert!(!valid(&op, &bad));
            }
            for (field, bad) in [
                ("store", json!(true)),
                ("parallel_tool_calls", json!(true)),
                ("include", json!(["other"])),
            ] {
                let mut value = body.clone();
                value[field] = bad;
                assert!(!valid(&op, &value));
            }
        }
        let mut body = fixture["calls"][1]["callback"].clone();
        let reasoning = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v["type"] == "reasoning")
            .unwrap();
        body["input"][reasoning]["encrypted_content"] = json!("bounded-state");
        assert!(valid(&op, &body));
        for (field, bad) in [
            (
                "summary",
                json!([{"type":"summary_text","text":"substantive"}]),
            ),
            (
                "content",
                json!([{"type":"reasoning_text","text":"substantive"}]),
            ),
            ("encrypted_content", json!("x".repeat(8193))),
            ("unknown", json!(true)),
        ] {
            let mut value = body.clone();
            value["input"][reasoning][field] = bad;
            assert!(!valid(&op, &value));
        }
    }
    #[test]
    fn model_identity_is_exact_in_json_and_streaming_responses() {
        for body in [
            r#"{"model":"pinned-model","output":[]}"#,
            "event: response.created\ndata: {\"response\":{\"model\":\"pinned-model\"}}\n\ndata: [DONE]\n",
            "data: {\"message\":{\"model\":\"pinned-model\"}}\n\ndata: {\"type\":\"content_block_delta\"}\n",
        ] {
            validate_reported_model("pinned-model", body.as_bytes()).unwrap();
        }
        for body in [
            r#"{"model":"alias-model"}"#,
            r#"{"output":[]}"#,
            "data: {\"model\":\"pinned-model\"}\n\ndata: {\"response\":{\"model\":\"different\"}}\n",
            "data: {malformed}\n",
            "data: [DONE]\n",
        ] {
            assert!(validate_reported_model("pinned-model", body.as_bytes()).is_err());
        }
    }
    #[test]
    fn all_model_protocols_require_exact_model_bounds_and_closed_tools() {
        for protocol in [
            PublicModelProtocol::OpenAiResponses,
            PublicModelProtocol::OpenAiCompletions,
            PublicModelProtocol::AnthropicMessages,
        ] {
            let op = operation(protocol);
            let original = body(protocol);
            assert!(valid(&op, &original));
            let mut value = original.clone();
            value["tools"] = fixed_tools(protocol).unwrap();
            assert!(valid(&op, &value));
            for (field, bad) in [
                ("model", json!("other")),
                ("base_url", json!("https://other.invalid")),
                ("tools", json!([{"type":"web_search"}])),
                ("store", json!(true)),
                ("metadata", json!({"payer":"other"})),
            ] {
                let mut value = original.clone();
                value[field] = bad;
                assert!(!valid(&op, &value), "{field} {protocol:?}");
            }
            let limit = match protocol {
                PublicModelProtocol::OpenAiResponses => "max_output_tokens",
                PublicModelProtocol::OpenAiCompletions => "max_completion_tokens",
                PublicModelProtocol::AnthropicMessages => "max_tokens",
            };
            for bad in [Value::Null, json!(0), json!(101), json!(-1), json!("100")] {
                let mut value = original.clone();
                value[limit] = bad;
                assert!(!valid(&op, &value));
            }
        }
    }
    #[test]
    fn paired_call_and_output_identity_grammar_is_closed() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/public-call-identities-v1.json"
        ))
        .unwrap();
        let op = operation(PublicModelProtocol::OpenAiResponses);
        let mut value = body(op.protocol);
        let call = json!({"type":"function_call","name":"public_faq_search","arguments":"{\"query\":\"NyxID\"}"});
        let output = json!({"type":"function_call_output","output":"public help"});
        let mut accepted = fixture["accepted"].as_array().unwrap().clone();
        accepted.push(json!("a".repeat(256)));
        for id in accepted {
            let mut c = call.clone();
            c["call_id"] = id.clone();
            let mut o = output.clone();
            o["call_id"] = id.clone();
            value["input"] = json!([c, o]);
            assert!(valid(&op, &value));
            assert_eq!(value["input"][0]["call_id"], id);
            assert_eq!(value["input"][1]["call_id"], id);
        }
        let mut rejected = fixture["rejected"].as_array().unwrap().clone();
        rejected.push(json!({"call_id":"a".repeat(257)}));
        for invalid in rejected {
            for item in [&call, &output] {
                let mut item = item.clone();
                item.as_object_mut()
                    .unwrap()
                    .extend(invalid.as_object().unwrap().clone());
                value["input"] = json!([item]);
                assert!(!valid(&op, &value), "unsupported call identity");
            }
        }
    }
    #[test]
    fn only_closed_function_round_trips_are_accepted_and_multimodal_is_denied() {
        let op = operation(PublicModelProtocol::OpenAiResponses);
        let mut value = body(op.protocol);
        value["input"] = json!([
            {"type":"function_call","call_id":"call_1","name":"public_faq_search","arguments":"{\"query\":\"NyxID\"}"},
            {"type":"function_call_output","call_id":"call_1","output":"public help"},
        ]);
        assert!(valid(&op, &value));
        value["input"][0]["name"] = json!("nyx__call_tool");
        assert!(!valid(&op, &value));
        value["input"] = json!([{"role":"user","content":[{"type":"input_image","image_url":"https://other.invalid"}]}]);
        assert!(!valid(&op, &value));
        let op = operation(PublicModelProtocol::OpenAiCompletions);
        let mut value = body(op.protocol);
        value["messages"] = json!([
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"public_signup_link","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call_1","content":"unavailable"},
        ]);
        assert!(valid(&op, &value));
        let op = operation(PublicModelProtocol::AnthropicMessages);
        let mut value = body(op.protocol);
        value["messages"] = json!([
            {"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"public_faq_read","input":{"article_id":"overview"}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"public help"}]},
        ]);
        assert!(valid(&op, &value));
        value["messages"][0]["content"][0]["name"] = json!("shell");
        assert!(!valid(&op, &value));
    }
    #[test]
    fn usage_bounds_include_separate_cache_and_reject_inconsistent_total_after_settlement() {
        let operation = operation(PublicModelProtocol::OpenAiResponses);
        let mut usage = usage::ReportedLlmUsage {
            prompt_tokens: 5,
            completion_tokens: 3,
            total_tokens: 8,
            cached_tokens_included_in_prompt: true,
            ..Default::default()
        };
        assert!(validate_reported_usage(&operation, &usage).is_ok());
        usage.total_tokens =
            u64::from(operation.max_input_tokens) + u64::from(operation.max_output_tokens) + 1;
        assert!(validate_reported_usage(&operation, &usage).is_err());
        usage.total_tokens = 7;
        assert!(validate_reported_usage(&operation, &usage).is_err());
        usage.cached_tokens_included_in_prompt = false;
        usage.cached_tokens = 2;
        usage.cache_creation_tokens = 4;
        usage.total_tokens = 8;
        assert!(validate_reported_usage(&operation, &usage).is_ok());
        let anthropic = usage::extract_reported_usage(&json!({"usage": {
            "input_tokens":5,"output_tokens":3,"cache_read_input_tokens":2,"cache_creation_input_tokens":4,
        }})).unwrap();
        assert!(validate_reported_usage(&operation, &anthropic).is_ok());
        usage.completion_tokens = u64::from(operation.max_output_tokens) + 1;
        usage.total_tokens = usage.prompt_tokens + usage.completion_tokens;
        assert!(validate_reported_usage(&operation, &usage).is_err());
    }
    #[test]
    fn outer_is_noauth_unpriced_and_catalog_pin_covers_target_and_credentials() {
        let mut service = crate::test_utils::test_auto_connected_catalog_service();
        service.slug = super::super::assistant_nyxagent::SERVICE_SLUG.into();
        service.forward_access_token = true;
        assert!(require_outer(&service).is_ok());
        let pin = catalog_digest(&service).unwrap();
        service.base_url = "https://changed.invalid".into();
        assert_ne!(pin, catalog_digest(&service).unwrap());
        service.auth_method = "bearer".into();
        assert!(require_outer(&service).is_err());
        service.auth_method = "none".into();
        service.billing = Some(serde_json::from_value(json!({})).unwrap());
        assert!(require_outer(&service).is_err());
    }
    #[tokio::test]
    async fn outer_attempt_and_unknown_upstream_evidence_commit_together_and_never_replay() {
        let db = crate::test_utils::connect_transaction_test_database("public_outer_commit").await;
        let (mut row, _, context) =
            super::super::platform_public_credential_service::tests::fixture();
        row.active_turn = Some(crate::models::platform_public_session::PublicTurnBinding {
            conversation_id: context.turn.conversation_id.clone(),
            turn_id: context.turn.turn_id.clone(),
            receipt_id: context.ingress.ingress.receipt_id.clone(),
            claim_id: context.ingress.ingress.claim_id.clone(),
            stop_requested: false,
            expires_at: context.expires_at,
        });
        db.collection::<PlatformPublicSession>(SESSIONS)
            .insert_one(&row)
            .await
            .unwrap();
        db.collection::<bson::Document>(TURNS).insert_one(doc! {
            "_id":&context.turn.turn_id,"public_session_id":&row.id,"conversation_id":&context.turn.conversation_id,
            "receipt_id":&context.ingress.ingress.receipt_id,"claim_id":&context.ingress.ingress.claim_id,
            "sponsor_owner_user_id":&row.sponsor_owner_user_id,"policy_digest":&row.policy_digest,
            "credential_api_key_id":&row.credential_api_key_id,"reply_id":bson::Bson::Null,
            "nyxagent_service_id":"outer","nyxagent_catalog_digest":"pin","outer_dispatch_admitted":false,
            "expires_at":bson::DateTime::from_chrono(row.expires_at),
        }).await.unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        mark_outer_attempt_in_session(
            &db,
            &mut session,
            &context,
            &row.credential_api_key_id,
            "outer",
            "pin",
        )
        .await
        .unwrap();
        session.abort_transaction().await.unwrap();
        assert!(
            !db.collection::<PlatformPublicSession>(SESSIONS)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap()
                .upstream_may_exist
        );
        session.start_transaction().await.unwrap();
        let mut stale = context.clone();
        stale.ingress.ingress.claim_id = "other".into();
        assert!(
            mark_outer_attempt_in_session(
                &db,
                &mut session,
                &stale,
                &row.credential_api_key_id,
                "outer",
                "pin"
            )
            .await
            .is_err()
        );
        session.abort_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        mark_outer_attempt_in_session(
            &db,
            &mut session,
            &context,
            &row.credential_api_key_id,
            "outer",
            "pin",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let saved = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.upstream_may_exist);
        assert!(saved.upstream_binding_ciphertext.is_empty());
        session.start_transaction().await.unwrap();
        assert!(
            mark_outer_attempt_in_session(
                &db,
                &mut session,
                &context,
                &row.credential_api_key_id,
                "outer",
                "pin"
            )
            .await
            .is_err()
        );
        session.abort_transaction().await.unwrap();
        db.drop().await.unwrap();
    }
}
