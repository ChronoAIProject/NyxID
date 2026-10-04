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
}

pub struct Job {
    scope: super::cancellation::Scope,
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
        let mut expired = Vec::new();
        for (id, job) in jobs.iter() {
            if job
                .state
                .lock()
                .await
                .finished
                .is_some_and(|at| at.elapsed() > Duration::from_secs(3600))
            {
                expired.push(id.clone());
            }
        }
        for id in expired {
            jobs.remove(&id);
        }
        if jobs.len() >= 1024 || jobs.contains_key(&request.job_id) {
            bail!("job retention limit reached or duplicate job id");
        }
        let mut child = command.spawn().context("command could not start")?;
        let pid = child.id().context("command process unavailable")? as i32;
        let stdout = child.stdout.take().context("stdout unavailable")?;
        let stderr = child.stderr.take().context("stderr unavailable")?;
        let input = request.stdin.map(Zeroizing::new);
        let mut stdin = child.stdin.take();
        let (cancel, mut cancelled) = watch::channel(false);
        let job = Arc::new(Job {
            scope: request.scope,
            uid: identity.uid,
            state: Mutex::new(State {
                stdout: OutputRing::with_head(self.output_bytes / 2),
                stderr: OutputRing::with_head(self.output_bytes / 2),
                exit_code: None,
                finished: None,
                started: Instant::now(),
                timed_out: false,
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
                s.conversation_id == job.scope.conversation_id
                    && (s.turn_id.is_empty() || s.turn_id == job.scope.turn_id)
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

    pub async fn finished(&self, id: &str) -> bool {
        let job = self.jobs.lock().await.get(id).cloned();
        match job {
            Some(job) => job.state.lock().await.finished.is_some(),
            None => false,
        }
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
        let state = job.state.lock().await;
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
            json!({"job_id":id,"status":if state.finished.is_some(){"finished"}else{"running"},"exit_code":state.exit_code,"stdout":String::from_utf8_lossy(&stdout),"stderr":String::from_utf8_lossy(&stderr),"stdout_bytes":state.stdout.total_bytes(),"stderr_bytes":state.stderr.total_bytes(),"output_offset":next,"stderr_offset":err_next,"truncated":out_truncated||err_truncated,"timed_out":state.timed_out,"duration_ms":state.finished.unwrap_or_else(Instant::now).duration_since(state.started).as_millis() as u64}),
        )
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
        assert!(!jobs.running(&id).await);
        assert!(jobs.result("missing", 0, 0, 0).await.is_err());
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
