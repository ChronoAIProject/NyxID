//! V3 external leases share the runtime and executor, but never a chat identity.
use anyhow::{Context, Result, bail};
use nyxid_machine::authority::ExternalAuthority;
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
    revoked: HashMap<String, i64>,
    #[serde(default)]
    contexts: HashMap<String, ContextIdentity>,
    #[serde(default)]
    grant_revisions: HashMap<String, i64>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ContextIdentity {
    principal: nyxid_machine::authority::ExternalPrincipal,
    owner_id: String,
    node_id: String,
    grant_id: String,
}
struct Lease {
    authority: ExternalAuthority,
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
        let path = directory.join("machine-authority-v3.json");
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
                    bail!("invalid external authority store");
                }
                serde_json::from_reader(file)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Durable::default(),
            Err(error) => return Err(error.into()),
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
        use std::{io::Write, os::unix::fs::PermissionsExt};
        let directory = self.path.parent().context("authority directory")?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        let bytes = serde_json::to_vec(durable)?;
        if bytes.len() > 2 * 1024 * 1024 {
            bail!("external authority capacity");
        }
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
    pub fn admit(
        &self,
        authority: &ExternalAuthority,
        node: &str,
        runtime: &str,
        execution: bool,
    ) -> Result<watch::Receiver<bool>> {
        let now = chrono::Utc::now().timestamp_millis();
        if !authority.valid(node, runtime, now) {
            bail!("invalid external authority");
        }
        let mut state = self.state.lock().expect("external authority lock");
        if state.durable.revoked.contains_key(&authority.lease_id)
            || state.leases.contains_key(&authority.lease_id)
            || state.leases.len() >= 8192
        {
            bail!("revoked or duplicate external lease");
        }
        let mut principal = authority.principal.clone();
        if let nyxid_machine::authority::ExternalPrincipal::ApiKey {
            api_key_credential_id,
            ..
        } = &mut principal
        {
            *api_key_credential_id = None;
        }
        let identity = ContextIdentity {
            principal,
            owner_id: authority.owner_id.clone(),
            node_id: authority.node_id.clone(),
            grant_id: authority.grant_id.clone(),
        };
        if state
            .durable
            .contexts
            .get(&authority.context_id)
            .is_some_and(|previous| previous != &identity)
            || state
                .durable
                .grant_revisions
                .get(&authority.grant_id)
                .is_some_and(|revision| *revision > authority.grant_revision)
        {
            bail!("external context rebound or stale grant revision");
        }
        if state.durable.contexts.get(&authority.context_id) != Some(&identity)
            || state.durable.grant_revisions.get(&authority.grant_id)
                != Some(&authority.grant_revision)
        {
            if (!state.durable.contexts.contains_key(&authority.context_id)
                && state.durable.contexts.len() >= 8192)
                || (!state
                    .durable
                    .grant_revisions
                    .contains_key(&authority.grant_id)
                    && state.durable.grant_revisions.len() >= 8192)
            {
                bail!("external context capacity");
            }
            let mut durable = state.durable.clone();
            durable
                .contexts
                .insert(authority.context_id.clone(), identity);
            durable
                .grant_revisions
                .insert(authority.grant_id.clone(), authority.grant_revision);
            self.persist(&durable)?;
            state.durable = durable;
        }
        for lease in state.leases.values() {
            if lease.authority.grant_id == authority.grant_id
                && lease.authority.grant_revision < authority.grant_revision
            {
                lease.stop.send_replace(true);
            }
        }
        let stop = watch::channel(false).0;
        let receiver = stop.subscribe();
        state.leases.insert(
            authority.lease_id.clone(),
            Lease {
                authority: authority.clone(),
                deadline: Instant::now()
                    + Duration::from_millis((authority.expires_at_ms - now) as u64),
                stop,
                job: execution.then(|| authority.job_id.clone()).flatten(),
            },
        );
        Ok(receiver)
    }
    pub fn renew(&self, authority: &ExternalAuthority, node: &str, runtime: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        if !authority.valid(node, runtime, now) {
            bail!("invalid external renewal");
        }
        let mut state = self.state.lock().expect("external authority lock");
        if state.durable.revoked.contains_key(&authority.lease_id)
            || state.durable.grant_revisions.get(&authority.grant_id)
                != Some(&authority.grant_revision)
        {
            bail!("revoked external lease");
        }
        let lease = state
            .leases
            .get_mut(&authority.lease_id)
            .context("unknown external lease")?;
        let mut expected = lease.authority.clone();
        // A verified new access token may extend auth time; neither a child
        // switch nor roots/services/grant/job changes can renew this lease.
        expected.expires_at_ms = authority.expires_at_ms;
        expected.auth_expires_at_ms = authority.auth_expires_at_ms;
        expected.funding_expires_at_ms = authority.funding_expires_at_ms;
        if expected != *authority
            || *lease.stop.borrow()
            || lease.deadline <= Instant::now()
            || authority.expires_at_ms <= lease.authority.expires_at_ms
            || authority.auth_expires_at_ms < lease.authority.auth_expires_at_ms
            || match (
                lease.authority.funding_expires_at_ms,
                authority.funding_expires_at_ms,
            ) {
                (None, None) => false,
                (Some(old), Some(new)) => new < old,
                _ => true,
            }
        {
            bail!("expired or changed external renewal");
        }
        lease.authority = authority.clone();
        lease.deadline =
            Instant::now() + Duration::from_millis((authority.expires_at_ms - now) as u64);
        Ok(())
    }
    pub fn revoke(&self, authority: &ExternalAuthority, node: &str, runtime: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        if !authority.valid_stop(node, runtime, now) {
            bail!("invalid external revocation");
        }
        let mut state = self.state.lock().expect("external authority lock");
        if let Some(lease) = state.leases.get(&authority.lease_id)
            && (lease.authority.principal != authority.principal
                || lease.authority.grant_id != authority.grant_id
                || lease.authority.job_id != authority.job_id)
        {
            bail!("external revocation binding mismatch");
        }
        state.durable.revoked.retain(|_, until| *until >= now);
        if state.durable.revoked.len() >= 8192
            && !state.durable.revoked.contains_key(&authority.lease_id)
        {
            bail!("external revocation capacity");
        }
        // Retain beyond all possible old requests and the original job's
        // lifetime. Disk failure does not acknowledge a durable revocation.
        state.durable.revoked.insert(
            authority.lease_id.clone(),
            authority.job_expires_at_ms.max(now.saturating_add(120_000)),
        );
        self.persist(&state.durable)?;
        if let Some(lease) = state.leases.get(&authority.lease_id) {
            lease.stop.send_replace(true);
        }
        Ok(())
    }
    pub fn live(&self, id: &str) -> bool {
        let state = self.state.lock().expect("external authority lock");
        state
            .leases
            .get(id)
            .is_some_and(|lease| lease.deadline > Instant::now() && !*lease.stop.borrow())
    }
    pub fn revoked(&self, id: &str) -> bool {
        self.state
            .lock()
            .expect("external authority lock")
            .durable
            .revoked
            .contains_key(id)
    }
    pub fn finish(&self, id: &str) {
        let mut state = self.state.lock().expect("external authority lock");
        if state
            .leases
            .get(id)
            .is_some_and(|lease| lease.job.is_none())
        {
            state.leases.remove(id);
        }
    }
    pub fn expired_jobs(&self) -> Vec<String> {
        let mut state = self.state.lock().expect("external authority lock");
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
}
