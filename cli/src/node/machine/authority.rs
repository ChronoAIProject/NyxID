//! Node authority fences survive socket reconnects and daemon restarts. Only
//! signed v2 messages reach this module; deadlines use the local monotonic clock.
use anyhow::{Context, Result, bail};
use nyxid_machine::authority::Authority;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Durable {
    enrolled: bool,
    revisions: HashMap<String, i64>,
    contexts: HashMap<String, Identity>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Identity {
    agent: String,
    owner: String,
    actor: String,
    group: Option<String>,
    #[serde(default = "shared_mode")]
    mode: String,
    #[serde(default = "first_generation")]
    generation: u64,
}
fn shared_mode() -> String {
    "shared_legacy".into()
}
fn first_generation() -> u64 {
    1
}
struct Lease {
    authority: Box<Authority>,
    deadline: Instant,
    stop: watch::Sender<bool>,
    job: Option<String>,
}
struct State {
    durable: Durable,
    leases: HashMap<String, Lease>,
}
pub struct Fences {
    path: PathBuf,
    state: Mutex<State>,
}
impl Fences {
    pub fn open(directory: &Path) -> Result<Self> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let path = directory.join("machine-authority-v2.json");
        let durable = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(file) => {
                let meta = file.metadata()?;
                if !meta.is_file()
                    || meta.uid() != unsafe { libc::geteuid() }
                    || meta.mode() & 0o077 != 0
                    || meta.len() > 2 * 1024 * 1024
                {
                    bail!("invalid machine authority store");
                }
                serde_json::from_reader(file)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Durable::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            state: Mutex::new(State {
                durable,
                leases: HashMap::new(),
            }),
        })
    }
    fn persist(&self, durable: &Durable) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let mut file =
            tempfile::NamedTempFile::new_in(self.path.parent().context("authority directory")?)?;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        let encoded = serde_json::to_vec(durable)?;
        if encoded.len() > 2 * 1024 * 1024 {
            bail!("authority capacity");
        }
        file.write_all(&encoded)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        std::fs::File::open(self.path.parent().context("authority directory")?)?.sync_all()?;
        Ok(())
    }
    pub fn enrolled(&self) -> bool {
        self.state.lock().expect("authority lock").durable.enrolled
    }
    pub fn admit(
        &self,
        authority: &Authority,
        runtime: &str,
        job: Option<String>,
    ) -> Result<watch::Receiver<bool>> {
        let now = chrono::Utc::now().timestamp_millis();
        if !authority.valid(runtime, now) {
            bail!("invalid machine authority");
        }
        let mut state = self.state.lock().expect("authority lock");
        if state
            .durable
            .revisions
            .get(&authority.agent_id)
            .is_some_and(|r| *r > authority.revision)
        {
            bail!("stale authority");
        }
        let identity = Identity {
            agent: authority.agent_id.clone(),
            owner: authority.owner_id.clone(),
            actor: authority.actor_id.clone(),
            group: authority.group_id.clone(),
            mode: authority.mode.clone(),
            generation: authority.generation,
        };
        if state
            .durable
            .contexts
            .get(&authority.context_id)
            .is_some_and(|old| {
                old.agent != identity.agent
                    || old.owner != identity.owner
                    || old.actor != identity.actor
                    || old.group != identity.group
                    || (old.mode == "separated"
                        && identity.mode == "separated"
                        && old.generation > identity.generation)
            })
        {
            bail!("context identity mismatch");
        }
        if state.leases.contains_key(&authority.lease_id) {
            bail!("duplicate authority lease");
        }
        if state.durable.contexts.len() >= 8192 || state.leases.len() >= 8192 {
            bail!("authority capacity");
        }
        let changed = (authority.require_v2 && !state.durable.enrolled)
            || state.durable.contexts.get(&authority.context_id) != Some(&identity)
            || state.durable.revisions.get(&authority.agent_id) != Some(&authority.revision);
        if changed {
            let mut durable = state.durable.clone();
            durable.enrolled |= authority.require_v2;
            durable
                .contexts
                .insert(authority.context_id.clone(), identity);
            durable
                .revisions
                .insert(authority.agent_id.clone(), authority.revision);
            // A failed fsync/write must not make a retry skip the durable fence.
            self.persist(&durable)?;
            state.durable = durable;
        }
        for lease in state.leases.values() {
            if lease.authority.agent_id == authority.agent_id
                && lease.authority.revision < authority.revision
            {
                lease.stop.send_replace(true);
            }
        }
        let stop = watch::channel(false).0;
        let rx = stop.subscribe();
        state.leases.insert(
            authority.lease_id.clone(),
            Lease {
                authority: Box::new(authority.clone()),
                deadline: Instant::now()
                    + Duration::from_millis((authority.expires_at_ms - now) as u64),
                stop,
                job,
            },
        );
        Ok(rx)
    }
    pub fn renew(&self, authority: &Authority, runtime: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        if !authority.valid(runtime, now) {
            bail!("invalid renewal");
        }
        let mut state = self.state.lock().expect("authority lock");
        if state.durable.revisions.get(&authority.agent_id) != Some(&authority.revision) {
            bail!("stale renewal");
        }
        let lease = state
            .leases
            .get_mut(&authority.lease_id)
            .context("unknown lease")?;
        let mut expected = lease.authority.clone();
        expected.expires_at_ms = authority.expires_at_ms;
        if *expected != *authority
            || *lease.stop.borrow()
            || lease.deadline <= Instant::now()
            || authority.expires_at_ms <= lease.authority.expires_at_ms
        {
            bail!("expired or changed renewal");
        }
        lease.authority.expires_at_ms = authority.expires_at_ms;
        lease.deadline =
            Instant::now() + Duration::from_millis((authority.expires_at_ms - now) as u64);
        Ok(())
    }
    pub fn revoke(&self, authority: &Authority, runtime: &str) -> Result<()> {
        if !authority.valid(runtime, chrono::Utc::now().timestamp_millis()) {
            bail!("invalid revocation");
        }
        let mut state = self.state.lock().expect("authority lock");
        let mut durable = state.durable.clone();
        durable.enrolled |= authority.require_v2;
        let revision = durable
            .revisions
            .entry(authority.agent_id.clone())
            .or_default();
        *revision = (*revision).max(authority.revision);
        self.persist(&durable)?;
        state.durable = durable;
        for lease in state.leases.values() {
            if lease.authority.agent_id == authority.agent_id
                && lease.authority.revision < authority.revision
            {
                lease.stop.send_replace(true);
            }
        }
        Ok(())
    }
    pub fn finish(&self, id: &str) {
        let mut state = self.state.lock().expect("authority lock");
        if state.leases.get(id).is_some_and(|l| l.job.is_none()) {
            state.leases.remove(id);
        }
    }
    /// Called every 50 ms even when the WebSocket is disconnected. A stopped
    /// lease is never renewed or resurrected, including after a late response.
    pub fn expired_jobs(&self) -> Vec<String> {
        let mut state = self.state.lock().expect("authority lock");
        let mut jobs = Vec::new();
        state.leases.retain(|_, lease| {
            if lease.deadline <= Instant::now() || *lease.stop.borrow() {
                lease.stop.send_replace(true);
                if let Some(job) = &lease.job {
                    jobs.push(job.clone());
                }
                false
            } else {
                true
            }
        });
        jobs
    }
    pub fn live(&self, id: &str) -> bool {
        self.state
            .lock()
            .expect("authority lock")
            .leases
            .get(id)
            .is_some_and(|l| !*l.stop.borrow() && l.deadline > Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn authority() -> Authority {
        let id = || uuid::Uuid::new_v4().to_string();
        Authority {
            require_v2: true,
            context_id: id(),
            generation: 1,
            mode: "shared_legacy".into(),
            agent_id: id(),
            owner_id: id(),
            actor_id: id(),
            group_id: None,
            runtime_id: id(),
            conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
            turn_id: id(),
            lease_id: id(),
            revision: 1,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 4_000,
            capabilities: nyxid_machine::authority::Capabilities {
                shell: true,
                ..Default::default()
            },
        }
    }
    #[test]
    fn failed_persistence_never_admits_or_skips_the_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let fences = Fences::open(dir.path()).unwrap();
        let authority = authority();
        std::fs::create_dir(&fences.path).unwrap();
        for _ in 0..2 {
            assert!(
                fences
                    .admit(&authority, &authority.runtime_id, None)
                    .is_err()
            );
            assert!(!fences.enrolled());
            assert!(!fences.live(&authority.lease_id));
            assert!(fences.revoke(&authority, &authority.runtime_id).is_err());
            assert!(!fences.enrolled());
        }
        std::fs::remove_dir(&fences.path).unwrap();
        fences
            .admit(&authority, &authority.runtime_id, None)
            .unwrap();
        assert!(fences.enrolled());
        assert!(Fences::open(dir.path()).unwrap().enrolled());
    }

    #[test]
    fn revocation_fences_live_work_late_renewals_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let fences = Fences::open(directory.path()).unwrap();
        let a = authority();
        let stopped = fences.admit(&a, &a.runtime_id, Some("job".into())).unwrap();
        let mut revoke = a.clone();
        revoke.revision = 2;
        fences.revoke(&revoke, &a.runtime_id).unwrap();
        assert!(*stopped.borrow());
        assert!(fences.renew(&a, &a.runtime_id).is_err());
        assert_eq!(fences.expired_jobs(), vec!["job"]);
        assert!(fences.expired_jobs().is_empty());
        let fences = Fences::open(directory.path()).unwrap();
        assert!(fences.enrolled());
        assert!(fences.admit(&a, &a.runtime_id, None).is_err());
        revoke.lease_id = uuid::Uuid::new_v4().to_string();
        assert!(fences.admit(&revoke, &a.runtime_id, None).is_ok());
    }
    #[test]
    fn renewal_binds_every_identity_and_cannot_revive_expiry() {
        let directory = tempfile::tempdir().unwrap();
        let fences = Fences::open(directory.path()).unwrap();
        let a = authority();
        fences.admit(&a, &a.runtime_id, None).unwrap();
        let mut next = a.clone();
        next.expires_at_ms += 100;
        for index in 0..7 {
            let mut changed = next.clone();
            match index {
                0 => changed.actor_id = uuid::Uuid::new_v4().to_string(),
                1 => changed.context_id = uuid::Uuid::new_v4().to_string(),
                2 => changed.revision += 1,
                3 => changed.capabilities.browser = true,
                4 => changed.turn_id = uuid::Uuid::new_v4().to_string(),
                5 => changed.group_id = Some(uuid::Uuid::new_v4().to_string()),
                _ => changed.require_v2 = false,
            }
            assert!(fences.renew(&changed, &a.runtime_id).is_err());
        }
        assert!(fences.renew(&next, &a.runtime_id).is_ok());
        assert!(fences.renew(&next, &a.runtime_id).is_err());
        fences
            .state
            .lock()
            .unwrap()
            .leases
            .get_mut(&a.lease_id)
            .unwrap()
            .deadline = Instant::now();
        next.expires_at_ms += 100;
        assert!(fences.renew(&next, &a.runtime_id).is_err());
    }
    #[test]
    fn contexts_cannot_move_between_people_and_legacy_does_not_enroll() {
        let directory = tempfile::tempdir().unwrap();
        let fences = Fences::open(directory.path()).unwrap();
        let mut a = authority();
        a.require_v2 = false;
        fences.admit(&a, &a.runtime_id, None).unwrap();
        assert!(!fences.enrolled());
        a.lease_id = uuid::Uuid::new_v4().to_string();
        a.actor_id = uuid::Uuid::new_v4().to_string();
        assert!(fences.admit(&a, &a.runtime_id, None).is_err());
    }
    #[test]
    fn authority_store_refuses_symlinks_and_public_modes() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        std::fs::write(&target, b"{}").unwrap();
        let path = directory.path().join("machine-authority-v2.json");
        symlink(&target, &path).unwrap();
        assert!(Fences::open(directory.path()).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, serde_json::to_vec(&Durable::default()).unwrap()).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Fences::open(directory.path()).is_err());
    }
}
