//! Bounded, tool-less text inference for server-owned assistant metadata tasks.
//! Never accepts an agent key, session, tool definitions, or arbitrary request JSON.
//! Service order is platform-available first, then the person's own keys through
//! the shared resolver. Preferring small advertised models is a heuristic, not
//! policy. Only the chosen model ID is cached; authority is resolved on every call.
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use futures::{StreamExt, TryStreamExt};
use mongodb::bson::doc;
use serde_json::{Value, json};
use zeroize::Zeroize;

use super::{billing, delegation_service, llm_usage_service, platform_key_service, proxy_service};
use crate::models::{
    downstream_service::{COLLECTION_NAME, DownstreamService, InferenceWireProtocol},
    service_billing::{BillingMetric, PlatformUsage, ResaleUsage},
    usage_meter::CredentialClass,
};
use crate::{AppState, errors::AppResult};

const MODEL_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const MODEL_CACHE_CAPACITY: usize = 128;
static MODEL_CACHE: LazyLock<Mutex<ModelCache>> = LazyLock::new(Mutex::default);

#[derive(Default)]
struct ModelCache {
    // No credentials, actor identity, ACL results, grants, or provider responses.
    entries: HashMap<(String, CredentialClass), (String, Instant)>,
}

impl ModelCache {
    fn get(&mut self, service: &str, class: CredentialClass, now: Instant) -> Option<String> {
        self.entries
            .retain(|_, (_, inserted)| now.duration_since(*inserted) < MODEL_CACHE_TTL);
        self.entries
            .get(&(service.to_owned(), class))
            .map(|(model, _)| model.clone())
    }

    fn insert(&mut self, service: &str, class: CredentialClass, model: &str, now: Instant) {
        self.entries
            .retain(|_, (_, inserted)| now.duration_since(*inserted) < MODEL_CACHE_TTL);
        let key = (service.to_owned(), class);
        if !self.entries.contains_key(&key)
            && self.entries.len() >= MODEL_CACHE_CAPACITY
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, inserted))| *inserted)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, (model.to_owned(), now));
    }

    fn invalidate(&mut self, service: &str, class: CredentialClass, failed_model: &str) {
        let key = (service.to_owned(), class);
        // A concurrent discovery may already have selected a different model.
        if self
            .entries
            .get(&key)
            .is_some_and(|(model, _)| model == failed_model)
        {
            self.entries.remove(&key);
        }
    }
}

/// Caller-supplied bounds are additionally capped by this helper. Invalid/oversized
/// input fails closed instead of silently changing a classifier's input.
#[derive(Clone, Copy)]
pub(crate) struct TextLimits {
    pub max_input_chars: usize,
    pub max_output_chars: usize,
    pub max_output_tokens: u16,
    pub timeout: Duration,
}

pub(crate) async fn one_shot_text(
    state: &AppState,
    actor: &str,
    prompt: &str,
    input: &str,
    limits: TextLimits,
) -> Option<String> {
    if limits.timeout.is_zero()
        || limits.max_output_tokens == 0
        || limits.max_output_chars == 0
        || prompt.chars().count() > 4_000
        || input.chars().count() > limits.max_input_chars.min(16_000)
    {
        return None;
    }
    tokio::time::timeout(
        limits.timeout.min(Duration::from_secs(15)),
        Box::pin(infer(state, actor, prompt, input, limits)),
    )
    .await
    .ok()?
    .ok()
    .flatten()
}

struct Route {
    target: proxy_service::ProxyTarget,
    delegated: Vec<delegation_service::DelegatedCredential>,
    class: CredentialClass,
}

impl Drop for Route {
    fn drop(&mut self) {
        self.target.credential.zeroize();
        for credential in &mut self.delegated {
            credential.credential.zeroize();
        }
    }
}

impl Route {
    fn reflects_credential(&self, text: &str) -> bool {
        std::iter::once(self.target.credential.as_str())
            .chain(
                self.delegated
                    .iter()
                    .map(|credential| credential.credential.as_str()),
            )
            .any(|secret| !secret.is_empty() && text.contains(secret))
    }
}

