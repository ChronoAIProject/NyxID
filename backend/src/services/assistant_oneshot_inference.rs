//! Bounded, tool-less text inference for server-owned assistant metadata tasks.
//! Never accepts an agent key, session, tool definitions, or arbitrary request JSON.
//! Prefer the configured platform utility route. If absent or unavailable, use
//! legacy platform-first selection. Preferring small advertised models is a heuristic, not
//! policy. Only model/capability metadata is cached; authority is resolved on every call.
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
    day: Option<chrono::NaiveDate>,
}

impl ModelCache {
    fn expire_shutdown_day(&mut self) {
        let today = chrono::Utc::now().date_naive();
        if self.day != Some(today) {
            self.entries.clear();
            self.day = Some(today);
        }
    }

    fn get(&mut self, service: &str, class: CredentialClass, now: Instant) -> Option<String> {
        self.expire_shutdown_day();
        self.entries
            .retain(|_, (_, inserted)| now.duration_since(*inserted) < MODEL_CACHE_TTL);
        self.entries
            .get(&(service.to_owned(), class))
            .map(|(model, _)| model.clone())
    }

    fn insert(&mut self, service: &str, class: CredentialClass, model: &str, now: Instant) {
        self.expire_shutdown_day();
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

/// Fixed caller labels, never derived from a prompt or agent-provided input.
#[derive(Clone, Copy)]
pub(crate) enum TextCaller {
    Title,
    Learning,
    Voice,
}
impl TextCaller {
    fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Learning => "learning",
            Self::Voice => "voice",
        }
    }
}

const MAX_ATTEMPTS: usize = 3;

// Only provider capability metadata, not authority. A rejection expires so a
// provider upgrade can regain the parameter. Never cache error bodies.
type ReasoningCapabilities = HashMap<(String, String), Instant>;
static REASONING_UNSUPPORTED: LazyLock<Mutex<ReasoningCapabilities>> =
    LazyLock::new(Mutex::default);
fn reasoning_disabled(service: &str, model: &str) -> bool {
    let Ok(mut cache) = REASONING_UNSUPPORTED.lock() else {
        return false;
    };
    cache.retain(|_, inserted| inserted.elapsed() < MODEL_CACHE_TTL);
    cache.contains_key(&(service.to_owned(), model.to_owned()))
}
fn disable_reasoning(service: &str, model: &str) {
    if let Ok(mut cache) = REASONING_UNSUPPORTED.lock() {
        cache.retain(|_, inserted| inserted.elapsed() < MODEL_CACHE_TTL);
        if cache.len() >= MODEL_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert((service.to_owned(), model.to_owned()), Instant::now());
    }
}

// Drop also diagnoses outer-deadline cancellation, exactly once for this attempt.
// Neither provider errors nor any request/response content enters tracing.
struct Attempt<'a> {
    service: &'a str,
    class: CredentialClass,
    model: String,
    caller: TextCaller,
    finished: bool,
}
impl Attempt<'_> {
    fn refuse(&mut self, stage: &str) {
        tracing::warn!(service_slug = self.service, credential_class = ?self.class,
            model = self.model, caller = self.caller.as_str(), stage,
            "Assistant utility inference attempt failed");
        self.finished = true;
    }
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.refuse("timeout");
        }
    }
}

struct RequestFailure {
    stage: String,
    retryable: bool,
}
impl RequestFailure {
    fn terminal(stage: &str) -> Self {
        Self {
            stage: stage.into(),
            retryable: false,
        }
    }
}

