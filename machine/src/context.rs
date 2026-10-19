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
