//! Provider evidence contract for managed runtime writer and stop fences.
//!
//! Test-only contracts for the deferred signed-provider integration. Native
//! production lifecycle does not depend on these physical-stop capabilities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RawRuntimeEvidence {
    pub sandbox_uid: String,
    pub pod_uid: Option<String>,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: String,
    pub provider_id: String,
    pub evidence_ref: String,
    pub physically_stopped: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBinding {
    pub sandbox_uid: String,
    pub pod_uid: Option<String>,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: String,
}

/// Constructed only after the provider transport verifies the signature and
/// exact request binding; an installed timer is not physical stop evidence.
#[derive(Clone, Debug)]
pub struct VerifiedDeadlineReceipt(nyxid_machine::managed::DeadlineReceipt);
impl VerifiedDeadlineReceipt {
    pub(super) fn from_signed_provider(receipt: nyxid_machine::managed::DeadlineReceipt) -> Self {
        Self(receipt)
    }
    pub fn receipt(&self) -> &nyxid_machine::managed::DeadlineReceipt {
        &self.0
    }
}

#[cfg(test)]
pub(crate) fn test_deadline_receipt(
    control: &nyxid_machine::managed::DeadlineControl,
) -> VerifiedDeadlineReceipt {
    use nyxid_machine::managed::{DeadlineAction, DeadlineReceipt, ResolvedRuntimeTarget};
    VerifiedDeadlineReceipt(DeadlineReceipt {
        version: 1,
        request_digest: control.digest().unwrap(),
        control_revision: control.control_revision,
        resolved_target: ResolvedRuntimeTarget {
            target: control.target.clone(),
            node_uid: uuid::Uuid::new_v4().to_string(),
            node_boot_id: uuid::Uuid::new_v4().to_string(),
            runtime_instance_digest: "a".repeat(64),
            runtime_attempt: 1,
            pv_uid: uuid::Uuid::new_v4().to_string(),
            csi_driver: "fixture.local".into(),
            volume_handle_digest: "b".repeat(64),
            writer_domain_digest: "c".repeat(64),
        },
        provider_instance_id: uuid::Uuid::new_v4().to_string(),
        provider_boot_id: uuid::Uuid::new_v4().to_string(),
        installed_run_not_after_ms: control.run_not_after_ms,
        guaranteed_stop_not_after_ms: control.run_not_after_ms + 1000,
        mechanism_version: "local-test-v1".into(),
        evidence_ref: uuid::Uuid::new_v4().to_string(),
        stop_requested: control.action == DeadlineAction::ShortenStop,
    })
}

/// Server/provider-verified cleanup proof.  A caller cannot construct this
/// token from booleans; only a configured verifier implementation can issue it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedCleanupEvidence {
    binding: RuntimeBinding,
    pvc_uid: Option<String>,
    evidence_ref: String,
    credentials_revoked: bool,
    workspace_released: bool,
    persistent_volume_released: bool,
    accounting_closed: bool,
    observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCleanupEvidence {
    pub binding: RuntimeBinding,
    pub pvc_uid: Option<String>,
    pub evidence_ref: String,
    pub credentials_revoked: bool,
    pub workspace_released: bool,
    pub persistent_volume_released: bool,
    pub accounting_closed: bool,
    pub observed_at: DateTime<Utc>,
    pub provider_id: String,
}

pub trait CleanupEvidenceVerifier: Send + Sync {
    fn verify(
        &self,
        expected: &RuntimeBinding,
        raw: &RawCleanupEvidence,
    ) -> Result<VerifiedCleanupEvidence, EvidenceError>;
}

#[derive(Clone, Debug, Default)]
pub struct UnavailableCleanupEvidenceVerifier;

impl CleanupEvidenceVerifier for UnavailableCleanupEvidenceVerifier {
    fn verify(
        &self,
        _expected: &RuntimeBinding,
        _raw: &RawCleanupEvidence,
    ) -> Result<VerifiedCleanupEvidence, EvidenceError> {
        Err(EvidenceError::ProviderUnavailable)
    }
}

