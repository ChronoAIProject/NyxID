//! Provider-independent HTTP transport. No identity/delegation headers are sent
//! to temporary upload recipients. Both transports use the checked DNS client.
use super::super::proxy_service as proxy;
use super::*;
use nyxid_service_adapters::image_publication::{ApiRequest, Upload, UploadCredential};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

/// A cancelled send remains a possible provider effect. Proven pre-dispatch
/// failures clear only the current attempt, retaining earlier child effects.
pub(crate) struct Evidence<'a> {
    billing: &'a super::super::billing::BillingService,
    metered: &'a super::super::billing::meter::MeteredProxyContext,
    reserved: AtomicBool,
    completed: AtomicBool,
    in_flight: AtomicBool,
    status: AtomicU16,
}
impl<'a> Evidence<'a> {
    pub(crate) fn new(
        billing: &'a super::super::billing::BillingService,
        metered: &'a super::super::billing::meter::MeteredProxyContext,
    ) -> Self {
        Self {
            billing,
            metered,
            reserved: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            in_flight: AtomicBool::new(false),
            status: AtomicU16::new(0),
        }
    }
    async fn reserve(&self) -> AppResult<()> {
        if !self.reserved.load(Ordering::Acquire) {
            self.billing.mark_forwarded(self.metered).await?;
            self.reserved.store(true, Ordering::Release);
        }
        Ok(())
    }
    fn begin(&self) {
        self.status.store(0, Ordering::Release);
        self.in_flight.store(true, Ordering::Release);
    }
    fn response(&self, status: u16) {
        self.status.store(status, Ordering::Release);
        self.completed.store(true, Ordering::Release);
        self.in_flight.store(false, Ordering::Release);
    }
    fn not_sent(&self) {
        self.in_flight.store(false, Ordering::Release);
    }
    fn unknown(&self) {
        self.completed.store(true, Ordering::Release);
        self.in_flight.store(false, Ordering::Release);
    }
    pub(crate) fn has_effect(&self) -> bool {
        self.completed.load(Ordering::Acquire) || self.in_flight.load(Ordering::Acquire)
    }
    pub(crate) fn status(&self) -> Option<u16> {
        let status = self.status.load(Ordering::Acquire);
        (status != 0).then_some(status)
    }
}

/// Installed only at the proxy's final send boundary, after credential
/// preparation. Ordinary authority checks do not imply a provider dispatch.
pub(crate) struct Effect<'a> {
    guard: &'a Guard<'a>,
    evidence: &'a Evidence<'a>,
}
impl Effect<'_> {
    pub(crate) fn revalidate(&self) -> futures::future::BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            self.evidence.reserve().await?;
            self.guard.revalidate().await?;
            self.evidence.begin();
            Ok(())
        })
    }
}

