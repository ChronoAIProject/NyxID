//! The public MCP stdio contract is the only interface to the MIT cua driver.
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use nyxid_machine::ComputerMode;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

use super::process::Identity;

pub const VERSION: &str = "0.31.0";
const MAX_MCP_LINE: usize = 12 * 1024 * 1024;
static TOOLS: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(|| {
    serde_json::from_str(nyxid_machine::CUA_TOOLS).expect("embedded contract")
});
const RELEASE: &str = include_str!("../../../resources/cua/release.json");

pub fn public_tool(name: &str) -> bool {
    TOOLS.iter().any(|tool| tool["name"] == name)
}
pub fn read_only(name: &str) -> bool {
    TOOLS
        .iter()
        .any(|tool| tool["name"] == name && tool["read_only"] == true)
}

pub fn platform_asset(os: &str, arch: &str) -> Result<(String, String)> {
    let platform = match (os, arch) {
        ("macos", "aarch64" | "x86_64") => "darwin-universal",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        _ => bail!("cua driver is unavailable for this platform"),
    };
    let release: Value = serde_json::from_str(RELEASE)?;
    let file = format!("cua-driver-rs-{VERSION}-{platform}.tar.gz");
    let asset = release["assets"]
        .as_array()
        .context("invalid embedded release")?
        .iter()
        .find(|a| a["name"] == file)
        .context("missing pinned asset")?;
    Ok((
        asset["url"]
            .as_str()
            .context("missing release URL")?
            .to_owned(),
        asset["sha256"]
            .as_str()
            .context("missing checksum")?
            .to_owned(),
    ))
}

pub async fn install(directory: &Path) -> Result<PathBuf> {
    let (url, expected) = platform_asset(std::env::consts::OS, std::env::consts::ARCH)?;
    let archive_name = url.rsplit('/').next().context("invalid asset URL")?;
    let root_name = archive_name.trim_end_matches(".tar.gz");
    let parent = directory.join("cua");
    tokio::fs::create_dir_all(&parent).await?;
    let destination = parent.join(root_name);
    let binary = destination.join("cua-driver");
    if binary.exists() {
        verify_version(&binary).await?;
        return Ok(binary);
    }
    let staging = tempfile::tempdir_in(&parent)?;
    let archive_path = staging.path().join("release.tar.gz");
    let mut output = tokio::fs::File::create(&archive_path).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;
    let response = client.get(&url).send().await?.error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut digest = Sha256::new();
    let mut size = 0usize;
    while let Some(bytes) = stream.next().await {
        let bytes = bytes?;
        size += bytes.len();
        if size > 256 * 1024 * 1024 {
            bail!("cua archive size limit exceeded");
        }
        digest.update(&bytes);
        output.write_all(&bytes).await?;
    }
    output.flush().await?;
    drop(output);
    if hex::encode(digest.finalize()) != expected {
        bail!("cua release checksum mismatch; nothing installed");
    }
    let staging_path = staging.path().to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = std::fs::File::open(archive_path)?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
        archive.unpack(staging_path)?;
        Ok(())
    })
    .await??;
    tokio::fs::rename(staging.path().join(root_name), &destination).await?;
    verify_version(&binary).await?;
    Ok(binary)
}

pub async fn verify_version(path: &Path) -> Result<String> {
    let mut command = Command::new(path);
    Identity::resolve(None)?.prepare(&mut command)?;
    command
        .arg("--version")
        .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false")
        .env("CUA_DRIVER_RS_UPDATE_CHECK", "false")
        .stderr(std::process::Stdio::null());
    let output = tokio::time::timeout(Duration::from_secs(10), command.output()).await??;
    let version = String::from_utf8_lossy(&output.stdout);
    if !output.status.success()
        || !version
            .split_whitespace()
            .any(|word| word.trim_start_matches('v') == VERSION)
    {
        bail!("cua driver version must be {VERSION}; use the managed install");
    }
    Ok(VERSION.into())
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum DriverError {
    #[error(
        "driver restarting; retry after {retry_after_ms} ms and observe before repeating an action"
    )]
    Restarting { retry_after_ms: u64 },
    #[error("computer permission missing; enable Accessibility and Screen Recording for the node")]
    PermissionMissing,
    #[error("computer tool is not supported on this platform")]
    ToolUnsupported,
    #[error("display unavailable; start the desktop session")]
    DisplayUnavailable,
    #[error("computer request refused by cua; check its permission mode and request arguments")]
    Refused,
}

