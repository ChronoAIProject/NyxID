use std::time::{Duration, Instant};

use async_trait::async_trait;
use ed25519_dalek::VerifyingKey;
use rand::RngCore;
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    MAX_HTTP_BYTES, MAX_LINKS, WitnessError, WitnessResult, now_ms, receipt_digest, valid_hash,
    verify_observation, verify_receipt,
};
use crate::models::audit_witness::*;

/// A verified fresh challenge response, held only in this process. A persisted
/// SignedReceipt cannot construct this proof or restart its monotonic clock.
pub struct VerifiedContact {
    receipt: SignedReceipt,
    contacted_at: Instant,
    witness_time_ms: u64,
}
impl VerifiedContact {
    pub fn receipt(&self) -> &SignedReceipt {
        &self.receipt
    }
    pub fn age(&self) -> Duration {
        self.contacted_at.elapsed()
    }
    pub fn witness_time_ms(&self) -> u64 {
        self.witness_time_ms
    }
}

/// A fresh, process-local exchange that another provider adapter can complete.
/// No Clone/Deserialize or caller-supplied nonce/clock can turn an old receipt
/// into renewed contact. Completion consumes the challenge exactly once.
pub struct ObservationChallenge {
    request: ObserveRequest,
    contacted_at: Instant,
    before_ms: u64,
}
impl ObservationChallenge {
    pub fn begin(chain_id: Uuid) -> WitnessResult<Self> {
        if !super::valid_id(chain_id) {
            return Err(WitnessError::Invalid);
        }
        let mut nonce = [0_u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        Ok(Self {
            request: ObserveRequest {
                chain_id,
                challenge: hex::encode(nonce),
            },
            contacted_at: Instant::now(),
            before_ms: now_ms()?,
        })
    }

    pub fn request(&self) -> &ObserveRequest {
        &self.request
    }

    pub fn complete(
        self,
        observed: SignedObservation,
        key: &VerifyingKey,
    ) -> WitnessResult<VerifiedContact> {
        let after = now_ms()?;
        // Subsequent admission uses Instant. Clock discontinuity during this
        // exchange fails instead of manufacturing a longer freshness window.
        if after < self.before_ms
            || after
                .saturating_sub(self.before_ms)
                .abs_diff(self.contacted_at.elapsed().as_millis() as u64)
                > 1000
        {
            return Err(WitnessError::Unavailable);
        }
        verify_observation(
            &observed,
            self.request.chain_id,
            key,
            &self.request.challenge,
            self.before_ms.saturating_sub(30_000),
            after.saturating_add(30_000),
        )?;
        Ok(VerifiedContact {
            receipt: observed.body.latest,
            contacted_at: self.contacted_at,
            witness_time_ms: observed.body.witness_time_ms,
        })
    }
}

/// Provider-neutral interface. Implementations must authenticate pinned chain
/// identity, durable receipts and fresh challenge responses. Enrollment/reset,
/// deletion and signing-key custody deliberately have no client methods.
#[async_trait]
pub trait WitnessClient: Send + Sync {
    async fn advance(&self, request: AdvanceRequest) -> WitnessResult<SignedReceipt>;
    async fn observe(&self) -> WitnessResult<VerifiedContact>;
    async fn history(&self, from_receipt: u64, limit: usize) -> WitnessResult<Vec<SignedReceipt>>;
    /// A retrieved claim is evidence, not reusable dispatch permission. The
    /// caller must consume its own attempt/process claim once, in memory, and
    /// treat recovery without a terminal outcome as uncertain. No lease expiry.
    async fn claim_dispatch(&self, request: DispatchRequest) -> WitnessResult<SignedReceipt>;
}

pub struct HttpWitness {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    chain: Uuid,
    key: VerifyingKey,
    token: Zeroizing<String>,
}

impl std::fmt::Debug for HttpWitness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpWitness([REDACTED])")
    }
}

impl HttpWitness {
    pub fn new(
        endpoint: &str,
        chain: Uuid,
        pinned_key: &str,
        token: Zeroizing<String>,
    ) -> WitnessResult<Self> {
        Self::build(endpoint, chain, pinned_key, token, false)
    }

    /// Explicit local validation only. This does not establish independent-host
    /// production readiness; the application runtime must enforce that gate.
    pub fn loopback_for_validation(
        endpoint: &str,
        chain: Uuid,
        pinned_key: &str,
        token: Zeroizing<String>,
    ) -> WitnessResult<Self> {
        Self::build(endpoint, chain, pinned_key, token, true)
    }

