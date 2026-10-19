//! Durable origin identities for known copies of one OAuth authorization.
//! These keyed identifiers establish shared provenance, not upstream grant scope.
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthGrantLineage {
    pub version: u8,
    pub origins: Vec<String>,
    /// Custom/imported OAuth material has no verified provider attribution.
    /// Known-copy fingerprints never turn this into remote isolation evidence.
    #[serde(default)]
    pub provider_unattributed: bool,
}
impl std::fmt::Debug for OAuthGrantLineage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthGrantLineage")
            .field("version", &self.version)
            .field("origins", &"[REDACTED]")
            .finish()
    }
}
