//! Library for the existing privileged provider integration. This is not a
//! daemon, Pod controller, or physical actuator. A selected runtime adapter
//! must supply target resolution, durable timer/restart fencing, and retained
//! terminal events before this library can make physical guarantees.
use anyhow::{Result, bail, ensure};
use nyxid_machine::managed::{
    AuthenticatedDeadlineControl, DEADLINE_PROTOCOL_VERSION, DeadlineAction, DeadlineControl,
    DeadlineReceipt, ResolvedRuntimeTarget, RuntimeTarget,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockSample {
    pub utc_ms: i64,
    pub monotonic_ms: u64,
    pub boot_id: String,
}
pub trait Clock: Send + Sync {
    fn sample(&self) -> Result<ClockSample>;
}

#[derive(Default)]
pub struct HostClock;
impl Clock for HostClock {
    fn sample(&self) -> Result<ClockSample> {
        let sample = nyxid_machine::managed::host_clock_sample().map_err(anyhow::Error::msg)?;
        Ok(ClockSample {
            utc_ms: sample.utc_ms,
            monotonic_ms: sample.monotonic_ms,
            boot_id: sample.boot_id,
        })
    }
}

/// Adapter-returned retained timer registration. It contains no "stopped"
/// boolean. The adapter contract requires journaled revision fencing and an
/// enforcement location independent of Manager/WSS/workload availability.
#[derive(Clone, Debug)]
pub struct InstalledTimerEvent {
    pub target: ResolvedRuntimeTarget,
    pub revision: u64,
    pub cutoff_ms: i64,
    pub guaranteed_stop_ms: i64,
    pub mechanism_version: String,
    pub event_id: String,
}

/// A terminal record is issued by the selected runtime's retained terminal
/// event stream, not a successful stop request or absence of an API object.
#[derive(Clone, Debug)]
pub struct TerminalRuntimeEvent {
    pub target: ResolvedRuntimeTarget,
    pub event_id: String,
    pub stopped_at_ms: Option<i64>,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct WriterAdmissionEvent {
    pub target: ResolvedRuntimeTarget,
    pub event_id: String,
    pub observed_at_ms: i64,
    /// Only a retained positive running observation advances customer time.
    pub running_at_ms: Option<i64>,
    /// Runtime/provider retained proof that the preceding writer has lost I/O
    /// authority. A new volume uses its creation/no-predecessor proof here.
    pub predecessor_exclusion_event_id: String,
}

pub trait RuntimeActuator: Send + Sync {
    fn resolve(&self, target: &RuntimeTarget) -> Result<ResolvedRuntimeTarget>;
    /// Must fence stale revisions atomically in the actual enforcement domain,
    /// persist its timer before returning, and forbid reacquisition after stop.
    fn install(
        &self,
        target: &ResolvedRuntimeTarget,
        control: &DeadlineControl,
    ) -> Result<InstalledTimerEvent>;
    /// Idempotently stop and prohibit every replacement attempt in this bound
    /// writer domain. A return value acknowledges the request, not completion.
    fn request_stop(&self, target: &ResolvedRuntimeTarget, revision: u64) -> Result<()>;
    fn terminal_event(
        &self,
        target: &ResolvedRuntimeTarget,
    ) -> Result<Option<TerminalRuntimeEvent>>;
    fn writer_event(
        &self,
        _target: &ResolvedRuntimeTarget,
    ) -> Result<Option<WriterAdmissionEvent>> {
        Ok(None)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeadlineState {
    Requested,
    Installed,
    StopRequested,
    Terminal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadlineRecord {
    pub store_version: u64,
    pub control: DeadlineControl,
    pub digest: String,
    pub resolved: ResolvedRuntimeTarget,
    pub state: DeadlineState,
    pub installed: Option<DeadlineReceipt>,
    pub last_utc_ms: i64,
    pub last_monotonic_ms: u64,
    pub clock_boot_id: String,
    pub monotonic_deadline_ms: u64,
    pub requested_monotonic_deadline_ms: u64,
    pub terminal_event_id: Option<String>,
    pub stopped_at_ms: Option<i64>,
    pub stop_observed_at_ms: Option<i64>,
}

pub trait DeadlineStore: Send + Sync {
    fn load(&self, machine_id: &str) -> Result<Option<DeadlineRecord>>;
    /// expected None means absent. Must commit durably before returning true.
    fn compare_exchange(
        &self,
        machine_id: &str,
        expected: Option<u64>,
        next: &DeadlineRecord,
    ) -> Result<bool>;
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<DeadlineRecord>>;
}

/// A verified installation effect is private and can only leave the engine
/// after the actuator event was bound and the receipt was durably committed.
pub struct VerifiedInstalledDeadline {
    receipt: DeadlineReceipt,
}
impl VerifiedInstalledDeadline {
    pub fn receipt(&self) -> &DeadlineReceipt {
        &self.receipt
    }
}
pub struct VerifiedTerminalRuntime {
    event: TerminalRuntimeEvent,
}
pub struct VerifiedWriterAdmission {
    event: WriterAdmissionEvent,
}
impl VerifiedTerminalRuntime {
    pub fn event(&self) -> &TerminalRuntimeEvent {
        &self.event
    }
}

/// Signers receive only an engine-verified effect. The concrete provider owns
/// signing-key custody and its existing authenticated HTTPS receiver.
pub struct ReceiptIssuer {
    key: jsonwebtoken::EncodingKey,
    issuer: String,
}
impl ReceiptIssuer {
    pub fn from_rsa_pem(pem: &[u8], issuer: String) -> Result<Self> {
        ensure!(
            !issuer.is_empty()
                && issuer.len() <= 128
                && issuer
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-:/".contains(&b)),
            "provider issuer invalid"
        );
        Ok(Self {
            key: jsonwebtoken::EncodingKey::from_rsa_pem(pem)?,
            issuer,
        })
    }
    pub fn issue_installed(
        &self,
        effect: &VerifiedInstalledDeadline,
        now_ms: i64,
    ) -> Result<String> {
        let claims = serde_json::json!({"iss":self.issuer,"aud":"nyxid-remote-machine-v1","iat":now_ms.div_euclid(1000),"exp":now_ms.div_euclid(1000)+30,"purpose":"deadline_installed","receipt":effect.receipt});
        Ok(jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &self.key,
        )?)
    }
    pub fn issue_terminal(
        &self,
        effect: &VerifiedTerminalRuntime,
        challenge: &str,
        now_ms: i64,
    ) -> Result<String> {
        ensure!(
            uuid::Uuid::parse_str(challenge).is_ok(),
            "invalid provider challenge"
        );
        let event = &effect.event;
        ensure!(
            event.observed_at_ms <= now_ms && event.observed_at_ms >= now_ms.saturating_sub(60_000),
            "terminal observation must be freshly revalidated"
        );
        let target = &event.target.target;
        let claims = serde_json::json!({"iss":self.issuer,"aud":"nyxid-remote-machine-v1","iat":now_ms.div_euclid(1000),"exp":now_ms.div_euclid(1000)+30,"purpose":"runtime","challenge":challenge,"machine_id":target.machine_id,"owner_id":target.owner_id,"binding":{"sandbox_uid":target.sandbox_uid,"pod_uid":target.pod_uid,"runtime_generation":target.runtime_generation,"credential_epoch":target.credential_epoch,"writer_fence":target.writer_fence},"pvc_uid":target.pvc_uid,"evidence_ref":event.event_id,"observed_at":event.observed_at_ms.div_euclid(1000),"stopped_at_ns":event.stopped_at_ms.and_then(|ms|ms.checked_mul(1_000_000)),"running_at_ns":serde_json::Value::Null,"state":"stopped","credentials_revoked":false,"workspace_released":false,"persistent_volume_released":false,"accounting_closed":false});
        Ok(jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &self.key,
        )?)
    }
    pub fn issue_writer(
        &self,
        effect: &VerifiedWriterAdmission,
        challenge: &str,
        now_ms: i64,
    ) -> Result<String> {
        ensure!(
            uuid::Uuid::parse_str(challenge).is_ok(),
            "invalid provider challenge"
        );
        let event = &effect.event;
        ensure!(
            event.observed_at_ms <= now_ms && event.observed_at_ms >= now_ms.saturating_sub(60_000),
            "writer observation must be fresh"
        );
        let target = &event.target.target;
        let claims = serde_json::json!({"iss":self.issuer,"aud":"nyxid-remote-machine-v1","iat":now_ms.div_euclid(1000),"exp":now_ms.div_euclid(1000)+30,"purpose":"runtime","challenge":challenge,"machine_id":target.machine_id,"owner_id":target.owner_id,"binding":{"sandbox_uid":target.sandbox_uid,"pod_uid":target.pod_uid,"runtime_generation":target.runtime_generation,"credential_epoch":target.credential_epoch,"writer_fence":target.writer_fence},"pvc_uid":target.pvc_uid,"evidence_ref":event.event_id,"observed_at":event.observed_at_ms.div_euclid(1000),"stopped_at_ns":serde_json::Value::Null,"running_at_ns":event.running_at_ms.and_then(|ms|ms.checked_mul(1_000_000)),"state":"writer_admitted","credentials_revoked":false,"workspace_released":false,"persistent_volume_released":false,"accounting_closed":false});
        Ok(jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &self.key,
        )?)
    }
}

/// Bounded receiver core to mount in the selected existing provider component.
/// TLS, request scheduling and effect-source wiring belong to that component;
/// no listener or process is started by this library.
pub fn handle_control<S: DeadlineStore, C: Clock, A: RuntimeActuator>(
    engine: &DeadlineEngine<S, C, A>,
    issuer: &ReceiptIssuer,
    bytes: &[u8],
) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() <= 32 * 1024,
        "provider control payload too large"
    );
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        purpose: String,
        request: AuthenticatedDeadlineControl,
    }
    let input: Input = serde_json::from_slice(bytes)?;
    ensure!(
        input.purpose == "deadline_control",
        "provider control purpose invalid"
    );
    let effect = engine.apply(&input.request)?;
    let token = issuer.issue_installed(&effect, engine.clock.sample()?.utc_ms)?;
    Ok(serde_json::to_vec(&serde_json::json!({"token":token}))?)
}

