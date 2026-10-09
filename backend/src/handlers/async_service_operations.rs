//! Background assistant calls use the same MCP authentication, authorization,
//! proxy resolution, metering and concurrency admission as foreground calls.
use super::*;
use crate::models::async_service_operation::AsyncServiceOperation;
use crate::services::async_service_operation as watches;

async fn call(
    state: &AppState,
    watch: &AsyncServiceOperation,
    name: &str,
) -> crate::errors::AppResult<mcp_service::ToolResponse> {
    use crate::errors::AppError;
    let denied = || AppError::Forbidden("Async operation authority lost".into());
    if !watches::owns_lease(&state.db, watch).await? {
        return Err(AppError::Conflict("Async operation claim changed".into()));
    }
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &state.db,
        &state.encryption_keys,
        &watch.user_id,
        &watch.conversation_id,
    )
    .await?
    .ok_or_else(denied)?;
    if credential.api_key_id != watch.api_key_id {
        return Err(denied());
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        credential.raw_key.as_str().parse().map_err(|_| denied())?,
    );
    let mut auth = authenticate_mcp(state, &headers, false)
        .await
        .map_err(|_| denied())?;
    // This watch was admitted only for an owner turn. A later guest message
    // on the same thread neither owns nor changes this background request.
    if let Some(chat) = auth.chat.as_mut() {
        chat.guest = false;
    }
    crate::mw::rate_limit::check_agent_rate_limit_raw(
        &state.per_agent_limiter,
        auth.api_key_id.as_deref(),
        auth.rate_limit_per_second,
        auth.rate_limit_burst,
    )
    .await?;
    let chat = auth.chat.as_ref().ok_or_else(denied)?;
    if chat.conversation_id != watch.conversation_id || chat.agent_id != watch.agent_id {
        return Err(denied());
    }
    if let Some(request_id) = &watch.delivery.voice_request_id
        && Some(name) != watch.contract.cancel_operation.as_deref()
        && state
            .db
            .collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
            .find_one(doc! {"_id":request_id,"user_id":&watch.user_id,"state":{"$ne":"cancelled"}})
            .await?
            .is_none()
    {
        return Err(denied());
    }
    let catalog = mcp_service::load_operation_catalog(
        &state.db,
        state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(&auth),
        mcp_service_scope(&auth),
    )
    .await?;
    let service = catalog
        .services
        .iter()
        .find(|s| s.service_id == watch.service_id)
        .ok_or_else(denied)?;
    ensure_service_in_scope(&auth, service, None).map_err(|_| denied())?;
    let submit = service
        .endpoints
        .iter()
        .find(|e| e.endpoint_id == watch.submit_endpoint_id)
        .ok_or_else(denied)?;
    if submit.async_operation.as_ref() != Some(&watch.contract) {
        return Err(denied());
    }
    watches::validate_endpoints(&watch.contract, service, submit).map_err(|_| denied())?;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(denied)?;
    if endpoint.target_id != submit.target_id
        || (name != watch.contract.cancel_operation.as_deref().unwrap_or("")
            && endpoint.method != "GET")
        || (Some(name) == watch.contract.cancel_operation.as_deref() && endpoint.method != "POST")
    {
        return Err(denied());
    }
    let id = watch.operation_id.as_deref().ok_or_else(denied)?;
    let args = serde_json::json!({watch.contract.id_parameter.clone():id});
    let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &args)?;
    prepared.authorize_agent_operations(&auth.assistant_operation_scopes, service, endpoint)?;
    authorize_mcp_tool_operation(
        state,
        &auth,
        service,
        &prepared.operation_descriptor(),
        None,
    )
    .await
    .map_err(|_| denied())?;
    // There is deliberately no synthetic live chat turn: this durable watch is
    // permission to poll only these exact operations, not arbitrary execution.
    let permit = crate::services::billing::route_inventory::enforce_billing_egress_classification(
        Some(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::route_inventory::BillingIngress::Mcp,
            ),
        ),
        crate::services::billing::route_inventory::BillingIngress::Mcp,
    )?;
    let mut exec = mcp_exec_context(&auth);
    exec.response_body_limit = Some(watches::RESULT_BYTES);
    mcp_service::execute_tool_response(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &auth.user_id,
        auth.billing_principal_user_id(),
        service,
        endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &exec,
        permit,
    )
    .await
}

