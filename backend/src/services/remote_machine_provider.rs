//! Signed provider attestation transport. Signature validation authenticates a
//! reviewed provider's statement; it does not implement or certify its physical
//! runtime/CSI fence. These deferred provider contracts are retained only for
//! regression tests; production lifecycle uses native Sandbox observations.
use chrono::{DateTime, TimeZone, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::remote_machine_fence::{
    RawCleanupEvidence, RuntimeBinding, VerifiedCleanupEvidence, VerifiedDeadlineReceipt,
    VerifiedRuntimeEvidence,
};
use crate::{
    errors::{AppError, AppResult},
    models::remote_machine::ManagedMachine,
};

pub const AUDIENCE: &str = "nyxid-remote-machine-v1";
const MAX_RESPONSE: usize = 64 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Attestation {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    purpose: String,
    challenge: String,
    machine_id: String,
    owner_id: String,
    binding: RuntimeBinding,
    #[serde(default)]
    pvc_uid: Option<String>,
    evidence_ref: String,
    observed_at: i64,
    /// Exact runtime terminal instant, only when the physical provider has
    /// retained it. Absence means observed-stop upper bound, never billable tail.
    #[serde(default)]
    stopped_at_ns: Option<i64>,
    /// Positive observation that this bound runtime was still running.
    /// Writer admission alone is permission, not a billable time observation.
    #[serde(default)]
    running_at_ns: Option<i64>,
    state: String,
    #[serde(default)]
    credentials_revoked: bool,
    #[serde(default)]
    workspace_released: bool,
    #[serde(default)]
    persistent_volume_released: bool,
    #[serde(default)]
    accounting_closed: bool,
}

pub struct SignedProvider {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    key: DecodingKey,
    issuer: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeadlineAttestation {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    purpose: String,
    receipt: nyxid_machine::managed::DeadlineReceipt,
}
fn rejected() -> AppError {
    AppError::Conflict("managed_provider_attestation_pending".into())
}

impl SignedProvider {
    fn verify_deadline(
        &self,
        token: &str,
        control: &nyxid_machine::managed::DeadlineControl,
    ) -> AppResult<VerifiedDeadlineReceipt> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[AUDIENCE]);
        let claims = decode::<DeadlineAttestation>(token, &self.key, &validation)
            .map_err(|_| rejected())?
            .claims;
        let now = Utc::now().timestamp();
        if claims.purpose != "deadline_installed"
            || claims.iat > now
            || claims.iat < now - 60
            || claims.exp <= now
            || claims.exp > claims.iat + 60
        {
            return Err(rejected());
        }
        claims
            .receipt
            .validate_for(control)
            .map_err(|_| rejected())?;
        Ok(VerifiedDeadlineReceipt::from_signed_provider(
            claims.receipt,
        ))
    }

    fn verify(
        &self,
        token: &str,
        node_id: &str,
        owner_id: &str,
        binding: &RuntimeBinding,
        purpose: &str,
        challenge: &str,
    ) -> AppResult<Attestation> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[AUDIENCE]);
        let claims = decode::<Attestation>(token, &self.key, &validation)
            .map_err(|_| rejected())?
            .claims;
        let now = Utc::now().timestamp();
        let invalid_stop = claims.stopped_at_ns.is_some_and(|stop| {
            claims.state != "stopped"
                || stop <= 0
                || stop.div_euclid(1_000_000_000) > claims.observed_at
                || stop > Utc::now().timestamp_nanos_opt().unwrap_or(i64::MIN)
        });
        let invalid_running = claims.running_at_ns.is_some_and(|running| {
            claims.state != "writer_admitted"
                || running <= 0
                || running.div_euclid(1_000_000_000) > claims.observed_at
                || running > Utc::now().timestamp_nanos_opt().unwrap_or(i64::MIN)
        });
        if claims.purpose != purpose
            || invalid_stop
            || invalid_running
            || claims.challenge != challenge
            || claims.machine_id != node_id
            || claims.owner_id != owner_id
            || claims.binding != *binding
            || claims.iat > now
            || claims.iat < now - 60
            || claims.exp <= now
            || claims.exp > claims.iat + 60
            || claims.observed_at > now
            || claims.observed_at < now - 60
            || uuid::Uuid::parse_str(&claims.evidence_ref).is_err()
            || !matches!(
                claims.state.as_str(),
                "writer_admitted" | "stopped" | "cleanup"
            )
        {
            return Err(rejected());
        }
        Ok(claims)
    }

    async fn fetch(
        &self,
        machine: &ManagedMachine,
        purpose: &str,
    ) -> AppResult<Option<Attestation>> {
        let Some(sandbox_uid) = machine.sandbox_uid.clone() else {
            return Ok(None);
        };
        let binding = RuntimeBinding {
            sandbox_uid,
            pod_uid: machine.pod_uid.clone(),
            runtime_generation: machine.runtime_generation,
            credential_epoch: machine.credential_epoch,
            writer_fence: machine.writer_fence.clone(),
        };
        let challenge = uuid::Uuid::new_v4().to_string();
        let mut response = self.client.post(self.endpoint.clone()).json(&json!({"purpose":purpose,"challenge":challenge,"machine_id":machine.node_id,"owner_id":machine.owner_id,"binding":binding,"pvc_uid":machine.pvc_uid})).send().await.map_err(|_| rejected())?;
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|len| len > MAX_RESPONSE as u64)
        {
            return Err(rejected());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| rejected())? {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err(rejected());
            }
            bytes.extend_from_slice(&chunk);
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Response {
            token: String,
        }
        let envelope: Response = serde_json::from_slice(&bytes).map_err(|_| rejected())?;
        self.verify(
            &envelope.token,
            &machine.node_id,
            &machine.owner_id,
            &binding,
            purpose,
            &challenge,
        )
        .map(Some)
    }
}