/// Caller-supplied bounds are additionally capped by this helper. Invalid/oversized
/// input fails closed instead of silently changing a classifier's input.
#[derive(Clone, Copy)]
pub(crate) struct TextLimits {
    pub caller: TextCaller,
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
    match tokio::time::timeout(
        limits.timeout.min(Duration::from_secs(15)),
        Box::pin(infer(state, actor, prompt, input, limits)),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => {
            tracing::warn!(
                service_slug = "",
                credential_class = "unresolved",
                model = "",
                caller = limits.caller.as_str(),
                stage = "route_unavailable",
                "Assistant utility inference setup failed"
            );
            None
        }
        // The active Attempt's Drop diagnoses cancellation; no duplicate warning.
        Err(_) => None,
    }
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
    let utility = super::utility_inference_service::load(&state.db).await?;
    if let Some(config) = &utility {
        let service = state.db.collection::<DownstreamService>(COLLECTION_NAME)
            .find_one(doc! {"is_active": true, "service_type": "http", "inference.model_list": true, "slug": {"$eq": &config.service_slug, "$ne": "llm-nyx"}}).await?;
        if let Some(service) = service
            && platform_key_service::available(&state.db, &service, actor).await?
        {
            // Force the catalog platform route: personal connection precedence is
            // inappropriate for an admin-selected background utility service.
            let route = Box::pin(proxy_service::resolve_catalog_platform_target(
                &state.db,
                &state.encryption_keys,
                actor,
                service.clone(),
                state.platform_user_rate_limit,
            ))
            .await;
            let Ok(target) = route else {
                let mut attempt = Attempt {
                    service: &service.slug,
                    class: CredentialClass::NyxidManagedMaster,
                    model: String::new(),
                    caller: limits.caller,
                    finished: false,
                };
                attempt.refuse("route_unavailable");
                return Ok(None);
            };
            let route = Route {
                target,
                delegated: vec![],
                class: CredentialClass::NyxidManagedMaster,
            };
            let client = inference_client()?;
            let mut attempts = 0;
            // Once available, do not escape to another service after model-list,
            // provider, billing, or credential-materialization failures.
            return Ok(infer_route(
                state,
                actor,
                &client,
                &service,
                &route,
                Some(&config.model),
                prompt,
                input,
                limits,
                &mut attempts,
            )
            .await
            .ok());
        }
    }
    // Absent/cleared config retains the legacy service-selection path. An
    // unavailable configured platform service is excluded even if personal BYOK exists.
    let mut services: Vec<DownstreamService> = state
        .db
        .collection(COLLECTION_NAME)
        .find(
            doc! {"is_active":true,"service_type":"http","inference.model_list":true,
            "slug":{"$nin": ["llm-nyx", utility.as_ref().map_or("", |c| c.service_slug.as_str())]}},
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
    let client = inference_client()?;
    let mut attempts = 0;
    for service in services {
        if attempts >= MAX_ATTEMPTS {
            break;
        }
        let Ok(route) = Box::pin(resolve(state, actor, &service)).await else {
            continue;
        };
        match infer_route(
            state,
            actor,
            &client,
            &service,
            &route,
            None,
            prompt,
            input,
            limits,
            &mut attempts,
        )
        .await
        {
            Ok(text) => return Ok(Some(text)),
            Err(true) => continue,
            Err(false) => return Ok(None),
        }
    }
    Ok(None)
}

fn inference_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(|_| crate::errors::AppError::Internal("Inference client unavailable".into()))
}