async fn resolve(state: &AppState, actor: &str, service: &DownstreamService) -> AppResult<Route> {
    // These are the LLM gateway's authoritative resolvers, including personal
    // binding precedence, live platform ACL before decryption, and org role/scope.
    let resolution = Box::pin(proxy_service::resolve_proxy_target_from_user_service(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        actor,
        None,
        Some(&service.id),
        proxy_service::ProxyExecutionContext::new(
            Some(&state.connection_expiry_notifier),
            state.platform_user_rate_limit,
        ),
    ))
    .await?;
    if let Some(resolution) = resolution {
        let class = super::llm_gateway_service::credential_class(
            true,
            resolution.master_credential,
            resolution.credential_source.as_deref(),
            &resolution.target,
        );
        return Ok(Route {
            target: resolution.target,
            delegated: vec![],
            class,
        });
    }
    proxy_service::guard_slug_against_viewer_orgs(&state.db, actor, None, Some(&service.id))
        .await?;
    let target = Box::pin(proxy_service::resolve_proxy_target(
        &state.db,
        &state.encryption_keys,
        actor,
        &service.id,
        state.platform_user_rate_limit,
    ))
    .await?;
    // Platform resolution has already materialized the catalog credential. A
    // legacy personal provider connection uses the gateway's delegation helper.
    let delegated = if proxy_service::uses_server_held_master(&target) {
        vec![]
    } else {
        delegation_service::resolve_delegated_credentials(
            &state.db,
            &state.encryption_keys,
            actor,
            &service.id,
            Some(&state.connection_expiry_notifier),
        )
        .await?
    };
    let class = if !delegated.is_empty() {
        CredentialClass::UserOwned
    } else {
        super::llm_gateway_service::credential_class(false, false, None, &target)
    };
    Ok(Route {
        target,
        delegated,
        class,
    })
}

async fn infer(
    state: &AppState,
    actor: &str,
    prompt: &str,
    input: &str,
    limits: TextLimits,
) -> AppResult<Option<String>> {
    use crate::models::user::{User, UserType};
    let Some(person) = state
        .db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":actor,"is_active":true})
        .await?
    else {
        return Ok(None);
    };
    if person.user_type != UserType::Person {
        return Ok(None);
    }
    let mut services: Vec<DownstreamService> = state
        .db
        .collection(COLLECTION_NAME)
        .find(
            doc! {"is_active":true,"service_type":"http","inference.model_list":true,
            // NyxAgent is an agent runtime, even for store:false. Never call it.
            "slug":{"$ne":"llm-nyx"}},
        )
        .sort(doc! {"slug":1})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    let grants = platform_key_service::OwnerGrants::load(&state.db, actor).await?;
    let providers = platform_key_service::load_providers(&state.db).await?;
    services.sort_by_key(|service| {
        !platform_key_service::available_with_grants(
            service,
            service
                .provider_config_id
                .as_ref()
                .and_then(|id| providers.get(id)),
            actor,
            &grants,
        )
    });
    // No redirected credentialed requests; a model endpoint is not a browser.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(|_| crate::errors::AppError::Internal("Inference client unavailable".into()))?;
    for service in services {
        let Some(inference) = service.inference.as_ref() else {
            continue;
        };
        let Ok(route) = Box::pin(resolve(state, actor, &service)).await else {
            continue;
        };
        // Only conventional provider API authentication. In particular, no
        // NyxID token injection, body rewriting, or agent/gateway credentials.
        if !matches!(
            route.target.auth_method.as_str(),
            "none" | "bearer" | "header" | "query"
        ) || route
            .delegated
            .iter()
            .any(|c| !matches!(c.injection_method.as_str(), "bearer" | "header" | "query"))
        {
            continue;
        }
        // Lookup follows fresh credential resolution and its live ACL checks.
        // Never hold this short synchronous lock across network/database awaits.
        let cached_model = MODEL_CACHE
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&service.id, route.class, Instant::now()));
        let model = if let Some(model) = cached_model {
            model
        } else {
            let models = Box::pin(request(
                state, actor, &client, &service, &route, "models", None, None,
            ));
            let Ok(Some(models)) = tokio::time::timeout(Duration::from_secs(3), models).await
            else {
                continue;
            };
            let Some(model) = choose_model(&models) else {
                continue;
            };
            // Do not persist credential reflections even in the model-ID cache.
            if route.reflects_credential(&model) {
                continue;
            }
            if let Ok(mut cache) = MODEL_CACHE.lock() {
                cache.insert(&service.id, route.class, &model, Instant::now());
            }
            model
        };
        if route.reflects_credential(&model) {
            continue;
        }
        let (path, body) = text_request(
            inference.wire_protocol,
            &model,
            prompt,
            input,
            limits.max_output_tokens.min(1_024),
        );
        let Some(value) = Box::pin(request(
            state,
            actor,
            &client,
            &service,
            &route,
            path,
            Some(body),
            Some(&model),
        ))
        .await
        else {
            return Ok(None); // No second paid inference or retry after dispatch.
        };
        let text = output_text(inference.wire_protocol, &value);
        if text.is_empty()
            || text.chars().count() > limits.max_output_chars.min(8_000)
            || route.reflects_credential(&text)
        {
            return Ok(None);
        }
        return Ok(Some(text));
    }
    Ok(None)
}

