//! Explicit, signed machine authority. Capability flags are API ceilings, not
//! filesystem or browser-session isolation; M1.2 uses shared legacy runtimes.
use crate::{MachineProfile, Operation};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 2;
/// Offline grace; push revocations still cancel work immediately.
pub const LEASE_MS: i64 = 45_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Capabilities {
    pub shell: bool,
    pub files: bool,
    pub browser: bool,
    pub computer: bool,
    pub developer_browser: bool,
}
impl Capabilities {
    pub fn legacy(profile: &MachineProfile) -> Self {
        Self {
            shell: profile.shell,
            files: profile.files,
            browser: profile.browser_enabled(),
            computer: profile.computer && profile.browser_enabled(),
            developer_browser: profile.browser_enabled(),
        }
    }
    pub fn valid(self) -> bool {
        (!self.computer && !self.developer_browser) || self.browser
    }
    pub fn subset_of(self, other: Self) -> bool {
        (!self.shell || other.shell)
            && (!self.files || other.files)
            && (!self.browser || other.browser)
            && (!self.computer || other.computer)
            && (!self.developer_browser || other.developer_browser)
    }
    pub fn allows(self, operation: Operation, parameters: &serde_json::Value) -> bool {
        match operation {
            Operation::Exec | Operation::Job | Operation::ServiceCall | Operation::JobFinished => {
                self.shell
            }
            Operation::ListFiles
            | Operation::ReadFile
            | Operation::WriteFile
            | Operation::EditFile
            | Operation::SaveAttachment
            | Operation::ShareFile => self.files,
            Operation::Browser => {
                self.browser && (parameters["browser"] != "dev" || self.developer_browser)
            }
            Operation::FillLogin => {
                self.browser && parameters["browser"].as_str().is_none_or(|b| b == "secure")
            }
            Operation::Computer => self.computer && self.browser,
            // Cancellation is available even after a grant reduction.
            Operation::JobCancel => true,
            Operation::DesktopControl => self.shell || self.files || self.browser || self.computer,
            _ => false,
        }
    }
}

/// All fields are server-derived and authenticated in the v2 signing domain.
/// An absent envelope is never an implicit v2 grant.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    pub require_v2: bool,
    pub context_id: String,
    pub generation: u64,
    pub mode: String,
    pub agent_id: String,
    pub owner_id: String,
    pub actor_id: String,
    pub group_id: Option<String>,
    pub runtime_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub lease_id: String,
    pub revision: i64,
    pub expires_at_ms: i64,
    pub capabilities: Capabilities,
}
impl Authority {
    pub fn valid(&self, runtime: &str, now_ms: i64) -> bool {
        self.mode == "shared_legacy"
            && self.generation == 1
            && self.revision > 0
            && self.runtime_id == runtime
            && self.capabilities.valid()
            && self.expires_at_ms > now_ms
            && self.expires_at_ms <= now_ms.saturating_add(LEASE_MS)
            && [
                &self.context_id,
                &self.agent_id,
                &self.owner_id,
                &self.actor_id,
                &self.runtime_id,
                &self.turn_id,
                &self.lease_id,
            ]
            .into_iter()
            .all(|id| uuid::Uuid::parse_str(id).is_ok())
            && (uuid::Uuid::parse_str(&self.conversation_id).is_ok()
                || self
                    .conversation_id
                    .strip_prefix("nyxa-")
                    .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())))
            && self.group_id.as_ref().is_none_or(|id| {
                id.strip_prefix("nyxg-")
                    .is_some_and(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_browser_inheritance_and_explicit_local_denial() {
        let mut profile = MachineProfile {
            version: 1,
            computer: true,
            ..Default::default()
        };
        assert!(Operation::Computer.allowed(&profile));
        assert!(Operation::Browser.allowed(&profile));
        profile.browser = Some(false);
        assert!(!Operation::Computer.allowed(&profile));
        assert!(!Operation::Browser.allowed(&profile));
        assert!(!Capabilities::legacy(&profile).browser);
        profile.computer = false;
        profile.browser = Some(true);
        assert!(profile.enabled());
        assert!(Operation::Browser.allowed(&profile));
        assert!(Operation::FillLogin.allowed(&profile));
        assert!(!Operation::Computer.allowed(&profile));
        assert!(!profile.authority_v2());
    }
    #[test]
    fn missing_flags_deny_and_desktop_cannot_bypass_browser_denial() {
        let caps: Capabilities = serde_json::from_str("{}").unwrap();
        assert!(!caps.allows(Operation::ReadFile, &serde_json::json!({})));
        let caps = Capabilities {
            computer: true,
            ..caps
        };
        assert!(!caps.valid());
        assert!(!caps.allows(Operation::Computer, &serde_json::json!({})));
        let caps = Capabilities {
            browser: true,
            ..Default::default()
        };
        assert!(caps.allows(Operation::Browser, &serde_json::json!({})));
        assert!(!caps.allows(Operation::Browser, &serde_json::json!({"browser":"dev"})));
        assert!(Capabilities::default().subset_of(caps));
        assert!(!caps.subset_of(Capabilities::default()));
    }
}
