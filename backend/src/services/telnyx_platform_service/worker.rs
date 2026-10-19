use super::*;
use crate::services::{
    assistant_service_execution::OwnedHttpEgressGuard,
    managed_execution_service as managed,
    proxy_service::{self, ForwardRequestError},
    service_concurrency_service::Lease,
};
use futures::future::BoxFuture;
use zeroize::Zeroize;

pub(super) struct SecretTarget(pub ProxyTarget);
impl std::ops::Deref for SecretTarget {
    type Target = ProxyTarget;
    fn deref(&self) -> &ProxyTarget {
        &self.0
    }
}
impl Drop for SecretTarget {
    fn drop(&mut self) {
        self.0.credential.zeroize();
    }
}

pub(super) struct Exchange {
    pub db: Database,
    pub billing: Arc<BillingService>,
    pub target: SecretTarget,
    pub contract: ManagedExecutionContract,
    pub body: Bytes,
    pub handle: managed::ManagedExecutionHandle,
    pub proof: ManagedEgressProof,
    pub lease: Option<Lease>,
    pub guard: OwnedHttpEgressGuard,
    #[cfg(test)]
    pub hooks: Option<TestHooks>,
}

// Keep construction and polling of the guarded exchange outside the already
// large HTTP/MCP dispatch frames, including when there is no assistant guard.
pub(super) fn run(
    exchange: Exchange,
) -> BoxFuture<'static, Result<ManagedResponse, ManagedExecutionError>> {
    Box::pin(run_inner(exchange))
}

async fn run_inner(exchange: Exchange) -> Result<ManagedResponse, ManagedExecutionError> {
    let mut rejected = None;
    let attempt = exchange.attempt(&mut rejected);
    let bounded = async {
        match &exchange.lease {
            Some(lease) => lease.run(attempt).await,
            None => attempt.await,
        }
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(120), bounded).await;
    // The proof lives outside both cancellation boundaries. Neither a timeout
    // nor lease loss may erase a refusal already returned by the final sender.
    if let Some(error) = rejected {
        if managed::reject_before_send(&exchange.db, &exchange.handle)
            .await
            .is_ok()
        {
            return Err(ManagedExecutionError::Rejected(error));
        }
        tracing::error!(execution_id = %exchange.handle.execution_id(),
            "Managed pre-send refusal cleanup requires durable recovery");
        // If the durable cancellation marker committed, reconcile completes
        // cleanup. Otherwise preserve the hold rather than guess its outcome.
    } else if let Ok(Ok(response)) = result {
        return Ok(response);
    }
    if managed::cancel_unforwarded(&exchange.db, &exchange.handle)
        .await
        .is_err()
        || managed::mark_pending(
            &exchange.db,
            &exchange.handle,
            "provider_outcome_unverified",
        )
        .await
        .is_err()
    {
        tracing::error!(execution_id = %exchange.handle.execution_id(),
            "Managed outcome requires durable recovery");
    }
    Err(ManagedExecutionError::OutcomeUnknown(
        AppError::ManagedExecutionPending(exchange.handle.execution_id().to_owned()),
    ))
}

impl Exchange {
    fn attempt<'a>(
        &'a self,
        rejected: &'a mut Option<AppError>,
    ) -> BoxFuture<'a, AppResult<ManagedResponse>> {
        Box::pin(self.attempt_inner(rejected))
    }

    async fn attempt_inner(&self, rejected: &mut Option<AppError>) -> AppResult<ManagedResponse> {
        #[cfg(test)]
        if let Some(pause) = self.hooks.as_ref().and_then(|h| h.funded.as_ref()) {
            pause.wait().await;
        }
        let guard = self.guard.borrowed(&self.target);
        if let Err(error) = guard.revalidate().await {
            *rejected = Some(error);
            return Err(invalid("authority rejected before dispatch"));
        }
        managed::admit_dispatch(&self.db, &self.handle).await?;
        #[cfg(test)]
        if let Some(pause) = self.hooks.as_ref().and_then(|h| h.admitted.as_ref()) {
            pause.wait().await;
        }
        let client = proxy_service::no_redirect_http_client();
        #[cfg(test)]
        let client = self.hooks.as_ref().map(|h| &h.client).unwrap_or(client);
        let response = proxy_service::forward_request_with_egress_guard(
            client,
            &self.target,
            reqwest::Method::POST,
            CHAT_PATH,
            None,
            reqwest::header::HeaderMap::from_iter([(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            )]),
            ProxyBody::Buffered(Some(self.body.clone())),
            Vec::new(),
            Vec::new(),
            None,
            &crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
            &crate::services::cloud_response_cache::CloudResponseCache::new(0),
            Vec::new(),
            crate::services::billing::route_inventory::managed_execution_permit(self.proof),
            Some(&guard),
        )
        .await;
        let mut response = match response {
            Ok(response) => response,
            Err(ForwardRequestError::PreDispatchRejected(error)) => {
                *rejected = Some(error);
                return Err(invalid("authority rejected before dispatch"));
            }
            Err(_) => return Err(invalid("provider outcome is pending")),
        };
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| invalid("provider response is incomplete"))?
        {
            if bytes.len().saturating_add(chunk.len()) as u64 > self.contract.max_response_bytes {
                return Err(invalid("provider response exceeded its bound"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !(200..300).contains(&status) {
            return Err(invalid("provider returned an unverified error outcome"));
        }
        let (usage, provider_id) = validate_usage(&self.contract, &bytes, self.body.len())?;
        managed::complete(
            &self.db,
            &self.billing,
            &self.handle,
            usage,
            Some(&provider_id),
        )
        .await?;
        #[cfg(test)]
        if let Some(completed) = self.hooks.as_ref().and_then(|h| h.completed.as_ref()) {
            completed.notify_one();
        }
        Ok(ManagedResponse {
            execution_id: self.handle.execution_id().to_owned(),
            status,
            body: Bytes::from(bytes),
        })
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct Pause {
    pub entered: Arc<tokio::sync::Notify>,
    pub resume: Arc<tokio::sync::Notify>,
}
#[cfg(test)]
impl Pause {
    async fn wait(&self) {
        self.entered.notify_one();
        self.resume.notified().await;
    }
}
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct TestHooks {
    pub client: reqwest::Client,
    pub funded: Option<Pause>,
    pub admitted: Option<Pause>,
    pub completed: Option<Arc<tokio::sync::Notify>>,
}
#[cfg(test)]
tokio::task_local! { pub(crate) static TEST_HOOKS: TestHooks; }
