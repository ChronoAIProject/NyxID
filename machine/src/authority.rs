//! Explicit, signed machine authority. Capability flags are API ceilings, not
//! filesystem or browser-session isolation; M1.2 uses shared legacy runtimes.
use crate::{MachineProfile, Operation};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 2;
pub const EXTERNAL_VERSION: u32 = 3;
/// Offline grace; push revocations still cancel work immediately.
pub const LEASE_MS: i64 = 45_000;
/// Maximum server/node clock difference. This is consumed from the lease,
/// never added to its lifetime. Request.timestamp is signed, in whole seconds.
pub const CLOCK_SKEW_MS: i64 = 5_000;

#[derive(Clone, Copy, Debug)]
pub struct LeaseClock {
    pub now_ms: i64,
    pub deadline: std::time::Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingRejection {
    ClockSkew,
    Lifetime,
    Expired,
}
impl TimingRejection {
    pub fn reason(self) -> &'static str {
        match self {
            Self::ClockSkew => "authority_clock_skew",
            Self::Lifetime => "authority_lifetime_invalid",
            Self::Expired => "authority_expired",
        }
    }
}
impl std::fmt::Display for TimingRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}
impl std::error::Error for TimingRejection {}

impl LeaseClock {
    /// Call only after verifying the request signature. Allow bounded skew but
    /// expire conservatively: node_now + skew is the latest possible server
    /// time. This keeps cancellation no later than the signed server deadline,
    /// and at most 45s locally, including delivery delay and whole-second
    /// timestamp rounding. Old signed envelopes require no new wire fields.
    pub fn signed(timestamp: i64, expires: i64, node_now: i64) -> Result<Self, TimingRejection> {
        let issued = timestamp
            .checked_mul(1000)
            .ok_or(TimingRejection::Lifetime)?;
        let upper = issued.checked_add(999).ok_or(TimingRejection::Lifetime)?;
        let latest = node_now
            .checked_add(CLOCK_SKEW_MS)
            .ok_or(TimingRejection::Lifetime)?;
        if issued > latest || node_now.saturating_sub(upper) > CLOCK_SKEW_MS {
            return Err(TimingRejection::ClockSkew);
        }
        if expires > upper.saturating_add(LEASE_MS) {
            return Err(TimingRejection::Lifetime);
        }
        if expires <= latest {
            return Err(TimingRejection::Expired);
        }
        Ok(Self {
            now_ms: latest,
            deadline: std::time::Instant::now()
                + std::time::Duration::from_millis((expires - latest).min(LEASE_MS) as u64),
        })
    }
}

/// Narrow first-party owner management, signed as request parameters. This
/// permits only reads and cancellation of an existing job, never execution
/// admission or renewal of that job's source authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerJobAccess {
    pub owner_id: String,
    pub node_id: String,
    pub runtime_id: String,
    pub job_id: String,
    pub expires_at_ms: i64,
}

impl OwnerJobAccess {
    pub fn valid(&self, node: &str, runtime: &str, job: &str, now_ms: i64) -> bool {
        self.node_id == node
            && self.runtime_id == runtime
            && self.job_id == job
            && self.expires_at_ms > now_ms
            && self.expires_at_ms <= now_ms.saturating_add(LEASE_MS)
            && [
                &self.owner_id,
                &self.node_id,
                &self.runtime_id,
                &self.job_id,
            ]
            .into_iter()
            .all(|id| uuid::Uuid::parse_str(id).is_ok())
    }
}

/// A reference to an already authenticated source identity. It never contains
/// source bearer material, and a child is attribution rather than a principal.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalPrincipal {
    #[serde(rename = "oauth")]
    OAuth {
        user_id: String,
        oauth_client_id: String,
    },
    ApiKey {
        user_id: String,
        api_key_id: String,
        api_key_credential_id: Option<String>,
    },
}

impl std::fmt::Debug for ExternalPrincipal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Key identifiers and child attribution must not escape through parent
        // authority/request Debug derives. Wire serialization stays unchanged.
        formatter.write_str(match self {
            Self::OAuth { .. } => "OAuth { identity: [REDACTED] }",
            Self::ApiKey { .. } => "ApiKey { identity: [REDACTED] }",
        })
    }
}