fn choose_model(value: &Value) -> Option<String> {
    let mut models: Vec<&str> = value["data"]
        .as_array()?
        .iter()
        .take(1_000)
        .filter_map(|model| model["id"].as_str())
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        .filter(|id| {
            ![
                "embedding",
                "embed-",
                "whisper",
                "tts",
                "audio",
                "realtime",
                "image",
                "dall-e",
                "moderation",
                "rerank",
                "transcrib",
            ]
            .iter()
            .any(|kind| id.to_ascii_lowercase().contains(kind))
        })
        .collect();
    // Prefer small/fast families advertised by this service, then any advertised
    // text model. Never guess a vendor model or reuse an agent model alias.
    models.sort_by_key(|id| {
        let lower = id.to_ascii_lowercase();
        (
            ["nano", "mini", "haiku", "flash", "small", "fast"]
                .iter()
                .position(|hint| lower.contains(hint))
                .unwrap_or(6),
            *id,
        )
    });
    models.first().map(|model| (*model).to_owned())
}

fn text_request(
    protocol: InferenceWireProtocol,
    model: &str,
    prompt: &str,
    input: &str,
    tokens: u16,
) -> (&'static str, Value) {
    use InferenceWireProtocol::*;
    match protocol {
        OpenaiResponses => (
            "responses",
            json!({"model":model,"instructions":prompt,"input":input,
            "stream":false,"store":false,"max_output_tokens":tokens}),
        ),
        OpenaiCompletions => {
            let mut body = json!({"model":model,"messages":[{"role":"system","content":prompt},{"role":"user","content":input}],
                "stream":false});
            let token_field = if model.starts_with("gpt-")
                || model.starts_with("o1")
                || model.starts_with("o3")
                || model.starts_with("o4")
            {
                "max_completion_tokens"
            } else {
                "max_tokens"
            };
            body[token_field] = json!(tokens);
            ("chat/completions", body)
        }
        AnthropicMessages => (
            "messages",
            json!({"model":model,"system":prompt,
            "messages":[{"role":"user","content":input}],"stream":false,"max_tokens":tokens}),
        ),
    }
}

fn output_text(protocol: InferenceWireProtocol, value: &Value) -> String {
    use InferenceWireProtocol::*;
    match protocol {
        OpenaiCompletions => {
            let message = &value["choices"][0]["message"];
            if message.get("tool_calls").is_some() || message.get("function_call").is_some() {
                return String::new();
            }
            message["content"].as_str().unwrap_or_default().to_owned()
        }
        OpenaiResponses => {
            let Some(output) = value["output"].as_array() else {
                return String::new();
            };
            let mut text = Vec::new();
            for item in output {
                // Responses reasoning metadata can accompany a plain-text
                // answer. Never surface it; only message text is usable output.
                if item["type"] == "reasoning" {
                    continue;
                }
                if item["type"] != "message" || item["role"] != "assistant" {
                    return String::new();
                }
                let Some(parts) = text_parts(&item["content"], "output_text") else {
                    return String::new();
                };
                text.extend(parts);
            }
            text.join(" ")
        }
        AnthropicMessages => text_parts(&value["content"], "text")
            .map(|parts| parts.join(" "))
            .unwrap_or_default(),
    }
}

fn text_parts<'a>(value: &'a Value, kind: &str) -> Option<Vec<&'a str>> {
    value
        .as_array()?
        .iter()
        .map(|part| {
            if part["type"] == kind {
                part["text"].as_str()
            } else {
                None
            }
        })
        .collect()
}

