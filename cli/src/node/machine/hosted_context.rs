//! Hosted-only durable substrate. No credentials or browser contents are stored
//! here. The caller must supply authenticated original-volume/runtime identities.
//! Missing storage is never permission to recreate a retained workspace.
use anyhow::{Context, Result, ensure};
use nyxid_machine::{
    authority::Authority,
    context::{Allocation, FIRST_UID, Registry},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
};

pub const ACCOUNT_COUNT: u32 = 2048;
pub const RETAINED_ROOT: &str = "/var/lib/nyxid-contexts";
const LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPool {
    version: u32,
    first_uid: u32,
    count: u32,
}
impl AccountPool {
    pub fn system() -> Result<Self> {
        let path = Path::new("/usr/share/nyxid/hosted-context-accounts.json");
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0 && meta.len() < 1024,
            "context_account_supply_invalid"
        );
        let pool: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        pool.validate(
            &std::fs::read_to_string("/etc/passwd")?,
            &std::fs::read_to_string("/etc/group")?,
        )?;
        Ok(pool)
    }
    pub fn contains(&self, uid: u32) -> bool {
        (FIRST_UID..FIRST_UID + ACCOUNT_COUNT).contains(&uid)
    }
    pub fn validate(&self, passwd: &str, groups: &str) -> Result<()> {
        ensure!(
            self.version == 1 && self.first_uid == FIRST_UID && self.count == ACCOUNT_COUNT,
            "context_account_supply_invalid"
        );
        let users: Vec<Vec<_>> = passwd
            .lines()
            .map(|line| line.split(':').collect())
            .collect();
        let groups: Vec<Vec<_>> = groups
            .lines()
            .map(|line| line.split(':').collect())
            .collect();
        ensure!(
            users.iter().all(|r| r.len() == 7) && groups.iter().all(|r| r.len() == 4),
            "context_account_supply_invalid"
        );
        for uid in FIRST_UID..FIRST_UID + ACCOUNT_COUNT {
            let id = uid.to_string();
            let name = format!("nyxc{uid}");
            let user: Vec<_> = users
                .iter()
                .filter(|r| r[0] == name || r[2] == id)
                .collect();
            let group: Vec<_> = groups
                .iter()
                .filter(|r| r[0] == name || r[2] == id)
                .collect();
            ensure!(
                user.len() == 1 && group.len() == 1,
                "context_account_collision"
            );
            let u = user[0];
            let g = group[0];
            ensure!(
                u[0] == name
                    && u[1] == "x"
                    && u[2] == id
                    && u[3] == id
                    && u[4] == "nyxid-hosted-context-v1"
                    && u[5] == "/nonexistent"
                    && u[6] == "/usr/sbin/nologin"
                    && g[0] == name
                    && g[2] == id
                    && g[3].is_empty(),
                "context_account_collision"
            );
            // Never inherit a privileged/supplementary group through NSS.
            ensure!(
                !groups
                    .iter()
                    .any(|g| g[3].split(',').any(|member| member == name))
                    && !users.iter().any(|u| u[3] == id && u[2] != id),
                "context_account_collision"
            );
        }
        Ok(())
    }
}

/// Values come from verified bootstrap/current runtime, never request parameters.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VolumeBinding {
    pub node_id: String,
    pub pvc_uid: String,
}
impl VolumeBinding {
    fn validate(&self) -> Result<()> {
        ensure!(
            uuid::Uuid::parse_str(&self.node_id).is_ok()
                && uuid::Uuid::parse_str(&self.pvc_uid).is_ok(),
            "context_volume_binding_invalid"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    ready: bool,
    runtime_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    binding: VolumeBinding,
    registry: Registry,
    workspaces: BTreeMap<String, Workspace>,
}

/// Pin an entirely supervisor-owned ancestor chain. Never follows a symlink,
/// broadens an existing directory, or repairs an agent-controlled parent.
pub(super) fn directory(path: &Path, create: bool) -> Result<OwnedFd> {
    ensure!(path.is_absolute(), "context_path_invalid");
    let raw = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    ensure!(raw >= 0, "context_directory_unavailable");
    let mut parent = unsafe { OwnedFd::from_raw_fd(raw) };
    for part in path.components() {
        let Component::Normal(name) = part else {
            ensure!(part == Component::RootDir, "context_path_invalid");
            continue;
        };
        let name = std::ffi::CString::new(name.as_bytes())?;
        let created = if create {
            let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o711) };
            ensure!(
                result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST),
                "context_directory_unavailable"
            );
            if result == 0 {
                File::from(parent.try_clone()?).sync_all()?;
            }
            result == 0
        } else {
            false
        };
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        ensure!(raw >= 0, "context_directory_unavailable");
        parent = unsafe { OwnedFd::from_raw_fd(raw) };
        let meta = File::from(parent.try_clone()?).metadata()?;
        ensure!(
            meta.uid() == 0 && meta.mode() & 0o022 == 0,
            "context_parent_not_private"
        );
        if created {
            // The supervisor uses umask 077. Newly created path ancestors need
            // search permission so a worker can reach its own 0710 UID gate;
            // directory names and existing private metadata remain unreadable.
            let directory = File::from(parent.try_clone()?);
            directory.set_permissions(std::fs::Permissions::from_mode(0o711))?;
            directory.sync_all()?;
        }
    }
    Ok(parent)
}
fn private_file(path: &Path, create: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == 0
            && meta.mode() & 0o077 == 0
            && meta.nlink() == 1
            && meta.len() <= LIMIT,
        "context_store_invalid"
    );
    Ok(file)
}