    fn build(
        endpoint: &str,
        chain: Uuid,
        pinned_key: &str,
        token: Zeroizing<String>,
        loopback: bool,
    ) -> WitnessResult<Self> {
        let endpoint = reqwest::Url::parse(endpoint).map_err(|_| WitnessError::Invalid)?;
        let local = endpoint
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if (endpoint.scheme() != "https" && !(loopback && endpoint.scheme() == "http" && local))
            || endpoint.host().is_none()
            || endpoint.port() == Some(0)
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
            || !super::valid_id(chain)
            || !valid_hash(pinned_key)
            || !valid_hash(&token)
        {
            return Err(WitnessError::Invalid);
        }
        let key_bytes: [u8; 32] = hex::decode(pinned_key)
            .map_err(|_| WitnessError::Invalid)?
            .try_into()
            .map_err(|_| WitnessError::Invalid)?;
        let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| WitnessError::Invalid)?;
        // This client can also be used outside server_main. Feature unification
        // may enable both rustls providers, so establish a default if absent.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| WitnessError::Unavailable)?;
        Ok(Self {
            client,
            endpoint,
            chain,
            key,
            token,
        })
    }

    async fn post<T: Serialize + Sync, R: DeserializeOwned>(
        &self,
        route: &str,
        request: &T,
    ) -> WitnessResult<R> {
        let mut response = self
            .client
            .post(
                self.endpoint
                    .join(route)
                    .map_err(|_| WitnessError::Invalid)?,
            )
            .bearer_auth(self.token.as_str())
            .json(request)
            .send()
            .await
            .map_err(|_| WitnessError::Unavailable)?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                401 => WitnessError::Unauthorized,
                400 | 413 | 422 => WitnessError::Invalid,
                409 => WitnessError::Conflict,
                _ => WitnessError::Unavailable,
            });
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_HTTP_BYTES as u64)
        {
            return Err(WitnessError::Corrupt);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| WitnessError::Unavailable)?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_HTTP_BYTES {
                return Err(WitnessError::Corrupt);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| WitnessError::Corrupt)
    }
}

#[async_trait]
impl WitnessClient for HttpWitness {
    async fn advance(&self, request: AdvanceRequest) -> WitnessResult<SignedReceipt> {
        if request.chain_id != self.chain
            || request.links.is_empty()
            || request.links.len() > MAX_LINKS
        {
            return Err(WitnessError::Invalid);
        }
        let record: SignedReceipt = self.post("v1/advance", &request).await?;
        verify_receipt(&record, self.chain, &self.key)?;
        if record.body.event
            != (WitnessEvent::Advance {
                request: request.clone(),
            })
            || record.body.previous_receipt != request.previous_receipt
            || Some(&record.body.head) != request.links.last()
        {
            return Err(WitnessError::Corrupt);
        }
        Ok(record)
    }

    async fn observe(&self) -> WitnessResult<VerifiedContact> {
        let challenge = ObservationChallenge::begin(self.chain)?;
        let observed = self.post("v1/observe", challenge.request()).await?;
        challenge.complete(observed, &self.key)
    }

    async fn history(&self, from_receipt: u64, limit: usize) -> WitnessResult<Vec<SignedReceipt>> {
        if limit == 0 || limit > MAX_LINKS {
            return Err(WitnessError::Invalid);
        }
        let records: Vec<SignedReceipt> = self
            .post(
                "v1/history",
                &HistoryRequest {
                    chain_id: self.chain,
                    from_receipt,
                    limit,
                },
            )
            .await?;
        if records.len() > limit {
            return Err(WitnessError::Corrupt);
        }
        for (i, record) in records.iter().enumerate() {
            verify_receipt(record, self.chain, &self.key)?;
            if record.body.receipt_seq
                != from_receipt
                    .checked_add(i as u64)
                    .ok_or(WitnessError::Invalid)?
                || (i > 0 && record.body.previous_receipt != receipt_digest(&records[i - 1]))
            {
                return Err(WitnessError::Corrupt);
            }
        }
        // The caller binds the first predecessor to its protected prior receipt
        // and compares the final cursor with observe(); history alone is not fresh.
        Ok(records)
    }

    async fn claim_dispatch(&self, request: DispatchRequest) -> WitnessResult<SignedReceipt> {
        if request.chain_id != self.chain {
            return Err(WitnessError::Invalid);
        }
        let record: SignedReceipt = self.post("v1/dispatch", &request).await?;
        verify_receipt(&record, self.chain, &self.key)?;
        if record.body.previous_receipt != request.previous_receipt
            || record.body.event != (WitnessEvent::Dispatch { request })
        {
            return Err(WitnessError::Corrupt);
        }
        Ok(record)
    }
}
