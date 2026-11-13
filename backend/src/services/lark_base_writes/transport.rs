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

pub(super) struct NotSent(pub AppError);

impl From<AppError> for NotSent {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}

impl Transport<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn send(
        &self,
        guard: &LarkManagedGuard<'_>,
        row: &LarkBaseWrite,
        phase: WriteState,
        method: reqwest::Method,
        child: &ChildPath,
        query: Option<&str>,
        body: Option<bytes::Bytes>,
        content_type: &str,
    ) -> Result<Reply, NotSent> {
        let path = child.logical.as_str();
        let canonical = super::super::proxy_authorization::CanonicalPath::from_mcp_built(path)?;
        let (_, selected) = super::super::destination_routing::select_target(
            &self.target.service,
            method.as_str(),
            &canonical,
        )?;
        if selected != self.target.target_id {
            return Err(attachment::eligibility::Reason::DestinationChanged
                .error()
                .into());
        }
        guard
            .revalidate_effect(row, phase)
            .await
            .inspect_err(diagnostic::error)?;
        let mut headers = reqwest::header::HeaderMap::new();
        if body.is_some() {
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                content_type.parse().map_err(|_| invalid())?,
            );
        }
        let effect = guard.effect(row, phase);
        let response = proxy::forward_request_with_resource_proof(
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
            None,
            None,
            None,
            child.route.as_ref(),
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
            Err(proxy::ForwardRequestError::PreDispatchRejected(error)) => Err(NotSent(error)),
            // Credential preparation precedes send for these HTTP auth paths.
            // With an execution permit, Application can also mean completion
            // failed AFTER send; retain uncertainty in that case.
            Err(proxy::ForwardRequestError::Application(error))
                if matches!(
                    self.target.auth_method.as_str(),
                    "bearer" | "token_exchange"
                ) && self.target.execution_permission.is_none()
                    && !super::super::credential_verification::checking() =>
            {
                Err(NotSent(Failure::CredentialPreparation.error(error)))
            }
            Err(_) => Ok(Reply::Transport),
        }
    }
}
