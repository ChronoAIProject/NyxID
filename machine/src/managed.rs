//! Additive hosted-runtime identity. This is not an execution credential.
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "remote-machine";
/// Immutable resource meaning of PROFILE; deployment configuration cannot resize it.
pub const PROFILE_CPU_MILLIS: u32 = 2_000;
pub const PROFILE_MEMORY_MIB: u32 = 2_048;
pub const PROFILE_WORKSPACE_GIB: u32 = 20;

pub fn is_supported_profile(profile: &str) -> bool {
    profile == PROFILE
}
/// Defensive tool ingress bound, separate from file size on the workspace.
pub const MANAGED_FILE_WRITE_BYTES: usize = 1024 * 1024;

/// Independent enforcement is provided by the selected runtime integration.
/// These messages never authorize Pod/PVC creation or imply physical stopping.
pub const DEADLINE_PROTOCOL_VERSION: u32 = 1;
pub const DEADLINE_CONTROL_DOMAIN: &str = "nyxid-managed-deadline-control-v1";

/// Linux boot clock includes suspend and survives process restart. Other local
/// development hosts conservatively use a process identity, requiring rekey
/// after process recreation rather than pretending to have a durable boot ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostClockSample {
    pub utc_ms: i64,
    pub monotonic_ms: u64,
    pub boot_id: String,
}
pub fn host_clock_sample() -> Result<HostClockSample, &'static str> {
    let utc_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "host clock invalid")?
            .as_millis(),
    )
    .map_err(|_| "host clock overflow")?;
    #[cfg(target_os = "linux")]
    let (boot_id, monotonic_ms) = {
        use std::io::Read;
        let mut boot = String::new();
        std::fs::File::open("/proc/sys/kernel/random/boot_id")
            .map_err(|_| "boot identity unavailable")?
            .take(64)
            .read_to_string(&mut boot)
            .map_err(|_| "boot identity unavailable")?;
        let boot = boot.trim().to_owned();
        uuid::Uuid::parse_str(&boot).map_err(|_| "boot identity invalid")?;
        let mut uptime = String::new();
        std::fs::File::open("/proc/uptime")
            .map_err(|_| "boot clock unavailable")?
            .take(128)
            .read_to_string(&mut uptime)
            .map_err(|_| "boot clock unavailable")?;
        let value = uptime
            .split_whitespace()
            .next()
            .ok_or("boot clock invalid")?;
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        if !fraction.bytes().all(|c| c.is_ascii_digit()) {
            return Err("boot clock invalid");
        }
        let seconds: u64 = whole.parse().map_err(|_| "boot clock invalid")?;
        let millis: u64 = format!("{fraction:0<3}")[..3]
            .parse()
            .map_err(|_| "boot clock invalid")?;
        let elapsed = seconds
            .checked_mul(1000)
            .and_then(|n| n.checked_add(millis))
            .ok_or("boot clock overflow")?;
        (boot, elapsed)
    };
    #[cfg(not(target_os = "linux"))]
    let (boot_id, monotonic_ms) = {
        static PROCESS: std::sync::OnceLock<(String, std::time::Instant)> =
            std::sync::OnceLock::new();
        let (id, start) =
            PROCESS.get_or_init(|| (uuid::Uuid::new_v4().to_string(), std::time::Instant::now()));
        (
            id.clone(),
            u64::try_from(start.elapsed().as_millis()).map_err(|_| "boot clock overflow")?,
        )
    };
    Ok(HostClockSample {
        utc_ms,
        monotonic_ms,
        boot_id,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTarget {
    pub machine_id: String,
    pub owner_id: String,
    pub sandbox_uid: String,
    pub pod_uid: String,
    pub pvc_uid: String,
    pub runtime_generation: u64,
    pub credential_epoch: u64,
    pub writer_fence: String,
}
impl RuntimeTarget {
    pub fn validate(&self) -> Result<(), &'static str> {
        for id in [
            &self.machine_id,
            &self.owner_id,
            &self.sandbox_uid,
            &self.pod_uid,
            &self.pvc_uid,
            &self.writer_fence,
        ] {
            if uuid::Uuid::parse_str(id).is_err() {
                return Err("invalid runtime target");
            }
        }
        if self.runtime_generation == 0 || self.credential_epoch == 0 {
            return Err("invalid runtime generation");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadlineAction {
    Arm,
    Renew,
    ShortenStop,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadlineControl {
    pub version: u32,
    pub operation_id: String,
    pub control_revision: u64,
    pub target: RuntimeTarget,
    pub action: DeadlineAction,
    pub run_not_after_ms: i64,
}
impl DeadlineControl {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.target.validate()?;
        if self.version != DEADLINE_PROTOCOL_VERSION
            || self.control_revision == 0
            || self.run_not_after_ms <= 0
            || uuid::Uuid::parse_str(&self.operation_id).is_err()
        {
            return Err("invalid deadline control");
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<String, &'static str> {
        use sha2::{Digest, Sha256};
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| "invalid deadline control")?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }
}

/// Manager authentication is distinct from the workload identity. A receiver
/// refreshes request lifetime on retry without changing the durable digest.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedDeadlineControl {
    pub control: DeadlineControl,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub authentication: String,
}
impl std::fmt::Debug for AuthenticatedDeadlineControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatedDeadlineControl")
            .field("request", &"[REDACTED]")
            .finish()
    }
}
impl AuthenticatedDeadlineControl {
    pub fn sign(control: DeadlineControl, key: &[u8], now_ms: i64) -> Result<Self, &'static str> {
        use hmac::{Hmac, Mac};
        if key.len() < 32 {
            return Err("invalid manager authentication key");
        }
        let expires_at_ms = now_ms.checked_add(30_000).ok_or("invalid request time")?;
        let input = format!(
            "{DEADLINE_CONTROL_DOMAIN}\n{}\n{now_ms}\n{expires_at_ms}",
            control.digest()?
        );
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key)
            .map_err(|_| "invalid manager authentication key")?;
        mac.update(input.as_bytes());
        Ok(Self {
            control,
            issued_at_ms: now_ms,
            expires_at_ms,
            authentication: hex::encode(mac.finalize().into_bytes()),
        })
    }
    pub fn verify(&self, key: &[u8], now_ms: i64) -> Result<(), &'static str> {
        use hmac::{Hmac, Mac};
        if key.len() < 32
            || self.issued_at_ms > now_ms
            || self.expires_at_ms <= now_ms
            || self
                .expires_at_ms
                .checked_sub(self.issued_at_ms)
                .is_none_or(|n| !(1..=30_000).contains(&n))
        {
            return Err("deadline request authentication rejected");
        }
        let input = format!(
            "{DEADLINE_CONTROL_DOMAIN}\n{}\n{}\n{}",
            self.control.digest()?,
            self.issued_at_ms,
            self.expires_at_ms
        );
        let signature = hex::decode(&self.authentication)
            .map_err(|_| "deadline request authentication rejected")?;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key)
            .map_err(|_| "deadline request authentication rejected")?;
        mac.update(input.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| "deadline request authentication rejected")
    }
}

