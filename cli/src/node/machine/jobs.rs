use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use nyxid_machine::text::{OutputRing, Redactor};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{Mutex, Notify, Semaphore, watch},
};
use zeroize::Zeroizing;

use super::{
    files::Roots,
    process::{Identity, pin_cwd, request_env},
};

#[derive(Deserialize)]
pub struct Exec {
    #[serde(flatten)]
    pub scope: super::cancellation::Scope,
    pub job_id: String,
    pub command: String,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub stdin: Option<String>,
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub background: bool,
}

struct State {
    stdout: OutputRing,
    stderr: OutputRing,
    exit_code: Option<i32>,
    finished: Option<Instant>,
    started: Instant,
    timed_out: bool,
    stop_reason: Option<&'static str>,
    output_unavailable: Option<&'static str>,
}

pub struct Job {
    scope: Option<super::cancellation::Scope>,
    external: Option<nyxid_machine::authority::ExternalAuthority>,
    // Selected by the supervisor, never supplied by the command request.
    uid: u32,
    state: Mutex<State>,
    cancel: watch::Sender<bool>,
    changed: Notify,
    pid: i32,
    running: AtomicBool,
}

pub struct Jobs {
    redactor: Arc<Mutex<Redactor>>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    permits: Arc<Semaphore>,
    limit_secs: u64,
    output_bytes: usize,
}