// A broken session must never escape as an unclassified machine-operation error.
// Keep transport/process failures separate from a well-formed MCP refusal.
#[derive(Debug, Clone, Copy)]
enum TransportFailure {
    Spawn,
    Write,
    Read,
    Eof,
    Decode,
    Protocol,
    Exited,
    Timeout,
    Cancelled,
}

#[derive(Debug)]
enum SessionError {
    Transport(TransportFailure),
    Refused(DriverError),
}

type SessionResult<T> = std::result::Result<T, SessionError>;
type DriverResult<T> = std::result::Result<T, DriverError>;

fn tool_refusal(value: &Value) -> Option<DriverError> {
    match value["code"].as_str() {
        Some(
            "permission_denied"
            | "accessibility_permission_denied"
            | "screen_recording_permission_denied",
        ) => Some(DriverError::PermissionMissing),
        Some("display_unavailable" | "display_not_found") => Some(DriverError::DisplayUnavailable),
        Some("unsupported_tool" | "tool_not_supported") => Some(DriverError::ToolUnsupported),
        _ => None,
    }
}

fn refusal(value: &Value) -> DriverError {
    tool_refusal(value).unwrap_or_else(|| {
        if value["code"] == -32601 {
            DriverError::ToolUnsupported
        } else {
            DriverError::Refused
        }
    })
}

pub struct Driver {
    human_input: bool,
    #[cfg(target_os = "macos")]
    memory_capture: bool,
    path: PathBuf,
    identity: Identity,
    mode: ComputerMode,
    session: Mutex<Option<Session>>,
    cancelled: tokio::sync::watch::Sender<u64>,
    failures: std::sync::atomic::AtomicU32,
    next_start: std::sync::Mutex<Option<Instant>>,
    known_tools: std::sync::RwLock<Vec<String>>,
    #[cfg(target_os = "macos")]
    known_permissions: std::sync::RwLock<Option<nyxid_machine::ComputerPermissions>>,
}

struct Session {
    #[cfg(target_os = "macos")]
    _capture: Option<super::memory_capture::MemoryCapture>,
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
    tools: Vec<String>,
}

impl Driver {
    pub fn new(path: PathBuf, identity: Identity, mode: ComputerMode) -> Self {
        Self {
            human_input: false,
            #[cfg(target_os = "macos")]
            memory_capture: true,
            path,
            identity,
            mode,
            session: Mutex::new(None),
            cancelled: tokio::sync::watch::channel(0).0,
            failures: std::sync::atomic::AtomicU32::new(0),
            next_start: std::sync::Mutex::new(None),
            known_tools: std::sync::RwLock::new(Vec::new()),
            #[cfg(target_os = "macos")]
            known_permissions: std::sync::RwLock::new(None),
        }
    }