// A deadline can drop the request at any await, including after provider dispatch.
// Keep ordinary billing recovery semantics: release unsent reservations, settle
// forwarded requests using the gateway's usage/byte estimate if no reply arrived.
struct MeterGuard {
    billing: Arc<billing::BillingService>,
    metered: billing::MeteredProxyContext,
    forwarded: bool,
    usage: PlatformUsage,
    model: Option<String>,
    armed: bool,
}
impl MeterGuard {
    async fn finish(&self) -> AppResult<()> {
        if !self.forwarded {
            return self
                .billing
                .fail(&self.metered, "oneshot_cancelled_before_dispatch")
                .await;
        }
        let resale = self
            .metered
            .route
            .as_ref()
            .and_then(|route| route.resale.as_ref())
            .and_then(|spec| {
                let quantity = match spec.metric {
                    BillingMetric::Tokens => self.usage.tokens,
                    BillingMetric::Requests => 1,
                    BillingMetric::Bytes => self.usage.bytes,
                    _ => return None,
                };
                Some(ResaleUsage {
                    metric: spec.metric,
                    quantity,
                })
            });
        self.billing
            .settle_deferred(
                &self.metered,
                self.usage.clone(),
                resale,
                self.model.clone(),
            )
            .await
    }
}
impl Drop for MeterGuard {
    fn drop(&mut self) {
        if !self.armed || !self.metered.is_enabled() {
            return;
        }
        let guard = Self {
            billing: self.billing.clone(),
            metered: self.metered.clone(),
            forwarded: self.forwarded,
            usage: self.usage.clone(),
            model: self.model.clone(),
            armed: false,
        };
        tokio::spawn(async move {
            let _ = Box::pin(guard.finish()).await;
        });
    }
}

#[allow(clippy::too_many_arguments)]
async fn request(
    state: &AppState,
    actor: &str,
    client: &reqwest::Client,
    service: &DownstreamService,
    route: &Route,
    path: &str,
    body: Option<Value>,
    model: Option<&str>,
) -> Option<Value> {
    let body = body
        .and_then(|body| serde_json::to_vec(&body).ok())
        .map(bytes::Bytes::from);
    let request_len = body.as_ref().map_or(0, |body| body.len() as i64);
    // The metadata task belongs to the acting person even for an org agent.
    // Credential access was separately established above by the shared resolver.
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_execution(actor, actor, route.class)
        .await
        .ok()?;
    let ctx = billing::BillingRouteContext::new(
        billing::BillingIngress::LlmProvider,
        uuid::Uuid::new_v4().to_string(),
        owner.owner_id,
        actor.to_owned(),
        None,
        None,
        Some(service.id.clone()),
        Some(service.slug.clone()),
        billing::NodeIntent::Direct,
        route.target.auth_method.clone(),
        route.class,
        if body.is_some() {
            BillingMetric::Tokens
        } else {
            BillingMetric::Requests
        },
        route
            .target
            .service
            .billing
            .as_ref()
            .or(service.billing.as_ref()),
        state.billing.resale_enabled(),
    )
    .with_request_body(body.as_deref());
    let metered = Box::pin(state.billing.open(&ctx)).await.ok()?;
    let mut guard = MeterGuard {
        billing: state.billing.clone(),
        metered,
        forwarded: false,
        usage: llm_usage_service::platform_usage(None, request_len, model.is_some()),
        model: model.map(str::to_owned),
        armed: true,
    };
    state.billing.mark_forwarded(&guard.metered).await.ok()?;
    guard.forwarded = true;
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        Some(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::LlmProvider,
        )),
        billing::BillingIngress::LlmProvider,
    )
    .ok()?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    let mut extra_headers = vec![];
    if service.inference.as_ref().is_some_and(|inference| {
        inference.wire_protocol == InferenceWireProtocol::AnthropicMessages
    }) {
        extra_headers.push(("anthropic-version".into(), "2023-06-01".into()));
    }
    let response = Box::pin(proxy_service::forward_request_with_extra_outbound_headers(
        client,
        &route.target,
        if body.is_some() {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        },
        path,
        None,
        headers,
        proxy_service::ProxyBody::Buffered(body),
        vec![],
        route.delegated.clone(),
        None,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        extra_headers,
        permit,
    ))
    .await
    .ok()?;
    // Invalidate before reading the body: a 4xx still counts if that body is
    // malformed, oversized, or stalls. The next call rediscovers; never retry a
    // paid inference here, and never invalidate on transient 5xx/transport errors.
    if response.status().is_client_error()
        && let Some(model) = model
        && let Ok(mut cache) = MODEL_CACHE.lock()
    {
        cache.invalidate(&service.id, route.class, model);
    }
    let success = response.status().is_success();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if bytes.len() + chunk.len() > 64 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    let usage = llm_usage_service::usage_from_body(&bytes, path, success);
    guard.usage = llm_usage_service::platform_usage(
        usage.as_ref(),
        request_len + bytes.len() as i64,
        model.is_some(),
    );
    Box::pin(guard.finish()).await.ok()?;
    guard.armed = false;
    if !success {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
#[path = "assistant_oneshot_inference_tests.rs"]
mod tests;
