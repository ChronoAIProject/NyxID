//! Service-independent checks before request rewriting and final HTTP dispatch.
use crate::errors::AppResult;

/// Borrowed caller inputs before authentication or delegated credential rewriting.
/// No Debug: bodies and headers can contain private request material.
pub(crate) struct OriginalRequest<'a> {
    pub(crate) method: &'a str,
    pub(crate) path: &'a str,
    pub(crate) query: Option<&'a str>,
    pub(crate) body: &'a [u8],
    pub(crate) headers: &'a reqwest::header::HeaderMap,
}

/// Protected requests must be buffered so the guard can bind their exact bytes.
#[async_trait::async_trait]
pub(crate) trait ServiceDispatchGuard: Send + Sync {
    /// Validation only: durable dispatch state belongs in `before_send` after
    /// authentication and request construction have succeeded.
    fn validate_original_request(&self, _request: OriginalRequest<'_>) -> AppResult<()> {
        Ok(())
    }

    async fn before_send(&self, request: &reqwest::Request) -> AppResult<()>;

    async fn observe_response(&self, response: reqwest::Response) -> AppResult<reqwest::Response>;
}
