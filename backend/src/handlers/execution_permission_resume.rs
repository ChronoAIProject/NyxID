//! Replays only a server-retained effect through the ordinary execution adapters.
use crate::models::execution_proposal::{PreparedAdapter, PreparedEffect};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
};
use axum::{
    body::Body,
    extract::{Path, State},
    response::Response,
};
pub(crate) async fn send(
    state: &AppState,
    auth: &AuthUser,
    effect: PreparedEffect,
    adapter: PreparedAdapter,
    independent_card: Option<String>,
) -> AppResult<Response> {
    match adapter {
        PreparedAdapter::Machine { binding, .. } => {
            Box::pin(http_inner(
                state,
                auth,
                effect,
                independent_card,
                Some(*binding),
            ))
            .await
        }
        PreparedAdapter::Unsupported => {
            Err(AppError::Forbidden("Retained source is unavailable".into()))
        }
        PreparedAdapter::Http | PreparedAdapter::Pool { .. } => {
            Box::pin(http(state, auth, effect, independent_card)).await
        }
        PreparedAdapter::Llm {
            provider,
            path,
            method,
            query,
            body,
            headers,
        } => {
            let uri = format!(
                "/api/v1/llm/{}/v1/{}{}",
                provider.as_deref().unwrap_or("gateway"),
                path,
                query.as_ref().map(|q| format!("?{q}")).unwrap_or_default()
            );
            let mut builder = axum::http::Request::builder()
                .method(method.as_str())
                .uri(uri);
            for (k, v) in headers {
                builder = builder.header(k, v);
            }
            let mut req = builder
                .body(Body::from(body))
                .map_err(|_| AppError::BadRequest("Invalid retained operation".into()))?;
            use crate::services::billing::route_inventory::{BillingIngress, BillingRoutePolicy};
            req.extensions_mut()
                .insert(BillingRoutePolicy::Metered(if provider.is_some() {
                    BillingIngress::LlmProvider
                } else {
                    BillingIngress::LlmGateway
                }));
            if let Some(provider) = provider {
                Box::pin(super::llm_gateway::llm_proxy_request(
                    State(state.clone()),
                    auth.clone(),
                    Path((provider, path)),
                    req,
                ))
                .await
            } else {
                Box::pin(super::llm_gateway::gateway_request(
                    State(state.clone()),
                    auth.clone(),
                    Path(path),
                    req,
                ))
                .await
            }
        }
        PreparedAdapter::Native {
            uri,
            method,
            body,
            content_type,
            mcp,
        } => {
            let mut builder = axum::http::Request::builder()
                .method(method.as_str())
                .uri(uri);
            if let Some(v) = content_type {
                builder = builder.header("content-type", v);
            }
            let mut req = builder
                .body(Body::from(body))
                .map_err(|_| AppError::BadRequest("Invalid retained operation".into()))?;
            req.extensions_mut().insert(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::route_inventory::BillingIngress::Proxy,
                ),
            );
            Box::pin(super::permission_keys::execute(
                state.clone(),
                auth.clone(),
                req,
                mcp,
            ))
            .await
        }
        PreparedAdapter::Watch { .. } => Err(AppError::Forbidden(
            "A watch operation requires its worker lease".into(),
        )),
    }
}
async fn http(
    state: &AppState,
    auth: &AuthUser,
    effect: PreparedEffect,
    independent_card: Option<String>,
) -> AppResult<Response> {
    http_inner(state, auth, effect, independent_card, None).await
}
async fn http_inner(
    state: &AppState,
    auth: &AuthUser,
    effect: PreparedEffect,
    independent_card: Option<String>,
    machine: Option<crate::models::execution_proposal::MachineSource>,
) -> AppResult<Response> {
    let query = if let Some(query) = &effect.query {
        format!("{query}&_nyxid_via={}", effect.connection)
    } else {
        format!("_nyxid_via={}", effect.connection)
    };
    let path = effect.path.as_str();
    // Generic MCP and Machine calls may use custom connections without a
    // catalog row. Machine ingress originally selected this same slug route.
    // The live slug selects the route; _nyxid_via pins the retained connection
    // and the coordinator checks its full snapshot before sending.
    let slug = if machine.is_some()
        || effect.endpoint.as_deref()
            == Some(crate::services::mcp_service::GENERIC_PROXY_ENDPOINT_ID)
    {
        Some(
            crate::services::user_service_service::find_user_service_by_id(
                &state.db,
                &effect.connection,
            )
            .await?
            .ok_or_else(|| AppError::NotFound("Service unavailable".into()))?
            .slug,
        )
    } else {
        None
    };
    let route = slug
        .as_ref()
        .map_or_else(|| effect.catalog.clone(), |slug| format!("s/{slug}"));
    let uri = format!(
        "/api/v1/proxy/{route}/{}?{query}",
        path.trim_start_matches('/')
    );
    let mut request = axum::http::Request::builder()
        .method(effect.method.as_str())
        .uri(uri);
    for (name, value) in &effect.headers {
        request = request.header(name, value);
    }
    if let Some(card) = independent_card {
        request = request.header(
            crate::services::assistant_service_execution::ACK_HEADER,
            card,
        );
    }
    let mut request = request
        .body(Body::from(effect.body))
        .map_err(|_| AppError::BadRequest("Invalid retained operation".into()))?;
    request
        .extensions_mut()
        .insert(super::assistant_nyxagent::SERVER_TURN_POLICY);
    if let Some(binding) = machine {
        request
            .extensions_mut()
            .insert(crate::services::machine_gateway_service::Ingress {
                declared_id: binding.declared_id.clone(),
                git: None,
                job: Some(
                    crate::services::machine_gateway_service::JobAuthority::Assistant(
                        binding.job_id.clone(),
                    ),
                ),
            });
        request.extensions_mut().insert(
            crate::services::machine_gateway_service::MachineUsageAttribution {
                node_id: binding.node_id,
                job_id: binding.job_id,
                runtime_id: binding.runtime_id,
                runtime_generation: binding.runtime_generation,
                grant_id: None,
                principal_kind: "assistant",
            },
        );
    }
    if let Some(slug) = slug {
        return Box::pin(super::proxy::proxy_request_by_slug_inner(
            state,
            auth,
            &slug,
            path,
            request,
            &mut String::new(),
        ))
        .await;
    }
    Box::pin(super::proxy::proxy_request_inner(
        state,
        auth,
        &effect.catalog,
        path,
        request,
        &mut String::new(),
    ))
    .await
}