/// Resolved by the trusted provider before API objects disappear, never by a
/// workload supplied path. Opaque runtime/volume handles stay provider-local;
/// the receipt exposes only their digest and the identity of the writer domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedRuntimeTarget {
    pub target: RuntimeTarget,
    pub node_uid: String,
    pub node_boot_id: String,
    pub runtime_instance_digest: String,
    pub runtime_attempt: u64,
    pub pv_uid: String,
    pub csi_driver: String,
    pub volume_handle_digest: String,
    pub writer_domain_digest: String,
}
impl ResolvedRuntimeTarget {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.target.validate()?;
        for id in [&self.node_uid, &self.node_boot_id, &self.pv_uid] {
            if uuid::Uuid::parse_str(id).is_err() {
                return Err("invalid resolved target");
            }
        }
        for digest in [
            &self.runtime_instance_digest,
            &self.volume_handle_digest,
            &self.writer_domain_digest,
        ] {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid resolved target digest");
            }
        }
        if self.csi_driver.is_empty()
            || self.csi_driver.len() > 253
            || !self
                .csi_driver
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return Err("invalid storage driver");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadlineReceipt {
    pub version: u32,
    pub request_digest: String,
    pub control_revision: u64,
    pub resolved_target: ResolvedRuntimeTarget,
    pub provider_instance_id: String,
    pub provider_boot_id: String,
    pub installed_run_not_after_ms: i64,
    pub guaranteed_stop_not_after_ms: i64,
    pub mechanism_version: String,
    pub evidence_ref: String,
    pub stop_requested: bool,
}
impl DeadlineReceipt {
    pub fn validate_for(&self, control: &DeadlineControl) -> Result<(), &'static str> {
        self.resolved_target.validate()?;
        for id in [
            &self.provider_instance_id,
            &self.provider_boot_id,
            &self.evidence_ref,
        ] {
            if uuid::Uuid::parse_str(id).is_err() {
                return Err("invalid deadline receipt");
            }
        }
        if self.version != DEADLINE_PROTOCOL_VERSION
            || self.request_digest != control.digest()?
            || self.control_revision != control.control_revision
            || self.resolved_target.target != control.target
            || self.installed_run_not_after_ms > control.run_not_after_ms
            || self.installed_run_not_after_ms <= 0
            || self.guaranteed_stop_not_after_ms < self.installed_run_not_after_ms
            || self.guaranteed_stop_not_after_ms - self.installed_run_not_after_ms > 300_000
            || self.stop_requested != (control.action == DeadlineAction::ShortenStop)
            || self.mechanism_version.is_empty()
            || self.mechanism_version.len() > 128
            || !self
                .mechanism_version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("deadline receipt mismatch");
        }
        Ok(())
    }
}