pub struct DeadlineEngine<S, C, A> {
    pub store: S,
    pub clock: C,
    pub actuator: A,
    authentication_key: Zeroizing<Vec<u8>>,
    provider_instance_id: String,
    provider_boot_id: String,
}
impl<S: DeadlineStore, C: Clock, A: RuntimeActuator> DeadlineEngine<S, C, A> {
    pub fn writer_admission(
        &self,
        target: &RuntimeTarget,
    ) -> Result<Option<VerifiedWriterAdmission>> {
        let now = self.clock.sample()?;
        let record = self
            .store
            .load(&target.machine_id)?
            .ok_or_else(|| anyhow::anyhow!("runtime deadline not installed"))?;
        ensure!(
            record.control.target == *target
                && record.state == DeadlineState::Installed
                && !self.clock_unsafe(&record, &now),
            "runtime writer fenced"
        );
        ensure!(
            self.actuator.resolve(target)? == record.resolved,
            "resolved runtime changed"
        );
        let receipt = record
            .installed
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("runtime deadline not installed"))?;
        ensure!(
            receipt.installed_run_not_after_ms > now.utc_ms,
            "runtime deadline expired"
        );
        let Some(event) = self.actuator.writer_event(&record.resolved)? else {
            return Ok(None);
        };
        ensure!(
            event.target == record.resolved
                && uuid::Uuid::parse_str(&event.event_id).is_ok()
                && uuid::Uuid::parse_str(&event.predecessor_exclusion_event_id).is_ok()
                && event.observed_at_ms <= now.utc_ms
                && event.observed_at_ms >= now.utc_ms.saturating_sub(60_000)
                && event
                    .running_at_ms
                    .is_none_or(|at| at > 0 && at <= event.observed_at_ms),
            "runtime writer event invalid"
        );
        // Re-read after observation so a completed stop cannot issue a writer
        // token. The selected actuator separately fences stop-vs-I/O admission.
        ensure!(
            self.store
                .load(&target.machine_id)?
                .is_some_and(|current| current.store_version == record.store_version
                    && current.state == DeadlineState::Installed),
            "runtime writer observation raced"
        );
        // Resolving and observing the privileged runtime can block. A timer
        // may expire without any journal write while that observation runs.
        let observed_now = self.clock.sample()?;
        ensure!(
            !self.clock_unsafe(&record, &observed_now)
                && receipt.installed_run_not_after_ms > observed_now.utc_ms,
            "runtime writer observation exceeded installed deadline"
        );
        Ok(Some(VerifiedWriterAdmission { event }))
    }
    /// Re-observe retained terminal proof after delivery loss or process restart.
    /// A persisted tombstone alone is not converted into a fresh physical event.
    pub fn terminal_observation(
        &self,
        target: &RuntimeTarget,
    ) -> Result<Option<VerifiedTerminalRuntime>> {
        let now = self.clock.sample()?;
        let record = self
            .store
            .load(&target.machine_id)?
            .ok_or_else(|| anyhow::anyhow!("runtime deadline absent"))?;
        ensure!(
            record.control.target == *target && record.state == DeadlineState::Terminal,
            "runtime is not terminal"
        );
        let Some(event) = self.actuator.terminal_event(&record.resolved)? else {
            return Ok(None);
        };
        ensure!(
            event.target == record.resolved
                && record.terminal_event_id.as_deref() == Some(event.event_id.as_str())
                && event.stopped_at_ms == record.stopped_at_ms
                && event.observed_at_ms <= now.utc_ms
                && event.observed_at_ms >= now.utc_ms.saturating_sub(60_000),
            "terminal re-observation mismatch"
        );
        Ok(Some(VerifiedTerminalRuntime { event }))
    }
    pub fn new(
        store: S,
        clock: C,
        actuator: A,
        authentication_key: Zeroizing<Vec<u8>>,
        provider_instance_id: String,
        provider_boot_id: String,
    ) -> Result<Self> {
        ensure!(
            authentication_key.len() >= 32,
            "provider authentication configuration invalid"
        );
        ensure!(
            uuid::Uuid::parse_str(&provider_instance_id).is_ok()
                && uuid::Uuid::parse_str(&provider_boot_id).is_ok(),
            "provider identity invalid"
        );
        Ok(Self {
            store,
            clock,
            actuator,
            authentication_key,
            provider_instance_id,
            provider_boot_id,
        })
    }
    pub fn apply(
        &self,
        request: &AuthenticatedDeadlineControl,
    ) -> Result<VerifiedInstalledDeadline> {
        let now = self.clock.sample()?;
        request
            .verify(&self.authentication_key, now.utc_ms)
            .map_err(anyhow::Error::msg)?;
        let control = &request.control;
        let digest = control.digest().map_err(anyhow::Error::msg)?;
        let prior = self.store.load(&control.target.machine_id)?;
        // Resolve the actual attempt/boot/storage domain before every replay or
        // renewal; a cached Pod UID alone cannot authorize a replacement.
        let resolved = self.actuator.resolve(&control.target)?;
        resolved.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            resolved.target == control.target,
            "resolved runtime mismatch"
        );
        if let Some(record) = &prior
            && record.control.target == control.target
            && resolved != record.resolved
        {
            self.stop_record(record, &now)?;
            bail!("resolved runtime changed");
        }
        if let Some(record) = &prior {
            if record.state != DeadlineState::Terminal
                && (self.clock_unsafe(record, &now)
                    || record
                        .installed
                        .as_ref()
                        .map_or(record.control.run_not_after_ms, |r| {
                            r.installed_run_not_after_ms
                        })
                        <= now.utc_ms)
                && control.action != DeadlineAction::ShortenStop
            {
                self.stop_record(record, &now)?;
                bail!("runtime deadline expired or clock changed");
            }
            if record.digest == digest {
                ensure!(
                    !matches!(
                        record.state,
                        DeadlineState::StopRequested | DeadlineState::Terminal
                    ) || control.action == DeadlineAction::ShortenStop,
                    "runtime already fenced"
                );
                if let Some(receipt) = &record.installed
                    && receipt.request_digest == digest
                {
                    return Ok(VerifiedInstalledDeadline {
                        receipt: receipt.clone(),
                    });
                }
            } else {
                ensure!(
                    control.control_revision > record.control.control_revision,
                    "stale deadline revision"
                );
                ensure!(
                    control.target.owner_id == record.control.target.owner_id,
                    "runtime owner mismatch"
                );
                if control.target == record.control.target {
                    ensure!(
                        control.action == DeadlineAction::ShortenStop
                            || !matches!(
                                record.state,
                                DeadlineState::StopRequested | DeadlineState::Terminal
                            ),
                        "runtime already fenced"
                    );
                    ensure!(
                        control.action != DeadlineAction::Arm,
                        "runtime already armed"
                    );
                    if control.action == DeadlineAction::Renew {
                        ensure!(
                            control.run_not_after_ms >= record.control.run_not_after_ms,
                            "renewal cannot shorten a deadline"
                        );
                    }
                    if control.action == DeadlineAction::ShortenStop {
                        ensure!(
                            control.run_not_after_ms <= record.control.run_not_after_ms,
                            "stop cannot extend a deadline"
                        );
                    }
                } else {
                    ensure!(
                        record.state == DeadlineState::Terminal
                            && control.action == DeadlineAction::Arm
                            && control.target.runtime_generation
                                > record.control.target.runtime_generation
                            && control.target.credential_epoch
                                > record.control.target.credential_epoch
                            && control.target.pvc_uid == record.control.target.pvc_uid,
                        "predecessor exclusion required"
                    );
                }
            }
        } else {
            ensure!(
                control.action != DeadlineAction::Renew,
                "runtime must be armed first"
            );
        }
        ensure!(
            control.action == DeadlineAction::ShortenStop || control.run_not_after_ms > now.utc_ms,
            "deadline already expired"
        );
        // Bound pre-install responsibility. Product run length is unlimited;
        // each safety/funding renewal is finite and authenticated.
        ensure!(
            control.run_not_after_ms <= now.utc_ms.saturating_add(600_000),
            "deadline window too large"
        );
        let remaining_ms = control
            .run_not_after_ms
            .checked_sub(now.utc_ms)
            .ok_or_else(|| anyhow::anyhow!("deadline clock difference overflow"))?
            .max(0) as u64;
        let sampled_deadline_ms = now
            .monotonic_ms
            .checked_add(remaining_ms)
            .ok_or_else(|| anyhow::anyhow!("deadline clock overflow"))?;
        let requested_monotonic_deadline_ms = if let Some(previous) = prior
            .as_ref()
            .filter(|p| p.control.target == control.target)
        {
            // A new revision authorizes only its explicit absolute cutoff
            // delta. Re-sampling a stalled UTC clock would otherwise grant
            // fresh elapsed time even for an unchanged cutoff. Forward UTC
            // movement may shorten this retained bound, never lengthen it.
            let delta = i128::from(control.run_not_after_ms)
                - i128::from(previous.control.run_not_after_ms);
            let translated =
                if delta >= 0 {
                    previous
                        .requested_monotonic_deadline_ms
                        .checked_add(u64::try_from(delta).map_err(|_| {
                            anyhow::anyhow!("deadline revision difference overflow")
                        })?)
                        .ok_or_else(|| anyhow::anyhow!("deadline revision clock overflow"))?
                } else {
                    previous.requested_monotonic_deadline_ms.saturating_sub(
                        u64::try_from(-delta)
                            .map_err(|_| anyhow::anyhow!("deadline shortening overflow"))?,
                    )
                };
            translated.min(sampled_deadline_ms)
        } else {
            sampled_deadline_ms
        };
        let mut next = DeadlineRecord {
            store_version: prior.as_ref().map_or(1, |p| p.store_version + 1),
            control: control.clone(),
            digest: digest.clone(),
            resolved: resolved.clone(),
            state: if prior.as_ref().is_some_and(|p| {
                p.state == DeadlineState::Terminal && p.control.target == control.target
            }) {
                DeadlineState::Terminal
            } else if control.action == DeadlineAction::ShortenStop {
                DeadlineState::StopRequested
            } else {
                DeadlineState::Requested
            },
            installed: prior
                .as_ref()
                .filter(|p| p.control.target == control.target)
                .and_then(|p| p.installed.clone()),
            last_utc_ms: now.utc_ms,
            last_monotonic_ms: now.monotonic_ms,
            clock_boot_id: now.boot_id.clone(),
            requested_monotonic_deadline_ms,
            monotonic_deadline_ms: prior
                .as_ref()
                .filter(|p| p.control.target == control.target)
                .map_or(requested_monotonic_deadline_ms, |p| p.monotonic_deadline_ms),
            terminal_event_id: prior
                .as_ref()
                .filter(|p| p.control.target == control.target)
                .and_then(|p| p.terminal_event_id.clone()),
            stopped_at_ms: prior
                .as_ref()
                .filter(|p| p.control.target == control.target)
                .and_then(|p| p.stopped_at_ms),
            stop_observed_at_ms: prior
                .as_ref()
                .filter(|p| p.control.target == control.target)
                .and_then(|p| p.stop_observed_at_ms),
        };
        ensure!(
            self.store.compare_exchange(
                &control.target.machine_id,
                prior.as_ref().map(|p| p.store_version),
                &next
            )?,
            "deadline journal raced"
        );
        let event = self.actuator.install(&resolved, control)?;
        ensure!(
            event.target == resolved && event.revision == control.control_revision,
            "timer event mismatch"
        );
        let receipt = DeadlineReceipt {
            version: DEADLINE_PROTOCOL_VERSION,
            request_digest: next.digest.clone(),
            control_revision: control.control_revision,
            resolved_target: resolved,
            provider_instance_id: self.provider_instance_id.clone(),
            provider_boot_id: self.provider_boot_id.clone(),
            installed_run_not_after_ms: event.cutoff_ms,
            guaranteed_stop_not_after_ms: event.guaranteed_stop_ms,
            mechanism_version: event.mechanism_version,
            evidence_ref: event.event_id,
            stop_requested: control.action == DeadlineAction::ShortenStop,
        };
        receipt.validate_for(control).map_err(anyhow::Error::msg)?;
        let prior_version = next.store_version;
        next.store_version += 1;
        next.installed = Some(receipt.clone());
        // Installation may acknowledge less coverage than requested. Derive
        // its elapsed deadline from the original request baseline so neither
        // delayed installation nor replay can lengthen the installed window.
        let uninstalled_ms = control
            .run_not_after_ms
            .checked_sub(receipt.installed_run_not_after_ms)
            .and_then(|duration| u64::try_from(duration).ok())
            .ok_or_else(|| anyhow::anyhow!("installed deadline exceeds request"))?;
        next.monotonic_deadline_ms = next
            .requested_monotonic_deadline_ms
            .saturating_sub(uninstalled_ms);
        if control.action != DeadlineAction::ShortenStop {
            next.state = DeadlineState::Installed;
        }
        ensure!(
            self.store
                .compare_exchange(&control.target.machine_id, Some(prior_version), &next)?,
            "deadline receipt raced"
        );
        if control.action == DeadlineAction::ShortenStop {
            self.actuator
                .request_stop(&next.resolved, control.control_revision)?;
        }
        Ok(VerifiedInstalledDeadline { receipt })
    }
    fn clock_unsafe(&self, prior: &DeadlineRecord, now: &ClockSample) -> bool {
        // Any boot ambiguity or rollback fences before replacement. A fresh
        // wall clock never extends a previously installed elapsed deadline.
        prior.clock_boot_id != now.boot_id
            || prior
                .installed
                .as_ref()
                .is_some_and(|r| r.provider_boot_id != self.provider_boot_id)
            || now.utc_ms < prior.last_utc_ms
            || now.monotonic_ms < prior.last_monotonic_ms
            || now.monotonic_ms >= prior.monotonic_deadline_ms
    }
    fn stop_record(&self, record: &DeadlineRecord, now: &ClockSample) -> Result<()> {
        if record.state == DeadlineState::Terminal {
            return Ok(());
        }
        let mut next = record.clone();
        next.store_version += 1;
        next.state = DeadlineState::StopRequested;
        next.last_utc_ms = next.last_utc_ms.max(now.utc_ms);
        if !self.store.compare_exchange(
            &record.control.target.machine_id,
            Some(record.store_version),
            &next,
        )? {
            bail!("deadline stop raced");
        }
        self.actuator
            .request_stop(&record.resolved, record.control.control_revision)
    }
    /// Caller runs this inside its existing privileged enforcement loop. Boot
    /// recovery must finish before allowing target restart/reacquisition.
    pub fn recover_batch(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<VerifiedTerminalRuntime>> {
        let now = self.clock.sample()?;
        let mut terminal = Vec::new();
        for mut record in self.store.list(after, limit.clamp(1, 128))? {
            if record.state == DeadlineState::Terminal {
                continue;
            }
            if self.clock_unsafe(&record, &now)
                || record
                    .installed
                    .as_ref()
                    .map_or(record.control.run_not_after_ms, |r| {
                        r.installed_run_not_after_ms
                    })
                    <= now.utc_ms
                || record.state == DeadlineState::StopRequested
            {
                self.stop_record(&record, &now)?;
                record = self
                    .store
                    .load(&record.control.target.machine_id)?
                    .ok_or_else(|| anyhow::anyhow!("deadline journal missing"))?;
                if let Some(event) = self.actuator.terminal_event(&record.resolved)? {
                    ensure!(
                        event.target == record.resolved
                            && uuid::Uuid::parse_str(&event.event_id).is_ok()
                            && event.observed_at_ms <= now.utc_ms
                            && event
                                .stopped_at_ms
                                .is_none_or(|t| t > 0 && t <= event.observed_at_ms),
                        "terminal event mismatch"
                    );
                    let version = record.store_version;
                    record.store_version += 1;
                    record.state = DeadlineState::Terminal;
                    record.terminal_event_id = Some(event.event_id.clone());
                    record.stopped_at_ms = event.stopped_at_ms;
                    record.stop_observed_at_ms = Some(event.observed_at_ms);
                    ensure!(
                        self.store.compare_exchange(
                            &record.control.target.machine_id,
                            Some(version),
                            &record
                        )?,
                        "terminal journal raced"
                    );
                    terminal.push(VerifiedTerminalRuntime { event });
                }
            } else {
                let version = record.store_version;
                record.store_version += 1;
                record.last_utc_ms = now.utc_ms;
                record.last_monotonic_ms = now.monotonic_ms;
                ensure!(
                    self.store.compare_exchange(
                        &record.control.target.machine_id,
                        Some(version),
                        &record
                    )?,
                    "deadline clock observation raced"
                );
            }
        }
        Ok(terminal)
    }
}

