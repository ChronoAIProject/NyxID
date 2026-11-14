//! Metadata-only diagnostics for provider refusals, never SSE/tool payloads.
use crate::{
    errors::AppError,
    services::{
        audit_service::{self, AuditActor},
        coordination_service::RateWindowStore,
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::{
    cell::RefCell,
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[derive(Clone, Copy)]
pub(super) enum Endpoint {
    BindingPut,
    ConversationPut,
    Responses,
    Events,
    ResponseGet,
}
impl Endpoint {
    fn name(self) -> &'static str {
        match self {
            Self::BindingPut => "binding_put",
            Self::ConversationPut => "conversation_put",
            Self::Responses => "responses",
            Self::Events => "events",
            Self::ResponseGet => "response_get",
        }
    }
}
#[derive(Default)]
struct Context {
    owner: Option<String>,
    channel: Option<String>,
    binding_match: Option<bool>,
    partition_known: bool,
    partition: Option<String>,
    event_hash: Option<String>,
}
tokio::task_local! { static CONTEXT: RefCell<Context>; }
fn update(f: impl FnOnce(&mut Context)) {
    let _ = CONTEXT.try_with(|c| f(&mut c.borrow_mut()));
}
pub(super) fn owner(owner: &str) {
    update(|c| c.owner = Some(owner.into()));
}
pub(super) fn channel(owner_id: &str, channel: &str) {
    update(|c| {
        c.owner = Some(owner_id.into());
        c.channel = Some(channel.into());
    });
}
pub(super) fn binding(matches: bool) {
    update(|c| c.binding_match = Some(matches));
}
pub(super) fn partition_known() {
    update(|c| c.partition_known = true);
}
pub(super) fn partition(value: &str) {
    if super::valid_partition(value) {
        update(|c| c.partition = Some(value.into()));
    }
}

async fn partition_history(db: &mongodb::Database, channel: &str, partition: &str) -> &'static str {
    // Uses the existing (channel_id, partition, event_id) index, only on a
    // throttled refusal. Absence is inconclusive: events have finite retention.
    let lookup = async {
        db.collection::<bson::Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
            .find_one(bson::doc! {"channel_id":channel,"partition":partition})
            .projection(bson::doc! {"_id":1})
            .max_time(Duration::from_millis(100))
            .await
    };
    match tokio::time::timeout(Duration::from_millis(250), lookup).await {
        Ok(Ok(Some(_))) => "retained_event",
        Ok(Ok(None)) => "no_retained_event",
        _ => "unavailable",
    }
}
pub(super) fn event(value: &str) {
    // Hash only bounded correlation IDs; never retain their raw value.
    if !value.is_empty() && value.len() <= 4096 {
        update(|c| {
            c.event_hash = audit_service::keyed_fingerprint(
                audit_service::audit_chain_hmac_key(),
                b"nyxbot-provider-event-v1",
                value.as_bytes(),
            )
        });
    }
}
#[derive(Clone)]
struct Code(String);
pub(super) fn coded(mut response: Response, code: &str) -> Response {
    response.extensions_mut().insert(Code(code.to_owned()));
    response
}
pub(super) fn error(error: AppError) -> Response {
    let code = error.error_key().to_owned();
    coded(error.into_response(), &code)
}

// A fixed number of slots; no unauthenticated identifiers or database work.
static WARN_AFTER: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
fn warn(endpoint: Endpoint, status: StatusCode, code: &str) {
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let slot = &WARN_AFTER[endpoint as usize];
    if slot
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |until| {
            (now >= until).then_some(now.saturating_add(60))
        })
        .is_ok()
    {
        tracing::warn!(
            endpoint = endpoint.name(),
            status = status.as_u16(),
            code,
            "NyxBot provider refusal diagnostics unavailable"
        );
    }
}

pub(super) async fn scope(
    db: &mongodb::Database,
    endpoint: Endpoint,
    call: impl Future<Output = Response>,
) -> Response {
    CONTEXT.scope(RefCell::new(Context::default()), async {
        let response = call.await;
        if response.status().is_success() { return response; }
        let context = CONTEXT.with(|c| c.take());
        let code = response.extensions().get::<Code>().map(|c| c.0.as_str()).unwrap_or("provider_unavailable");
        // Authentication may be revoked between the custom/native probe and
        // ordinary channel resolution. A remembered owner is not a live channel.
        if response.status() == StatusCode::UNAUTHORIZED && context.channel.is_none() {
            warn(endpoint, response.status(), code);
            return response;
        }
        let Some(owner) = context.owner else {
            warn(endpoint, response.status(), code);
            return response;
        };
        let actor = AuditActor { user_id: owner, ip_address: None, user_agent: None, api_key_id: None, api_key_name: None };
        let key = format!("{}:{}:{}", context.channel.as_deref().unwrap_or(&actor.user_id), endpoint.name(), code);
        let mut data = serde_json::json!({"endpoint":endpoint.name(), "status":response.status().as_u16(),
            "code":code,"partition_known":context.partition_known});
        if let Some(channel) = &context.channel { data["channel_agent_id"] = channel.clone().into(); }
        if matches!(endpoint, Endpoint::ConversationPut) {
            data["binding_match"] = context.binding_match.map_or(serde_json::Value::String("unknown".into()), serde_json::Value::Bool);
        }
        if let Some(hash) = context.event_hash { data["event_correlation_hash"] = hash.into(); }
        // Diagnostics cannot hold a provider request indefinitely or alter its outcome.
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            if RateWindowStore::admit(db, "nyxbot-provider-refused-v1", &key, 1, Duration::from_secs(60)).await?.allowed {
                if let (Some(channel), Some(partition)) = (&context.channel, &context.partition) {
                    if let Some(hash) = audit_service::keyed_fingerprint(audit_service::audit_chain_hmac_key(),
                        b"nyxbot-provider-partition-v1",format!("{channel}\0{partition}").as_bytes()) {
                        data["partition_correlation_hash"] = hash.into();
                    }
                    if matches!(endpoint,Endpoint::Responses) && code == "conversation_not_found" && !context.partition_known {
                        data["partition_history"] = partition_history(db,channel,partition).await.into();
                    }
                }
                audit_service::log_actor_event(db.clone(), &actor, "nyxbot_provider_refused", Some(data)).await?;
            }
            Ok::<_, AppError>(())
        }).await;
        if !matches!(result, Ok(Ok(()))) { warn(endpoint, response.status(), code); }
        response
    }).await
}