/// Supplemental supervisor admission deadline signed with this Node's
/// existing request key. It is not a provider receipt or a physical fence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeWindow {
    pub binding: Binding,
    pub control_revision: u64,
    pub run_not_after_ms: i64,
    pub issued_at_ms: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRuntimeWindow {
    pub window: RuntimeWindow,
    pub signature: String,
}
impl std::fmt::Debug for SignedRuntimeWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedRuntimeWindow")
            .field("window", &self.window)
            .field("signature", &"[REDACTED]")
            .finish()
    }
}
impl SignedRuntimeWindow {
    pub fn sign(window: RuntimeWindow, key: &[u8]) -> Result<Self, &'static str> {
        use hmac::{Hmac, Mac};
        window.binding.validate()?;
        if key.len() != 32
            || window.control_revision == 0
            || window.run_not_after_ms <= window.issued_at_ms
        {
            return Err("invalid runtime window");
        }
        let mut mac =
            Hmac::<sha2::Sha256>::new_from_slice(key).map_err(|_| "invalid runtime key")?;
        mac.update(b"nyxid-managed-node-window-v1\n");
        mac.update(&serde_json::to_vec(&window).map_err(|_| "invalid runtime window")?);
        Ok(Self {
            window,
            signature: hex::encode(mac.finalize().into_bytes()),
        })
    }
    pub fn verify(&self, binding: &Binding, key: &[u8], now_ms: i64) -> Result<(), &'static str> {
        use hmac::{Hmac, Mac};
        if &self.window.binding != binding
            || self.window.control_revision == 0
            || self.window.run_not_after_ms <= now_ms
            || self.window.issued_at_ms > now_ms
            || self.window.issued_at_ms < now_ms.saturating_sub(30_000)
            || self.window.run_not_after_ms > now_ms.saturating_add(600_000)
            || key.len() != 32
        {
            return Err("runtime window rejected");
        }
        binding.validate()?;
        let mut mac =
            Hmac::<sha2::Sha256>::new_from_slice(key).map_err(|_| "runtime window rejected")?;
        mac.update(b"nyxid-managed-node-window-v1\n");
        mac.update(&serde_json::to_vec(&self.window).map_err(|_| "runtime window rejected")?);
        mac.verify_slice(&hex::decode(&self.signature).map_err(|_| "runtime window rejected")?)
            .map_err(|_| "runtime window rejected")
    }
}

/// Safe measurements sent only on the authenticated managed Node socket.
/// Temporary-path scans cannot observe container logs or writable-layer quota,
/// so their lower bound must never be presented as complete pod accounting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageVolume {
    /// Measured capacity only when the source can establish it. Temporary
    /// path scans have no per-Machine allocation or available-space evidence.
    #[serde(default)]
    pub capacity_bytes: Option<u64>,
    pub used_bytes: u64,
    pub available_bytes: Option<u64>,
    pub measurement: String,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub runtime_id: String,
    pub managed_runtime: Binding,
    pub sampled_at_ms: i64,
    pub workspace: UsageVolume,
    pub temporary: UsageVolume,
    pub disk_full: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub node_id: String,
    pub sandbox_uid: String,
    pub pod_uid: String,
    pub runtime_generation: u64,
    pub credential_epoch: u64,
    pub writer_fence: String,
    pub operation_id: String,
    pub delivery_id: String,
}