impl Jobs {
    pub async fn register_secret(&self, value: &str) -> Result<()> {
        self.redactor
            .lock()
            .await
            .register(value)
            .map_err(anyhow::Error::msg)
    }
    pub fn new(config: &nyxid_machine::config::Config, redactor: Arc<Mutex<Redactor>>) -> Self {
        Self {
            redactor,
            jobs: Mutex::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(config.max_jobs)),
            limit_secs: config.max_timeout_secs,
            output_bytes: config.output_bytes,
        }
    }

    pub async fn start(
        &self,
        request: Exec,
        identity: &Identity,
        roots: &Roots,
        gateway_env: &BTreeMap<String, String>,
    ) -> Result<Value> {
        self.start_inner(request, identity, roots, gateway_env, None)
            .await
    }

    pub async fn start_external(
        &self,
        request: Exec,
        identity: &Identity,
        roots: &Roots,
        gateway_env: &BTreeMap<String, String>,
        authority: &nyxid_machine::authority::ExternalAuthority,
    ) -> Result<Value> {
        self.start_inner(
            request,
            identity,
            roots,
            gateway_env,
            Some(authority.clone()),
        )
        .await
    }

    async fn start_inner(
        &self,
        request: Exec,
        identity: &Identity,
        roots: &Roots,
        gateway_env: &BTreeMap<String, String>,
        external: Option<nyxid_machine::authority::ExternalAuthority>,
    ) -> Result<Value> {
        if uuid::Uuid::parse_str(&request.job_id).is_err()
            || request.command.is_empty()
            || request.command.len() > 32768
            || request.stdin.as_ref().is_some_and(|v| v.len() > 65536)
        {
            bail!("invalid command or job id");
        }
        let timeout = request.timeout_secs.unwrap_or(120);
        if timeout == 0 || timeout > self.limit_secs {
            bail!("command timeout exceeds node limit");
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .context("maximum concurrent jobs reached")?;
        let mut command = Command::new("/bin/sh");
        identity.prepare_agent(&mut command)?;
        request_env(&mut command, &request.env)?;
        command.envs(gateway_env);
        pin_cwd(&mut command, roots.cwd(request.cwd.as_deref())?);
        command
            .args(["-lc", &request.command])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(if request.stdin.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            });
        let mut jobs = self.jobs.lock().await;
        let mut finished = Vec::new();
        let mut retained_bytes = 0usize;
        for (id, job) in jobs.iter() {
            let mut state = job.state.lock().await;
            expire_output(&mut state);
            if state.output_unavailable.is_none() {
                retained_bytes = retained_bytes.saturating_add(self.output_bytes);
            }
            if let Some(at) = state.finished {
                finished.push((at, id.clone(), job.clone()));
            }
        }
        finished.sort_by_key(|(at, _, _)| *at);
        for (_, id, job) in &finished {
            if retained_bytes.saturating_add(self.output_bytes) > MAX_OUTPUT_CACHE_BYTES {
                let mut state = job.state.lock().await;
                if state.output_unavailable.is_none() {
                    discard_output(&mut state, "capacity_evicted");
                    retained_bytes = retained_bytes.saturating_sub(self.output_bytes);
                }
            }
            if jobs.len() >= 1024 {
                jobs.remove(id);
            }
        }
        if retained_bytes.saturating_add(self.output_bytes) > MAX_OUTPUT_CACHE_BYTES
            || jobs.len() >= 1024
            || jobs.contains_key(&request.job_id)
        {
            bail!("job retention limit reached or duplicate job id");
        }
        // The request's bounded temporary environment is known before any
        // process effect. Protect its non-empty values in the existing session
        // registry before capture starts; exhaustion must refuse admission.
        // The registry remains live for completed-job and paginated reads too.
        {
            let mut redactor = self.redactor.lock().await;
            for value in request.env.values().filter(|value| !value.is_empty()) {
                redactor.register(value).map_err(anyhow::Error::msg)?;
            }
        }
        let mut child = command.spawn().context("command could not start")?;
        let pid = child.id().context("command process unavailable")? as i32;
        let stdout = child.stdout.take().context("stdout unavailable")?;
        let stderr = child.stderr.take().context("stderr unavailable")?;
        let input = request.stdin.map(Zeroizing::new);
        let mut stdin = child.stdin.take();
        let (cancel, mut cancelled) = watch::channel(false);
        let job = Arc::new(Job {
            scope: external.is_none().then_some(request.scope),
            external,
            uid: identity.uid,
            state: Mutex::new(State {
                stdout: OutputRing::with_head(self.output_bytes / 2),
                stderr: OutputRing::with_head(self.output_bytes / 2),
                exit_code: None,
                finished: None,
                started: Instant::now(),
                timed_out: false,
                stop_reason: None,
                output_unavailable: None,
            }),
            cancel,
            changed: Notify::new(),
            pid,
            running: AtomicBool::new(true),
        });
        jobs.insert(request.job_id.clone(), job.clone());
        drop(jobs);
        let active = job.clone();
        let redactor = self.redactor.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut out =
                tokio::spawn(read_output(stdout, active.clone(), false, redactor.clone()));
            let mut err = tokio::spawn(read_output(stderr, active.clone(), true, redactor));
            let input = tokio::spawn(async move {
                if let (Some(mut stdin), Some(input)) = (stdin.take(), input) {
                    let _ = stdin.write_all(input.as_bytes()).await;
                    let _ = stdin.shutdown().await;
                }
            });
            let mut timed_out = false;
            let status = tokio::select! {
                status = child.wait() => status,
                _ = cancelled.changed() => terminate(&mut child, pid).await,
                _ = tokio::time::sleep(Duration::from_secs(timeout)) => { timed_out = true; terminate(&mut child, pid).await },
            };
            // A shell may exit leaving descendants holding output pipes. Every
            // job owns the whole group, including on normal shell completion.
            signal_group(pid, libc::SIGKILL);
            active.running.store(false, Ordering::Release);
            input.abort();
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let _ = (&mut out).await;
                let _ = (&mut err).await;
            })
            .await;
            out.abort();
            err.abort();
            let mut state = active.state.lock().await;
            state.exit_code = Some(status.ok().and_then(|s| s.code()).unwrap_or(-1));
            state.finished = Some(Instant::now());
            state.timed_out = timed_out;
            state.stop_reason = Some(if timed_out {
                "timeout"
            } else if *cancelled.borrow() {
                "cancelled"
            } else if state.exit_code == Some(-1) {
                "signal"
            } else {
                "completed"
            });
            drop(state);
            active.changed.notify_waiters();
        });
        if request.background {
            return Ok(json!({"job_id":request.job_id}));
        }
        self.foreground_result(&request.job_id, timeout + 5).await
    }

    pub async fn cancel(&self, id: &str) -> Result<Value> {
        let job = self
            .jobs
            .lock()
            .await
            .get(id)
            .cloned()
            .context(super::MachineError::JobNotFound)?;
        let _ = job.cancel.send(true);
        Ok(json!({"job_id":id,"cancel_requested":true}))
    }

    /// Emergency takeover: signal this runtime's process groups immediately. Do not wait
    /// for output readers, exit status collection or graceful termination.
    pub async fn preempt_identity(&self, uid: u32) {
        let jobs: Vec<_> = self
            .jobs
            .lock()
            .await
            .values()
            .filter(|job| job.uid == uid)
            .cloned()
            .collect();
        for job in jobs {
            if job.running.load(Ordering::Acquire) {
                signal_group(job.pid, libc::SIGKILL);
                job.cancel.send_replace(true);
            }
        }
    }

    pub async fn preempt_scope(&self, scope: Option<&super::cancellation::Scope>) -> Vec<String> {
        let jobs = self.jobs.lock().await;
        let mut ids = Vec::new();
        for (id, job) in jobs.iter() {
            if scope.is_none_or(|s| {
                job.scope.as_ref().is_some_and(|job_scope| {
                    s.conversation_id == job_scope.conversation_id
                        && (s.turn_id.is_empty() || s.turn_id == job_scope.turn_id)
                })
            }) {
                if job.running.load(Ordering::Acquire) {
                    signal_group(job.pid, libc::SIGKILL);
                }
                job.cancel.send_replace(true);
                ids.push(id.clone());
            }
        }
        ids
    }

    pub async fn preempt_ids(&self, ids: &[String]) {
        let jobs = self.jobs.lock().await;
        for id in ids {
            if let Some(job) = jobs.get(id) {
                if job.running.load(Ordering::Acquire) {
                    signal_group(job.pid, libc::SIGKILL);
                }
                job.cancel.send_replace(true);
            }
        }
    }
    pub async fn cancel_all(&self) {
        let jobs: Vec<_> = self.jobs.lock().await.values().cloned().collect();
        for job in &jobs {
            let _ = job.cancel.send(true);
        }
        for job in jobs {
            loop {
                let notified = job.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if job.state.lock().await.finished.is_some() {
                    break;
                }
                notified.await;
            }
        }
    }

    /// Bounded terminal metadata only. Safe to persist after output expires.
    pub async fn terminal_metadata(&self, id: &str) -> Option<Value> {
        let job = self.jobs.lock().await.get(id).cloned()?;
        let state = job.state.lock().await;
        let finished = state.finished?;
        Some(json!({
            "exit_code":state.exit_code,
            "timed_out":state.timed_out,
            "stop_reason":state.stop_reason,
            "duration_ms":finished.duration_since(state.started).as_millis().min(u64::MAX as u128) as u64,
        }))
    }

    pub async fn any_running(&self) -> bool {
        self.jobs
            .lock()
            .await
            .values()
            .any(|job| job.running.load(Ordering::Acquire))
    }

    pub async fn running(&self, id: &str) -> bool {
        let job = self.jobs.lock().await.get(id).cloned();
        match job {
            Some(job) => job.running.load(Ordering::Acquire) && !*job.cancel.borrow(),
            None => false,
        }
    }

    pub async fn result(
        &self,
        id: &str,
        wait: u64,
        offset: u64,
        stderr_offset: u64,
    ) -> Result<Value> {
        self.result_view(id, wait, offset, stderr_offset, false)
            .await
    }

    /// View sharing excludes child identity. This never mutates the original
    /// execution binding used by renewal or precise child revocation.
    pub async fn authorize_external(
        &self,
        id: &str,
        authority: &nyxid_machine::authority::ExternalAuthority,
    ) -> Result<()> {
        let job = self
            .jobs
            .lock()
            .await
            .get(id)
            .cloned()
            .context(super::MachineError::JobNotFound)?;
        let binding = job
            .external
            .as_ref()
            .context(super::MachineError::JobNotFound)?;
        if !binding.principal.same_principal(&authority.principal)
            || binding.grant_id != authority.grant_id
            || binding.grant_revision != authority.grant_revision
            || binding.assignment_revision != authority.assignment_revision
            || binding.node_id != authority.node_id
            || binding.runtime_id != authority.runtime_id
            || binding.runtime_generation != authority.runtime_generation
            || binding.credential_epoch != authority.credential_epoch
            || authority.job_id.as_deref() != Some(id)
        {
            return Err(super::MachineError::JobNotFound.into());
        }
        Ok(())
    }

    /// Legacy Assistant readers must never reinterpret an external job ID.
    pub async fn authorize_assistant(&self, id: &str) -> Result<()> {
        let jobs = self.jobs.lock().await;
        let job = jobs.get(id).context(super::MachineError::JobNotFound)?;
        anyhow::ensure!(job.external.is_none(), super::MachineError::JobNotFound);
        Ok(())
    }

    /// Admit owner cancellation and signal under one lock. A delayed lookup
    /// cannot start an effect after the verified management deadline.
    pub async fn cancel_owner(&self, id: &str, owner_id: &str, deadline: Instant) -> Result<()> {
        let jobs = self.jobs.lock().await;
        let job = jobs.get(id).context(super::MachineError::JobNotFound)?;
        anyhow::ensure!(
            job.external.as_ref().is_none_or(|a| a.owner_id == owner_id),
            super::MachineError::JobNotFound
        );
        anyhow::ensure!(
            deadline > Instant::now(),
            super::MachineError::AuthorityStale
        );
        if job.running.load(Ordering::Acquire) {
            signal_group(job.pid, libc::SIGKILL);
        }
        job.cancel.send_replace(true);
        Ok(())
    }

    pub async fn authorize_owner(&self, id: &str, owner_id: &str) -> Result<()> {
        let jobs = self.jobs.lock().await;
        let job = jobs.get(id).context(super::MachineError::JobNotFound)?;
        // Shared legacy Assistant jobs predate local owner bindings. Their
        // first-party proof is issued only after the backend's persisted ACL.
        anyhow::ensure!(
            job.external.as_ref().is_none_or(|a| a.owner_id == owner_id),
            super::MachineError::JobNotFound
        );
        Ok(())
    }

    pub async fn sweep_output(&self) {
        let jobs: Vec<_> = self.jobs.lock().await.values().cloned().collect();
        for job in jobs {
            let mut state = job.state.lock().await;
            expire_output(&mut state);
        }
    }

    pub async fn foreground_result(&self, id: &str, wait: u64) -> Result<Value> {
        self.result_view(id, wait, 0, 0, true).await
    }

    async fn result_view(
        &self,
        id: &str,
        wait: u64,
        offset: u64,
        stderr_offset: u64,
        summary: bool,
    ) -> Result<Value> {
        let job = self
            .jobs
            .lock()
            .await
            .get(id)
            .cloned()
            .context(super::MachineError::JobNotFound)?;
        let notified = job.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if wait > 0 && job.state.lock().await.finished.is_none() {
            let _ = tokio::time::timeout(Duration::from_secs(wait), notified).await;
        }
        let redactor = self.redactor.lock().await;
        let mut state = job.state.lock().await;
        expire_output(&mut state);
        if let Some(reason) = state.output_unavailable {
            return Ok(
                json!({"job_id":id,"status":"finished","exit_code":state.exit_code,"stdout":"","stderr":"","stdout_bytes":state.stdout.total_bytes(),"stderr_bytes":state.stderr.total_bytes(),"output_offset":state.stdout.total_bytes(),"stderr_offset":state.stderr.total_bytes(),"truncated":true,"output_available":false,"output_unavailable_reason":reason,"timed_out":state.timed_out}),
            );
        }
        let read = |ring: &OutputRing, offset| {
            if summary {
                let (bytes, truncated) = ring.summary_redacted(3000, &redactor);
                (ring.total_bytes(), bytes, truncated)
            } else {
                ring.read_redacted(offset, 3000, &redactor)
            }
        };
        let (next, stdout, out_truncated) = read(&state.stdout, offset);
        let (err_next, stderr, err_truncated) = read(&state.stderr, stderr_offset);
        Ok(
            json!({"job_id":id,"status":if state.finished.is_some(){"finished"}else{"running"},"exit_code":state.exit_code,"stdout":String::from_utf8_lossy(&stdout),"stderr":String::from_utf8_lossy(&stderr),"stdout_bytes":state.stdout.total_bytes(),"stderr_bytes":state.stderr.total_bytes(),"output_offset":next,"stderr_offset":err_next,"truncated":out_truncated||err_truncated,"output_available":true,"timed_out":state.timed_out,"duration_ms":state.finished.unwrap_or_else(Instant::now).duration_since(state.started).as_millis() as u64}),
        )
    }
}