impl SignedProvider {
    async fn runtime(
        &self,
        machine: &ManagedMachine,
    ) -> AppResult<Option<VerifiedRuntimeEvidence>> {
        let Some(claims) = self.fetch(machine, "runtime").await? else {
            return Ok(None);
        };
        if !matches!(claims.state.as_str(), "writer_admitted" | "stopped") {
            return Err(rejected());
        }
        let stopped = claims.state == "stopped";
        let observed = if stopped || claims.running_at_ns.is_some() {
            // Receipt bounds an authenticated subsecond observation. The
            // signed seconds timestamp is too coarse for precise accounting;
            // only the separate running/terminal instant can advance charges.
            Utc::now()
        } else {
            Utc.timestamp_opt(claims.observed_at, 0)
                .single()
                .ok_or_else(rejected)?
        };
        Ok(Some(VerifiedRuntimeEvidence::from_signed_provider(
            claims.binding,
            claims.iss,
            claims.evidence_ref,
            stopped,
            observed,
            claims
                .stopped_at_ns
                .map(DateTime::<Utc>::from_timestamp_nanos),
            claims
                .running_at_ns
                .map(DateTime::<Utc>::from_timestamp_nanos),
        )))
    }
    async fn cleanup(
        &self,
        machine: &ManagedMachine,
    ) -> AppResult<Option<VerifiedCleanupEvidence>> {
        let Some(claims) = self.fetch(machine, "cleanup").await? else {
            return Ok(None);
        };
        if claims.state != "cleanup" || claims.pvc_uid != machine.pvc_uid {
            return Err(rejected());
        }
        Ok(Some(VerifiedCleanupEvidence::from_signed_provider(
            RawCleanupEvidence {
                binding: claims.binding,
                pvc_uid: claims.pvc_uid,
                evidence_ref: claims.evidence_ref,
                credentials_revoked: claims.credentials_revoked,
                workspace_released: claims.workspace_released,
                persistent_volume_released: claims.persistent_volume_released,
                accounting_closed: claims.accounting_closed,
                observed_at: Utc
                    .timestamp_opt(claims.observed_at, 0)
                    .single()
                    .ok_or_else(rejected)?,
                provider_id: claims.iss,
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    fn fixture() -> (SignedProvider, Attestation, EncodingKey) {
        let keys = crate::test_utils::cached_test_jwt_keys();
        let now = Utc::now().timestamp();
        let claims = Attestation {
            iss: "reviewed-provider".into(),
            aud: AUDIENCE.into(),
            iat: now,
            exp: now + 60,
            purpose: "runtime".into(),
            challenge: uuid::Uuid::new_v4().to_string(),
            machine_id: uuid::Uuid::new_v4().to_string(),
            owner_id: uuid::Uuid::new_v4().to_string(),
            binding: RuntimeBinding {
                sandbox_uid: uuid::Uuid::new_v4().to_string(),
                pod_uid: Some(uuid::Uuid::new_v4().to_string()),
                runtime_generation: 2,
                credential_epoch: 4,
                writer_fence: uuid::Uuid::new_v4().to_string(),
            },
            pvc_uid: Some(uuid::Uuid::new_v4().to_string()),
            evidence_ref: uuid::Uuid::new_v4().to_string(),
            observed_at: now,
            stopped_at_ns: None,
            running_at_ns: None,
            state: "writer_admitted".into(),
            credentials_revoked: false,
            workspace_released: false,
            persistent_volume_released: false,
            accounting_closed: false,
        };
        (
            SignedProvider {
                client: reqwest::Client::new(),
                endpoint: "https://provider.invalid/attest".parse().unwrap(),
                key: keys.decoding.clone(),
                issuer: claims.iss.clone(),
            },
            claims,
            keys.encoding.clone(),
        )
    }
    fn token(claims: &Attestation, key: &EncodingKey) -> String {
        encode(&Header::new(Algorithm::RS256), claims, key).unwrap()
    }

    #[test]
    fn provider_signature_binds_fresh_challenge_owner_and_runtime() {
        let (provider, claims, key) = fixture();
        let signed = token(&claims, &key);
        assert!(
            provider
                .verify(
                    &signed,
                    &claims.machine_id,
                    &claims.owner_id,
                    &claims.binding,
                    "runtime",
                    &claims.challenge
                )
                .is_ok()
        );
        assert!(
            provider
                .verify(
                    &signed,
                    &claims.machine_id,
                    &claims.owner_id,
                    &claims.binding,
                    "runtime",
                    "replayed-challenge"
                )
                .is_err()
        );
        assert!(
            provider
                .verify(
                    &signed,
                    &claims.machine_id,
                    "different-owner",
                    &claims.binding,
                    "runtime",
                    &claims.challenge
                )
                .is_err()
        );
        assert!(
            provider
                .verify(
                    &signed,
                    &claims.machine_id,
                    &claims.owner_id,
                    &claims.binding,
                    "cleanup",
                    &claims.challenge
                )
                .is_err()
        );
        let mut changed = claims.binding.clone();
        changed.credential_epoch += 1;
        assert!(
            provider
                .verify(
                    &signed,
                    &claims.machine_id,
                    &claims.owner_id,
                    &changed,
                    "runtime",
                    &claims.challenge
                )
                .is_err()
        );
    }

    #[test]
    fn installed_receipt_requires_signed_exact_control_target_and_revision() {
        use nyxid_machine::managed::{DeadlineAction, DeadlineControl};
        let (provider, claims, key) = fixture();
        let control = DeadlineControl {
            version: 1,
            operation_id: uuid::Uuid::new_v4().to_string(),
            control_revision: 7,
            target: nyxid_machine::managed::RuntimeTarget {
                machine_id: claims.machine_id,
                owner_id: claims.owner_id,
                sandbox_uid: claims.binding.sandbox_uid,
                pod_uid: claims.binding.pod_uid.unwrap(),
                pvc_uid: claims.pvc_uid.unwrap(),
                runtime_generation: 2,
                credential_epoch: 4,
                writer_fence: claims.binding.writer_fence,
            },
            action: DeadlineAction::Renew,
            run_not_after_ms: Utc::now().timestamp_millis() + 60_000,
        };
        let receipt = super::super::remote_machine_fence::test_deadline_receipt(&control)
            .receipt()
            .clone();
        let make = |receipt| DeadlineAttestation {
            iss: provider.issuer.clone(),
            aud: AUDIENCE.into(),
            iat: Utc::now().timestamp(),
            exp: Utc::now().timestamp() + 30,
            purpose: "deadline_installed".into(),
            receipt,
        };
        let signed = encode(&Header::new(Algorithm::RS256), &make(receipt.clone()), &key).unwrap();
        assert!(provider.verify_deadline(&signed, &control).is_ok());
        for kind in ["revision", "cutoff", "pod", "boot", "digest", "stop"] {
            let mut bad = receipt.clone();
            match kind {
                "revision" => bad.control_revision += 1,
                "cutoff" => bad.installed_run_not_after_ms += 1,
                "pod" => bad.resolved_target.target.pod_uid = uuid::Uuid::new_v4().to_string(),
                "boot" => bad.provider_boot_id = "ambiguous".into(),
                "digest" => bad.request_digest = "a".repeat(64),
                _ => bad.stop_requested = true,
            }
            let signed = encode(&Header::new(Algorithm::RS256), &make(bad), &key).unwrap();
            assert!(
                provider.verify_deadline(&signed, &control).is_err(),
                "{kind}"
            );
        }
    }

    #[test]
    fn provider_rejects_wrong_issuer_audience_deadline_and_untrusted_error_text() {
        let (provider, claims, key) = fixture();
        for field in [
            "issuer",
            "audience",
            "expired",
            "future",
            "old_observation",
            "long_lifetime",
            "unsafe_ref",
            "stop_without_stopped_state",
            "stop_after_observation",
            "running_after_observation",
            "running_after_stop",
        ] {
            let mut invalid = claims.clone();
            match field {
                "issuer" => invalid.iss = "untrusted".into(),
                "audience" => invalid.aud = "other-service".into(),
                "expired" => invalid.exp = claims.iat - 1,
                "future" => invalid.iat = claims.iat + 60,
                "old_observation" => invalid.observed_at = claims.iat - 61,
                "long_lifetime" => invalid.exp = claims.iat + 900,
                "stop_without_stopped_state" => {
                    invalid.stopped_at_ns = Some(claims.observed_at * 1_000_000_000)
                }
                "stop_after_observation" => {
                    invalid.state = "stopped".into();
                    invalid.stopped_at_ns = Some((claims.observed_at + 2) * 1_000_000_000);
                }
                "running_after_observation" => {
                    invalid.running_at_ns = Some((claims.observed_at + 2) * 1_000_000_000)
                }
                "running_after_stop" => {
                    invalid.state = "stopped".into();
                    invalid.running_at_ns = Some(claims.observed_at * 1_000_000_000);
                }
                _ => invalid.evidence_ref = "raw-provider-secret-or-payload".into(),
            }
            let error = provider
                .verify(
                    &token(&invalid, &key),
                    &claims.machine_id,
                    &claims.owner_id,
                    &claims.binding,
                    "runtime",
                    &claims.challenge,
                )
                .err()
                .unwrap();
            assert_eq!(error.to_string(), rejected().to_string());
        }
        let wrong_alg = jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"not-the-provider-key"),
        )
        .unwrap();
        assert!(
            provider
                .verify(
                    &wrong_alg,
                    &claims.machine_id,
                    &claims.owner_id,
                    &claims.binding,
                    "runtime",
                    &claims.challenge
                )
                .is_err()
        );
    }
    #[tokio::test]
    async fn signed_http_transport_requires_current_challenge_and_exact_cleanup_volume() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let (mut provider, claims, key) = fixture();
        let server = MockServer::start().await;
        provider.endpoint = server.uri().parse().unwrap();
        let mut machine = super::super::remote_machine_service::CreateMachineInput::fixed(
            &claims.owner_id,
            true,
            "registry.test/node@sha256:fixture",
        )
        .preview(&claims.machine_id, Utc::now());
        machine.sandbox_uid = Some(claims.binding.sandbox_uid.clone());
        machine.pod_uid = claims.binding.pod_uid.clone();
        machine.runtime_generation = claims.binding.runtime_generation;
        machine.credential_epoch = claims.binding.credential_epoch;
        machine.writer_fence = claims.binding.writer_fence.clone();
        machine.pvc_uid = claims.pvc_uid.clone();
        let template = claims.clone();
        Mock::given(method("POST"))
            .respond_with(move |request: &wiremock::Request| {
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                let mut signed = template.clone();
                signed.purpose = body["purpose"].as_str().unwrap().into();
                signed.challenge = body["challenge"].as_str().unwrap().into();
                if signed.purpose == "cleanup" {
                    signed.state = "cleanup".into();
                    signed.credentials_revoked = true;
                    signed.workspace_released = true;
                    signed.persistent_volume_released = true;
                    signed.accounting_closed = true;
                }
                ResponseTemplate::new(200).set_body_json(json!({"token":token(&signed,&key)}))
            })
            .mount(&server)
            .await;
        let runtime = provider.runtime(&machine).await.unwrap().unwrap();
        assert_eq!(runtime.binding(), &claims.binding);
        assert!(!runtime.physically_stopped());
        let cleanup = provider.cleanup(&machine).await.unwrap().unwrap();
        assert!(cleanup.all_final());
        assert_eq!(cleanup.pvc_uid(), machine.pvc_uid.as_deref());
        machine.pvc_uid = Some(uuid::Uuid::new_v4().to_string());
        assert!(provider.cleanup(&machine).await.is_err());
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(MAX_RESPONSE + 1)))
            .mount(&server)
            .await;
        assert!(provider.runtime(&machine).await.is_err());
    }
}
