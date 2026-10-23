//! Durable allocation contract for separated contexts. No paths, credentials or
//! browser contents belong here. UIDs are monotonically allocated and retained
//! across generation changes; deleting a context never makes its UIDs reusable.
use crate::authority::Authority;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Support {
    pub available: bool,
    pub landlock_abi: Option<u32>,
    pub reason: Option<String>,
}

/// Versioned, node-observed prerequisites for selecting a *new* separated
/// assignment. The older Support advertisement is deliberately insufficient:
/// it predates browser launch diagnostics. None means not observed, never true.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Evidence {
    pub version: u32,
    pub os: String,
    pub landlock_abi: Option<u32>,
    pub separate_users: Option<bool>,
    /// Real node/sibling file and selected key-service denial under every role.
    pub credential_custody: Option<bool>,
    pub managed_browser_configured: Option<bool>,
    pub command_seccomp: Option<bool>,
    pub landlock_enforced: Option<bool>,
    /// Live renderer namespace, identity, NNP and additional seccomp filters
    /// observed under both configured browser identities.
    pub managed_browser_sandbox: Option<bool>,
    /// The secure identity positively refused DevTools without opening a listener.
    pub managed_browser_policy: Option<bool>,
    /// Diagnostic only: actual browser launch decides support under AppArmor.
    pub apparmor_userns_restricted: Option<bool>,
    /// Diagnostic classification of a failed probe; never evidence of support.
    pub probe_timed_out: bool,
    /// Required browser observations were inaccessible in the container. This
    /// diagnostic can only deny support, never substitute for a positive probe.
    pub browser_sandbox_unobservable_in_container: bool,
}

// v1 inferred pre-exec namespaces; v2 used DevTools-backed DOM diagnostics
// incompatible with secure policy. v3 observes live OS sandbox and policy effects.
// v4 additionally requires observed credential custody under every worker identity.
pub const EVIDENCE_VERSION: u32 = 4;
pub const MIN_LANDLOCK_ABI: u32 = 6;

impl Evidence {
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        if self.version != EVIDENCE_VERSION {
            return Some("separated_probe_version_unsupported");
        }
        if self.os != "linux" {
            return Some("separated_requires_linux");
        }
        if self.browser_sandbox_unobservable_in_container {
            return Some("separated_browser_sandbox_unobservable_in_container");
        }
        if self.probe_timed_out {
            return Some("separated_probe_timeout");
        }
        if self.managed_browser_policy == Some(false) {
            return Some("separated_browser_policy_not_effective");
        }
        if self.landlock_abi.is_none_or(|abi| abi < MIN_LANDLOCK_ABI) {
            return Some("separated_requires_landlock_abi_6");
        }
        for (observation, failure) in [
            (self.separate_users, "separated_requires_separate_users"),
            (
                self.managed_browser_configured,
                "separated_requires_managed_browser",
            ),
            (self.command_seccomp, "separated_seccomp_probe_failed"),
            (self.landlock_enforced, "separated_landlock_probe_failed"),
            (
                self.managed_browser_policy,
                "separated_browser_policy_not_effective",
            ),
            (
                self.managed_browser_sandbox,
                "separated_browser_sandbox_probe_failed",
            ),
        ] {
            match observation {
                Some(true) => {}
                Some(false) => return Some(failure),
                None => return Some("separated_probe_evidence_incomplete"),
            }
        }
        if self.credential_custody != Some(true) {
            return Some("node_custody_unverified");
        }
        None
    }

    pub fn support(&self) -> Support {
        let reason = self.unavailable_reason();
        Support {
            available: reason.is_none(),
            landlock_abi: self.landlock_abi,
            reason: reason.map(str::to_owned),
        }
    }
}

/// New setup intent only. Existing persisted assignments keep their own mode;
/// this selector is never a migration or a fallback for failed separated work.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    #[default]
    Automatic,
    RequireSeparated,
    SharedLegacy,
}

impl Choice {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::RequireSeparated => "require_separated",
            Self::SharedLegacy => "shared_legacy",
        }
    }
}