const MAX_OUTPUT_CACHE_BYTES: usize = 32 * 1024 * 1024;
fn discard_output(state: &mut State, reason: &'static str) {
    state.stdout.discard();
    state.stderr.discard();
    state.output_unavailable = Some(reason);
}
fn expire_output(state: &mut State) {
    if state.output_unavailable.is_none()
        && state
            .finished
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(3600))
    {
        discard_output(state, "expired");
    }
}

async fn read_output(
    mut reader: impl AsyncRead + Unpin,
    job: Arc<Job>,
    stderr: bool,
    redactor: Arc<Mutex<Redactor>>,
) {
    let mut buffer = Zeroizing::new([0u8; 16384]);
    let mut pending = Zeroizing::new(Vec::new());
    loop {
        let count = reader.read(&mut buffer[..]).await.unwrap_or(0);
        pending.extend_from_slice(&buffer[..count]);
        let output = redactor.lock().await.stream(&mut pending, count == 0);
        let mut state = job.state.lock().await;
        if stderr {
            state.stderr.push(&output);
        } else {
            state.stdout.push(&output);
        }
        if count == 0 {
            break;
        }
    }
}

fn signal_group(pid: i32, signal: i32) {
    if pid > 1 {
        unsafe {
            libc::kill(-pid, signal);
        }
    }
}