/// Atomic fsync journal for a selected provider's protected local state. This
/// preserves replay tombstones; it does not claim host-loss storage durability.
pub struct FileDeadlineStore {
    root: PathBuf,
}
impl FileDeadlineStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        let meta = std::fs::symlink_metadata(&root)?;
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "deadline store must be a protected directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { root })
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        ensure!(
            uuid::Uuid::parse_str(id).is_ok(),
            "invalid deadline identity"
        );
        Ok(self.root.join(format!("{id}.json")))
    }
    fn lock(&self) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".lock"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            ensure!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0,
                "deadline journal lock unavailable"
            );
        }
        Ok(file)
    }
    fn read(path: &Path) -> Result<Option<DeadlineRecord>> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 64 * 1024,
            "deadline journal invalid"
        );
        Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
    }
}
impl DeadlineStore for FileDeadlineStore {
    fn load(&self, machine_id: &str) -> Result<Option<DeadlineRecord>> {
        let _lock = self.lock()?;
        Self::read(&self.path(machine_id)?)
    }
    fn compare_exchange(
        &self,
        machine_id: &str,
        expected: Option<u64>,
        next: &DeadlineRecord,
    ) -> Result<bool> {
        let _lock = self.lock()?;
        let path = self.path(machine_id)?;
        if Self::read(&path)?.as_ref().map(|r| r.store_version) != expected {
            return Ok(false);
        }
        ensure!(
            next.control.target.machine_id == machine_id
                && next.store_version == expected.unwrap_or(0) + 1,
            "invalid deadline journal transition"
        );
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(&serde_json::to_vec(next)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path)?;
        File::open(&self.root)?.sync_all()?;
        Ok(true)
    }
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<DeadlineRecord>> {
        let _lock = self.lock()?;
        let mut names = std::collections::BTreeSet::new();
        for (scanned, entry) in std::fs::read_dir(&self.root)?.enumerate() {
            ensure!(
                scanned < 100_000,
                "deadline store inventory exceeds bounded local capacity"
            );
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if uuid::Uuid::parse_str(id).is_ok() && after.is_none_or(|a| id > a) {
                names.insert(id.to_owned());
                if names.len() > limit.clamp(1, 128) {
                    names.pop_last();
                }
            }
        }
        names
            .into_iter()
            .map(|id| {
                Self::read(&self.path(&id)?)?
                    .ok_or_else(|| anyhow::anyhow!("deadline journal disappeared"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct TestClock(Arc<Mutex<ClockSample>>);
    impl Clock for TestClock {
        fn sample(&self) -> Result<ClockSample> {
            Ok(self.0.lock().unwrap().clone())
        }
    }
    #[derive(Clone, Default)]
    struct Actuator {
        calls: Arc<Mutex<Vec<&'static str>>>,
        terminal: Arc<Mutex<Option<TerminalRuntimeEvent>>>,
        resolved: Arc<Mutex<std::collections::HashMap<String, ResolvedRuntimeTarget>>>,
        installed_cutoff_ms: Arc<Mutex<Option<i64>>>,
        writer_clock: Arc<Mutex<Option<TestClock>>>,
    }
    impl RuntimeActuator for Actuator {
        fn resolve(&self, target: &RuntimeTarget) -> Result<ResolvedRuntimeTarget> {
            let mut resolved = self.resolved.lock().unwrap();
            let key = format!("{}:{}", target.machine_id, target.runtime_generation);
            Ok(resolved
                .entry(key)
                .or_insert_with(|| ResolvedRuntimeTarget {
                    target: target.clone(),
                    node_uid: uuid::Uuid::new_v4().to_string(),
                    node_boot_id: uuid::Uuid::new_v4().to_string(),
                    runtime_instance_digest: "a".repeat(64),
                    runtime_attempt: 1,
                    pv_uid: uuid::Uuid::new_v4().to_string(),
                    csi_driver: "test.local".into(),
                    volume_handle_digest: "b".repeat(64),
                    writer_domain_digest: "c".repeat(64),
                })
                .clone())
        }
        fn install(
            &self,
            target: &ResolvedRuntimeTarget,
            control: &DeadlineControl,
        ) -> Result<InstalledTimerEvent> {
            self.calls.lock().unwrap().push("install");
            let cutoff_ms = self
                .installed_cutoff_ms
                .lock()
                .unwrap()
                .unwrap_or(control.run_not_after_ms);
            Ok(InstalledTimerEvent {
                target: target.clone(),
                revision: control.control_revision,
                cutoff_ms,
                guaranteed_stop_ms: cutoff_ms + 1000,
                mechanism_version: "test-v1".into(),
                event_id: uuid::Uuid::new_v4().to_string(),
            })
        }
        fn request_stop(&self, _: &ResolvedRuntimeTarget, _: u64) -> Result<()> {
            self.calls.lock().unwrap().push("stop");
            Ok(())
        }
        fn terminal_event(
            &self,
            _: &ResolvedRuntimeTarget,
        ) -> Result<Option<TerminalRuntimeEvent>> {
            Ok(self.terminal.lock().unwrap().clone())
        }
        fn writer_event(
            &self,
            target: &ResolvedRuntimeTarget,
        ) -> Result<Option<WriterAdmissionEvent>> {
            let Some(clock) = self.writer_clock.lock().unwrap().clone() else {
                return Ok(None);
            };
            // Model an observation blocking across the elapsed cutoff while
            // wall time and journal version stay unchanged.
            let mut now = clock.0.lock().unwrap();
            now.monotonic_ms += 25_000;
            Ok(Some(WriterAdmissionEvent {
                target: target.clone(),
                event_id: uuid::Uuid::new_v4().to_string(),
                observed_at_ms: now.utc_ms,
                running_at_ms: Some(now.utc_ms),
                predecessor_exclusion_event_id: uuid::Uuid::new_v4().to_string(),
            }))
        }
    }
    fn control() -> DeadlineControl {
        DeadlineControl {
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
        }
    }
    fn engine(dir: &Path) -> DeadlineEngine<FileDeadlineStore, TestClock, Actuator> {
        DeadlineEngine::new(
            FileDeadlineStore::open(dir).unwrap(),
            TestClock(Arc::new(Mutex::new(ClockSample {
                utc_ms: 100_000,
                monotonic_ms: 100,
                boot_id: uuid::Uuid::new_v4().to_string(),
            }))),
            Actuator::default(),
            Zeroizing::new(vec![7; 32]),
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
    }
    fn apply(
        engine: &DeadlineEngine<FileDeadlineStore, TestClock, Actuator>,
        control: DeadlineControl,
    ) -> Result<VerifiedInstalledDeadline> {
        engine.apply(
            &AuthenticatedDeadlineControl::sign(control, &[7; 32], engine.clock.sample()?.utc_ms)
                .unwrap(),
        )
    }
    #[test]
    fn authenticated_install_lost_ack_replay_and_revision_are_durable() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let c = control();
        let receipt = apply(&e, c.clone()).unwrap().receipt.clone();
        assert_eq!(apply(&e, c.clone()).unwrap().receipt, receipt);
        assert_eq!(e.actuator.calls.lock().unwrap().as_slice(), ["install"]);
        let mut conflicting = c.clone();
        conflicting.run_not_after_ms += 1;
        assert!(apply(&e, conflicting).is_err());
        let loaded = FileDeadlineStore::open(dir.path())
            .unwrap()
            .load(&c.target.machine_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.installed, Some(receipt));
        let mut forged = AuthenticatedDeadlineControl::sign(c, &[8; 32], 100_000).unwrap();
        assert!(e.apply(&forged).is_err());
        forged.authentication = "bad".into();
        assert!(e.apply(&forged).is_err());
    }
    #[test]
    fn renewal_stale_timer_and_stop_win_without_claiming_physical_stop() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let mut c = control();
        apply(&e, c.clone()).unwrap();
        c.action = DeadlineAction::Renew;
        c.control_revision = 2;
        c.run_not_after_ms = 140_000;
        apply(&e, c.clone()).unwrap();
        e.clock.0.lock().unwrap().utc_ms = 125_000;
        assert!(e.recover_batch(None, 10).unwrap().is_empty());
        assert_eq!(e.actuator.calls.lock().unwrap().len(), 2);
        c.action = DeadlineAction::ShortenStop;
        c.control_revision = 3;
        c.run_not_after_ms = 125_000;
        apply(&e, c.clone()).unwrap();
        let state = e.store.load(&c.target.machine_id).unwrap().unwrap();
        assert_eq!(state.state, DeadlineState::StopRequested);
        assert!(state.terminal_event_id.is_none());
        c.action = DeadlineAction::Renew;
        c.control_revision = 4;
        c.run_not_after_ms = 150_000;
        assert!(apply(&e, c).is_err());
    }
    #[test]
    fn restart_rollback_and_elapsed_partition_fence_before_renewal() {
        for mode in ["boot", "rollback", "elapsed", "expiry"] {
            let dir = tempfile::tempdir().unwrap();
            let e = engine(dir.path());
            let mut c = control();
            apply(&e, c.clone()).unwrap();
            {
                let mut now = e.clock.0.lock().unwrap();
                match mode {
                    "boot" => now.boot_id = uuid::Uuid::new_v4().to_string(),
                    "rollback" => now.utc_ms -= 1,
                    "elapsed" => now.monotonic_ms += 25_000,
                    _ => now.utc_ms = 120_001,
                }
            }
            e.recover_batch(None, 10).unwrap();
            assert_eq!(
                e.store.load(&c.target.machine_id).unwrap().unwrap().state,
                DeadlineState::StopRequested
            );
            assert!(e.actuator.calls.lock().unwrap().contains(&"stop"));
            c.action = DeadlineAction::Renew;
            c.control_revision += 1;
            c.run_not_after_ms = 180_000;
            assert!(apply(&e, c).is_err());
        }
    }
    #[test]
    fn terminal_requires_retained_event_and_new_target_requires_predecessor_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let c = control();
        apply(&e, c.clone()).unwrap();
        let mut next = c.clone();
        next.control_revision = 2;
        next.target.runtime_generation = 2;
        next.target.credential_epoch = 2;
        next.target.pod_uid = uuid::Uuid::new_v4().to_string();
        assert!(apply(&e, next.clone()).is_err());
        e.clock.0.lock().unwrap().utc_ms = 120_001;
        e.recover_batch(None, 10).unwrap();
        let record = e.store.load(&c.target.machine_id).unwrap().unwrap();
        *e.actuator.terminal.lock().unwrap() = Some(TerminalRuntimeEvent {
            target: record.resolved,
            event_id: uuid::Uuid::new_v4().to_string(),
            stopped_at_ms: Some(120_000),
            observed_at_ms: 120_001,
        });
        assert_eq!(e.recover_batch(None, 10).unwrap().len(), 1);
        next.run_not_after_ms = 180_000;
        assert!(apply(&e, next).is_ok());
    }
    #[test]
    fn repeated_observation_does_not_extend_stalled_wall_clock_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let c = control();
        apply(&e, c.clone()).unwrap();
        for _ in 0..21 {
            e.clock.0.lock().unwrap().monotonic_ms += 1000;
            e.recover_batch(None, 10).unwrap();
        }
        assert_eq!(
            e.store.load(&c.target.machine_id).unwrap().unwrap().state,
            DeadlineState::StopRequested
        );
        assert!(apply(&e, c).is_err());
    }
    #[test]
    fn shorter_installed_coverage_fences_elapsed_time_with_stalled_wall_and_replay() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let c = control();
        *e.actuator.installed_cutoff_ms.lock().unwrap() = Some(110_000);
        let receipt = apply(&e, c.clone()).unwrap().receipt;
        let record = e.store.load(&c.target.machine_id).unwrap().unwrap();
        assert_eq!(record.requested_monotonic_deadline_ms, 20_100);
        assert_eq!(record.monotonic_deadline_ms, 10_100);

        // A lost ACK replay has the same absolute elapsed deadline, even
        // though UTC has stalled and several elapsed seconds have passed.
        e.clock.0.lock().unwrap().monotonic_ms = 5_100;
        assert_eq!(apply(&e, c.clone()).unwrap().receipt, receipt);
        assert!(e.writer_admission(&c.target).is_ok());
        assert_eq!(e.actuator.calls.lock().unwrap().as_slice(), ["install"]);
        e.clock.0.lock().unwrap().monotonic_ms = 10_100;
        assert!(e.writer_admission(&c.target).is_err());
        assert!(e.recover_batch(None, 10).unwrap().is_empty());
        let record = e.store.load(&c.target.machine_id).unwrap().unwrap();
        assert_eq!(record.state, DeadlineState::StopRequested);
        assert_eq!(record.monotonic_deadline_ms, 10_100);
        assert!(e.actuator.calls.lock().unwrap().contains(&"stop"));
        assert!(apply(&e, c).is_err());
    }
    #[test]
    fn renewed_revision_extends_elapsed_deadline_only_by_explicit_cutoff_delta() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let mut c = control();
        apply(&e, c.clone()).unwrap();
        e.clock.0.lock().unwrap().monotonic_ms = 5_100;
        c.action = DeadlineAction::Renew;
        c.control_revision = 2;
        apply(&e, c.clone()).unwrap();
        let record = e.store.load(&c.target.machine_id).unwrap().unwrap();
        assert_eq!(record.requested_monotonic_deadline_ms, 20_100);
        assert_eq!(record.monotonic_deadline_ms, 20_100);

        e.clock.0.lock().unwrap().monotonic_ms = 10_100;
        c.control_revision = 3;
        c.run_not_after_ms += 10_000;
        apply(&e, c.clone()).unwrap();
        let record = e.store.load(&c.target.machine_id).unwrap().unwrap();
        assert_eq!(record.requested_monotonic_deadline_ms, 30_100);
        assert_eq!(record.monotonic_deadline_ms, 30_100);

        // Forward wall movement narrows the retained mapping. It is never
        // ignored in favor of a longer elapsed deadline from an older sample.
        {
            let mut now = e.clock.0.lock().unwrap();
            now.utc_ms = 120_000;
            now.monotonic_ms = 15_100;
        }
        c.control_revision = 4;
        c.run_not_after_ms += 10_000;
        apply(&e, c.clone()).unwrap();
        let record = e.store.load(&c.target.machine_id).unwrap().unwrap();
        assert_eq!(record.requested_monotonic_deadline_ms, 35_100);
        assert_eq!(record.monotonic_deadline_ms, 35_100);
        e.clock.0.lock().unwrap().monotonic_ms = 35_100;
        assert!(e.writer_admission(&c.target).is_err());
        assert!(e.recover_batch(None, 10).unwrap().is_empty());
        c.control_revision = 5;
        c.run_not_after_ms += 10_000;
        assert!(apply(&e, c).is_err());
    }
    #[test]
    fn blocking_writer_observation_cannot_outlive_installed_elapsed_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let c = control();
        apply(&e, c.clone()).unwrap();
        let original = e.store.load(&c.target.machine_id).unwrap().unwrap();
        *e.actuator.writer_clock.lock().unwrap() = Some(e.clock.clone());
        assert!(e.writer_admission(&c.target).is_err());
        let retained = e.store.load(&c.target.machine_id).unwrap().unwrap();
        assert_eq!(retained.store_version, original.store_version);
        assert_eq!(e.clock.sample().unwrap().utc_ms, 100_000);
        assert!(e.clock.sample().unwrap().monotonic_ms > retained.monotonic_deadline_ms);
    }
    #[test]
    fn replay_and_writer_observation_reject_replaced_runtime_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path());
        let c = control();
        apply(&e, c.clone()).unwrap();
        let key = format!("{}:{}", c.target.machine_id, c.target.runtime_generation);
        e.actuator
            .resolved
            .lock()
            .unwrap()
            .get_mut(&key)
            .unwrap()
            .runtime_attempt += 1;
        assert!(e.writer_admission(&c.target).is_err());
        assert!(apply(&e, c.clone()).is_err());
        assert_eq!(
            e.store.load(&c.target.machine_id).unwrap().unwrap().state,
            DeadlineState::StopRequested
        );
    }
    #[test]
    fn new_engine_recovers_retained_store_before_same_target_renewal() {
        let directory = tempfile::tempdir().unwrap();
        let original = engine(directory.path());
        let mut control = control();
        apply(&original, control.clone()).unwrap();
        let recovered = DeadlineEngine::new(
            FileDeadlineStore::open(directory.path()).unwrap(),
            original.clock.clone(),
            original.actuator.clone(),
            Zeroizing::new(vec![7; 32]),
            original.provider_instance_id.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        assert!(recovered.recover_batch(None, 10).unwrap().is_empty());
        let record = recovered
            .store
            .load(&control.target.machine_id)
            .unwrap()
            .unwrap();
        assert_eq!(record.state, DeadlineState::StopRequested);
        assert!(record.terminal_event_id.is_none());
        assert!(recovered.actuator.calls.lock().unwrap().contains(&"stop"));
        control.control_revision += 1;
        control.action = DeadlineAction::Renew;
        control.run_not_after_ms += 60_000;
        assert!(apply(&recovered, control).is_err());
    }
    #[test]
    fn concrete_host_clock_has_bounded_monotonic_identity() {
        let first = HostClock.sample().unwrap();
        let second = HostClock.sample().unwrap();
        assert!(first.utc_ms > 0);
        assert_eq!(first.boot_id, second.boot_id);
        assert!(second.monotonic_ms >= first.monotonic_ms);
    }
}