impl ExternalPrincipal {
    pub fn user_id(&self) -> &str {
        match self {
            Self::OAuth { user_id, .. } | Self::ApiKey { user_id, .. } => user_id,
        }
    }
    pub fn valid(&self) -> bool {
        let valid = |value: &str| uuid::Uuid::parse_str(value).is_ok();
        match self {
            Self::OAuth {
                user_id,
                oauth_client_id,
            } => valid(user_id) && valid(oauth_client_id),
            Self::ApiKey {
                user_id,
                api_key_id,
                api_key_credential_id,
            } => {
                valid(user_id)
                    && valid(api_key_id)
                    && api_key_credential_id.as_deref().is_none_or(valid)
            }
        }
    }
    /// Shared view identity. This deliberately does not authorize renewal.
    pub fn same_principal(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::OAuth {
                    user_id: a,
                    oauth_client_id: b,
                },
                Self::OAuth {
                    user_id: x,
                    oauth_client_id: y,
                },
            ) => a == x && b == y,
            (
                Self::ApiKey {
                    user_id: a,
                    api_key_id: b,
                    ..
                },
                Self::ApiKey {
                    user_id: x,
                    api_key_id: y,
                    ..
                },
            ) => a == x && b == y,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OAuthConsent {
    pub consent_id: String,
    pub consent_revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeclaredService {
    pub id: String,
    pub slug: String,
}

/// V3 has no chat/turn/agent placeholders. Context generation, persistent
/// managed generation and the actual daemon runtime are distinct fences.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuthority {
    pub principal: ExternalPrincipal,
    pub oauth_consent: Option<OAuthConsent>,
    pub context_id: String,
    pub generation: u64,
    pub mode: String,
    pub owner_id: String,
    pub actor_id: String,
    pub node_id: String,
    pub runtime_id: String,
    pub runtime_generation: Option<u64>,
    pub credential_epoch: Option<u64>,
    pub grant_id: String,
    pub grant_revision: i64,
    pub assignment_revision: i64,
    pub lease_id: String,
    pub job_id: Option<String>,
    pub expires_at_ms: i64,
    pub auth_expires_at_ms: i64,
    pub job_expires_at_ms: i64,
    pub funding_expires_at_ms: Option<i64>,
    pub capabilities: Capabilities,
    pub file_roots: Vec<String>,
    pub services: Vec<DeclaredService>,
}

impl ExternalAuthority {
    /// The narrow stop path still needs signed, live transport authority but
    /// can stop a job after its source authentication or funding has expired.
    pub fn valid_stop(&self, node: &str, runtime: &str, now_ms: i64) -> bool {
        self.principal.valid()
            && self.actor_id == self.principal.user_id()
            && self.owner_id == self.actor_id
            && self.node_id == node
            && self.runtime_id == runtime
            && self.mode == "shared_legacy"
            && self.generation == 1
            && self.grant_revision > 0
            && self.assignment_revision > 0
            && self.expires_at_ms > now_ms
            && self.expires_at_ms <= now_ms.saturating_add(LEASE_MS)
            && [
                &self.context_id,
                &self.owner_id,
                &self.actor_id,
                &self.node_id,
                &self.runtime_id,
                &self.grant_id,
                &self.lease_id,
            ]
            .into_iter()
            .all(|id| uuid::Uuid::parse_str(id).is_ok())
            && self
                .job_id
                .as_ref()
                .is_none_or(|id| uuid::Uuid::parse_str(id).is_ok())
            && matches!(
                (self.runtime_generation, self.credential_epoch),
                (None, None) | (Some(1..), Some(1..))
            )
            && match (&self.principal, &self.oauth_consent) {
                (ExternalPrincipal::OAuth { .. }, Some(consent)) => {
                    uuid::Uuid::parse_str(&consent.consent_id).is_ok()
                        && !consent.consent_revision.is_empty()
                        && consent.consent_revision.len() <= 256
                }
                (ExternalPrincipal::ApiKey { .. }, None) => true,
                _ => false,
            }
    }
    pub fn valid(&self, node: &str, runtime: &str, now_ms: i64) -> bool {
        self.valid_stop(node, runtime, now_ms)
            && self.expires_at_ms <= self.auth_expires_at_ms
            && self.expires_at_ms <= self.job_expires_at_ms
            && self
                .funding_expires_at_ms
                .is_none_or(|until| self.expires_at_ms <= until)
            && self.capabilities.valid()
            && !self.capabilities.browser
            && !self.capabilities.computer
            && !self.capabilities.developer_browser
            && !self.file_roots.is_empty()
            && self.file_roots.len() <= 16
            && self.file_roots.iter().all(|root| {
                let path = std::path::Path::new(root);
                root.len() <= 4096
                    && !root.contains('\0')
                    && path.is_absolute()
                    && !path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
            })
            && self.services.len() <= 128
            && self.services.iter().all(|service| {
                uuid::Uuid::parse_str(&service.id).is_ok()
                    && !service.slug.is_empty()
                    && service.slug.len() <= 128
                    && service
                        .slug
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
            && self.services.iter().enumerate().all(|(i, service)| {
                self.services[..i]
                    .iter()
                    .all(|other| service.id != other.id && service.slug != other.slug)
            })
    }
}

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
        ((self.mode == "shared_legacy" && self.generation == 1)
            || (self.mode == crate::context::MODE && self.generation > 0 && self.require_v2))
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
pub(crate) fn external_fixture() -> ExternalAuthority {
    let id = || uuid::Uuid::new_v4().to_string();
    let owner = id();
    ExternalAuthority {
        principal: ExternalPrincipal::ApiKey {
            user_id: owner.clone(),
            api_key_id: id(),
            api_key_credential_id: None,
        },
        oauth_consent: None,
        context_id: id(),
        generation: 1,
        mode: "shared_legacy".into(),
        owner_id: owner.clone(),
        actor_id: owner,
        node_id: id(),
        runtime_id: id(),
        runtime_generation: None,
        credential_epoch: None,
        grant_id: id(),
        grant_revision: 1,
        assignment_revision: 1,
        lease_id: id(),
        job_id: None,
        expires_at_ms: 1100,
        auth_expires_at_ms: 2000,
        job_expires_at_ms: 2000,
        funding_expires_at_ms: None,
        capabilities: Capabilities {
            shell: true,
            files: true,
            ..Default::default()
        },
        file_roots: vec!["/workspace".into()],
        services: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_lease_clock_handles_skew_without_extending_cancellation() {
        let server = 1_000_500;
        let expiry = server + LEASE_MS;
        for skew in [-5_000, -3_000, 0, 3_000, 5_000] {
            let node = server + skew;
            let clock = LeaseClock::signed(server / 1000, expiry, node).unwrap();
            assert_eq!(clock.now_ms, node + CLOCK_SKEW_MS);
            assert!(
                clock
                    .deadline
                    .duration_since(std::time::Instant::now())
                    .as_millis()
                    <= LEASE_MS as u128
            );
            // Even a node 5s behind must finish by the server's signed expiry.
            assert!(clock.now_ms >= server);
            assert!(expiry - clock.now_ms <= LEASE_MS);
        }
        assert_eq!(
            LeaseClock::signed(server / 1000, expiry, server - 5_501).unwrap_err(),
            TimingRejection::ClockSkew
        );
        assert_eq!(
            LeaseClock::signed(server / 1000, expiry, server + 6_000).unwrap_err(),
            TimingRejection::ClockSkew
        );
        assert_eq!(
            LeaseClock::signed(server / 1000, 1_046_000, server).unwrap_err(),
            TimingRejection::Lifetime
        );
        assert_eq!(
            LeaseClock::signed(server / 1000, server + CLOCK_SKEW_MS, server).unwrap_err(),
            TimingRejection::Expired
        );
        assert!(LeaseClock::signed(i64::MAX, i64::MAX, 0).is_err());
    }

    #[test]
    fn external_principal_debug_redacts_identifiers_without_changing_wire_identity() {
        let identities = [
            ExternalPrincipal::ApiKey {
                user_id: "user-identifier-sensitive".into(),
                api_key_id: "parent-key-identifier-sensitive".into(),
                api_key_credential_id: Some("child-key-identifier-sensitive".into()),
            },
            ExternalPrincipal::OAuth {
                user_id: "user-identifier-sensitive".into(),
                oauth_client_id: "oauth-client-identifier-sensitive".into(),
            },
        ];
        for principal in identities {
            for debug in [format!("{principal:?}"), format!("{principal:#?}")] {
                assert!(debug.contains("[REDACTED]"));
                assert!(!debug.contains("identifier-sensitive"));
            }
            let wire = serde_json::to_value(&principal).unwrap();
            assert_eq!(wire["user_id"], "user-identifier-sensitive");
            assert_eq!(
                serde_json::from_value::<ExternalPrincipal>(wire).unwrap(),
                principal
            );
        }
    }
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

    #[test]
    fn external_source_deadlines_and_personal_shell_files_are_independent_fences() {
        let mut authority = external_fixture();
        assert!(authority.valid(&authority.node_id, &authority.runtime_id, 1000));
        authority.auth_expires_at_ms = 1099;
        assert!(!authority.valid(&authority.node_id, &authority.runtime_id, 1000));
        assert!(authority.valid_stop(&authority.node_id, &authority.runtime_id, 1000));
        authority.auth_expires_at_ms = 2000;
        authority.capabilities.browser = true;
        assert!(!authority.valid(&authority.node_id, &authority.runtime_id, 1000));
        authority.capabilities.browser = false;
        authority.runtime_generation = Some(1);
        assert!(!authority.valid(&authority.node_id, &authority.runtime_id, 1000));
        authority.credential_epoch = Some(1);
        assert!(authority.valid(&authority.node_id, &authority.runtime_id, 1000));
        authority.file_roots = vec!["/workspace/../identity".into()];
        assert!(!authority.valid(&authority.node_id, &authority.runtime_id, 1000));
    }

    #[test]
    fn parent_shares_view_but_child_identity_is_not_equal_execution_authority() {
        let mut authority = external_fixture();
        let mut child = authority.principal.clone();
        if let ExternalPrincipal::ApiKey {
            api_key_credential_id,
            ..
        } = &mut child
        {
            *api_key_credential_id = Some(uuid::Uuid::new_v4().to_string());
        }
        assert!(authority.principal.same_principal(&child));
        assert_ne!(authority.principal, child);
        authority.oauth_consent = Some(OAuthConsent {
            consent_id: uuid::Uuid::new_v4().to_string(),
            consent_revision: uuid::Uuid::new_v4().to_string(),
        });
        assert!(!authority.valid(&authority.node_id, &authority.runtime_id, 1000));
    }

    #[test]
    fn oauth_wire_identity_and_owner_job_cutoffs_are_exact() {
        let authority = external_fixture();
        let principal = ExternalPrincipal::OAuth {
            user_id: authority.owner_id.clone(),
            oauth_client_id: uuid::Uuid::new_v4().to_string(),
        };
        let wire = serde_json::to_value(&principal).unwrap();
        assert_eq!(wire["kind"], "oauth");
        assert_eq!(
            serde_json::from_value::<ExternalPrincipal>(wire).unwrap(),
            principal
        );
        let access = OwnerJobAccess {
            owner_id: authority.owner_id,
            node_id: authority.node_id,
            runtime_id: authority.runtime_id,
            job_id: uuid::Uuid::new_v4().to_string(),
            expires_at_ms: 46_000,
        };
        assert!(access.valid(&access.node_id, &access.runtime_id, &access.job_id, 1_000));
        assert!(!access.valid(&access.node_id, &access.runtime_id, &access.job_id, 999));
        assert!(!access.valid(&access.node_id, &access.runtime_id, &access.job_id, 46_000));
        assert!(!access.valid(
            &access.node_id,
            &uuid::Uuid::new_v4().to_string(),
            &access.job_id,
            1_000
        ));
    }
}