pub(crate) struct Transport<'a> {
    pub target: &'a proxy::ProxyTarget,
    pub tokens: &'a super::super::provider_token_exchange_service::TokenExchangeCache,
    pub cache: &'a super::super::cloud_response_cache::CloudResponseCache,
    pub permit: super::super::billing::route_inventory::BillingEgressPermit,
    pub evidence: &'a Evidence<'a>,
}
pub(super) enum ApiFailure {
    NotSent(AppError),
    Unknown(AppError),
}
impl From<AppError> for ApiFailure {
    fn from(error: AppError) -> Self {
        Self::NotSent(error)
    }
}
impl From<ApiFailure> for AppError {
    fn from(error: ApiFailure) -> Self {
        match error {
            ApiFailure::NotSent(e) | ApiFailure::Unknown(e) => e,
        }
    }
}
pub(super) struct Reply {
    pub status: u16,
    pub body: Value,
    pub receipt: Option<String>,
    pub body_complete: bool,
}
impl Transport<'_> {
    pub(super) async fn api(
        &self,
        guard: &Guard<'_>,
        adapter: &dyn ImagePublisher,
        request: ApiRequest,
    ) -> Result<Reply, ApiFailure> {
        validate_origin(adapter, &self.target.base_url)?;
        // An adapter's API requests must stay at its selected connection origin.
        if !request.path.starts_with('/')
            || request.path.starts_with("//")
            || request.path.contains(['?', '#', '\\'])
        {
            return Err(refused("adapter_path").into());
        }
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in adapter.headers() {
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| refused("adapter_header"))?,
                value.parse().map_err(|_| refused("adapter_header"))?,
            );
        }
        let body = request
            .body
            .map(|body| serde_json::to_vec(&body).map(bytes::Bytes::from))
            .transpose()
            .map_err(|_| refused("adapter_body"))?;
        if body.is_some() {
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                "application/json".parse().unwrap(),
            );
        }
        let dispatch = Effect {
            guard,
            evidence: self.evidence,
        };
        let effect =
            super::super::assistant_service_execution::HttpEgressGuard::for_image_publication(
                &dispatch,
            );
        let response = proxy::forward_request_with_resource_proof(
            proxy::Redirects::Return,
            self.target,
            reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|_| refused("adapter_method"))?,
            &request.path,
            request.query.as_deref(),
            headers,
            proxy::ProxyBody::Buffered(body),
            Vec::new(),
            Vec::new(),
            None,
            self.tokens,
            self.cache,
            Vec::new(),
            self.permit,
            Some(&effect),
            None,
            None,
            None,
            None,
        )
        .await;
        let mut response = match response {
            Ok(response) => response,
            Err(proxy::ForwardRequestError::PreDispatchRejected(error)) => {
                self.evidence.not_sent();
                return Err(ApiFailure::NotSent(error));
            }
            Err(proxy::ForwardRequestError::Transport(error))
                if error.is_connect() || error.is_builder() =>
            {
                self.evidence.not_sent();
                return Err(ApiFailure::NotSent(refused("provider_unreachable")));
            }
            Err(_) => {
                self.evidence.unknown();
                return Err(ApiFailure::Unknown(refused(
                    "provider_transport_outcome_unknown",
                )));
            }
        };
        let status = response.status().as_u16();
        self.evidence.response(status);
        // Capture the trusted receipt before reading any body. A body failure
        // must never destroy evidence of a successful HTTP creation.
        let receipt = adapter.receipt(response.headers());
        if let Some((id, "dispatching", attempt)) = guard.row {
            let filter = doc! {"_id": id, "state": "dispatching", "attempt": attempt};
            let values = doc! {
                "post": &receipt,
                "http_status": i32::from(status),
                "posted_at": (200..300).contains(&status).then(bson::DateTime::now),
                "updated_at": bson::DateTime::now(),
            };
            let changed = guard
                .db
                .collection::<ImagePublication>(COLLECTION_NAME)
                .update_one(filter, doc! {"$set": values})
                .await
                .map_err(|error| ApiFailure::Unknown(error.into()))?;
            if changed.matched_count != 1 {
                return Err(ApiFailure::Unknown(refused("creation_receipt_not_saved")));
            }
            // Create has no useful body. Do not let an unbounded/stalled body
            // discard a 201 or its header receipt.
            return Ok(Reply {
                status,
                receipt,
                body: Value::Null,
                body_complete: true,
            });
        }
        let mut bytes = Vec::new();
        let mut complete = true;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if bytes.len() + chunk.len() <= 1024 * 1024 => {
                    bytes.extend_from_slice(&chunk)
                }
                Ok(None) => break,
                _ => {
                    complete = false;
                    break;
                }
            }
        }
        Ok(Reply {
            status,
            receipt,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            body_complete: complete,
        })
    }
    pub(super) async fn upload(
        &self,
        guard: &Guard<'_>,
        adapter: &dyn ImagePublisher,
        upload: &Upload,
    ) -> AppResult<u16> {
        if !guard.prepared.upload() || guard.prepared.bytes.is_empty() {
            return Err(refused("file_transfer_not_admitted"));
        }
        if !adapter.permits_upload(&upload.url)
            || upload.expires_at_ms <= Utc::now().timestamp_millis()
        {
            return Err(refused("upload_recipient_or_expiry"));
        }
        let (url, client) = super::super::proxy_destination::http::public_delivery(&upload.url)?;
        // The fixed adapter recipient is checked before selected BYO credential
        // injection. No catalog defaults, proxy identity or caller headers cross.
        let mut request = client
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, &guard.prepared.mime);
        for (name, value) in &upload.headers {
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization" | "cookie" | "host"
            ) || name.to_ascii_lowercase().starts_with("x-nyxid-")
            {
                return Err(refused("adapter_upload_header"));
            }
            request = request.header(name, value);
        }
        request = match upload.credential {
            UploadCredential::None => request,
            UploadCredential::SelectedBearer => {
                request.bearer_auth(self.target.credential.as_str())
            }
        };
        let request = request
            .body(guard.prepared.bytes.clone())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| refused("upload_request"))?;
        self.evidence.reserve().await?;
        guard.revalidate().await?;
        if upload.expires_at_ms <= Utc::now().timestamp_millis() {
            return Err(refused("upload_expired"));
        }
        self.evidence.begin();
        match client.execute(request).await {
            Ok(response) => {
                let status = response.status().as_u16();
                self.evidence.response(status);
                Ok(status)
            }
            Err(error) if error.is_connect() || error.is_builder() => {
                self.evidence.not_sent();
                Err(refused("upload_not_sent"))
            }
            Err(_) => {
                self.evidence.unknown();
                Err(refused("upload_transport"))
            }
        }
    }
}