pub const SHARED_LEGACY_WARNING: &str = "Shared legacy access gives agent commands the full access of their OS user. A misbehaving or prompt-injected agent may read node tokens, signing secrets and stored credentials accessible to that user, including the node config and local credential store. Workspace roots constrain file tools and working directories, not the shell. Supported separated contexts still share the host kernel and retain kernel-exploit and supervisor-compromise risks; they are not a VM boundary. Use a separate machine container or VM per agent for stronger isolation; containers also share their host kernel.";

/// Fixed, actionable text suitable for CLI, API and tool responses. Never
/// include probe output, browser contents or a node-supplied diagnostic here.
pub fn unavailable_message(reason: &str) -> &'static str {
    match reason {
        "separated_browser_sandbox_unobservable_in_container" => {
            "This container does not expose every kernel observation required to confirm the managed browser sandbox. Separated contexts are unavailable; use a supported native Linux installation or a separate machine container or VM per agent. Do not add tracing capabilities or disable the browser sandbox."
        }
        "node_custody_unverified" => {
            "Node credential custody could not be verified under every command and browser identity. Protect node tokens, signing secrets and stored credentials with a separate supervisor identity and private storage, then restart the node to repeat the probes. Shared legacy mode retains the owner's OS access."
        }
        "separated_probe_timeout" => {
            "A separation capability probe exceeded its five-second bound. A cold Chromium start can cause this; rerun setup when the host is less busy. Separation remains unavailable until every probe succeeds."
        }
        "separated_browser_policy_not_effective" => {
            "The actual managed secure browser did not confirm that its policy blocks DevTools. Reinstall managed browser policies for this executable and rerun setup; separation remains unavailable until the live policy check succeeds."
        }
        "separated_requires_linux" => {
            "This runtime supports per-agent separation only on Linux; macOS has no supported per-agent sandbox. Use a supported Linux host."
        }
        "separated_requires_separate_users" => {
            "Separation requires a root supervisor with distinct non-root command, secure-browser and developer-browser users. On Linux, rerun setup with sudo and --separate-users."
        }
        "separated_requires_landlock_abi_6" => {
            "Separation requires working Landlock ABI 6 or newer. Upgrade the Linux kernel or use a supported host."
        }
        "separated_requires_managed_browser" => {
            "Separation requires a configured managed Chromium executable. Install Chromium and rerun setup with --browser and managed browser policies enabled."
        }
        "separated_seccomp_probe_failed" => {
            "The command user's seccomp policy could not be confirmed. Check the host security policy before retrying setup."
        }
        "separated_landlock_probe_failed" => {
            "The command user's Landlock enforcement could not be confirmed. Check the host security policy before retrying setup."
        }
        "separated_browser_sandbox_probe_failed" => {
            "The managed Chromium sandbox could not be confirmed under both browser users. Check the configured executable and its AppArmor user-namespace policy before retrying setup."
        }
        "separated_probe_version_unsupported"
        | "separated_capabilities_not_reported"
        | "separated_probe_evidence_incomplete" => {
            "The node has not reported complete supported separation evidence. Upgrade or reconnect the node and rerun setup to confirm its capabilities."
        }
        _ => {
            "Separation is unavailable. Check the node's capability report and rerun setup on a supported host."
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Separated,
    SharedLegacy,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Separated => MODE,
            Self::SharedLegacy => "shared_legacy",
        }
    }
}

pub fn select_new(choice: Choice, evidence: Option<&Evidence>) -> Result<Mode, &'static str> {
    if choice == Choice::SharedLegacy {
        return Ok(Mode::SharedLegacy);
    }
    let evidence = evidence.ok_or("separated_capabilities_not_reported")?;
    if let Some(reason) = evidence.unavailable_reason() {
        return Err(reason);
    }
    Ok(Mode::Separated)
}