#[allow(clippy::too_many_arguments)]
async fn infer_route(
    state: &AppState,
    actor: &str,
    client: &reqwest::Client,
    service: &DownstreamService,
    route: &Route,
    configured: Option<&str>,
    prompt: &str,
    input: &str,
    limits: TextLimits,
    attempts: &mut usize,
) -> Result<String, bool> {
    let mut attempt = Attempt {
        service: &service.slug,
        class: route.class,
        model: configured
            .filter(|id| valid_model_id(id) && !route.reflects_credential(id))
            .unwrap_or_default()
            .to_owned(),
        caller: limits.caller,
        finished: false,
    };
    *attempts += 1;
    if !matches!(
        route.target.auth_method.as_str(),
        "none" | "bearer" | "header" | "query"
    ) || route
        .delegated
        .iter()
        .any(|c| !matches!(c.injection_method.as_str(), "bearer" | "header" | "query"))
    {
        attempt.refuse("route_unavailable");
        return Err(true);
    }
    let protocol = service.inference.as_ref().ok_or(false)?.wire_protocol;
    // Utility selection verifies the live list on every call. Only the legacy
    // selection uses the short-lived chosen-ID cache; neither caches authority.
    let cached = if configured.is_none() {
        MODEL_CACHE
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&service.id, route.class, Instant::now()))
    } else {
        None
    };
    let models = if let Some(model) = cached {
        vec![model]
    } else {
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            Box::pin(request(
                state, actor, client, service, route, "models", None, None,
            )),
        )
        .await;
        let value = match response {
            Ok(Ok(value)) => value,
            _ => {
                attempt.refuse("models_unavailable");
                return Err(true);
            }
        };
        let mut models = suitable_models(&value, chrono::Utc::now().date_naive());
        if let Some(model) = configured {
            if let Some(i) = models.iter().position(|id| id == model) {
                let preferred = models.remove(i);
                models.retain(|id| reasoning_effort(id).is_none());
                models.insert(0, preferred);
            } else {
                if valid_model_id(model) && !route.reflects_credential(model) {
                    attempt.model = model.into();
                }
                attempt.refuse("no_model");
                if *attempts >= MAX_ATTEMPTS {
                    return Err(true);
                }
                *attempts += 1;
                attempt.finished = false;
                attempt.model.clear();
                models.retain(|id| reasoning_effort(id).is_none());
            }
        }
        models
    };
    if models.is_empty() {
        attempt.refuse("no_model");
        return Err(true);
    }
    for (index, model) in models.into_iter().enumerate() {
        if index > 0 {
            if configured.is_none() || *attempts >= MAX_ATTEMPTS {
                return Err(true);
            }
            *attempts += 1;
            attempt.finished = false;
        }
        // Never put a reflected secret in diagnostics, cache, or billing metadata.
        if route.reflects_credential(&model) {
            attempt.model.clear();
            attempt.refuse("credential_reflection");
            return Err(false);
        }
        attempt.model = model.clone();
        if let Ok(mut cache) = MODEL_CACHE.lock() {
            cache.insert(&service.id, route.class, &model, Instant::now());
        }
        let (path, body) = text_request(
            protocol,
            &model,
            prompt,
            input,
            limits.max_output_tokens.min(1024),
            !reasoning_disabled(&service.id, &model),
        );
        let result = Box::pin(request(
            state,
            actor,
            client,
            service,
            route,
            path,
            Some(body),
            Some(&model),
        ))
        .await;
        let value = match result {
            Ok(value) => value,
            Err(failure) => {
                attempt.refuse(&failure.stage);
                if failure.retryable {
                    continue;
                }
                return Err(false);
            }
        };
        if let Some(reason) = incomplete_reason(protocol, &value) {
            attempt.refuse(reason);
            return Err(false);
        }
        let text = output_text(protocol, &value);
        let failure = if text.trim().is_empty() {
            Some("empty_output")
        } else if text.chars().count() > limits.max_output_chars.min(8000) {
            Some("too_long")
        } else if route.reflects_credential(&text) {
            Some("credential_reflection")
        } else {
            None
        };
        if let Some(stage) = failure {
            attempt.refuse(stage);
            return Err(false);
        }
        attempt.finished = true;
        return Ok(text);
    }
    Err(true)
}

pub(crate) fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && !id.chars().any(char::is_whitespace)
        && !id.chars().any(char::is_control)
}

fn reasoning_effort(model: &str) -> Option<&'static str> {
    let model = model.rsplit('/').next().unwrap_or(model);
    if model.starts_with("gpt-6") {
        Some(if model.contains("luna") {
            "none"
        } else {
            "low"
        })
    } else if model.starts_with("gpt-5.") {
        Some("none")
    } else if model.starts_with("gpt-5") {
        Some("minimal")
    } else if model
        .strip_prefix('o')
        .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit()))
    {
        Some("low")
    } else {
        None
    }
}

