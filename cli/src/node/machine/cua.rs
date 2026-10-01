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

pub const VERSION: &str = "0.30.4";
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

pub struct Driver {
    human_input: bool,
    #[cfg(target_os = "macos")]
    memory_capture: bool,
    path: PathBuf,
    identity: Identity,
    mode: ComputerMode,
    session: Mutex<Option<Session>>,
    cancelled: tokio::sync::watch::Sender<u64>,
    attempts: Mutex<Vec<Instant>>,
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
            attempts: Mutex::new(Vec::new()),
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

    async fn start(&self) -> Result<Session> {
        let mut attempts = self.attempts.lock().await;
        attempts.retain(|at| at.elapsed() < Duration::from_secs(60));
        if attempts.len() >= 3 {
            bail!("cua driver restart limit reached; retry after one minute");
        }
        attempts.push(Instant::now());
        drop(attempts);
        let mut command = Command::new(&self.path);
        self.identity.prepare(&mut command)?;
        for key in [
            "DISPLAY",
            "XAUTHORITY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .args(["mcp", "--direct"])
            .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false")
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
            Some(super::memory_capture::MemoryCapture::create().await?)
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        if let Some(capture) = &capture {
            command.env("TMPDIR", capture.path());
        }
        let mut child = command.spawn().context("cua driver unavailable")?;
        let input = child.stdin.take().context("cua stdin unavailable")?;
        let output = BufReader::new(child.stdout.take().context("cua stdout unavailable")?);
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
            .await?;
        let tools = session.rpc("tools/list", json!({})).await?;
        session.tools = tools["tools"]
            .as_array()
            .context("invalid cua tool list")?
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
                bail!("cua human cursor configuration unavailable");
            }
        }
        Ok(session)
    }

    pub async fn tools(&self) -> Result<Vec<String>> {
        let mut cancelled = self.cancelled.subscribe();
        tokio::select! {
            biased;
            _ = cancelled.changed() => bail!("cua initialization cancelled"),
            result = async {
                let mut session = self.session.lock().await;
                let mut active = session.take();
                if active.is_none() {
                    active = Some(tokio::time::timeout(Duration::from_secs(20), self.start()).await??);
                }
                let tools = active.as_ref().context("cua session unavailable")?.tools.clone();
                *session = active;
                Ok(tools)
            } => result,
        }
    }

    /// Local readiness probe only; never expose this diagnostic tool to an
    /// agent. In direct MCP mode it reports this process's real TCC attribution.
    #[cfg(target_os = "macos")]
    pub async fn permissions(&self) -> Result<nyxid_machine::ComputerPermissions> {
        let mut session = self.session.lock().await;
        let active = session.as_mut().context("cua session unavailable")?;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            active.rpc(
                "tools/call",
                json!({"name":"check_permissions","arguments":{"prompt":false}}),
            ),
        )
        .await??;
        Ok(nyxid_machine::ComputerPermissions {
            screen_recording: result["structuredContent"]["screen_recording"].as_bool(),
            accessibility: result["structuredContent"]["accessibility"].as_bool(),
        })
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        if !public_tool(name) {
            bail!("cua tool is outside the supported public contract");
        }
        let mut cancelled = self.cancelled.subscribe();
        let mut session = tokio::select! {
            biased;
            _ = cancelled.changed() => bail!("cua action cancelled"),
            session = self.session.lock() => session,
        };
        let mut active = session.take();
        let result = tokio::select! {
            biased;
            _ = cancelled.changed() => Err(anyhow::anyhow!("cua action cancelled")),
            result = async {
                if active.is_none() {
                    active = Some(tokio::time::timeout(Duration::from_secs(20), self.start()).await??);
                }
                let active = active.as_mut().context("cua session unavailable")?;
                if !active.tools.iter().any(|tool| tool == name) {
                    bail!("cua tool is not advertised on this platform");
                }
                tokio::time::timeout(Duration::from_secs(30), active.rpc("tools/call", json!({"name":name,"arguments":arguments}))).await?
            } => result,
        };
        if result.is_ok() {
            *session = active;
        }
        // The local session owns a kill_on_drop child. Dropping a cancelled
        // call kills it even when the outer operation future was dropped.
        result
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
    async fn rpc(&mut self, method: &str, parameters: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":parameters});
        self.input.write_all(request.to_string().as_bytes()).await?;
        self.input.write_all(b"\n").await?;
        self.input.flush().await?;
        for _ in 0..64 {
            let mut bytes = Vec::new();
            loop {
                let buffer = self.output.fill_buf().await?;
                if buffer.is_empty() {
                    bail!("cua driver closed its output");
                }
                let length = buffer
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(buffer.len(), |index| index + 1);
                if bytes.len() + length > MAX_MCP_LINE {
                    bail!("cua result size limit exceeded");
                }
                bytes.extend_from_slice(&buffer[..length]);
                self.output.consume(length);
                if bytes.last() == Some(&b'\n') {
                    break;
                }
            }
            let response: Value = serde_json::from_slice(&bytes).context("invalid cua response")?;
            if response["id"] != id {
                continue;
            }
            if response.get("error").is_some() {
                bail!("cua refused the request; check its permission mode and OS permissions");
            }
            return response
                .get("result")
                .cloned()
                .context("missing cua result");
        }
        bail!("too many cua notifications")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_pins_cover_supported_platforms_and_exclude_perception() {
        for (os, arch) in [
            ("linux", "aarch64"),
            ("linux", "x86_64"),
            ("macos", "aarch64"),
            ("macos", "x86_64"),
        ] {
            let (url, hash) = platform_asset(os, arch).unwrap();
            assert!(url.contains("cua-driver-rs-v0.30.4"));
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
