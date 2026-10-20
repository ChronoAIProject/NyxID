//! Audit configuration is deliberately local-first. Absence of a witness is
//! an explicit unwitnessed deployment, never an implicit production-ready sink.
mod configuration;
mod diagnostics;
mod evidence;
pub mod intent;
mod monitor;
mod relay;
pub use configuration::ConfiguredWitness;
pub use diagnostics::{AuditDiagnostics, AuditRuntime};
pub use monitor::spawn_monitor;
pub use relay::WitnessRelay;
pub mod policy;
pub mod transaction;
pub(super) use evidence::load_in_session;
pub use evidence::{ExternalAuditEvidence, external_evidence};

use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct AuditConfig {
    pub chain_id: Option<Uuid>,
    /// Required only when adopting a preexisting v1 chain for the first time.
    pub cutover_drained: bool,
    pub admission: policy::AdmissionLimits,
    pub witness: Option<ConfiguredWitness>,
}

impl AuditConfig {
    pub fn from_env() -> Result<Self, &'static str> {
        let optional = |name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("Audit configuration must be UTF-8"),
        };
        let mut config = Self::parse(
            optional("AUDIT_CHAIN_ID")?.as_deref(),
            optional("AUDIT_CUTOVER_DRAINED")?.as_deref(),
        )?;
        config.admission = policy::AdmissionLimits::from_env()?;
        config.witness = ConfiguredWitness::from_env()?;
        if let Some(witness) = &config.witness {
            let enrolled = witness.pin.enrollment.body.chain_id;
            if config.chain_id.is_some_and(|id| id != enrolled) {
                return Err("Audit chain identity does not match the administrator witness pin");
            }
            config.chain_id = Some(enrolled);
        }
        Ok(config)
    }

    fn parse(chain: Option<&str>, drained: Option<&str>) -> Result<Self, &'static str> {
        let chain_id = chain
            .filter(|v| !v.trim().is_empty())
            .map(|v| Uuid::parse_str(v).map_err(|_| "AUDIT_CHAIN_ID must be a UUID v4"))
            .transpose()?;
        if chain_id.is_some_and(|id| id.get_version_num() != 4) {
            return Err("AUDIT_CHAIN_ID must be a UUID v4");
        }
        let cutover_drained = match drained {
            None | Some("") | Some("false") => false,
            Some("true") => true,
            _ => return Err("AUDIT_CUTOVER_DRAINED must be true or false"),
        };
        Ok(Self {
            chain_id,
            cutover_drained,
            admission: policy::AdmissionLimits::default(),
            witness: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_defaults_and_explicit_cutover_are_distinct() {
        let local = AuditConfig::parse(None, None).unwrap();
        assert!(local.chain_id.is_none());
        assert!(!local.cutover_drained);
        let id = Uuid::new_v4();
        let upgrade = AuditConfig::parse(Some(&id.to_string()), Some("true")).unwrap();
        assert_eq!(upgrade.chain_id, Some(id));
        assert!(upgrade.cutover_drained);
        for chain in ["invalid", "00000000-0000-0000-0000-000000000000"] {
            assert!(AuditConfig::parse(Some(chain), None).is_err());
        }
        for flag in ["1", "yes", "TRUE", " false "] {
            assert!(AuditConfig::parse(None, Some(flag)).is_err());
        }
    }
}