pub fn verify_cleanup_evidence(
    verifier: &dyn CleanupEvidenceVerifier,
    expected: &RuntimeBinding,
    raw: &RawCleanupEvidence,
) -> Result<VerifiedCleanupEvidence, EvidenceError> {
    if raw.binding != *expected {
        return Err(EvidenceError::Invalid("cleanup binding mismatch".into()));
    }
    if raw.evidence_ref.trim().is_empty() || raw.provider_id.trim().is_empty() {
        return Err(EvidenceError::Invalid(
            "cleanup evidence reference is missing".into(),
        ));
    }
    if raw.observed_at > Utc::now() + chrono::Duration::minutes(5) {
        return Err(EvidenceError::Invalid(
            "cleanup evidence is from the future".into(),
        ));
    }
    verifier.verify(expected, raw)
}

impl VerifiedCleanupEvidence {
    pub(super) fn from_signed_provider(raw: RawCleanupEvidence) -> Self {
        Self {
            binding: raw.binding,
            pvc_uid: raw.pvc_uid,
            evidence_ref: raw.evidence_ref,
            credentials_revoked: raw.credentials_revoked,
            workspace_released: raw.workspace_released,
            persistent_volume_released: raw.persistent_volume_released,
            accounting_closed: raw.accounting_closed,
            observed_at: raw.observed_at,
        }
    }
    pub fn pvc_uid(&self) -> Option<&str> {
        self.pvc_uid.as_deref()
    }
    pub fn all_final(&self) -> bool {
        // The provider proves physical and credential/storage finality only.
        // Its legacy accounting_closed field is accepted for wire compatibility;
        // lifecycle's mandatory local Billing transaction is authoritative.
        self.credentials_revoked && self.workspace_released && self.persistent_volume_released
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedRuntimeEvidence {
    binding: RuntimeBinding,
    provider_id: String,
    evidence_ref: String,
    physically_stopped: bool,
    observed_at: DateTime<Utc>,
    stopped_at: Option<DateTime<Utc>>,
    running_through: Option<DateTime<Utc>>,
}

impl VerifiedRuntimeEvidence {
    pub(super) fn from_signed_provider(
        binding: RuntimeBinding,
        provider_id: String,
        evidence_ref: String,
        physically_stopped: bool,
        observed_at: DateTime<Utc>,
        stopped_at: Option<DateTime<Utc>>,
        running_through: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            binding,
            provider_id,
            evidence_ref,
            physically_stopped,
            observed_at,
            stopped_at,
            running_through,
        }
    }
    pub fn binding(&self) -> &RuntimeBinding {
        &self.binding
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }

    pub fn physically_stopped(&self) -> bool {
        self.physically_stopped
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EvidenceError {
    #[error("runtime provider evidence is unavailable")]
    ProviderUnavailable,
    #[error("runtime provider evidence is invalid: {0}")]
    Invalid(String),
}

pub trait RuntimeEvidenceVerifier: Send + Sync {
    fn verify(
        &self,
        binding: &RuntimeBinding,
        raw: &RawRuntimeEvidence,
    ) -> Result<VerifiedRuntimeEvidence, EvidenceError>;
}

#[derive(Clone, Debug, Default)]
pub struct UnavailableRuntimeEvidenceVerifier;

impl RuntimeEvidenceVerifier for UnavailableRuntimeEvidenceVerifier {
    fn verify(
        &self,
        _binding: &RuntimeBinding,
        _raw: &RawRuntimeEvidence,
    ) -> Result<VerifiedRuntimeEvidence, EvidenceError> {
        Err(EvidenceError::ProviderUnavailable)
    }
}

pub fn verify_runtime_evidence(
    verifier: &dyn RuntimeEvidenceVerifier,
    binding: &RuntimeBinding,
    raw: &RawRuntimeEvidence,
) -> Result<VerifiedRuntimeEvidence, EvidenceError> {
    if raw.sandbox_uid != binding.sandbox_uid
        || raw.pod_uid != binding.pod_uid
        || raw.runtime_generation != binding.runtime_generation
        || raw.credential_epoch != binding.credential_epoch
        || raw.writer_fence != binding.writer_fence
    {
        return Err(EvidenceError::Invalid("runtime binding mismatch".into()));
    }
    if raw.provider_id.trim().is_empty() || raw.evidence_ref.trim().is_empty() {
        return Err(EvidenceError::Invalid(
            "provider evidence reference is missing".into(),
        ));
    }
    if raw.observed_at > Utc::now() + chrono::Duration::minutes(5) {
        return Err(EvidenceError::Invalid(
            "provider evidence is from the future".into(),
        ));
    }
    verifier.verify(binding, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestVerifier;

    impl RuntimeEvidenceVerifier for TestVerifier {
        fn verify(
            &self,
            binding: &RuntimeBinding,
            raw: &RawRuntimeEvidence,
        ) -> Result<VerifiedRuntimeEvidence, EvidenceError> {
            Ok(VerifiedRuntimeEvidence {
                binding: binding.clone(),
                provider_id: raw.provider_id.clone(),
                evidence_ref: raw.evidence_ref.clone(),
                physically_stopped: raw.physically_stopped,
                observed_at: raw.observed_at,
                stopped_at: None,
                running_through: None,
            })
        }
    }

    fn binding() -> RuntimeBinding {
        RuntimeBinding {
            sandbox_uid: "sandbox".into(),
            pod_uid: Some("pod".into()),
            runtime_generation: 4,
            credential_epoch: 9,
            writer_fence: "fence".into(),
        }
    }

    fn raw() -> RawRuntimeEvidence {
        RawRuntimeEvidence {
            sandbox_uid: "sandbox".into(),
            pod_uid: Some("pod".into()),
            runtime_generation: 4,
            credential_epoch: 9,
            writer_fence: "fence".into(),
            provider_id: "provider-test".into(),
            evidence_ref: "evidence-1".into(),
            physically_stopped: true,
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn default_verifier_keeps_release_closed() {
        let error =
            verify_runtime_evidence(&UnavailableRuntimeEvidenceVerifier, &binding(), &raw())
                .expect_err("no provider proof must remain closed");
        assert_eq!(error, EvidenceError::ProviderUnavailable);
    }

    #[test]
    fn binding_mismatch_is_rejected_before_provider_call() {
        let mut input = raw();
        input.credential_epoch = 10;
        let error = verify_runtime_evidence(&TestVerifier, &binding(), &input).unwrap_err();
        assert_eq!(
            error,
            EvidenceError::Invalid("runtime binding mismatch".into())
        );
    }

    #[test]
    fn verified_token_exposes_only_nonsecret_provider_reference() {
        let verified = verify_runtime_evidence(&TestVerifier, &binding(), &raw()).unwrap();
        assert!(verified.physically_stopped());
        assert_eq!(verified.provider_id(), "provider-test");
        assert_eq!(verified.evidence_ref(), "evidence-1");
    }

    #[test]
    fn cleanup_without_provider_proof_stays_closed() {
        let binding = binding();
        let raw = RawCleanupEvidence {
            binding: binding.clone(),
            pvc_uid: Some("pvc-fixture".into()),
            evidence_ref: "cleanup-1".into(),
            credentials_revoked: true,
            workspace_released: true,
            persistent_volume_released: true,
            accounting_closed: true,
            observed_at: Utc::now(),
            provider_id: "provider-test".into(),
        };
        let error = verify_cleanup_evidence(&UnavailableCleanupEvidenceVerifier, &binding, &raw)
            .unwrap_err();
        assert_eq!(error, EvidenceError::ProviderUnavailable);
    }
}