async fn step(state: &AppState, w: &AsyncServiceOperation) -> crate::errors::AppResult<()> {
    if w.state == "ready" {
        if watches::enqueue(&state.db, w).await? {
            crate::handlers::assistant_team::wake(state, &w.user_id, &w.conversation_id).await;
        } else {
            watches::defer(&state.db, w).await?;
        }
        return Ok(());
    }
    if w.state == "cancelling" {
        if w.operation_id.is_none()
            && chrono::Utc::now() < w.created_at + chrono::Duration::minutes(2)
        {
            return watches::defer(&state.db, w).await;
        }
        if let Some(name) = w.contract.cancel_operation.as_deref()
            && w.operation_id.is_some()
        {
            let _ = tokio::time::timeout(Duration::from_secs(5), call(state, w, name)).await;
        }
        return watches::cancelled(&state.db, w).await;
    }
    if chrono::Utc::now() >= w.deadline {
        return watches::ready(&state.db, w, None, "timeout", None).await;
    }
    if w.state == "submitting" {
        return watches::ready(&state.db, w, None, "submission_uncertain", None).await;
    }
    let status = call(state, w, &w.contract.status_operation).await?;
    if matches!(status.status, 401 | 403 | 404 | 410) {
        return watches::ready(&state.db, w, None, "authority_lost", None).await;
    }
    if !(200..300).contains(&status.status) {
        return watches::defer(&state.db, w).await;
    }
    if status.text.len() > watches::RESULT_BYTES {
        return watches::ready(&state.db, w, None, "result_too_large", None).await;
    }
    let parsed: serde_json::Value = serde_json::from_str(&status.text).map_err(|_| {
        crate::errors::AppError::ValidationError("Invalid async operation status".into())
    })?;
    // The result is a separate proxy call and must acquire its own slot.
    drop(status);
    let phase = parsed
        .pointer(&w.contract.status_field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if w.contract.failure_states.iter().any(|s| s == phase) {
        // Error payloads are equally untrusted; retain only bounded encrypted data.
        let detail = w
            .contract
            .error_field
            .as_ref()
            .and_then(|f| parsed.pointer(f))
            .map(serde_json::Value::to_string);
        return watches::ready(
            &state.db,
            w,
            Some(&state.encryption_keys),
            "operation_failed",
            detail.as_deref(),
        )
        .await;
    }
    if !w.contract.success_states.iter().any(|s| s == phase) {
        return watches::defer(&state.db, w).await;
    }
    let result = call(state, w, &w.contract.result_operation).await?;
    if !(200..300).contains(&result.status) {
        return watches::ready(&state.db, w, None, "result_unavailable", None).await;
    }
    if result.text.len() > watches::RESULT_BYTES || result.media.is_some() {
        return watches::ready(&state.db, w, None, "result_too_large", None).await;
    }
    watches::ready(
        &state.db,
        w,
        Some(&state.encryption_keys),
        "completed",
        Some(&result.text),
    )
    .await
}

pub(crate) async fn sweep(state: &AppState) -> crate::errors::AppResult<()> {
    watches::expire_results(&state.db).await?;
    // Claim just before dispatch: never queue leased jobs behind slow I/O.
    let results = futures::future::join_all((0..4).map(|_| Box::pin(sweep_worker(state)))).await;
    for result in results {
        result?;
    }
    Ok(())
}

async fn sweep_worker(state: &AppState) -> crate::errors::AppResult<()> {
    use crate::errors::AppError;
    for _ in 0..8 {
        let Some(w) = watches::claim(&state.db).await? else {
            break;
        };
        match tokio::time::timeout(Duration::from_secs(45), Box::pin(step(state, &w))).await {
            Ok(Ok(())) => {}
            Ok(Err(
                AppError::Forbidden(_)
                | AppError::Unauthorized(_)
                | AppError::ApiKeyScopeForbidden(_)
                | AppError::NotFound(_),
            )) => {
                watches::ready(&state.db, &w, None, "authority_lost", None).await?;
            }
            _ => watches::defer(&state.db, &w).await?,
        }
    }
    Ok(())
}

pub(crate) async fn cancel_conversation(
    state: &AppState,
    user: &str,
    id: &str,
) -> crate::errors::AppResult<()> {
    watches::cancel(&state.db, user, id).await?;
    drain_cancellations(state, user, id).await
}

pub(crate) async fn drain_cancellations(
    state: &AppState,
    user: &str,
    id: &str,
) -> crate::errors::AppResult<()> {
    // Immediate best-effort cancellation; leases fence the background retry.
    // A submit still in flight is left for the sweep once its ID arrives.
    let mut claimed = Vec::new();
    for _ in 0..watches::MAX_CONVERSATION {
        let Some(w) = watches::claim_cancel(&state.db, user, id).await? else {
            break;
        };
        claimed.push(w);
    }
    for result in futures::future::join_all(claimed.iter().map(|w| Box::pin(step(state, w)))).await
    {
        result?;
    }
    Ok(())
}

pub(crate) fn spawn_sweep(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = Box::pin(sweep(&state)).await {
                tracing::debug!(code = error.error_code(), "Async service sweep deferred");
            }
        }
    });
}
