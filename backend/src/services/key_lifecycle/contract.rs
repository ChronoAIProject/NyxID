//! Version-one lifecycle API. References are opaque. Provenance is assigned
//! by the authenticated recording path, independently of the assertion.
use super::{AppResult, invalid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    pub deployment_id: String,
    pub deployment_revision: String,
    pub restore_epoch: String,
    pub keys: BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recording {
    pub request_id: String,
    pub expected_revision: i64,
    pub owner: String,
    pub evidence_ref: String,
}

impl Recording {
    pub(super) fn validate(&self) -> AppResult<()> {
        uuid(&self.request_id)?;
        uuid(&self.evidence_ref)?;
        label(&self.owner)?;
        if !(0..i64::MAX).contains(&self.expected_revision) {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(super) fn uuid(s: &str) -> AppResult<()> {
    if uuid::Uuid::parse_str(s).is_ok_and(|u| u.get_version_num() == 4 && u.to_string() == s) {
        Ok(())
    } else {
        Err(invalid())
    }
}

pub(super) fn label(s: &str) -> AppResult<()> {
    if s.is_empty()
        || s.len() > 96
        || !s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-@".contains(&c))
    {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Enroll {
    pub recording: Recording,
    pub deployment_revision: String,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Inventory {
    WriterCohort,
    ReaderCohort,
    IndependentHmac,
    MigrationCanary,
    DatabaseCredential,
    FullBackup,
    IncrementalBackup,
    Pitr,
    Clone,
    Archive,
    RestoreDrill,
    Rollback,
    CloudVersion,
    Unknown,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Resolution {
    VerifiedDisposal,
    TestedSurvivorRestore,
    TestedSurvivorRollback,
    Withdrawn,
    RevokedOrRotated,
    ProviderVerified,
    VerifiedIndependentHmac,
    TestedSurvivorReader,
    TestedSurvivorCanary,
}

impl Inventory {
    pub(super) fn permits(self, resolution: Resolution) -> bool {
        use Resolution::*;
        match self {
            Self::ReaderCohort => matches!(resolution, Withdrawn | TestedSurvivorReader),
            Self::IndependentHmac => matches!(resolution, VerifiedIndependentHmac),
            Self::MigrationCanary => matches!(resolution, TestedSurvivorCanary),
            Self::WriterCohort => matches!(resolution, Withdrawn),
            Self::DatabaseCredential => matches!(resolution, RevokedOrRotated),
            Self::Rollback => matches!(resolution, Withdrawn | TestedSurvivorRollback),
            Self::CloudVersion => matches!(resolution, ProviderVerified),
            _ => matches!(resolution, VerifiedDisposal | TestedSurvivorRestore),
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Review {
    ReviewedEmpty,
    Reviewed,
    Withdrawn,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Assertion {
    OldWritersWithdrawn,
    DatabaseAccessRevoked,
    RestoreContinuity,
    RecoveryDependenciesClosed,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Change {
    OpenDependency {
        inventory: Inventory,
        parents: BTreeSet<String>,
        formats: BTreeSet<String>,
    },
    ReopenDependency {
        dependency_id: String,
    },
    CloseDependency {
        dependency_id: String,
        resolution: Resolution,
    },
    ReviewInventory {
        inventory: Inventory,
        disposition: Review,
    },
    Attest {
        assertion: Assertion,
        affirmed: bool,
        valid_until_ms: Option<i64>,
    },
    VerifyRing,
    AdvanceRestoreEpoch,
    #[serde(rename = "change_deployment_revision")]
    SetDeploymentRevision {
        deployment_revision: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Append {
    pub recording: Recording,
    pub scope: Scope,
    pub change: Change,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Receipt {
    pub entry_id: String,
    pub revision: i64,
    pub scope: Scope,
    pub recorded_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Key {
    pub key_ref: String,
    pub role: String,
    pub provider: String,
    pub internal_version: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Status {
    pub revision: i64,
    pub scope: Scope,
    pub keys: Vec<Key>,
    pub rotation_open: bool,
    pub restore_required: bool,
    pub scope_revision: i64,
    pub current: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Current>>,
    pub independent_hmac_file_verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Page {
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub schema_version: i32,
    pub receipt: Receipt,
    pub previous_revision: i64,
    pub input_scope: Scope,
    pub owner: String,
    pub evidence_ref: String,
    pub issuer: String,
    pub producer: String,
    pub assertion: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Dependency {
    pub dependency_id: String,
    pub inventory: Inventory,
    pub opened: Receipt,
    pub last_record: Receipt,
    pub parents: BTreeSet<String>,
    pub formats: BTreeSet<String>,
    pub owner: String,
    pub resolution: Option<Resolution>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Current {
    pub entry_id: String,
    pub revision: i64,
    pub disposition: String,
    pub valid_until_ms: Option<i64>,
}
