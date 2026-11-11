//! Acquisition owns verification and persistence; transport stays in handlers.
use super::*;
use crate::services::{async_service_operation as watches, mcp_service::ToolResponse};
static ACQUISITIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static CPU: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
pub async fn acquire<F, Fut>(
    db: &Database,
    keys: &EncryptionKeys,
    w: &AsyncServiceOperation,
    before: &serde_json::Value,
    call: F,
) -> AppResult<()>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = AppResult<ToolResponse>>,
{
    use crate::services::{
        assistant_upload_service::byte_budget,
        generated_artifacts::{self as artifacts, sandbox},
    };
    let fail = |reason: &'static str| watches::ready(db, w, None, reason, None);
    if w.deadline <= Utc::now() {
        return fail("timeout").await;
    }
    let operation = w
        .operation_id
        .as_deref()
        .ok_or_else(|| crate::errors::AppError::Conflict("artifact_source_changed".into()))?
        .to_owned();
    let before = match sandbox::status(before, &operation, chrono::Utc::now()) {
        Ok(s) => s,
        Err(reason) => return fail(reason).await,
    };
    let _worker = ACQUISITIONS
        .acquire()
        .await
        .map_err(|_| crate::errors::AppError::Internal("Artifact worker unavailable".into()))?;
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await?;
    let mut response =
        match tokio::time::timeout(CALL_TIMEOUT, call(w.contract.result_operation.clone())).await {
            Ok(response) => response?,
            Err(_) => return watches::defer(db, w).await,
        };
    if response.status == 410 {
        return fail("result_expired").await;
    }
    // Explicit source expiry keeps its contract above. Otherwise known
    // malformed/over-cap bytes are terminal even when the provider returned 5xx.
    if response.is_body_too_large() {
        return fail("artifact_too_large").await;
    }
    if response.is_malformed_utf8() {
        return fail("artifact_invalid").await;
    }
    if response.status >= 500 {
        return watches::defer(db, w).await;
    }
    if !(200..300).contains(&response.status) {
        return fail("result_unavailable").await;
    }
    let body = zeroize::Zeroizing::new(std::mem::take(&mut response.text));
    let cpu = CPU.clone().acquire_owned().await.map_err(|_| invalid())?;
    // The blocking closure owns both body and budget even if its async caller is cancelled.
    let (parsed, reservation) = tokio::task::spawn_blocking(move || {
        let _cpu = cpu;
        let parsed = sandbox::parse(&body, &operation);
        (parsed, reservation)
    })
    .await
    .map_err(|_| AppError::Internal("Artifact validator failed".into()))?;
    let result = match parsed {
        Ok(result) => result,
        Err(reason) => return fail(reason).await,
    };
    drop(response); // Releases service concurrency before the recheck.
    if w.deadline <= Utc::now() {
        return fail("timeout").await;
    }
    let response =
        match tokio::time::timeout(CALL_TIMEOUT, call(w.contract.status_operation.clone())).await {
            Ok(response) => response?,
            Err(_) => return watches::defer(db, w).await,
        };
    if response.status == 410 {
        return fail("result_expired").await;
    }
    if !(200..300).contains(&response.status) {
        return watches::defer(db, w).await;
    }
    let value: serde_json::Value = match serde_json::from_str(&response.text) {
        Ok(value) => value,
        Err(_) => return fail("artifact_invalid").await,
    };
    let mut after = match sandbox::status(
        &value,
        w.operation_id.as_deref().ok_or_else(invalid)?,
        chrono::Utc::now(),
    ) {
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
    if w.deadline <= Utc::now() {
        return fail("timeout").await;
    }
    if w.artifact_result
        .as_ref()
        .is_some_and(|m| m.source_result_identity != result.identity)
    {
        return fail("artifact_source_changed").await;
    }
    artifacts::persist(db, keys, w, &result, &after, &reservation).await
}