    #[cfg(test)]
    pub(super) fn without_capture_for_test(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.memory_capture = false;
        }
    }

    /// Human pointer input must not wait for the agent cursor's cosmetic glide.
    pub fn for_human(mut self) -> Self {
        self.human_input = true;
        self
    }

    async fn start(&self) -> SessionResult<Session> {
        let next = *self.next_start.lock().expect("driver backoff lock");
        if let Some(at) = next {
            tokio::time::sleep(at.saturating_duration_since(Instant::now())).await;
        }
        let mut command = Command::new(&self.path);
        self.identity
            .prepare(&mut command)
            .map_err(|_| SessionError::Transport(TransportFailure::Spawn))?;
        self.identity.desktop_env(&mut command);
        command
            .args(["mcp", "--direct"])
            .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false")
            .env("CUA_DRIVER_RS_UPDATE_CHECK", "false")
            .env(
                "CUA_DRIVER_PERMISSION_MODE",
                if self.mode == ComputerMode::Unrestricted {
                    "unrestricted"
                } else {
                    "standard"
                },
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        if self.mode == ComputerMode::Unrestricted {
            command.env("CUA_DRIVER_DANGEROUSLY_BYPASS_APPROVALS", "1");
        }
        #[cfg(target_os = "macos")]
        let capture = if self.memory_capture {
            Some(
                super::memory_capture::MemoryCapture::create()
                    .await
                    .map_err(|_| SessionError::Transport(TransportFailure::Spawn))?,
            )
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        if let Some(capture) = &capture {
            command.env("TMPDIR", capture.path());
        }
        let mut child = command
            .spawn()
            .map_err(|_| SessionError::Transport(TransportFailure::Spawn))?;
        let input = child
            .stdin
            .take()
            .ok_or(SessionError::Transport(TransportFailure::Spawn))?;
        let output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or(SessionError::Transport(TransportFailure::Spawn))?,
        );
        let mut session = Session {
            #[cfg(target_os = "macos")]
            _capture: capture,
            _child: child,
            input,
            output,
            next_id: 1,
            tools: Vec::new(),
        };
        session.rpc("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"nyxid-node","version":env!("CARGO_PKG_VERSION")}})).await?;
        session
            .input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .await
            .map_err(|_| SessionError::Transport(TransportFailure::Write))?;
        let tools = session.rpc("tools/list", json!({})).await?;
        session.tools = tools["tools"]
            .as_array()
            .ok_or(SessionError::Transport(TransportFailure::Protocol))?
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .filter(|name| public_tool(name))
            .map(str::to_owned)
            .collect();
        if self.human_input
            && session
                .tools
                .iter()
                .any(|tool| tool == "set_agent_cursor_motion")
        {
            let result = session.rpc("tools/call", json!({
                "name":"set_agent_cursor_motion",
                "arguments":{"session":"nyxid-owner","glide_duration_ms":50,"dwell_after_click_ms":0,"spring":1,"arc_size":0}
            })).await?;
            if result["isError"] == true {
                return Err(SessionError::Refused(refusal(&result["structuredContent"])));
            }
        }
        *self.known_tools.write().expect("driver tools lock") = session.tools.clone();
        *self.next_start.lock().expect("driver backoff lock") = None;
        Ok(session)
    }

    /// A transport restart does not withdraw installed capability support.
    pub fn advertised_tools(&self) -> Vec<String> {
        let cached = self.known_tools.read().expect("driver tools lock");
        if cached.is_empty() {
            TOOLS
                .iter()
                .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                .collect()
        } else {
            cached.clone()
        }
    }

    fn restarting(&self, failure: TransportFailure) -> DriverError {
        // Metadata only: never log driver response text or request arguments.
        tracing::debug!(?failure, "cua session restarting");
        let failures = self
            .failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .min(5);
        let delay = Duration::from_millis((100u64 << failures).min(2000));
        *self.next_start.lock().expect("driver backoff lock") = Some(Instant::now() + delay);
        DriverError::Restarting {
            retry_after_ms: delay.as_millis() as u64,
        }
    }

    async fn start_bounded(&self) -> SessionResult<Session> {
        tokio::time::timeout(Duration::from_secs(20), self.start())
            .await
            .map_err(|_| SessionError::Transport(TransportFailure::Timeout))?
    }

    // Retain a healthy session after a genuine MCP refusal. Every transport error
    // instead drops/kills the child and leaves the next call to restart with backoff.
    fn finish<T>(
        &self,
        slot: &mut Option<Session>,
        active: Option<Session>,
        result: SessionResult<T>,
    ) -> DriverResult<T> {
        match result {
            Ok(value) => {
                self.failures.store(0, std::sync::atomic::Ordering::Relaxed);
                *slot = active;
                Ok(value)
            }
            Err(SessionError::Refused(error)) => {
                *slot = active;
                Err(error)
            }
            Err(SessionError::Transport(failure)) => Err(self.restarting(failure)),
        }
    }

    pub async fn tools(&self) -> DriverResult<Vec<String>> {
        self.with_session(None, None).await.map(|value| {
            value
                .as_array()
                .expect("tools result")
                .iter()
                .map(|name| name.as_str().expect("tool name").to_owned())
                .collect()
        })
    }

    /// Local readiness probe only; never expose this diagnostic tool to an
    /// agent. In direct MCP mode it reports this process's real TCC attribution.
    #[cfg(target_os = "macos")]
    pub async fn permissions(&self) -> DriverResult<nyxid_machine::ComputerPermissions> {
        let result = self
            .with_session(
                Some(("check_permissions", json!({"prompt":false}))),
                Some(Duration::from_secs(5)),
            )
            .await?;
        let permissions = nyxid_machine::ComputerPermissions {
            screen_recording: result["structuredContent"]["screen_recording"].as_bool(),
            accessibility: result["structuredContent"]["accessibility"].as_bool(),
        };
        *self.known_permissions.write().expect("permissions lock") = Some(permissions.clone());
        Ok(permissions)
    }

    #[cfg(target_os = "macos")]
    pub fn last_permissions(&self) -> Option<nyxid_machine::ComputerPermissions> {
        self.known_permissions
            .read()
            .expect("permissions lock")
            .clone()
    }

    pub async fn call(&self, name: &str, arguments: Value) -> DriverResult<Value> {
        if !public_tool(name) {
            return Err(DriverError::ToolUnsupported);
        }
        self.with_session(Some((name, arguments)), None).await
    }

    async fn with_session(
        &self,
        call: Option<(&str, Value)>,
        timeout: Option<Duration>,
    ) -> DriverResult<Value> {
        let mut cancelled = self.cancelled.subscribe();
        let mut session = tokio::select! {
            biased;
            _ = cancelled.changed() => return Err(self.restarting(TransportFailure::Cancelled)),
            session = self.session.lock() => session,
        };
        let mut active = session.take();
        let result = tokio::select! {
            biased;
            _ = cancelled.changed() => Err(SessionError::Transport(TransportFailure::Cancelled)),
            result = async {
                if active.is_none() {
                    active = Some(self.start_bounded().await?);
                }
                let active = active.as_mut().expect("started session");
                active.check_running()?;
                let Some((name, arguments)) = call else {
                    return Ok(json!(active.tools));
                };
                if name != "check_permissions" && !active.tools.iter().any(|tool| tool == name) {
                    return Err(SessionError::Refused(DriverError::ToolUnsupported));
                }
                let value = tokio::time::timeout(
                    timeout.unwrap_or(Duration::from_secs(30)),
                    active.rpc("tools/call", json!({"name":name,"arguments":arguments})),
                ).await.map_err(|_| SessionError::Transport(TransportFailure::Timeout))??;
                if let Some(error) = tool_refusal(&value["structuredContent"]) {
                    return Err(SessionError::Refused(error));
                }
                // Ordinary tool mistakes retain cua's explanation so callers can
                // scrub it and the agent can correct its arguments or observation.
                Ok(value)
            } => result,
        };
        self.finish(&mut session, active, result)
    }

    pub async fn stop(&self) {
        self.cancelled
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        if let Ok(mut session) = self.session.try_lock() {
            session.take();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(pid) = self._child.id() {
            // cua subprocesses belong to this process group as well.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

impl Session {
    fn check_running(&mut self) -> SessionResult<()> {
        match self._child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(_)) | Err(_) => Err(SessionError::Transport(TransportFailure::Exited)),
        }
    }

    async fn rpc(&mut self, method: &str, parameters: Value) -> SessionResult<Value> {
        self.check_running()?;
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":parameters});
        self.input
            .write_all(request.to_string().as_bytes())
            .await
            .map_err(|_| SessionError::Transport(TransportFailure::Write))?;
        self.input
            .write_all(b"\n")
            .await
            .map_err(|_| SessionError::Transport(TransportFailure::Write))?;
        self.input
            .flush()
            .await
            .map_err(|_| SessionError::Transport(TransportFailure::Write))?;
        for _ in 0..64 {
            let mut bytes = Vec::new();
            loop {
                let buffer = self
                    .output
                    .fill_buf()
                    .await
                    .map_err(|_| SessionError::Transport(TransportFailure::Read))?;
                if buffer.is_empty() {
                    return Err(SessionError::Transport(TransportFailure::Eof));
                }
                let length = buffer
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(buffer.len(), |index| index + 1);
                if bytes.len() + length > MAX_MCP_LINE {
                    return Err(SessionError::Transport(TransportFailure::Protocol));
                }
                bytes.extend_from_slice(&buffer[..length]);
                self.output.consume(length);
                if bytes.last() == Some(&b'\n') {
                    break;
                }
            }
            let response: Value = serde_json::from_slice(&bytes)
                .map_err(|_| SessionError::Transport(TransportFailure::Decode))?;
            if response["id"] != id {
                continue;
            }
            if let Some(error) = response.get("error").filter(|e| !e.is_null()) {
                let error = if error["data"].is_object() {
                    &error["data"]
                } else {
                    error
                };
                return Err(SessionError::Refused(refusal(error)));
            }
            return response
                .get("result")
                .cloned()
                .ok_or(SessionError::Transport(TransportFailure::Protocol));
        }
        Err(SessionError::Transport(TransportFailure::Protocol))
    }
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: tempfile::TempDir,
        driver: Driver,
    }

    impl Fixture {
        fn new(fault: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("driver");
            let script = r#"#!/usr/bin/env python3
import sys,json,os,time
from pathlib import Path
fault=FAULT
marker=Path(__file__+'.failed')
ready=Path(__file__+'.ready')
first=not marker.exists()
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r:continue
 method=r['method']
 result={'tools':[{'name':'list_windows'}]} if method=='tools/list' else {}
 if method=='tools/call':
  if fault in ('refusal','rpc_refusal'):
   code=-32601 if fault=='refusal' else -32099
   print(json.dumps({'jsonrpc':'2.0','id':r['id'],'error':{'code':code,'message':'refused'}}),flush=True)
   continue
  if fault.startswith('tool:'):result={'isError':True,'structuredContent':{'code':fault[5:]},'content':[{'type':'text','text':'Element index is stale; get_window_state again'}]}
  elif fault=='permission_without_is_error':result={'structuredContent':{'code':'permission_denied'}}
  elif fault in ('generic_refusal','numeric_tool_code'):
   result={'isError':True,'content':[{'type':'text','text':'Element index is stale; get_window_state again'}]}
   if fault=='numeric_tool_code':result['structuredContent']={'code':-32601}
  elif first:
   marker.touch()
   if fault=='truncated_json':os.write(1,b'{"jsonrpc":\n')
   if fault=='partial_eof':os.write(1,b'{"jsonrpc":')
   if fault in ('eof','partial_eof'):os.close(1)
   time.sleep(30)
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':result}),flush=True)
 if method=='tools/list' and first and fault=='broken_pipe':
  marker.touch()
  os.close(0)
  ready.touch()
  time.sleep(30)
"#.replace("FAULT", &serde_json::to_string(fault).unwrap());
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut driver = Driver::new(
                path,
                Identity::resolve(None).unwrap(),
                ComputerMode::Standard,
            );
            driver.without_capture_for_test();
            Self { root, driver }
        }

        async fn ready(&self) {
            self.driver.tools().await.unwrap();
        }

        async fn assert_restart(&self, error: DriverError) {
            assert_eq!(
                super::super::MachineError::Driver(error.clone()).public().0,
                12414
            );
            assert!(
                matches!(error, DriverError::Restarting { retry_after_ms } if retry_after_ms > 0)
            );
            assert!(self.driver.session.lock().await.is_none());
            assert!(self.driver.next_start.lock().unwrap().is_some());
            assert_eq!(self.driver.advertised_tools(), vec!["list_windows"]);
            assert!(self.driver.call("list_windows", json!({})).await.is_ok());
        }
    }

    #[tokio::test]
    async fn broken_pipe_on_write_restarts_with_12414() {
        let fixture = Fixture::new("broken_pipe");
        fixture.ready().await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !fixture.root.path().join("driver.ready").exists() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let error = fixture
            .driver
            .call("list_windows", json!({}))
            .await
            .unwrap_err();
        fixture.assert_restart(error).await;
    }

    #[tokio::test]
    async fn eof_before_response_restarts_with_12414() {
        let fixture = Fixture::new("eof");
        fixture.ready().await;
        let error = fixture
            .driver
            .call("list_windows", json!({}))
            .await
            .unwrap_err();
        fixture.assert_restart(error).await;
    }

    #[tokio::test]
    async fn truncated_json_line_restarts_with_12414() {
        let fixture = Fixture::new("truncated_json");
        fixture.ready().await;
        let error = fixture
            .driver
            .call("list_windows", json!({}))
            .await
            .unwrap_err();
        fixture.assert_restart(error).await;
    }

    #[tokio::test]
    async fn partial_line_before_eof_restarts_with_12414() {
        let fixture = Fixture::new("partial_eof");
        fixture.ready().await;
        let error = fixture
            .driver
            .call("list_windows", json!({}))
            .await
            .unwrap_err();
        fixture.assert_restart(error).await;
    }

    #[tokio::test]
    async fn already_exited_child_restarts_with_12414() {
        let fixture = Fixture::new("exited");
        fixture.ready().await;
        std::fs::write(fixture.root.path().join("driver.failed"), []).unwrap();
        fixture
            .driver
            .session
            .lock()
            .await
            .as_mut()
            .unwrap()
            ._child
            .kill()
            .await
            .unwrap();
        let error = fixture
            .driver
            .call("list_windows", json!({}))
            .await
            .unwrap_err();
        fixture.assert_restart(error).await;
    }

    #[tokio::test]
    async fn timeout_restarts_with_12414() {
        let fixture = Fixture::new("timeout");
        fixture.ready().await;
        let error = fixture
            .driver
            .with_session(
                Some(("list_windows", json!({}))),
                Some(Duration::from_millis(100)),
            )
            .await
            .unwrap_err();
        fixture.assert_restart(error).await;
    }

    #[tokio::test]
    async fn spawn_failure_restarts_with_12414() {
        let fixture = Fixture::new("normal");
        std::fs::remove_file(&fixture.driver.path).unwrap();
        let error = fixture
            .driver
            .call("list_windows", json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            super::super::MachineError::Driver(error.clone()).public().0,
            12414
        );
        assert!(matches!(error, DriverError::Restarting { retry_after_ms } if retry_after_ms > 0));
        assert!(fixture.driver.next_start.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn mcp_refusals_keep_the_session_and_specific_code() {
        for (fault, code) in [
            ("refusal", Some(12416)),
            ("rpc_refusal", Some(12406)),
            ("tool:permission_denied", Some(12415)),
            ("tool:accessibility_permission_denied", Some(12415)),
            ("tool:screen_recording_permission_denied", Some(12415)),
            ("permission_without_is_error", Some(12415)),
            ("tool:display_unavailable", Some(12417)),
            ("tool:display_not_found", Some(12417)),
            ("tool:unsupported_tool", Some(12416)),
            ("tool:tool_not_supported", Some(12416)),
            ("generic_refusal", None),
            ("tool:stale_element_index", None),
            ("numeric_tool_code", None),
        ] {
            let fixture = Fixture::new(fault);
            fixture.ready().await;
            let before = fixture
                .driver
                .session
                .lock()
                .await
                .as_ref()
                .unwrap()
                ._child
                .id();
            let result = fixture.driver.call("list_windows", json!({})).await;
            if let Some(code) = code {
                assert_eq!(
                    super::super::MachineError::Driver(result.unwrap_err())
                        .public()
                        .0,
                    code,
                    "{fault}"
                );
            } else {
                let mut expected = json!({
                    "isError": true,
                    "content": [{
                        "type": "text",
                        "text": "Element index is stale; get_window_state again"
                    }]
                });
                if let Some(code) = fault.strip_prefix("tool:") {
                    expected["structuredContent"] = json!({"code": code});
                } else if fault == "numeric_tool_code" {
                    expected["structuredContent"] = json!({"code": -32601});
                }
                assert_eq!(result.unwrap(), expected, "{fault}");
            }
            assert_eq!(
                fixture
                    .driver
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .unwrap()
                    ._child
                    .id(),
                before
            );
            assert!(fixture.driver.next_start.lock().unwrap().is_none());
        }
    }

    #[test]
    fn release_pins_cover_supported_platforms_and_exclude_perception() {
        for (os, arch) in [
            ("linux", "aarch64"),
            ("linux", "x86_64"),
            ("macos", "aarch64"),
            ("macos", "x86_64"),
        ] {
            let (url, hash) = platform_asset(os, arch).unwrap();
            assert!(url.contains("cua-driver-rs-v0.31.0"));
            assert_eq!(hex::decode(hash).unwrap().len(), 32);
        }
        assert!(!public_tool("parse_visual_regions"));
        assert!(!public_tool("shell"));
        assert!(public_tool("type_text"));
        assert!(read_only("get_desktop_state"));
        assert!(!read_only("click"));
    }
    #[tokio::test]
    async fn fake_stdio_driver_receives_telemetry_opt_out_and_public_calls_only() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("driver");
        std::fs::write(&path, r#"#!/usr/bin/env python3
import sys,json,os
assert os.environ['CUA_DRIVER_RS_TELEMETRY_ENABLED']=='false'
assert os.environ['CUA_DRIVER_RS_UPDATE_CHECK']=='false'
assert os.environ['CUA_DRIVER_PERMISSION_MODE']=='standard'
configured=False
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r: continue
 if r['method']=='tools/call' and r['params']['name']=='set_agent_cursor_motion':
  args=r['params']['arguments']
  assert args['session']=='nyxid-owner' and args['glide_duration_ms']==50 and args['dwell_after_click_ms']==0
  configured=True
 result={'tools':[{'name':'click'},{'name':'parse_visual_regions'},{'name':'set_agent_cursor_motion'}]} if r['method']=='tools/list' else {'content':[{'type':'text','text':'ok'}],'structuredContent':{'human_motion':configured}}
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':result}),flush=True)
"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let driver = Driver::new(
            path.clone(),
            Identity::resolve(None).unwrap(),
            ComputerMode::Standard,
        );
        // This fake child never captures a screen. Keep this transport unit
        // test independent of macOS volume management and desktop permissions.
        #[cfg(target_os = "macos")]
        let driver = Driver {
            memory_capture: false,
            ..driver
        };
        assert_eq!(
            driver.tools().await.unwrap(),
            vec!["click", "set_agent_cursor_motion"]
        );
        assert_eq!(
            driver.call("click", json!({})).await.unwrap()["structuredContent"]["human_motion"],
            false
        );
        assert!(
            driver
                .call("parse_visual_regions", json!({}))
                .await
                .is_err()
        );
        let owner = Driver::new(
            path,
            Identity::resolve(None).unwrap(),
            ComputerMode::Standard,
        )
        .for_human();
        #[cfg(target_os = "macos")]
        let owner = Driver {
            memory_capture: false,
            ..owner
        };
        assert_eq!(
            owner.call("click", json!({})).await.unwrap()["structuredContent"]["human_motion"],
            true
        );
    }
}

/// Keep actionable refs and visible labels before structural AX nodes. Never
/// duplicate the tree as markdown, and never expose an input's current value.
pub fn compact_window_state(mut result: Value) -> Value {
    if result["isError"] == true {
        return result;
    }
    let Some(state) = result.get_mut("structuredContent") else {
        return result;
    };
    let Some(rows) = state.get_mut("elements").and_then(Value::as_array_mut) else {
        return result;
    };
    let total = rows.len();
    rows.sort_by_key(|row| {
        let role = row["role"].as_str().unwrap_or_default().to_lowercase();
        if row["actions"].as_array().is_some_and(|a| !a.is_empty())
            || [
                "button",
                "link",
                "entry",
                "text field",
                "checkbox",
                "combo",
                "radio",
                "slider",
                "menu item",
            ]
            .iter()
            .any(|name| role.contains(name))
        {
            0
        } else if row["label"]
            .as_str()
            .is_some_and(|label| !label.trim().is_empty())
        {
            1
        } else {
            2
        }
    });
    let mut kept = Vec::new();
    let mut bytes = 0;
    for row in rows.iter() {
        let mut compact = json!({});
        for name in [
            "element_index",
            "element_token",
            "role",
            "label",
            "frame",
            "enabled",
            "selected",
            "actions",
        ] {
            if let Some(value) = row.get(name) {
                let value = match value {
                    Value::String(text) if name != "element_token" => {
                        json!(text.chars().take(200).collect::<String>())
                    }
                    _ => value.clone(),
                };
                compact[name] = value;
            }
        }
        let size = compact.to_string().len();
        if bytes + size > 6800 {
            continue;
        }
        bytes += size;
        kept.push(compact);
        if kept.len() >= 60 {
            break;
        }
    }
    let retained = kept.len();
    state["elements"] = json!(kept);
    state["returned_element_count"] = json!(retained);
    if total > retained {
        state["truncated"] = json!(true);
        state["hint"] = json!(
            "Narrow get_window_state with query for omitted elements; refs keep their original indices."
        );
    }
    if let Some(object) = state.as_object_mut() {
        object.remove("tree_markdown");
    }
    if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
        content.retain(|item| item["type"] != "text");
    }
    result
}
