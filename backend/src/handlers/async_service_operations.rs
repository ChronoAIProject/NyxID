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
    if crate::services::generated_artifacts::admitted(watch) {
        let trusted = crate::services::generated_artifacts::trusted_sandbox_catalog(
            &state.db, service, submit,
        )
        .await?;
        if trusted.is_none() || trusted != watch.artifact_catalog_service_id {
            return Err(AppError::Conflict("artifact_source_changed".into()));
        }
    }
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
    let id = crate::services::cma_operation::poll_id(watch).ok_or_else(denied)?;
    let args = serde_json::json!({watch.contract.id_parameter.clone():id});
    let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &args)?
        .with_async_continuation(
            crate::services::assistant_service_execution::AsyncContinuation::new(
                watch,
                endpoint,
                state.node_ws_manager.clone(),
            )?,
        );
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
    exec.response_body_limit = Some(
        if crate::services::generated_artifacts::admitted(watch)
            && name == watch.contract.result_operation
        {
            if !crate::services::generated_artifacts::sandbox_contract(service, submit) {
                return Err(denied());
            }
            crate::services::generated_artifacts::sandbox::RESPONSE_BYTES
        } else {
            watches::RESULT_BYTES
        },
    );
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

// The contract does not promise that a successful cancel response means stopped.
// Only a bounded, declared read of a terminal state can confirm settlement.
fn terminal_contract_status(
    contract: &crate::models::async_service_operation::AsyncOperationContract,
    text: &str,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    let Some(phase) = value
        .pointer(&contract.status_field)
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    contract
        .success_states
        .iter()
        .chain(&contract.failure_states)
        .any(|state| state == phase)
}