fn suitable_models(value: &Value, today: chrono::NaiveDate) -> Vec<String> {
    let Some(data) = value["data"].as_array() else {
        return vec![];
    };
    let mut models: Vec<String> = data
        .iter()
        .take(1000)
        .filter(|model| {
            if let Some(date) = model["shutdown_date"].as_str() {
                let shutdown = chrono::DateTime::parse_from_rfc3339(date)
                    .map(|d| d.date_naive())
                    .ok()
                    .or_else(|| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok());
                if shutdown.is_some_and(|date| date <= today) {
                    return false;
                }
            }
            // Some gateways attach task metadata; never treat non-chat task kinds as text.
            for field in ["type", "task", "kind"] {
                if let Some(kind) = model[field].as_str()
                    && non_chat_kind(kind)
                {
                    return false;
                }
            }
            true
        })
        .filter_map(|m| m["id"].as_str())
        .filter(|id| valid_model_id(id) && !non_chat_kind(id))
        .map(str::to_owned)
        .collect();
    models.sort_by_key(|id| {
        let lower = id.to_ascii_lowercase();
        (
            reasoning_effort(&lower).is_some(),
            ["nano", "mini", "haiku", "flash", "small", "fast", "luna"]
                .iter()
                .position(|hint| lower.contains(hint))
                .unwrap_or(7),
            id.clone(),
        )
    });
    models.dedup();
    models
}

fn non_chat_kind(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if lower == "ada" || lower.starts_with("ada-") || lower.starts_with("text-ada") {
        return true;
    }
    [
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
        "search",
        "deep-research",
        "deep_research",
        "codex",
        "computer-use",
        "computer_use",
        "instruct",
        "-pro",
        "babbage",
        "davinci",
        "curie",
        "safeguard",
        "sora",
        "video",
    ]
    .iter()
    .any(|kind| lower.contains(kind))
}

#[cfg(test)]
fn choose_model(value: &Value) -> Option<String> {
    suitable_models(value, chrono::Utc::now().date_naive())
        .into_iter()
        .next()
}