pub struct HostedStore {
    state: PathBuf,
    data: PathBuf,
    runtime_id: String,
    pool: AccountPool,
    journal: Journal,
    _lock: File,
}
impl Drop for HostedStore {
    fn drop(&mut self) {
        // A concurrent spawn can temporarily inherit this open file description
        // before CLOEXEC. Release our supervisor lease explicitly on orderly
        // shutdown instead of waiting for an unrelated child's exec.
        unsafe {
            libc::flock(self._lock.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
impl HostedStore {
    /// `initialize` is an authenticated *new allocation* fact, not !exists().
    /// A crash during first initialization may refuse recovery; it cannot
    /// initialize a replacement empty volume as the retained workspace.
    pub fn open(
        root: &Path,
        binding: VolumeBinding,
        runtime_id: &str,
        initialize: bool,
        pool: AccountPool,
    ) -> Result<Self> {
        ensure!(
            unsafe { libc::geteuid() } == 0,
            "context_supervisor_required"
        );
        binding.validate()?;
        ensure!(
            uuid::Uuid::parse_str(runtime_id).is_ok(),
            "context_runtime_invalid"
        );
        directory(root, false)?;
        let state = root.join("metadata");
        let fresh = if initialize {
            match std::fs::create_dir(&state) {
                Ok(()) => true,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(e) => return Err(e.into()),
            }
        } else {
            false
        };
        if fresh {
            ensure!(initialize, "context_original_volume_missing");
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
            File::open(root)?.sync_all()?;
        }
        let state_fd = directory(&state, false)?;
        let meta = File::from(state_fd).metadata()?;
        ensure!(meta.mode() & 0o777 == 0o700, "context_store_invalid");
        let lock = private_file(&state.join("lock"), true)?;
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "context_supervisor_already_running"
        );
        let data = root.join("contexts");
        let journal = if fresh {
            ensure!(!data.exists(), "context_store_inconsistent");
            directory(&data, true)?;
            Journal {
                version: 1,
                binding: binding.clone(),
                registry: Registry::default(),
                workspaces: BTreeMap::new(),
            }
        } else {
            let file = private_file(&state.join("journal.json"), false)
                .context("context_original_journal_missing")?;
            let mut bytes = Vec::new();
            file.take(LIMIT + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() as u64 <= LIMIT, "context_store_invalid");
            serde_json::from_slice::<Journal>(&bytes).context("context_store_invalid")?
        };
        ensure!(
            journal.version == 1 && journal.binding == binding,
            "context_original_volume_mismatch"
        );
        let store = Self {
            state,
            data,
            runtime_id: runtime_id.into(),
            pool,
            journal,
            _lock: lock,
        };
        store.validate()?;
        if fresh {
            store.persist(&store.journal)?;
        }
        Ok(store)
    }
    pub(super) fn volume_binding(&self) -> &VolumeBinding {
        &self.journal.binding
    }
    fn workspace(&self, allocation: &Allocation) -> PathBuf {
        self.data
            .join(allocation.command_uid.to_string())
            .join("workspace")
    }
    fn validate(&self) -> Result<()> {
        self.journal
            .registry
            .validate()
            .map_err(anyhow::Error::msg)?;
        directory(&self.data, false)?;
        ensure!(
            self.journal.workspaces.len() == self.journal.registry.contexts.len(),
            "context_store_inconsistent"
        );
        for (id, allocation) in &self.journal.registry.contexts {
            let state = self
                .journal
                .workspaces
                .get(id)
                .context("context_store_inconsistent")?;
            ensure!(
                uuid::Uuid::parse_str(&state.runtime_id).is_ok(),
                "context_store_invalid"
            );
            ensure!(
                self.pool.contains(allocation.command_uid)
                    && allocation
                        .browsers
                        .values()
                        .all(|b| self.pool.contains(b.secure_uid) && self.pool.contains(b.dev_uid)),
                "context_account_supply_exhausted"
            );
            if state.ready {
                self.verify_workspace(allocation)?;
            }
        }
        // Unknown directories cannot be rebound to a newly allocated identity.
        for child in std::fs::read_dir(&self.data)? {
            let name = child?.file_name();
            ensure!(
                self.journal
                    .registry
                    .contexts
                    .values()
                    .any(|a| name == a.command_uid.to_string().as_str()),
                "context_store_inconsistent"
            );
        }
        Ok(())
    }
    fn persist(&self, journal: &Journal) -> Result<()> {
        journal.registry.validate().map_err(anyhow::Error::msg)?;
        let bytes = serde_json::to_vec(journal)?;
        ensure!(bytes.len() as u64 <= LIMIT, "context_store_capacity");
        let mut file = tempfile::NamedTempFile::new_in(&self.state)?;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(self.state.join("journal.json"))?;
        File::open(&self.state)?.sync_all()?;
        Ok(())
    }
    fn verify_workspace(&self, allocation: &Allocation) -> Result<()> {
        let path = self.workspace(allocation);
        let gate = directory(path.parent().context("context_path_invalid")?, false)?;
        let meta = File::from(gate).metadata()?;
        ensure!(
            meta.gid() == allocation.command_uid && meta.mode() & 0o777 == 0o710,
            "context_workspace_gate_invalid"
        );
        let meta =
            std::fs::symlink_metadata(&path).context("context_original_workspace_missing")?;
        ensure!(
            meta.is_dir()
                && meta.uid() == allocation.command_uid
                && meta.gid() == allocation.command_uid
                && meta.mode() & 0o777 == 0o700,
            "context_workspace_invalid"
        );
        Ok(())
    }
    pub fn provision(&mut self, authority: &Authority) -> Result<(Allocation, PathBuf)> {
        let mut next = self.journal.clone();
        let old = next.registry.contexts.get(&authority.context_id).cloned();
        if let Some(old) = &old {
            let state = next
                .workspaces
                .get(&authority.context_id)
                .context("context_store_inconsistent")?;
            ensure!(
                state.runtime_id == self.runtime_id || authority.generation > old.generation,
                "context_profile_generation_required"
            );
        }
        let allocation = next
            .registry
            .allocate(authority, |uid| !self.pool.contains(uid))
            .map_err(anyhow::Error::msg)?;
        let ready = next
            .workspaces
            .get(&authority.context_id)
            .is_some_and(|state| state.ready);
        next.workspaces.insert(
            authority.context_id.clone(),
            Workspace {
                ready,
                runtime_id: self.runtime_id.clone(),
            },
        );
        self.persist(&next)?;
        self.journal = next;
        let path = self.workspace(&allocation);
        if !ready {
            let gate = directory(path.parent().context("context_path_invalid")?, true)?;
            ensure!(
                unsafe { libc::fchmod(gate.as_raw_fd(), 0o710) } == 0
                    && unsafe { libc::fchown(gate.as_raw_fd(), 0, allocation.command_uid) } == 0,
                "context_workspace_gate_invalid"
            );
            // mkdir as root, chmod before chown: the formal Pod has CHOWN but
            // deliberately lacks FOWNER/DAC_OVERRIDE.
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
                    std::os::unix::fs::chown(
                        &path,
                        Some(allocation.command_uid),
                        Some(allocation.command_uid),
                    )?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let meta = std::fs::symlink_metadata(&path)?;
                    ensure!(
                        meta.is_dir()
                            && ((meta.uid() == 0 && meta.mode() & 0o077 == 0)
                                || (meta.uid() == allocation.command_uid
                                    && meta.mode() & 0o777 == 0o700)),
                        "context_workspace_invalid"
                    );
                    if meta.uid() == 0 {
                        std::os::unix::fs::chown(
                            &path,
                            Some(allocation.command_uid),
                            Some(allocation.command_uid),
                        )?;
                    }
                }
                Err(e) => return Err(e.into()),
            }
            File::from(gate).sync_all()?;
            self.verify_workspace(&allocation)?;
            let mut next = self.journal.clone();
            next.workspaces
                .get_mut(&authority.context_id)
                .unwrap()
                .ready = true;
            self.persist(&next)?;
            self.journal = next;
        } else {
            self.verify_workspace(&allocation)?;
        }
        Ok((allocation, path))
    }
    pub fn quarantine(&mut self, agent: &str, before_revision: i64) -> Result<Vec<u32>> {
        let mut next = self.journal.clone();
        let users = next.registry.quarantine(agent, before_revision);
        self.persist(&next)?;
        self.journal = next;
        Ok(users)
    }
}
