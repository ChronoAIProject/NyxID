use super::super::{assistant_service_execution::LarkManagedGuard, proxy_service as proxy};
use super::*;

pub(crate) struct Transport<'a> {
    pub target: &'a proxy::ProxyTarget,
    pub tokens: &'a super::super::provider_token_exchange_service::TokenExchangeCache,
    pub cache: &'a super::super::cloud_response_cache::CloudResponseCache,
    pub permit: super::super::billing::route_inventory::BillingEgressPermit,
    pub identity: Vec<(String, String)>,
    pub delegated: Vec<super::super::delegation_service::DelegatedCredential>,
    pub caller: Option<&'a str>,
}

pub(super) enum Reply {
    Json(u16, Value),
    Transport,
}
impl Transport<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn send(
        &self,
        guard: &LarkManagedGuard<'_>,
        row: &LarkBaseWrite,
        phase: WriteState,
        method: reqwest::Method,
        path: &str,
        query: Option<&str>,
        body: Option<bytes::Bytes>,
        content_type: &str,
    ) -> AppResult<Reply> {
        let canonical = super::super::proxy_authorization::CanonicalPath::from_mcp_built(path)?;
        let (_, selected) = super::super::destination_routing::select_target(
            &self.target.service,
            method.as_str(),
            &canonical,
        )?;
        if selected != self.target.target_id {
            return Err(attachment::refused());
        }
        guard.revalidate_effect(row, phase).await?;
        let mut headers = reqwest::header::HeaderMap::new();
        if body.is_some() {
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                content_type.parse().map_err(|_| invalid())?,
            );
        }
        let effect = guard.effect(row, phase);
        let response = proxy::forward_request_with_egress_guard(
            proxy::no_redirect_http_client(),
            self.target,
            method,
            path,
            query,
            headers,
            proxy::ProxyBody::Buffered(body),
            self.identity.clone(),
            self.delegated.clone(),
            self.caller,
            self.tokens,
            self.cache,
            Vec::new(),
            self.permit,
            Some(&effect),
        )
        .await;
        match response {
            Ok(mut response) => {
                let status = response.status().as_u16();
                let mut bytes = Vec::new();
                loop {
                    match response.chunk().await {
                        Ok(Some(chunk)) if bytes.len() + chunk.len() <= 1024 * 1024 => {
                            bytes.extend_from_slice(&chunk)
                        }
                        Ok(None) => break,
                        _ => return Ok(Reply::Transport),
                    }
                }
                Ok(Reply::Json(
                    status,
                    serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                ))
            }
            Err(proxy::ForwardRequestError::PreDispatchRejected(error)) => Err(error),
            Err(_) => Ok(Reply::Transport),
        }
    }
}