fn text_request(
    protocol: InferenceWireProtocol,
    model: &str,
    prompt: &str,
    input: &str,
    tokens: u16,
    send_reasoning: bool,
) -> (&'static str, Value) {
    use InferenceWireProtocol::*;
    let effort = reasoning_effort(model);
    let tokens = if effort.is_some() {
        tokens.max(1024)
    } else {
        tokens
    };
    let (path, mut body) = match protocol {
        OpenaiResponses => (
            "responses",
            json!({"model":model,"instructions":prompt,"input":input,
            "stream":false,"store":false,"max_output_tokens":tokens}),
        ),
        OpenaiCompletions => {
            let mut body = json!({"model":model,"messages":[{"role":"system","content":prompt},{"role":"user","content":input}],"stream":false});
            let token_field = if model.starts_with("gpt-") || effort.is_some() {
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
    };
    if send_reasoning && let Some(effort) = effort {
        match protocol {
            OpenaiResponses => body["reasoning"] = json!({"effort": effort}),
            OpenaiCompletions => body["reasoning_effort"] = json!(effort),
            AnthropicMessages => {}
        }
    }
    (path, body)
}

fn incomplete_reason(protocol: InferenceWireProtocol, value: &Value) -> Option<&'static str> {
    use InferenceWireProtocol::*;
    let reason = match protocol {
        OpenaiResponses if value["status"] == "incomplete" => value["incomplete_details"]["reason"]
            .as_str()
            .unwrap_or("unknown"),
        OpenaiResponses if value["status"] == "failed" => "unknown",
        OpenaiCompletions => value["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("stop"),
        AnthropicMessages => value["stop_reason"].as_str().unwrap_or("end_turn"),
        _ => return None,
    };
    match reason {
        "max_output_tokens" | "length" | "max_tokens" => Some("incomplete:max_output_tokens"),
        "content_filter" | "refusal" => Some("incomplete:content_filter"),
        "stop" | "end_turn" | "stop_sequence" => None,
        _ => Some("incomplete:unknown"),
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
                    BillingMetric::Requests => self.usage.requests,
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
) -> Result<Value, RequestFailure> {
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
        .map_err(|_| RequestFailure::terminal("dispatch_unavailable"))?;
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
    let metered = Box::pin(state.billing.open(&ctx))
        .await
        .map_err(|_| RequestFailure::terminal("dispatch_unavailable"))?;
    let mut guard = MeterGuard {
        billing: state.billing.clone(),
        metered,
        forwarded: false,
        usage: llm_usage_service::platform_usage(None, request_len, model.is_some()),
        model: model.map(str::to_owned),
        armed: true,
    };
    state
        .billing
        .mark_forwarded(&guard.metered)
        .await
        .map_err(|_| RequestFailure::terminal("dispatch_unavailable"))?;
    guard.forwarded = true;
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        Some(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::LlmProvider,
        )),
        billing::BillingIngress::LlmProvider,
    )
    .map_err(|_| RequestFailure::terminal("dispatch_unavailable"))?;
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
    .map_err(|error| {
        RequestFailure::terminal(match error {
            proxy_service::ForwardRequestError::Transport(error) if error.is_timeout() => "timeout",
            _ => "dispatch_unavailable",
        })
    })?;
    // Invalidate before reading the body: a 4xx still counts if that body is
    // malformed, oversized, or stalls. The next call rediscovers; never retry a
    // paid inference here, and never invalidate on transient 5xx/transport errors.
    if response.status().is_client_error()
        && let Some(model) = model
        && let Ok(mut cache) = MODEL_CACHE.lock()
    {
        cache.invalidate(&service.id, route.class, model);
    }
    let status = response.status();
    let success = status.is_success();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            RequestFailure::terminal(if error.is_timeout() {
                "timeout"
            } else {
                "dispatch_unavailable"
            })
        })?;
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(RequestFailure::terminal("too_long"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Option<Value> = serde_json::from_slice(&bytes).ok();
    // Never repeat generation if a gateway supplied usage or output along with
    // an error. 408/409 and redirects are ambiguous, just like 5xx/transport errors.
    let generated = value.as_ref().is_some_and(|v| {
        v.get("usage").is_some_and(|u| !u.is_null())
            || v.get("output").is_some()
            || v.get("choices").is_some()
            || v.get("content").is_some()
    });
    let retryable = matches!(
        status.as_u16(),
        400 | 401 | 403 | 404 | 405 | 406 | 415 | 422 | 429
    ) && !generated;
    if retryable {
        if status == reqwest::StatusCode::BAD_REQUEST
            && let (Some(model), Some(value)) = (model, &value)
            && unsupported_reasoning(value)
        {
            disable_reasoning(&service.id, model);
        }
        // A definite pre-generation refusal settles every lane at zero through
        // the existing durable settlement path, releasing holds. `fail` only
        // releases unforwarded requests and must not be used after egress.
        guard.usage = PlatformUsage::default();
        Box::pin(guard.finish())
            .await
            .map_err(|_| RequestFailure::terminal("dispatch_unavailable"))?;
        guard.armed = false;
        return Err(RequestFailure {
            stage: format!("http_status:{}", status.as_u16()),
            retryable: true,
        });
    }
    let usage = llm_usage_service::usage_from_body(&bytes, path, success);
    guard.usage = llm_usage_service::platform_usage(
        usage.as_ref(),
        request_len + bytes.len() as i64,
        model.is_some(),
    );
    Box::pin(guard.finish())
        .await
        .map_err(|_| RequestFailure::terminal("dispatch_unavailable"))?;
    guard.armed = false;
    if !success {
        return Err(RequestFailure {
            stage: format!("http_status:{}", status.as_u16()),
            retryable: false,
        });
    }
    value.ok_or_else(|| RequestFailure::terminal("invalid_response"))
}

fn unsupported_reasoning(value: &Value) -> bool {
    let error = &value["error"];
    let parameter = error["param"].as_str().unwrap_or_default();
    let code = error["code"].as_str().unwrap_or_default();
    // Inspect only to classify, never retain or print provider prose. Compatible
    // gateways sometimes omit `param`; support their conventional error wording.
    let message = error["message"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let names_parameter = matches!(
        parameter,
        "reasoning" | "reasoning.effort" | "reasoning_effort"
    ) || [
        "reasoning_effort",
        "reasoning.effort",
        "'reasoning'",
        "\"reasoning\"",
    ]
    .iter()
    .any(|p| message.contains(p));
    names_parameter
        && (matches!(
            code,
            "unsupported_parameter" | "unsupported_value" | "unknown_parameter"
        ) || message.contains("unsupported")
            || message.contains("not supported")
            || message.contains("unknown parameter"))
}

#[cfg(test)]
#[path = "assistant_oneshot_inference_tests.rs"]
pub(crate) mod tests;