impl Binding {
    pub fn validate(&self) -> Result<(), &'static str> {
        for id in [
            &self.node_id,
            &self.sandbox_uid,
            &self.pod_uid,
            &self.operation_id,
            &self.delivery_id,
            &self.writer_fence,
        ] {
            if uuid::Uuid::parse_str(id).is_err() {
                return Err("managed identity must contain UUIDs");
            }
        }
        if self.runtime_generation == 0 || self.credential_epoch == 0 {
            return Err("managed runtime generation and credential epoch must be positive");
        }
        Ok(())
    }
    pub fn context_field(&self) -> String {
        // Struct field order is part of the v1 sealed-delivery AAD contract.
        serde_json::to_string(self).expect("managed binding serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn managed_profile_accepts_only_current_full_machine() {
        assert!(is_supported_profile("remote-machine"));
        for obsolete in ["remote-shell-files-v1", "remote-shell-files-v2", "unknown"] {
            assert!(!is_supported_profile(obsolete));
        }
    }

    #[test]
    fn usage_capacity_represents_unknown_temporary_and_measured_workspace() {
        let base = serde_json::json!({"used_bytes": 123, "available_bytes": null,
            "measurement": "node_paths_lower_bound", "complete": false});
        let absent = serde_json::from_value::<UsageVolume>(base.clone());
        assert!(absent.is_ok(), "temporary capacity may be absent");
        let mut explicit = base.clone();
        explicit["capacity_bytes"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<UsageVolume>(explicit).is_ok());
        let mut measured_workspace = base;
        measured_workspace["capacity_bytes"] = serde_json::json!(2147483648_u64);
        let decoded = serde_json::from_value::<UsageVolume>(measured_workspace).unwrap();
        assert_eq!(
            serde_json::to_value(decoded).unwrap()["capacity_bytes"],
            2147483648_u64
        );
    }

    #[test]
    fn zero_generation_cannot_be_a_managed_runtime() {
        let binding = Binding {
            node_id: uuid::Uuid::new_v4().to_string(),
            sandbox_uid: uuid::Uuid::new_v4().to_string(),
            pod_uid: uuid::Uuid::new_v4().to_string(),
            runtime_generation: 0,
            credential_epoch: 1,
            writer_fence: uuid::Uuid::new_v4().to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            delivery_id: uuid::Uuid::new_v4().to_string(),
        };
        assert!(binding.validate().is_err());
    }

    #[test]
    fn authenticated_control_rejects_target_mutation_expiry_and_wrong_manager_key() {
        let control = DeadlineControl {
            version: 1,
            operation_id: uuid::Uuid::new_v4().to_string(),
            control_revision: 1,
            target: RuntimeTarget {
                machine_id: uuid::Uuid::new_v4().to_string(),
                owner_id: uuid::Uuid::new_v4().to_string(),
                sandbox_uid: uuid::Uuid::new_v4().to_string(),
                pod_uid: uuid::Uuid::new_v4().to_string(),
                pvc_uid: uuid::Uuid::new_v4().to_string(),
                runtime_generation: 1,
                credential_epoch: 1,
                writer_fence: uuid::Uuid::new_v4().to_string(),
            },
            action: DeadlineAction::Arm,
            run_not_after_ms: 120_000,
        };
        let signed = AuthenticatedDeadlineControl::sign(control, &[7; 32], 100_000).unwrap();
        assert!(signed.verify(&[7; 32], 100_001).is_ok());
        assert!(signed.verify(&[8; 32], 100_001).is_err());
        assert!(signed.verify(&[7; 32], 130_000).is_err());
        let mut forged = signed.clone();
        forged.control.target.pod_uid = uuid::Uuid::new_v4().to_string();
        assert!(forged.verify(&[7; 32], 100_001).is_err());
        assert!(!format!("{signed:?}").contains(&signed.authentication));
    }

    #[test]
    fn signed_node_window_binds_current_runtime_deadline_and_signature() {
        let binding = Binding {
            node_id: uuid::Uuid::new_v4().to_string(),
            sandbox_uid: uuid::Uuid::new_v4().to_string(),
            pod_uid: uuid::Uuid::new_v4().to_string(),
            runtime_generation: 1,
            credential_epoch: 1,
            writer_fence: uuid::Uuid::new_v4().to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            delivery_id: uuid::Uuid::new_v4().to_string(),
        };
        let signed = SignedRuntimeWindow::sign(
            RuntimeWindow {
                binding: binding.clone(),
                control_revision: 1,
                run_not_after_ms: 120_000,
                issued_at_ms: 100_000,
            },
            &[7; 32],
        )
        .unwrap();
        assert!(signed.verify(&binding, &[7; 32], 100_001).is_ok());
        assert!(signed.verify(&binding, &[7; 32], 120_000).is_err());
        let mut next = binding.clone();
        next.credential_epoch += 1;
        assert!(signed.verify(&next, &[7; 32], 100_001).is_err());
        let mut changed = signed.clone();
        changed.window.run_not_after_ms += 1;
        assert!(changed.verify(&binding, &[7; 32], 100_001).is_err());
    }
}
