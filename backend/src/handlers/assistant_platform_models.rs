//! HTTP projections for the private-model adapter. Never return private routes.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        assistant_platform_model as adapter, platform_model_execution as execution,
        platform_model_service as registry,
    },
};
use async_trait::async_trait;
use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, Method, Request},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn header<'a>(headers: &'a HeaderMap, name: &str) -> AppResult<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| uuid::Uuid::parse_str(v).is_ok())
        .ok_or_else(registry::unavailable)
}

pub(crate) async fn context(
    state: &AppState,
    auth: &AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    if request.method() != Method::GET
        || request.uri().query().is_some()
        || request.headers().contains_key("upgrade")
    {
        return Err(registry::unavailable());
    }
    let handle = header(request.headers(), adapter::CONTEXT_HEADER)?;
    let (admitted, _) = adapter::live(state, auth, handle).await?;
    let plan = adapter::selection_plan(state, &admitted.user_id, &admitted.model).await?;
    let manifest = adapter::manifest(&plan, state.config.llm_max_body_size)?;
    Ok(Json(manifest).into_response())
}
use axum::response::IntoResponse;

struct Access {
    state: AppState,
    auth: AuthUser,
    handle: String,
    body: bytes::Bytes,
    image_digests: std::collections::HashSet<String>,
    headers: HeaderMap,
}
#[async_trait]
impl execution::Authority for Access {
    async fn check(&self, plan: &execution::Plan) -> AppResult<()> {
        let live =
            adapter::check_request(&self.state, &self.auth, &self.handle, plan, &self.body).await?;
        // Retain exact material binding, including rotation; a context never
        // substitutes for authentication with the original credential.
        let raw = self
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .or_else(|| self.headers.get("x-api-key").and_then(|v| v.to_str().ok()))
            .ok_or_else(registry::unavailable)?;
        let (actor, key, child) =
            crate::services::key_service::validate_api_key(&self.state.db, raw).await?;
        if child.is_some()
            || Some(&key.id) != self.auth.api_key_id.as_ref()
            || actor != self.auth.user_id.to_string()
        {
            return Err(registry::unavailable());
        }
        adapter::check_images(
            &self.state.db,
            live.assistant_chat
                .as_deref()
                .ok_or(AppError::AssistantTurnRequired)?,
            &self.handle,
            &self.image_digests,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    state: AppState,
    auth: AuthUser,
    path: String,
    method: Method,
    query: Option<String>,
    headers: HeaderMap,
    body: bytes::Bytes,
    value: Value,
    cancel: CancellationToken,
    runtime: std::sync::Arc<execution::Runtime>,
) -> AppResult<Response> {
    if query.is_some() || headers.contains_key("upgrade") {
        return Err(registry::unavailable());
    }
    let handle = header(&headers, adapter::CONTEXT_HEADER)?.to_owned();
    let occurrence = header(&headers, adapter::OCCURRENCE_HEADER)?.to_owned();
    let (admitted, live) = adapter::live(&state, &auth, &handle).await?;
    if value["model"] != admitted.model {
        return Err(adapter::unavailable());
    }
    // PR3b readiness precedes parsing/allocation. Do not retain a separate image
    // permit through approvals; its executor owns final dispatch reservation.
    adapter::require_ready(&state.db).await?;
    if body.len() > state.config.llm_max_body_size || body.len() > execution::images::MAX_REQUEST {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: state
                .config
                .llm_max_body_size
                .min(execution::images::MAX_REQUEST),
            context: "Platform model gateway".into(),
        });
    }
    if execution::images::contains_images(&value) {
        crate::services::security_readiness_service::require_serving(
            &state.db,
            execution::images::READINESS,
        )
        .await?;
    }
    // This is the exact conversation's original acting person, including shared
    // and org-member lanes; never the agent owner or private credential account.
    let person = admitted.user_id.clone();
    let request = execution::Request::parse_async(method, path, value, &person)
        .await?
        .with_ingress_limit(state.config.llm_max_body_size);
    let plan = execution::Plan::load(
        &state.db,
        &person,
        &admitted.model,
        None,
        state.config.billing_rate_cache_ttl_secs,
        &request,
    )
    .await?;
    if request.has_images() && body.len() as i64 > plan.tariff.tariff().bounds.max_request_bytes {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: plan.tariff.tariff().bounds.max_request_bytes as usize,
            context: "Platform model request".into(),
        });
    }
    let image_digests = adapter::image_digests(&request.body)?;
    let access = Access {
        state: state.clone(),
        auth: auth.clone(),
        handle,
        body,
        image_digests,
        headers,
    };
    execution::Authority::check(&access, &plan).await?;
    // Logical model offerings have no user connection. Existing connection-bound
    // execution permissions cannot cover them by alias; retain the current
    // bounded approval/card fallback, including enrolled callers.
    let chat = live
        .assistant_chat
        .as_deref()
        .ok_or(AppError::AssistantTurnRequired)?;
    use crate::services::assistant_acknowledgement_service as acks;
    if let Some(need) = acks::confirmation_needed(
        &state.db,
        chat,
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        false,
    )
    .await?
    .ask()
    {
        let args = json!({"model":admitted.model,"occurrence":occurrence});
        if let Some(refusal) = acks::confirm_action(
            &state.db,
            chat,
            "nyxid.platform_model",
            &args,
            &need,
            "Use the selected platform model",
        )
        .await?
        {
            return Ok((http::StatusCode::FORBIDDEN, Json(refusal)).into_response());
        }
    }
    let request_id =
        adapter::claim_occurrence(&state.db, &admitted, &occurrence, &access.body).await?;
    let approval_owner = adapter::approval_owner(&live);
    let approval_auto = super::llm_gateway::check_llm_approval_descriptor(
        &state,
        &live,
        &plan.offering.id,
        &plan.model.display_name,
        &plan.model.public_id,
        adapter::inference_operation(&access.body),
        Some(&approval_owner),
        false,
        false,
    )
    .await?;
    execution::Authority::check(&access, &plan).await?;
    let context = execution::Context {
        db: &state.db,
        keys: &state.encryption_keys,
        billing: &state.billing,
        actor: &person,
        api_key: auth.api_key_id.clone(),
        request_id: request_id.clone(),
        authority: &access,
        cancel: cancel.clone(),
        runtime: &runtime,
    };
    let accepted = execution::dispatch(&context, &plan, &request).await?;
    let receipt = adapter::inference_operation(&access.body);
    crate::services::approval_service::record_chat_approval(
        &state.db,
        Some(chat),
        approval_auto,
        &plan.offering.id,
        &plan.model.display_name,
        &receipt,
    )
    .await;
    if !request.streaming {
        let value = execution::consume(&context, &plan, accepted, None).await?;
        let mut response = Json(value).into_response();
        response.headers_mut().insert(
            "x-nyxid-request-id",
            request_id.parse().map_err(|_| adapter::unavailable())?,
        );
        return Ok(response);
    }
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let response_request_id = request_id.clone();
    let worker_cancel = cancel.clone();
    tokio::spawn(async move {
        let context = execution::Context {
            db: &state.db,
            keys: &state.encryption_keys,
            billing: &state.billing,
            actor: &person,
            api_key: auth.api_key_id.clone(),
            request_id,
            authority: &access,
            cancel: worker_cancel,
            runtime: &runtime,
        };
        if execution::consume(&context, &plan, accepted, Some(&tx))
            .await
            .is_err()
        {
            let _=tx.send(Ok(bytes::Bytes::from_static(b"data: {\"error\":{\"code\":\"model_response_unavailable\",\"message\":\"Model response could not be completed\"}}\n\n"))).await;
        }
    });
    let stream = crate::downstream_disconnect::CancelOnDropStream::new(
        tokio_stream::wrappers::ReceiverStream::new(rx),
        cancel,
    );
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-store")
        .header("x-nyxid-request-id", response_request_id)
        .body(Body::from_stream(stream))
        .map_err(|_| AppError::Internal("Model response failed".into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    after: Option<String>,
    limit: Option<u32>,
    agent_id: Option<String>,
    conversation_id: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<super::platform_models::PublicList>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let person = auth.user_id.to_string();
    crate::services::assistant_nyxagent::require_enabled(&state.db, &person).await?;
    let empty = || {
        Json(super::platform_models::PublicList {
            models: vec![],
            next_after: None,
        })
    };
    if !crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        &person,
        registry::FLAG,
    )
    .await?
    {
        return Ok(empty());
    }
    // Readiness is availability metadata here, not an execution permit.
    // Missing/unknown fleet state must not turn discovery into a 503.
    let ready = match adapter::require_ready(&state.db).await {
        Ok(()) => true,
        Err(AppError::SecurityReadinessUnavailable) => false,
        Err(error) => return Err(error),
    };
    if query.agent_id.is_some() && query.conversation_id.is_some() {
        return Err(AppError::BadRequest(
            "Choose one model selection context".into(),
        ));
    }
    let agent = if let Some(id) = &query.conversation_id {
        let row = crate::services::assistant_nyxagent::get(&state.db, &person, id).await?;
        if row.guest_turn {
            return Ok(empty());
        }
        crate::services::assistant_team_service::agent_for_conversation(&state.db, &row).await?
    } else if let Some(id) = &query.agent_id {
        crate::services::assistant_team_service::agent(&state.db, &person, id).await?
    } else {
        crate::services::assistant_team_service::ensure_nyxbot(&state.db, &person).await?
    };
    let snapshot = registry::EntitlementSnapshot::load(&state.db, &person).await?;
    let mut scope = agent
        .grants
        .platform_service_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    if agent.user_id != person {
        let access =
            crate::services::org_agent_service::resolve_agent_access(&state.db, &person, &agent)
                .await?;
        if let Some(shared) = access.as_ref().and_then(|a| a.shared()) {
            scope.retain(|id| shared.level(id).is_some());
        }
    }
    scope.retain(|id| {
        crate::services::agent_operation_scope_service::route_visible(
            &agent.operation_scopes,
            id,
            Some(id),
            "POST",
            "/v1/chat/completions",
        )
    });
    let limit = query.limit.unwrap_or(50);
    let mut rows = registry::list_authorized(
        &state.db,
        &snapshot,
        (!agent.is_nyxbot()).then_some(&scope),
        query.after.as_deref(),
        limit,
    )
    .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_after = more.then(|| rows.last().unwrap().public_id.clone());
    let paired = ready && super::assistant_nyxagent::platform_bridge_ready(&state, &auth).await;
    let mut prices = registry::retail_explanations(
        &state.db,
        &rows,
        state.config.billing_rate_cache_ttl_secs,
        state.billing.billing_enabled(),
    )
    .await?;
    let models = rows
        .iter()
        .map(|row| {
            let mut model = super::platform_models::PublicModel::from(row);
            model.retail_prices = prices
                .remove(&row.id)
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect();
            if !paired {
                for price in &mut model.retail_prices {
                    price.image_input = None;
                }
            }
            let cap = &row.capabilities;
            if paired
                && cap.tools == Some(true)
                && cap.streaming == Some(true)
                && cap.context_tokens.is_some()
                && cap.output_tokens.is_some()
                && model
                    .retail_prices
                    .iter()
                    .any(|p| p.available && p.variant.is_empty())
            {
                model.availability = "request_dependent";
            }
            model
        })
        .collect();
    Ok(Json(super::platform_models::PublicList {
        models,
        next_after,
    }))
}

#[cfg(test)]
mod tests;