pub const MODE: &str = "separated";
pub const MAX_CONTEXTS: usize = 128;
pub const MAX_GENERATIONS: usize = 128;
pub const FIRST_UID: u32 = 20000;
const LAST_UID: u32 = 999_999;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub agent: String,
    pub owner: String,
    pub actor: String,
    pub group: Option<String>,
}
impl From<&Authority> for Binding {
    fn from(a: &Authority) -> Self {
        Self {
            agent: a.agent_id.clone(),
            owner: a.owner_id.clone(),
            actor: a.actor_id.clone(),
            group: a.group_id.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Browsers {
    pub secure_uid: u32,
    pub dev_uid: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    pub binding: Binding,
    pub command_uid: u32,
    pub generation: u64,
    #[serde(default)]
    pub quarantined: bool,
    pub revision: i64,
    /// Old generations are tombstones and quarantined profiles, never reopened.
    pub browsers: BTreeMap<u64, Browsers>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    next_uid: u32,
    pub contexts: BTreeMap<String, Allocation>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            next_uid: FIRST_UID,
            contexts: BTreeMap::new(),
        }
    }
}
impl Registry {
    fn uid(&mut self, occupied: &mut impl FnMut(u32) -> bool) -> Result<u32, &'static str> {
        for _ in 0..4096 {
            let uid = self.next_uid;
            if uid > LAST_UID {
                return Err("context_uid_capacity");
            }
            self.next_uid += 1;
            if !occupied(uid) {
                return Ok(uid);
            }
        }
        Err("context_uid_capacity")
    }
    /// Mutate a clone and durably persist it before any user or directory is
    /// created. A provisioning failure intentionally burns its allocated UIDs.
    pub fn allocate(
        &mut self,
        a: &Authority,
        mut occupied: impl FnMut(u32) -> bool,
    ) -> Result<Allocation, &'static str> {
        if a.mode != MODE
            || !a.require_v2
            || a.revision < 1
            || a.generation == 0
            || uuid::Uuid::parse_str(&a.context_id).is_err()
        {
            return Err("context_authority_invalid");
        }
        let binding = Binding::from(a);
        let mut entry = if let Some(old) = self.contexts.get(&a.context_id) {
            if old.binding != binding {
                return Err("context_identity_mismatch");
            }
            if a.generation < old.generation || a.revision < old.revision {
                return Err("context_generation_quarantined");
            }
            if a.generation == old.generation {
                if old.quarantined {
                    return Err("context_generation_quarantined");
                }
                return Ok(old.clone());
            }
            if old.browsers.len() >= MAX_GENERATIONS {
                return Err("context_generation_capacity");
            }
            old.clone()
        } else {
            if self.contexts.len() >= MAX_CONTEXTS {
                return Err("context_capacity");
            }
            Allocation {
                binding,
                command_uid: self.uid(&mut occupied)?,
                generation: a.generation,
                quarantined: false,
                revision: a.revision,
                browsers: BTreeMap::new(),
            }
        };
        let browsers = Browsers {
            secure_uid: self.uid(&mut occupied)?,
            dev_uid: self.uid(&mut occupied)?,
        };
        entry.generation = a.generation;
        entry.revision = a.revision;
        entry.quarantined = false;
        entry.browsers.insert(a.generation, browsers);
        self.contexts.insert(a.context_id.clone(), entry.clone());
        Ok(entry)
    }
    pub fn quarantine(&mut self, agent: &str, before_revision: i64) -> Vec<u32> {
        let mut users = Vec::new();
        for context in self
            .contexts
            .values_mut()
            .filter(|c| c.binding.agent == agent && c.revision < before_revision)
        {
            context.quarantined = true;
            if let Some(browser) = context.browsers.get(&context.generation) {
                users.extend([browser.secure_uid, browser.dev_uid]);
            }
        }
        users
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let mut uids = std::collections::BTreeSet::new();
        if self.contexts.len() > MAX_CONTEXTS
            || !(FIRST_UID..=LAST_UID + 1).contains(&self.next_uid)
        {
            return Err("context_store_invalid");
        }
        for (id, entry) in &self.contexts {
            if uuid::Uuid::parse_str(id).is_err()
                || entry.revision < 1
                || [
                    &entry.binding.agent,
                    &entry.binding.owner,
                    &entry.binding.actor,
                ]
                .iter()
                .any(|id| uuid::Uuid::parse_str(id).is_err())
                || entry.binding.group.as_ref().is_some_and(|id| {
                    !id.strip_prefix("nyxg-")
                        .is_some_and(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
                })
                || entry.browsers.contains_key(&0)
                || entry.generation == 0
                || entry.browsers.len() > MAX_GENERATIONS
                || entry.browsers.last_key_value().map(|(g, _)| *g) != Some(entry.generation)
            {
                return Err("context_store_invalid");
            }
            for uid in std::iter::once(entry.command_uid).chain(
                entry
                    .browsers
                    .values()
                    .flat_map(|b| [b.secure_uid, b.dev_uid]),
            ) {
                if !(FIRST_UID..self.next_uid).contains(&uid) || !uids.insert(uid) {
                    return Err("context_store_invalid");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confirmed() -> Evidence {
        Evidence {
            version: EVIDENCE_VERSION,
            os: "linux".into(),
            landlock_abi: Some(MIN_LANDLOCK_ABI),
            separate_users: Some(true),
            credential_custody: Some(true),
            managed_browser_configured: Some(true),
            command_seccomp: Some(true),
            landlock_enforced: Some(true),
            managed_browser_sandbox: Some(true),
            managed_browser_policy: Some(true),
            apparmor_userns_restricted: None,
            probe_timed_out: false,
            browser_sandbox_unobservable_in_container: false,
        }
    }

    #[test]
    fn inaccessible_container_browser_observations_never_claim_support() {
        for sandbox in [None, Some(false), Some(true)] {
            let report = Evidence {
                browser_sandbox_unobservable_in_container: true,
                managed_browser_sandbox: sandbox,
                ..confirmed()
            };
            let reason = "separated_browser_sandbox_unobservable_in_container";
            assert_eq!(report.unavailable_reason(), Some(reason));
            assert!(!report.support().available);
            assert!(
                unavailable_message(reason).contains("separate machine container or VM per agent")
            );
            for choice in [Choice::Automatic, Choice::RequireSeparated] {
                assert_eq!(select_new(choice, Some(&report)), Err(reason));
            }
            assert_eq!(
                select_new(Choice::SharedLegacy, Some(&report)),
                Ok(Mode::SharedLegacy)
            );
        }
        let old = serde_json::to_value(confirmed()).unwrap();
        let mut missing = old.as_object().unwrap().clone();
        missing.remove("browser_sandbox_unobservable_in_container");
        let restored: Evidence = serde_json::from_value(missing.into()).unwrap();
        assert!(!restored.browser_sandbox_unobservable_in_container);
        assert!(restored.support().available);
    }

    #[test]
    fn new_selection_uses_confirmed_support_and_never_implicitly_falls_back() {
        for abi in [6, 8] {
            let evidence = Evidence {
                landlock_abi: Some(abi),
                ..confirmed()
            };
            for choice in [Choice::Automatic, Choice::RequireSeparated] {
                assert_eq!(select_new(choice, Some(&evidence)), Ok(Mode::Separated));
                assert!(select_new(choice, None).is_err());
                assert!(select_new(choice, Some(&Evidence::default())).is_err());
            }
        }
        for evidence in [None, Some(Evidence::default()), Some(confirmed())] {
            assert_eq!(
                select_new(Choice::SharedLegacy, evidence.as_ref()),
                Ok(Mode::SharedLegacy)
            );
        }
        for version in [0, 1, 2, 3, EVIDENCE_VERSION + 1] {
            let old = Evidence {
                version,
                ..confirmed()
            };
            for choice in [Choice::Automatic, Choice::RequireSeparated] {
                assert_eq!(
                    select_new(choice, Some(&old)),
                    Err("separated_probe_version_unsupported")
                );
            }
            assert_eq!(
                select_new(Choice::SharedLegacy, Some(&old)),
                Ok(Mode::SharedLegacy)
            );
        }
        assert_eq!(Choice::default(), Choice::Automatic);
        assert_eq!(Mode::Separated.as_str(), "separated");
        assert_eq!(Mode::SharedLegacy.as_str(), "shared_legacy");
    }

    #[test]
    fn timeout_is_distinct_actionable_and_cannot_confirm_support() {
        let report = Evidence {
            probe_timed_out: true,
            ..confirmed()
        };
        assert_eq!(
            report.support().reason.as_deref(),
            Some("separated_probe_timeout")
        );
        for choice in [Choice::Automatic, Choice::RequireSeparated] {
            assert_eq!(
                select_new(choice, Some(&report)),
                Err("separated_probe_timeout")
            );
        }
        let message = unavailable_message("separated_probe_timeout");
        assert!(message.contains("five-second") && message.contains("rerun"));
        for warning in ["node tokens", "signing secrets", "stored credentials"] {
            assert!(SHARED_LEGACY_WARNING.contains(warning));
        }
    }

    #[test]
    fn v4_custody_is_required_and_old_evidence_never_recovers_support() {
        for credential_custody in [None, Some(false)] {
            let report = Evidence {
                credential_custody,
                ..confirmed()
            };
            assert_eq!(report.unavailable_reason(), Some("node_custody_unverified"));
            assert_eq!(
                select_new(Choice::Automatic, Some(&report)),
                Err("node_custody_unverified")
            );
            assert_eq!(
                select_new(Choice::RequireSeparated, Some(&report)),
                Err("node_custody_unverified")
            );
            assert_eq!(
                select_new(Choice::SharedLegacy, Some(&report)),
                Ok(Mode::SharedLegacy)
            );
        }
        let mut v3 = serde_json::to_value(confirmed()).unwrap();
        v3["version"] = serde_json::json!(3);
        v3.as_object_mut().unwrap().remove("credential_custody");
        let report: Evidence = serde_json::from_value(v3).unwrap();
        assert_eq!(report.credential_custody, None);
        assert_eq!(
            report.unavailable_reason(),
            Some("separated_probe_version_unsupported")
        );
        assert!(unavailable_message("node_custody_unverified").contains("stored credentials"));
    }

    #[test]
    fn every_missing_or_failed_mechanism_prevents_the_new_default() {
        for (field, failure) in [
            ("separate_users", "separated_requires_separate_users"),
            (
                "managed_browser_configured",
                "separated_requires_managed_browser",
            ),
            ("command_seccomp", "separated_seccomp_probe_failed"),
            ("landlock_enforced", "separated_landlock_probe_failed"),
            (
                "managed_browser_policy",
                "separated_browser_policy_not_effective",
            ),
            (
                "managed_browser_sandbox",
                "separated_browser_sandbox_probe_failed",
            ),
        ] {
            for value in [serde_json::Value::Null, serde_json::json!(false)] {
                let mut report = serde_json::to_value(confirmed()).unwrap();
                report[field] = value.clone();
                let report: Evidence = serde_json::from_value(report).unwrap();
                let reason = if value.is_null() {
                    "separated_probe_evidence_incomplete"
                } else {
                    failure
                };
                for choice in [Choice::Automatic, Choice::RequireSeparated] {
                    assert_eq!(select_new(choice, Some(&report)), Err(reason), "{field}");
                }
                assert!(!report.support().available);
                assert_eq!(report.support().reason.as_deref(), Some(reason));
            }
        }
    }

    #[test]
    fn platform_abi_and_probe_version_are_required_independently() {
        for (report, reason) in [
            (
                Evidence {
                    os: "macos".into(),
                    ..confirmed()
                },
                "separated_requires_linux",
            ),
            (
                Evidence {
                    os: "windows".into(),
                    ..confirmed()
                },
                "separated_requires_linux",
            ),
            (
                Evidence {
                    landlock_abi: None,
                    ..confirmed()
                },
                "separated_requires_landlock_abi_6",
            ),
            (
                Evidence {
                    landlock_abi: Some(5),
                    ..confirmed()
                },
                "separated_requires_landlock_abi_6",
            ),
            (
                Evidence {
                    version: 0,
                    ..confirmed()
                },
                "separated_probe_version_unsupported",
            ),
            (
                Evidence {
                    version: 1,
                    ..confirmed()
                },
                "separated_probe_version_unsupported",
            ),
            (
                Evidence {
                    version: EVIDENCE_VERSION + 1,
                    ..confirmed()
                },
                "separated_probe_version_unsupported",
            ),
        ] {
            assert_eq!(select_new(Choice::Automatic, Some(&report)), Err(reason));
        }
    }

    #[test]
    fn old_protocol_or_support_boolean_does_not_prove_new_default_support() {
        let old: crate::MachineProfile = serde_json::from_value(serde_json::json!({
            "version": 1,
            "authority_versions": [2, 3],
            "separated": {"available": true, "landlock_abi": 8}
        }))
        .unwrap();
        assert!(old.separated.unwrap().available);
        assert_eq!(
            select_new(Choice::Automatic, old.separation_evidence.as_ref()),
            Err("separated_capabilities_not_reported")
        );
        let v1: Evidence = serde_json::from_value(serde_json::json!({
            "version": 1, "os": "linux", "landlock_abi": 8,
            "separate_users": true, "managed_browser_configured": true,
            "command_seccomp": true, "landlock_enforced": true,
            "browser_user_namespaces": true
        }))
        .unwrap();
        assert_eq!(v1.managed_browser_sandbox, None);
        assert_eq!(
            select_new(Choice::Automatic, Some(&v1)),
            Err("separated_probe_version_unsupported")
        );
    }

    #[test]
    fn evidence_wire_compatibility_is_additive_but_partial_reports_never_confirm() {
        let mut report = serde_json::to_value(confirmed()).unwrap();
        report["future_probe"] = serde_json::json!({"result":true});
        let profile: crate::MachineProfile = serde_json::from_value(serde_json::json!({
            "separated": confirmed().support(), "separation_evidence": report
        }))
        .unwrap();
        assert_eq!(profile.separation_evidence.as_ref(), Some(&confirmed()));
        let encoded = serde_json::to_value(&profile).unwrap();
        #[derive(Deserialize)]
        struct OldReader {
            separated: Support,
        }
        let old: OldReader = serde_json::from_value(encoded).unwrap();
        assert!(old.separated.available);
        for value in [
            serde_json::json!({}),
            serde_json::json!({"version":1,"os":"linux","landlock_abi":8}),
        ] {
            let partial: Evidence = serde_json::from_value(value).unwrap();
            assert!(select_new(Choice::Automatic, Some(&partial)).is_err());
        }
    }

    #[test]
    fn advertised_support_accepts_new_fields_and_defaults_missing_authority_to_denied() {
        let profile: crate::MachineProfile = serde_json::from_value(serde_json::json!({
            "separated": {"available": true, "landlock_abi": 8, "future_probe": "ok"}
        }))
        .unwrap();
        assert_eq!(
            profile.separated.unwrap(),
            Support {
                available: true,
                landlock_abi: Some(8),
                reason: None
            }
        );
        let profile: crate::MachineProfile = serde_json::from_value(serde_json::json!({
            "separated": {"future_probe": "ok"}
        }))
        .unwrap();
        assert_eq!(profile.separated.unwrap(), Support::default());
        let profile: crate::MachineProfile = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(profile.separated.is_none());
    }

    #[test]
    fn durable_context_records_still_reject_unknown_fields() {
        let mut registry = Registry::default();
        let allocation = registry.allocate(&authority(), |_| false).unwrap();
        for (mut value, kind) in [
            (serde_json::to_value(&registry).unwrap(), "registry"),
            (serde_json::to_value(&allocation).unwrap(), "allocation"),
            (
                serde_json::to_value(&allocation.binding).unwrap(),
                "binding",
            ),
            (
                serde_json::to_value(allocation.browsers.get(&1).unwrap()).unwrap(),
                "browsers",
            ),
        ] {
            value["unknown"] = serde_json::json!(true);
            let rejected = match kind {
                "registry" => serde_json::from_value::<Registry>(value).is_err(),
                "allocation" => serde_json::from_value::<Allocation>(value).is_err(),
                "binding" => serde_json::from_value::<Binding>(value).is_err(),
                _ => serde_json::from_value::<Browsers>(value).is_err(),
            };
            assert!(rejected, "{kind}");
        }
    }

    fn authority() -> Authority {
        Authority {
            require_v2: true,
            context_id: uuid::Uuid::new_v4().to_string(),
            generation: 1,
            mode: MODE.into(),
            agent_id: uuid::Uuid::new_v4().to_string(),
            owner_id: uuid::Uuid::new_v4().to_string(),
            actor_id: uuid::Uuid::new_v4().to_string(),
            group_id: None,
            runtime_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            turn_id: uuid::Uuid::new_v4().to_string(),
            lease_id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            expires_at_ms: 1,
            capabilities: Default::default(),
        }
    }
    #[test]
    fn allocation_replays_after_crash_without_reusing_users() {
        let mut state = Registry::default();
        let mut a = authority();
        let first = state.allocate(&a, |uid| uid == FIRST_UID).unwrap();
        assert_eq!(first.command_uid, FIRST_UID + 1);
        let encoded = serde_json::to_vec(&state).unwrap();
        let mut recovered: Registry = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(first, recovered.allocate(&a, |_| true).unwrap());
        a.generation = 2;
        let next = recovered.allocate(&a, |_| false).unwrap();
        assert_eq!(first.command_uid, next.command_uid);
        assert_eq!(next.browsers[&1], first.browsers[&1]);
        assert!(next.browsers[&2].secure_uid > first.browsers[&1].dev_uid);
        a.generation = 1;
        assert_eq!(
            recovered.allocate(&a, |_| false).unwrap_err(),
            "context_generation_quarantined"
        );
        recovered.validate().unwrap();
    }
    #[test]
    fn context_identity_and_mode_cannot_be_rebound() {
        let mut state = Registry::default();
        let a = authority();
        state.allocate(&a, |_| false).unwrap();
        for field in 0..4 {
            let mut wrong = a.clone();
            match field {
                0 => wrong.actor_id = uuid::Uuid::new_v4().to_string(),
                1 => wrong.agent_id = uuid::Uuid::new_v4().to_string(),
                2 => wrong.owner_id = uuid::Uuid::new_v4().to_string(),
                _ => wrong.group_id = Some("another-group".into()),
            }
            assert_eq!(
                state.allocate(&wrong, |_| false).unwrap_err(),
                "context_identity_mismatch"
            );
        }
        let mut wrong = a.clone();
        wrong.mode = "shared_legacy".into();
        assert!(state.allocate(&wrong, |_| false).is_err());
        wrong = a;
        wrong.require_v2 = false;
        assert!(state.allocate(&wrong, |_| false).is_err());
    }
    #[test]
    fn corrupt_uid_maps_and_generation_rollback_fail_closed() {
        let mut state = Registry::default();
        let a = authority();
        state.allocate(&a, |_| false).unwrap();
        let mut invalid = state.clone();
        invalid.next_uid = FIRST_UID;
        assert!(invalid.validate().is_err());
        let entry = state.contexts.get_mut(&a.context_id).unwrap();
        entry.browsers.get_mut(&1).unwrap().dev_uid = entry.command_uid;
        assert!(state.validate().is_err());
    }
    #[test]
    fn quarantine_survives_restart_and_late_retries_keep_the_successor() {
        let mut registry = Registry::default();
        let mut a = authority();
        let first = registry.allocate(&a, |_| false).unwrap();
        let mut b = authority();
        b.actor_id = a.actor_id.clone();
        let sibling = registry.allocate(&b, |_| false).unwrap();
        assert_eq!(
            registry.quarantine(&a.agent_id, 2),
            vec![first.browsers[&1].secure_uid, first.browsers[&1].dev_uid]
        );
        let mut registry: Registry =
            serde_json::from_slice(&serde_json::to_vec(&registry).unwrap()).unwrap();
        assert!(registry.allocate(&a, |_| false).is_err());
        assert_eq!(registry.allocate(&b, |_| false).unwrap(), sibling);
        a.revision = 2;
        a.generation = 2;
        let next = registry.allocate(&a, |_| false).unwrap();
        assert!(registry.quarantine(&a.agent_id, 2).is_empty());
        assert_eq!(registry.allocate(&a, |_| false).unwrap(), next);
        assert_ne!(first.browsers[&1], next.browsers[&2]);
        registry.validate().unwrap();
    }
    #[test]
    fn group_bindings_and_v2_generation_are_checked() {
        let mut a = authority();
        a.group_id = Some(format!("nyxg-{}", uuid::Uuid::new_v4().simple()));
        assert!(a.valid(&a.runtime_id, 0));
        let mut registry = Registry::default();
        registry.allocate(&a, |_| false).unwrap();
        registry.validate().unwrap();
        a.require_v2 = false;
        assert!(!a.valid(&a.runtime_id, 0));
        a.require_v2 = true;
        a.generation = 0;
        assert!(!a.valid(&a.runtime_id, 0));
        a.generation = 3;
        a.mode = "isolated".into();
        assert!(!a.valid(&a.runtime_id, 0));
    }
}