async fn terminate(
    child: &mut tokio::process::Child,
    pid: i32,
) -> std::io::Result<std::process::ExitStatus> {
    signal_group(pid, libc::SIGTERM);
    let status = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    signal_group(pid, libc::SIGKILL);
    match status {
        Ok(status) => status,
        Err(_) => child.wait().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_request(id: &str, command: &str, value: &str, background: bool) -> Exec {
        Exec {
            scope: Default::default(),
            job_id: id.into(),
            command: command.into(),
            cwd: None,
            env: BTreeMap::from([
                ("API_TOKEN".into(), value.into()),
                ("EMPTY_VALUE".into(), String::new()),
            ]),
            stdin: None,
            timeout_secs: Some(10),
            background,
        }
    }

    fn external_authority(job_id: &str) -> nyxid_machine::authority::ExternalAuthority {
        use nyxid_machine::authority::{Capabilities, ExternalAuthority, ExternalPrincipal};
        let id = || uuid::Uuid::new_v4().to_string();
        let owner = id();
        let until = chrono::Utc::now().timestamp_millis() + 30_000;
        ExternalAuthority {
            principal: ExternalPrincipal::ApiKey {
                user_id: owner.clone(),
                api_key_id: id(),
                api_key_credential_id: Some(id()),
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
            job_id: Some(job_id.into()),
            expires_at_ms: until,
            auth_expires_at_ms: until,
            job_expires_at_ms: until,
            funding_expires_at_ms: None,
            capabilities: Capabilities {
                shell: true,
                ..Default::default()
            },
            file_roots: vec![],
            services: vec![],
        }
    }

    async fn assert_env_foreground(external: bool) {
        let root = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[root.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        );
        let identity = Identity::resolve(None).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let value = "synthetic-supplied-env-foreground-49a7b52f";
        let request = env_request(
            &id,
            "printf 'out:%s:end' \"$API_TOKEN\"; printf 'err:%s:end' \"$API_TOKEN\" >&2; test -z \"$EMPTY_VALUE\"",
            value,
            false,
        );
        let result = if external {
            let authority = external_authority(&id);
            let result = jobs
                .start_external(request, &identity, &roots, &BTreeMap::new(), &authority)
                .await
                .unwrap();
            jobs.authorize_external(&id, &authority).await.unwrap();
            assert!(jobs.authorize_assistant(&id).await.is_err());
            result
        } else {
            let result = jobs
                .start(request, &identity, &roots, &BTreeMap::new())
                .await
                .unwrap();
            jobs.authorize_assistant(&id).await.unwrap();
            result
        };
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["stdout"], "out:[redacted]:end");
        assert_eq!(result["stderr"], "err:[redacted]:end");
        let retained = jobs.result(&id, 0, 0, 0).await.unwrap();
        assert_eq!(retained["stdout"], result["stdout"]);
        assert_eq!(retained["stderr"], result["stderr"]);
    }

    #[tokio::test]
    async fn supplied_env_is_redacted_by_ordinary_foreground_executor() {
        assert_env_foreground(false).await;
    }

    #[tokio::test]
    async fn supplied_env_is_redacted_by_external_foreground_executor() {
        assert_env_foreground(true).await;
    }

    async fn assert_env_background(external: bool) {
        let root = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[root.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        );
        let identity = Identity::resolve(None).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let value = "synthetic-supplied-env-background-49a7b52f";
        // The replacement crosses a 3000-byte result page. Both pipe consumers
        // run through the executor; no manual registry setup supplies protection.
        let request = env_request(
            &id,
            "head -c 2995 /dev/zero | tr '\\000' x; printf '%s' \"$API_TOKEN\"; head -c 3500 /dev/zero | tr '\\000' y; printf '%s' \"$API_TOKEN\" >&2",
            value,
            true,
        );
        let result = if external {
            jobs.start_external(
                request,
                &identity,
                &roots,
                &BTreeMap::new(),
                &external_authority(&id),
            )
            .await
            .unwrap()
        } else {
            jobs.start(request, &identity, &roots, &BTreeMap::new())
                .await
                .unwrap()
        };
        assert_eq!(result, json!({"job_id":id}));
        assert_eq!(
            jobs.result(&id, 10, 0, 0).await.unwrap()["status"],
            "finished"
        );
        // A later job must not inherit the temporary environment. Its admission
        // must not make the completed job's retained output unsafe either.
        let later = jobs
            .start(
                Exec {
                    env: BTreeMap::new(),
                    ..env_request(
                        &uuid::Uuid::new_v4().to_string(),
                        "printf '%s' \"${API_TOKEN-unset}\"",
                        "",
                        false,
                    )
                },
                &identity,
                &roots,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        assert_eq!(later["stdout"], "unset");
        let mut offset = 0;
        let mut stderr_offset = 0;
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut pages = 0;
        loop {
            let page = jobs.result(&id, 0, offset, stderr_offset).await.unwrap();
            stdout.push_str(page["stdout"].as_str().unwrap());
            stderr.push_str(page["stderr"].as_str().unwrap());
            offset = page["output_offset"].as_u64().unwrap();
            stderr_offset = page["stderr_offset"].as_u64().unwrap();
            pages += 1;
            if offset == page["stdout_bytes"].as_u64().unwrap()
                && stderr_offset == page["stderr_bytes"].as_u64().unwrap()
            {
                break;
            }
            assert!(pages < 10);
        }
        assert!(pages >= 3);
        assert_eq!(
            stdout,
            format!("{}[redacted]{}", "x".repeat(2995), "y".repeat(3500))
        );
        assert_eq!(stderr, "[redacted]");
        // Every arbitrary offset inside the former value sees only the
        // already-scrubbed cache; the value cannot be recovered by pagination.
        for offset in 2995..2995 + value.len() as u64 {
            let page = jobs.result(&id, 0, offset, 0).await.unwrap();
            assert!(!page.to_string().contains("synthetic-supplied"));
        }
    }

    #[tokio::test]
    async fn supplied_env_is_redacted_by_ordinary_background_and_paginated_reads() {
        assert_env_background(false).await;
    }

    #[tokio::test]
    async fn supplied_env_is_redacted_by_external_background_and_paginated_reads() {
        assert_env_background(true).await;
    }

    #[tokio::test]
    async fn supplied_env_fragments_are_held_by_actual_capture_until_complete() {
        let root = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[root.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        );
        let id = uuid::Uuid::new_v4().to_string();
        let value = "synthetic-fragmented-env-123456789";
        // A FIFO gates the second half until both actual pipe consumers have
        // captured the first half. This forces fragmentation without timing sleeps.
        jobs.start(env_request(&id,
            "mkfifo release; head -c 17000 /dev/zero | tr '\\000' x; printf '%.18s' \"$API_TOKEN\"; head -c 17000 /dev/zero | tr '\\000' y >&2; printf '%.18s' \"$API_TOKEN\" >&2; IFS= read -r gate < release; printf '%s' \"$API_TOKEN\" | cut -c 19-; printf '%s' \"$API_TOKEN\" | cut -c 19- >&2",
            value, true), &Identity::resolve(None).unwrap(), &roots, &BTreeMap::new()).await.unwrap();
        let job = jobs.jobs.lock().await[&id].clone();
        let captured_prefix = 17_018 - jobs.redactor.lock().await.max_pattern_bytes() as u64;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = job.state.lock().await;
                if state.stdout.total_bytes() >= captured_prefix
                    && state.stderr.total_bytes() >= captured_prefix
                {
                    break;
                }
                drop(state);
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let during = jobs.result(&id, 0, 16990, 16990).await.unwrap();
        tokio::fs::write(root.path().join("release"), b"go\n")
            .await
            .unwrap();
        let result = jobs.result(&id, 10, 16990, 16990).await.unwrap();
        assert_eq!(result["status"], "finished");
        assert!(!during.to_string().contains("synthetic"));
        assert_eq!(result["stdout"], "xxxxxxxxxx[redacted]\n");
        assert_eq!(result["stderr"], "yyyyyyyyyy[redacted]\n");
    }

    #[tokio::test]
    async fn supplied_env_registry_exhaustion_refuses_before_executor_effects() {
        let root = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[root.path().into()], &[]).unwrap();
        let registry = Arc::new(Mutex::new(Redactor::default()));
        let jobs = Jobs::new(&Default::default(), registry.clone());
        // Fill the pre-existing bounded session registry, then exercise actual
        // admission with the value which could not be protected.
        let mut rejected = None;
        for index in 0..100 {
            let value = format!("synthetic-capacity-{index}-{}", "q".repeat(14000));
            if registry.lock().await.register(&value).is_err() {
                rejected = Some(value);
                break;
            }
        }
        let value = rejected.expect("registry must have a finite byte bound");
        let result = jobs
            .start(
                env_request(
                    &uuid::Uuid::new_v4().to_string(),
                    "touch must-not-exist; printf '%s' \"$API_TOKEN\"",
                    &value,
                    false,
                ),
                &Identity::resolve(None).unwrap(),
                &roots,
                &BTreeMap::new(),
            )
            .await;
        assert!(result.is_err(), "unprotected input must not execute");
        assert!(!root.path().join("must-not-exist").exists());
        assert!(jobs.jobs.lock().await.is_empty());
        assert_eq!(
            jobs.permits.available_permits(),
            nyxid_machine::config::Config::default().max_jobs
        );
    }
    #[tokio::test]
    async fn output_expires_on_read_without_waiting_for_sweep_or_new_job() {
        let root = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[root.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        );
        let id = uuid::Uuid::new_v4().to_string();
        jobs.start(
            Exec {
                scope: Default::default(),
                job_id: id.clone(),
                command: "printf private-output".into(),
                cwd: None,
                env: BTreeMap::new(),
                stdin: None,
                timeout_secs: Some(10),
                background: false,
            },
            &Identity::resolve(None).unwrap(),
            &roots,
            &BTreeMap::new(),
        )
        .await
        .unwrap();
        let job = jobs.jobs.lock().await[&id].clone();
        job.state.lock().await.finished = Some(Instant::now() - Duration::from_secs(3600));
        let result = jobs.result(&id, 0, 0, 0).await.unwrap();
        assert_eq!(result["output_available"], false);
        assert_eq!(result["output_unavailable_reason"], "expired");
        assert_eq!(result["stdout"], "");
        assert_eq!(result["stdout_bytes"], 14);
        jobs.sweep_output().await;
        assert_eq!(
            jobs.result(&id, 0, 0, 0).await.unwrap()["output_unavailable_reason"],
            "expired"
        );
    }

    #[tokio::test]
    async fn aggregate_output_cache_evicts_old_terminal_bytes_before_admission() {
        let root = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[root.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        );
        let identity = Identity::resolve(None).unwrap();
        let mut ids = Vec::new();
        for _ in 0..33 {
            let id = uuid::Uuid::new_v4().to_string();
            jobs.start(
                Exec {
                    scope: Default::default(),
                    job_id: id.clone(),
                    command: "printf output".into(),
                    cwd: None,
                    env: BTreeMap::new(),
                    stdin: None,
                    timeout_secs: Some(10),
                    background: false,
                },
                &identity,
                &roots,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
            ids.push(id);
        }
        assert_eq!(
            jobs.result(&ids[0], 0, 0, 0).await.unwrap()["output_unavailable_reason"],
            "capacity_evicted"
        );
        assert_eq!(
            jobs.result(&ids[32], 0, 0, 0).await.unwrap()["stdout"],
            "output"
        );
        let mut retained = 0;
        for job in jobs.jobs.lock().await.values() {
            if job.state.lock().await.output_unavailable.is_none() {
                retained += jobs.output_bytes;
            }
        }
        assert!(retained <= MAX_OUTPUT_CACHE_BYTES);
    }
    #[tokio::test]
    async fn background_jobs_retain_output_and_cancel_process_groups() {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[temp.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        );
        let identity = Identity::resolve(None).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        jobs.start(
            Exec {
                scope: Default::default(),
                job_id: id.clone(),
                command: "printf before; sleep 30 & wait".into(),
                cwd: None,
                env: BTreeMap::new(),
                stdin: None,
                timeout_secs: Some(30),
                background: true,
            },
            &identity,
            &roots,
            &BTreeMap::new(),
        )
        .await
        .unwrap();
        assert!(jobs.running(&id).await);
        jobs.preempt_identity(identity.uid.wrapping_add(1)).await;
        assert!(jobs.running(&id).await);
        assert!(!*jobs.jobs.lock().await[&id].cancel.borrow());
        jobs.preempt_identity(identity.uid).await;
        assert!(*jobs.jobs.lock().await[&id].cancel.borrow());
        let result = jobs.result(&id, 5, 0, 0).await.unwrap();
        assert_eq!(result["status"], "finished");
        let terminal = jobs.terminal_metadata(&id).await.unwrap();
        assert_eq!(terminal["stop_reason"], "cancelled");
        assert_eq!(terminal.as_object().unwrap().len(), 4);
        assert!(terminal.get("stdout").is_none() && terminal.get("stderr").is_none());
        assert!(!jobs.running(&id).await);
        assert!(jobs.result("missing", 0, 0, 0).await.is_err());
    }
    #[tokio::test]
    async fn exact_cleanup_job_ids_do_not_cancel_a_new_turn_in_the_same_conversation() {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[temp.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &Default::default(),
            Arc::new(Mutex::new(Redactor::default())),
        );
        let identity = Identity::resolve(None).unwrap();
        let old = uuid::Uuid::new_v4().to_string();
        let new = uuid::Uuid::new_v4().to_string();
        for (id, turn) in [(&old, "old-turn"), (&new, "new-turn")] {
            jobs.start(
                Exec {
                    scope: super::super::cancellation::Scope {
                        conversation_id: "same-conversation".into(),
                        turn_id: turn.into(),
                    },
                    job_id: id.clone(),
                    command: "sleep 30".into(),
                    cwd: None,
                    env: BTreeMap::new(),
                    stdin: None,
                    timeout_secs: Some(30),
                    background: true,
                },
                &identity,
                &roots,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        }
        jobs.preempt_ids(std::slice::from_ref(&old)).await;
        assert!(*jobs.jobs.lock().await[&old].cancel.borrow());
        assert!(!*jobs.jobs.lock().await[&new].cancel.borrow());
        assert!(jobs.running(&new).await);
        jobs.preempt_ids(std::slice::from_ref(&old)).await;
        assert!(!*jobs.jobs.lock().await[&new].cancel.borrow());
        jobs.preempt_ids(std::slice::from_ref(&new)).await;
        jobs.result(&old, 5, 0, 0).await.unwrap();
        jobs.result(&new, 5, 0, 0).await.unwrap();
    }

    #[tokio::test]
    async fn timeout_and_output_bound_are_enforced() {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[temp.path().into()], &[]).unwrap();
        let jobs = Jobs::new(
            &nyxid_machine::config::Config {
                output_bytes: 16384,
                ..Default::default()
            },
            Arc::new(Mutex::new(Redactor::default())),
        );
        let result = jobs
            .start(
                Exec {
                    scope: Default::default(),
                    job_id: uuid::Uuid::new_v4().to_string(),
                    command: "printf BEGIN; yes x | head -c 20000; printf END; sleep 30".into(),
                    cwd: None,
                    env: BTreeMap::new(),
                    stdin: None,
                    timeout_secs: Some(1),
                    background: false,
                },
                &Identity::resolve(None).unwrap(),
                &roots,
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        assert_eq!(result["timed_out"], true);
        assert_eq!(result["truncated"], true);
        assert!(result["stdout"].as_str().unwrap().starts_with("BEGIN"));
        assert!(result["stdout"].as_str().unwrap().ends_with("END"));
        assert_eq!(result["stdout_bytes"], 20008);
        assert!(result.to_string().len() < 9000);
    }
}