async fn tracked_cancel_outcome<F, Fut, V, Verified>(
    operation_id: Option<&str>,
    cancel_operation: Option<&str>,
    timeout: Duration,
    dispatch: F,
    verify: V,
) -> (bool, &'static str)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = crate::errors::AppResult<u16>>,
    V: FnOnce() -> Verified,
    Verified: std::future::Future<Output = crate::errors::AppResult<bool>>,
{
    if operation_id.is_none() {
        return (false, "cancellation_operation_id_missing");
    }
    if cancel_operation.is_none() {
        return (false, "cancellation_unsupported");
    }
    let outcome = tokio::time::timeout(timeout, async {
        let status = dispatch().await?;
        if !(200..300).contains(&status) {
            return Ok(false);
        }
        verify().await
    })
    .await;
    match outcome {
        Ok(Ok(true)) => (true, "cancelled"),
        _ => (false, "cancellation_unknown"),
    }
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
        if crate::services::cma_operation::poll_id(w).is_none()
            && chrono::Utc::now() < w.created_at + chrono::Duration::minutes(2)
        {
            return watches::defer(&state.db, w).await;
        }
        if w.delivery.invocation_id.is_some() {
            if !watches::begin_tracked_cancellation(&state.db, w).await? {
                return watches::finish_tracked_cancellation(
                    &state.db,
                    w,
                    false,
                    "cancellation_unknown",
                )
                .await;
            }
            let (confirmed, reason) = tracked_cancel_outcome(
                w.operation_id.as_deref(),
                w.contract.cancel_operation.as_deref(),
                Duration::from_secs(5),
                || async {
                    let name = w
                        .contract
                        .cancel_operation
                        .as_deref()
                        .expect("validated operation");
                    Ok(call(state, w, name).await?.status)
                },
                || async {
                    let status = call(state, w, &w.contract.status_operation).await?;
                    Ok((200..300).contains(&status.status)
                        && status.text.len() <= watches::RESULT_BYTES
                        && terminal_contract_status(&w.contract, &status.text))
                },
            )
            .await;
            return watches::finish_tracked_cancellation(&state.db, w, confirmed, reason).await;
        }
        if let Some(name) = w.contract.cancel_operation.as_deref()
            && w.operation_id.is_some()
        {
            let _ = tokio::time::timeout(Duration::from_secs(5), call(state, w, name)).await;
        }
        return watches::cancelled(&state.db, w, &state.config.frontend_url).await;
    }
    if chrono::Utc::now() >= w.deadline {
        return watches::ready(&state.db, w, None, "timeout", None).await;
    }
    if w.state == "submitting" {
        return watches::ready(&state.db, w, None, "submission_uncertain", None).await;
    }
    let status = call(state, w, &w.contract.status_operation).await?;
    if status.status == 410 && crate::services::generated_artifacts::admitted(w) {
        return watches::ready(&state.db, w, None, "result_expired", None).await;
    }
    if matches!(status.status, 401 | 403 | 404 | 410) {
        return watches::ready(&state.db, w, None, "authority_lost", None).await;
    }
    if !(200..300).contains(&status.status) {
        return watches::defer(&state.db, w).await;
    }
    if status.text.len() > watches::RESULT_BYTES {
        return watches::ready(&state.db, w, None, "result_too_large", None).await;
    }
    let parsed: serde_json::Value = match serde_json::from_str(&status.text) {
        Ok(value) => value,
        Err(_) if crate::services::generated_artifacts::admitted(w) => {
            // Generated-artifact status is a bounded, server-owned lifecycle
            // contract. A successful response that is not JSON cannot become
            // a retrying defer: settle it terminally before any result fetch.
            return watches::ready(&state.db, w, None, "artifact_invalid", None).await;
        }
        Err(_) => {
            // Preserve ordinary async operation behavior and error contract.
            return Err(crate::errors::AppError::ValidationError(
                "Invalid async operation status".into(),
            ));
        }
    };
    // The result is a separate proxy call and must acquire its own slot.
    drop(status);
    let phase = parsed
        .pointer(&w.contract.status_field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let agent_not_started =
        crate::services::cma_operation::record_observation(&state.db, w, &parsed).await?;
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
    if agent_not_started || !w.contract.success_states.iter().any(|s| s == phase) {
        return watches::defer(&state.db, w).await;
    }
    if crate::services::generated_artifacts::admitted(w) {
        return artifact_result(state, w, &parsed).await;
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

// One artifact decode/encrypt worker per process. Other watches remain text-bounded.
static ARTIFACT_WORKER: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
async fn artifact_result(
    state: &AppState,
    w: &AsyncServiceOperation,
    before: &serde_json::Value,
) -> crate::errors::AppResult<()> {
    use crate::services::{
        assistant_upload_service::byte_budget,
        generated_artifacts::{self as artifacts, sandbox},
    };
    let fail = |reason: &'static str| watches::ready(&state.db, w, None, reason, None);
    let operation = w
        .operation_id
        .as_deref()
        .ok_or_else(|| crate::errors::AppError::Conflict("artifact_source_changed".into()))?;
    let before = match sandbox::status(before, operation, chrono::Utc::now()) {
        Ok(s) => s,
        Err(reason) => return fail(reason).await,
    };
    let _worker = ARTIFACT_WORKER
        .acquire()
        .await
        .map_err(|_| crate::errors::AppError::Internal("Artifact worker unavailable".into()))?;
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await?;
    let mut response = call(state, w, &w.contract.result_operation).await?;
    if response.status == 410 {
        return fail("result_expired").await;
    }
    if response.status >= 500 {
        return watches::defer(&state.db, w).await;
    }
    if !(200..300).contains(&response.status) {
        return fail("result_unavailable").await;
    }
    let body = zeroize::Zeroizing::new(std::mem::take(&mut response.text));
    let result = match sandbox::parse(&body, operation) {
        Ok(result) => result,
        Err(reason) => return fail(reason).await,
    };
    drop(body);
    drop(response); // Releases service concurrency before the recheck.
    let response = call(state, w, &w.contract.status_operation).await?;
    if response.status == 410 {
        return fail("result_expired").await;
    }
    if !(200..300).contains(&response.status) {
        return watches::defer(&state.db, w).await;
    }
    let value: serde_json::Value = match serde_json::from_str(&response.text) {
        Ok(value) => value,
        Err(_) => return fail("artifact_invalid").await,
    };
    let mut after = match sandbox::status(&value, operation, chrono::Utc::now()) {
        Ok(s) => s,
        Err(reason) => return fail(reason).await,
    };
    // Lifecycle versions fence this fetch, including a token appearing or
    // disappearing. Content identity remains derived from the operation/bytes.
    if before.version != after.version {
        return fail("artifact_source_changed").await;
    }
    after.expires_at = before.expires_at.into_iter().chain(after.expires_at).min();
    drop(response);
    if w.artifact_result
        .as_ref()
        .is_some_and(|m| m.source_result_identity != result.identity)
    {
        return fail("artifact_source_changed").await;
    }
    artifacts::persist(
        &state.db,
        &state.encryption_keys,
        w,
        &result,
        &after,
        &reservation,
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
            Ok(Err(AppError::Conflict(reason)))
                if crate::services::generated_artifacts::admitted(&w)
                    && reason == "artifact_source_changed" =>
            {
                watches::ready(&state.db, &w, None, "artifact_source_changed", None).await?;
            }
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

#[cfg(test)]
mod invocation_cancellation_tests {
    use super::*;
    #[tokio::test]
    async fn invocation_async_cancel_outcomes_do_not_claim_accepted_or_failed_requests_stopped() {
        let budget = Duration::from_millis(10);
        for (operation, endpoint, reason) in [
            (None, Some("cancel"), "cancellation_operation_id_missing"),
            (Some("id"), None, "cancellation_unsupported"),
        ] {
            let result = tracked_cancel_outcome(
                operation,
                endpoint,
                budget,
                || async { panic!("must not dispatch") },
                || async { panic!("must not verify") },
            )
            .await;
            assert_eq!(result, (false, reason));
        }
        for (status, confirmed) in [
            (200, true),
            (204, true),
            (202, true),
            (400, false),
            (503, false),
        ] {
            assert_eq!(
                tracked_cancel_outcome(
                    Some("id"),
                    Some("cancel"),
                    budget,
                    || async { Ok(status) },
                    || async { Ok(true) }
                )
                .await
                .0,
                confirmed
            );
        }
        for status in [200, 202, 204] {
            assert!(
                !tracked_cancel_outcome(
                    Some("id"),
                    Some("cancel"),
                    budget,
                    || async { Ok(status) },
                    || async { Ok(false) }
                )
                .await
                .0
            );
        }
        assert!(
            !tracked_cancel_outcome(
                Some("id"),
                Some("cancel"),
                budget,
                std::future::pending,
                || async { Ok(true) }
            )
            .await
            .0
        );
    }
}

#[cfg(test)]
mod invocation_cancel_http_tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn invocation_async_cancel_requires_terminal_declared_status_after_http_acceptance() {
        let contract = serde_json::from_value(serde_json::json!({
            "status_operation":"status","result_operation":"result","cancel_operation":"cancel",
            "id_field":"/id","id_parameter":"id","status_field":"/status",
            "success_states":["done"],"failure_states":["failed","cancelled"],"error_field":null
        }))
        .unwrap();
        for (code, phase, expected) in [
            (200, "running", false),
            (200, "cancelling", false),
            (202, "running", false),
            (202, "cancelled", true),
            (204, "done", true),
            (200, "failed", true),
            (503, "done", false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/cancel"))
                .respond_with(ResponseTemplate::new(code))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/status"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":phase})),
                )
                .expect(if code < 300 { 1 } else { 0 })
                .mount(&server)
                .await;
            let client = reqwest::Client::new();
            let cancel = format!("{}/cancel", server.uri());
            let status = format!("{}/status", server.uri());
            let outcome = tracked_cancel_outcome(
                Some("fixture"),
                Some("cancel"),
                Duration::from_secs(2),
                || async {
                    let response = client.post(&cancel).send().await.map_err(|_| {
                        crate::errors::AppError::Internal("Loopback cancel failed".into())
                    })?;
                    Ok(response.status().as_u16())
                },
                || async {
                    let response = client.get(&status).send().await.map_err(|_| {
                        crate::errors::AppError::Internal("Loopback status failed".into())
                    })?;
                    let text = response.text().await.map_err(|_| {
                        crate::errors::AppError::Internal("Loopback status failed".into())
                    })?;
                    Ok(terminal_contract_status(&contract, &text))
                },
            )
            .await;
            assert_eq!(outcome.0, expected);
            server.verify().await;
        }
    }
}
