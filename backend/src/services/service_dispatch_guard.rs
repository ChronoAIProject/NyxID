//! Service-independent checks at the final HTTP dispatch boundary.
use crate::errors::AppResult;

/// Protected requests must be buffered so the guard can bind their exact bytes.
#[async_trait::async_trait]
pub(crate) trait ServiceDispatchGuard: Send + Sync {
    async fn before_send(&self, request: &reqwest::Request) -> AppResult<()>;

    async fn observe_response(&self, response: reqwest::Response) -> AppResult<reqwest::Response>;
}
