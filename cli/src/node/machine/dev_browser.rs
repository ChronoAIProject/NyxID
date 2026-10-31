//! Supervisor-only CDP pipe and separate policy/profile for web development.
//! No saved-login extension, TCP debug port, or agent-visible pipe descriptor.
use super::{browser, process::Identity};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::{Child, Command},
};

const ACTIONS: &str = include_str!("../../../resources/machine-browser/browser-actions.js");
const MAX_CDP_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Refusal(&'static str);

/// Fixed, secret-free diagnostics. Never include a CDP payload or child stderr.
#[derive(Debug, Clone, Copy)]
pub enum Failure {
    Setup,
    RuntimeDirectory,
    ProfileDirectory,
    AlreadyRunning,
    Identity,
    #[cfg(target_os = "linux")]
    Display,
    Spawn,
    Exited,
    PipeWrite,
    PipeRead,
    PipeClosed,
    Protocol,
    Timeout,
}

impl Failure {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Setup => {
                "machine_browser_unavailable: developer browser setup is incomplete; repair managed browser policies and profile permissions"
            }
            Self::RuntimeDirectory => {
                "machine_browser_unavailable: developer browser runtime directory is unavailable; check supervisor directory ownership and permissions"
            }
            Self::ProfileDirectory => {
                "machine_browser_unavailable: developer browser profile directory initialization failed; check profile ownership and permissions or update the machine"
            }
            Self::AlreadyRunning => {
                "machine_browser_unavailable: another supervisor owns this developer browser; use the running daemon"
            }
            Self::Identity => {
                "machine_browser_unavailable: developer browser OS user is missing or cannot be used; rerun setup --separate-users or update the machine image"
            }
            #[cfg(target_os = "linux")]
            Self::Display => {
                "machine_browser_unavailable: developer display is unavailable; check Xvfb, openbox and its private Xauthority"
            }
            #[cfg(target_os = "linux")]
            Self::Spawn => {
                "machine_browser_unavailable: cannot start developer browser; install Chromium and, on native Linux, bubblewrap"
            }
            #[cfg(target_os = "macos")]
            Self::Spawn => {
                "machine_browser_unavailable: cannot start developer browser; install Google Chrome Beta and repair its managed policies"
            }
            Self::Exited => {
                "machine_browser_unavailable: developer browser exited; on native Linux check bubblewrap user-namespace permissions; restart the browser or update the machine image"
            }
            Self::PipeWrite => {
                "machine_browser_unavailable: developer browser pipe write failed; retry to start a fresh browser"
            }
            Self::PipeRead => {
                "machine_browser_unavailable: developer browser pipe read failed; retry to start a fresh browser"
            }
            Self::PipeClosed => {
                "machine_browser_unavailable: developer browser closed its pipe; on native Linux check bubblewrap user-namespace permissions; retry or update the machine image"
            }
            Self::Protocol => {
                "machine_browser_unavailable: invalid developer browser pipe response; restart the browser or update Chromium"
            }
            Self::Timeout => {
                "machine_browser_unavailable: developer browser timed out; observe before retrying the action"
            }
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Failure {}

pub struct DevBrowser {
    _lock: std::fs::File,
    child: Child,
    pipe: BufReader<UnixStream>,
    next_id: u64,
    selected: Option<String>,
    sessions: HashMap<String, String>,
    console: VecDeque<Value>,
    network: VecDeque<Value>,
}

impl Drop for DevBrowser {
    fn drop(&mut self) {
        if let Some(pid) = self.child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

impl DevBrowser {
    pub async fn launch(
        directory: &Path,
        identity: &Identity,
        secure_binary: &Path,
        container: bool,
    ) -> Result<Self> {
        Self::launch_inner(directory, identity, secure_binary, container)
            .await
            .map_err(|error| {
                let failure = error
                    .downcast_ref::<Failure>()
                    .copied()
                    .unwrap_or(Failure::Setup);
                browser::startup_failure(
                    "developer",
                    format_args!("{failure:?}"),
                    identity,
                    &error,
                );
                anyhow::Error::new(failure)
            })
    }

    async fn launch_inner(
        directory: &Path,
        identity: &Identity,
        secure_binary: &Path,
        container: bool,
    ) -> Result<Self> {
        if identity.uid == 0 {
            return Err(Failure::Identity.into());
        }
        browser::protected_runtime_parent(directory).context(Failure::RuntimeDirectory)?;
        let run = directory.join("dev-browser-run");
        browser::runtime_directory(&run, unsafe { libc::geteuid() }, identity.gid, 0o750)
            .context(Failure::RuntimeDirectory)?;
        let lock = browser::runtime_lock(&run).map_err(|error| {
            let failure = if matches!(
                error.downcast_ref::<browser::Failure>(),
                Some(browser::Failure::AlreadyRunning)
            ) {
                Failure::AlreadyRunning
            } else {
                Failure::RuntimeDirectory
            };
            error.context(failure)
        })?;
        let profile = directory.join("dev-browser-profile");
        browser::runtime_directory(&profile, identity.uid, identity.gid, 0o700)
            .context(Failure::ProfileDirectory)?;
        let (parent, child_pipe) = std::os::unix::net::UnixStream::pair()?;
        parent.set_nonblocking(true)?;
        // Reserve a descriptor above Chromium's fixed pipe slots before pre_exec.
        let fd = unsafe { libc::fcntl(child_pipe.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        #[cfg(target_os = "linux")]
        let mut command = if container || identity.desktop.is_some() {
            // Container policies use browser groups; native separated contexts
            // use supervisor-managed per-UID policy ACLs. Both use dedicated
            // users and private DAC gates without bwrap policy mounts, while
            // Chromium still establishes its own sandbox.
            Command::new(secure_binary)
        } else {
            let mut cmd = Command::new("bwrap");
            cmd.args([
                "--die-with-parent",
                "--unshare-user",
                "--ro-bind",
                "/",
                "/",
                "--bind",
            ])
            .arg(&identity.home)
            .arg(&identity.home)
            .arg("--bind")
            .arg(&profile)
            .arg(&profile)
            .args([
                "--bind",
                "/tmp",
                "/tmp",
                "--bind",
                "/proc",
                "/proc",
                "--dev-bind",
                "/dev",
                "/dev",
                "--ro-bind",
                "/opt/nyxid/machine-browser/dev-policies",
                &format!(
                    "/{}",
                    super::browser_layout::Layout::detect(secure_binary)?.policies()
                ),
                "--",
                "/bin/sh",
                "-c",
                "exec \"$@\" 3<&0 4>&1 </dev/null >/dev/null",
                "nyxid-dev-browser",
            ])
            .arg(secure_binary);
            cmd
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let _ = (secure_binary, container);
            let binary =
                Path::new("/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta");
            if !binary.is_file() {
                return Err(Failure::Spawn.into());
            }
            Command::new(binary)
        };
        identity.prepare(&mut command).context(Failure::Identity)?;
        identity.desktop_env(&mut command);
        if identity.desktop.is_none() {
            command
                .env_remove("DBUS_SESSION_BUS_ADDRESS")
                .env_remove("AT_SPI_BUS_ADDRESS");
        }
        #[cfg(target_os = "linux")]
        {
            if identity.desktop.is_none() {
                super::dev_display::ensure(identity)
                    .await
                    .context(Failure::Display)?;
                let (display, authority) =
                    super::dev_display::endpoint().context(Failure::Display)?;
                command.env("DISPLAY", display).env("XAUTHORITY", authority);
            }
            // The supervisor cannot traverse a worker-owned 0700 profile under
            // the formal capabilities. Remove an old copied X11 cookie as the
            // developer user; never take ownership of profile contents.
            clear_legacy_authority(&profile, identity).await?;
        }
        command
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--remote-debugging-pipe",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-sync",
                "--disable-breakpad",
                "--disable-crash-reporter",
                "--password-store=basic",
                "--window-size=1280,800",
                "about:blank",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(target_os = "linux")]
        let direct = container || identity.desktop.is_some();
        unsafe {
            command.pre_exec(move || {
                // Bubblewrap preserves stdio, but closes other descriptors.
                // Its fixed wrapper moves these to Chromium's CDP slots after
                // the namespace is established. Neither is exposed to jobs.
                #[cfg(target_os = "linux")]
                let (read_fd, write_fd) = if direct { (3, 4) } else { (0, 1) };
                #[cfg(target_os = "macos")]
                let (read_fd, write_fd) = (3, 4);
                if libc::dup2(fd.as_raw_fd(), read_fd) < 0
                    || libc::dup2(fd.as_raw_fd(), write_fd) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = spawn(&mut command)?;
        drop(command);
        drop(child_pipe);
        let mut browser = Self {
            _lock: lock,
            child,
            pipe: BufReader::new(UnixStream::from_std(parent)?),
            next_id: 0,
            selected: None,
            sessions: HashMap::new(),
            console: VecDeque::new(),
            network: VecDeque::new(),
        };
        tokio::time::timeout(
            Duration::from_secs(15),
            browser.rpc(None, "Browser.getVersion", json!({})),
        )
        .await
        .context(Failure::Timeout)??;
        Ok(browser)
    }

    fn record(&mut self, event: &Value) {
        let row = match event["method"].as_str() {
            Some("Runtime.consoleAPICalled") => {
                let args = event["params"]["args"]
                    .as_array()
                    .map(|args| {
                        args.iter()
                            .take(8)
                            .map(|v| {
                                let text = v.get("value").unwrap_or(&v["description"]).to_string();
                                text.chars().take(500).collect::<String>()
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Some((
                    &mut self.console,
                    json!({"type":event["params"]["type"],"args":args}),
                ))
            }
            Some("Network.requestWillBeSent") => Some((
                &mut self.network,
                json!({
                    "url":event["params"]["request"]["url"],"method":event["params"]["request"]["method"]
                }),
            )),
            Some("Network.responseReceived") => Some((
                &mut self.network,
                json!({
                    "url":event["params"]["response"]["url"],"status":event["params"]["response"]["status"]
                }),
            )),
            _ => None,
        };
        if let Some((queue, value)) = row {
            if value.to_string().len() < 4096 {
                queue.push_back(value);
            }
            while queue.len() > 32 {
                queue.pop_front();
            }
        }
    }

    async fn rpc(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value> {
        let result = self.rpc_inner(session, method, params).await;
        if let Err(error) = &result
            && let Some(reason) = error.downcast_ref::<Failure>()
        {
            let exit_status = self.child.try_wait().ok().flatten();
            tracing::warn!(?reason, ?exit_status, "developer browser transport failed");
        }
        result
    }

    /// Check the CDP child and pipe before a caller starts a business action.
    /// This is deliberately a read-only handshake; recovery callers may drop
    /// this instance and launch a fresh browser before submitting an action.
    pub(crate) async fn probe(&mut self) -> Result<()> {
        self.rpc(None, "Browser.getVersion", json!({}))
            .await
            .map(|_| ())
    }

    /// Kill and reap Chromium before a replacement uses the same profile.
    /// The Drop implementation remains a last-resort kill path for cancelled
    /// requests, while recovery waits for the child to release its locks.
    pub(crate) async fn terminate(mut self) -> Result<()> {
        if let Some(pid) = self.child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), self.child.wait())
            .await
            .context(Failure::Exited)?
            .context(Failure::Exited)?;
        Ok(())
    }

    async fn rpc_inner(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        if self.child.try_wait().context(Failure::Exited)?.is_some() {
            return Err(Failure::Exited.into());
        }
        self.next_id += 1;
        let id = self.next_id;
        let mut request = json!({"id":id,"method":method,"params":params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(0);
        self.pipe
            .get_mut()
            .write_all(&bytes)
            .await
            .context(Failure::PipeWrite)?;
        loop {
            let mut bytes = Vec::new();
            loop {
                let buffer = self.pipe.fill_buf().await.context(Failure::PipeRead)?;
                if buffer.is_empty() {
                    return Err(Failure::PipeClosed.into());
                }
                let size = buffer
                    .iter()
                    .position(|b| *b == 0)
                    .map_or(buffer.len(), |i| i + 1);
                if bytes.len() + size > MAX_CDP_BYTES {
                    return Err(Failure::Protocol.into());
                }
                bytes.extend_from_slice(&buffer[..size]);
                self.pipe.consume(size);
                if bytes.last() == Some(&0) {
                    bytes.pop();
                    break;
                }
            }
            let value: Value = serde_json::from_slice(&bytes).context(Failure::Protocol)?;
            if value["id"] == id {
                if value.get("error").is_some() {
                    bail!("developer browser operation refused");
                }
                return Ok(value["result"].clone());
            }
            self.record(&value);
        }
    }

    async fn target(&mut self, requested: Option<&str>) -> Result<(String, String, Vec<Value>)> {
        let result = self.rpc(None, "Target.getTargets", json!({})).await?;
        let mut tabs: Vec<Value> = result["targetInfos"]
            .as_array()
            .context("browser tabs unavailable")?
            .iter()
            .filter(|t| t["type"] == "page")
            .cloned()
            .collect();
        self.sessions
            .retain(|id, _| tabs.iter().any(|t| t["targetId"] == *id));
        if tabs.is_empty() {
            let created = self
                .rpc(None, "Target.createTarget", json!({"url":"about:blank"}))
                .await?;
            tabs.push(json!({"targetId":created["targetId"],"type":"page","url":"about:blank"}));
        }
        let id = select_target_id(requested, self.selected.as_deref(), &tabs)?;
        let session = if let Some(session) = self.sessions.get(&id) {
            session.clone()
        } else {
            if self.sessions.len() >= 24 {
                bail!("Close a developer browser tab before opening another");
            }
            let session = self
                .rpc(
                    None,
                    "Target.attachToTarget",
                    json!({"targetId":id,"flatten":true}),
                )
                .await?["sessionId"]
                .as_str()
                .context("browser session unavailable")?
                .to_owned();
            self.rpc(Some(&session), "Runtime.enable", json!({}))
                .await?;
            self.rpc(Some(&session), "Network.enable", json!({}))
                .await?;
            self.sessions.insert(id.clone(), session.clone());
            session
        };
        self.selected = Some(id.clone());
        Ok((id, session, tabs))
    }

    async fn frame_context(&mut self, session: &str, frame: &str) -> Result<(String, Value)> {
        let args = json!({"frameId":frame,"worldName":"nyxid-browser"});
        let (session, result) = match self
            .rpc(Some(session), "Page.createIsolatedWorld", args.clone())
            .await
        {
            Ok(value) => (session.to_owned(), value),
            Err(error) if error.downcast_ref::<Failure>().is_some() => return Err(error),
            Err(_) => {
                // Cross-site out-of-process iframes have their own CDP target.
                let child = if let Some(child) = self.sessions.get(frame) {
                    child.clone()
                } else {
                    anyhow::ensure!(
                        self.sessions.len() < 88,
                        "Browser frame session limit exceeded"
                    );
                    let attached = self
                        .rpc(
                            None,
                            "Target.attachToTarget",
                            json!({"targetId":frame,"flatten":true}),
                        )
                        .await?;
                    let child = attached["sessionId"]
                        .as_str()
                        .context("Child frame session unavailable")?
                        .to_owned();
                    self.sessions.insert(frame.to_owned(), child.clone());
                    child
                };
                let result = self
                    .rpc(Some(&child), "Page.createIsolatedWorld", args)
                    .await?;
                (child, result)
            }
        };
        Ok((session, result["executionContextId"].clone()))
    }

    async fn frame_action(&mut self, session: &str, frame: &str, request: &Value) -> Result<Value> {
        let (session, context) = self.frame_context(session, frame).await?;
        let expression = format!(
            "if (!globalThis.NyxIdBrowser) {{ {ACTIONS} }}; NyxIdBrowser.act({})",
            serde_json::to_string(request)?
        );
        let result = self.rpc(Some(&session),"Runtime.evaluate",json!({"expression":expression,"contextId":context,"awaitPromise":true,"returnByValue":true})).await?;
        if result.get("exceptionDetails").is_some() {
            let description = result["exceptionDetails"]["exception"]["description"]
                .as_str()
                .unwrap_or_default();
            let reason = description
                .strip_prefix("Error: ")
                .unwrap_or(description)
                .split([':', '\n'])
                .next()
                .unwrap_or_default();
            let reason = [
                "stale_ref",
                "element_disabled",
                "protected_input",
                "input_not_writable",
                "invalid_text",
                "input_refused",
                "invalid_option",
                "key_not_supported",
                "action_not_supported",
                "overlay_mismatch",
            ]
            .into_iter()
            .find(|known| *known == reason)
            .unwrap_or("browser_action_refused");
            return Err(Refusal(reason).into());
        }
        Ok(result["result"]["value"].clone())
    }

    async fn frame_point(
        &mut self,
        session: &str,
        frames: &[(String, Option<String>)],
        frame: &str,
        mut point: Value,
        scroll: bool,
    ) -> Result<Value> {
        let mut child = frame.to_owned();
        for _ in 0..16 {
            let Some(parent) = frames
                .iter()
                .find(|(id, _)| *id == child)
                .and_then(|(_, p)| p.clone())
            else {
                return Ok(point);
            };
            self.frame_action(session, &parent, &json!({"action":"_visibility"}))
                .await?;
            let (parent_session, context) = self.frame_context(session, &parent).await?;
            let owner = self
                .rpc(
                    Some(&parent_session),
                    "DOM.getFrameOwner",
                    json!({"frameId":child}),
                )
                .await?;
            let object = self
                .rpc(
                    Some(&parent_session),
                    "DOM.resolveNode",
                    json!({"backendNodeId":owner["backendNodeId"],"executionContextId":context}),
                )
                .await?;
            let mapped=self.rpc(Some(&parent_session),"Runtime.callFunctionOn",json!({
                "objectId":object["object"]["objectId"], "functionDeclaration":"function(point,scroll){return NyxIdBrowser.framePoint(this,point,scroll)}",
                "arguments":[{"value":point},{"value":scroll}],"returnByValue":true
            })).await?;
            if mapped.get("exceptionDetails").is_some() {
                return Err(Refusal("overlay_mismatch").into());
            }
            point = mapped["result"]["value"].clone();
            child = parent;
        }
        bail!("frame depth exceeded")
    }

    async fn page_action(&mut self, session: &str, request: &Value) -> Result<Value> {
        let tree = self
            .rpc(Some(session), "Page.getFrameTree", json!({}))
            .await?;
        fn collect(tree: &Value, parent: Option<String>, rows: &mut Vec<(String, Option<String>)>) {
            if rows.len() >= 64 {
                return;
            }
            let Some(id) = tree["frame"]["id"].as_str() else {
                return;
            };
            rows.push((id.to_owned(), parent));
            if let Some(children) = tree["childFrames"].as_array() {
                for child in children {
                    collect(child, Some(id.to_owned()), rows);
                }
            }
        }
        let mut frames = Vec::new();
        collect(&tree["frameTree"], None, &mut frames);
        // Chromium omits out-of-process children from Page.getFrameTree.
        // Join their browser-authored parentFrameId to this tab's known tree;
        // never include a target belonging to a different page.
        let targets = self.rpc(None, "Target.getTargets", json!({})).await?;
        if let Some(targets) = targets["targetInfos"].as_array() {
            for _ in 0..16 {
                let before = frames.len();
                for target in targets.iter().filter(|t| t["type"] == "iframe") {
                    let Some(id) = target["targetId"].as_str() else {
                        continue;
                    };
                    let Some(parent) = target["parentFrameId"].as_str() else {
                        continue;
                    };
                    if frames.len() >= 64
                        || frames.iter().any(|(f, _)| f == id)
                        || !frames.iter().any(|(f, _)| f == parent)
                    {
                        continue;
                    }
                    let Some((child_session, _)) =
                        optional_frame(self.frame_context(session, id).await)?
                    else {
                        continue;
                    };
                    let Some(child_tree) = optional_frame(
                        self.rpc(Some(&child_session), "Page.getFrameTree", json!({}))
                            .await,
                    )?
                    else {
                        continue;
                    };
                    collect(&child_tree["frameTree"], Some(parent.into()), &mut frames);
                }
                if frames.len() == before {
                    break;
                }
            }
        }
        let root = frames
            .first()
            .context("browser frame unavailable")?
            .0
            .clone();
        let parse = |value: &str| {
            value
                .strip_prefix('f')
                .and_then(|v| v.split_once(':'))
                .filter(|(frame, reference)| {
                    !reference.is_empty() && frames.iter().any(|(id, _)| id == frame)
                })
                .map(|(f, r)| (f.to_owned(), r.to_owned()))
                .ok_or_else(|| anyhow::Error::new(Refusal("stale_ref")))
        };
        let target = request["ref"].as_str().map(parse).transpose()?;
        let scope = request["scope"].as_str().map(parse).transpose()?;
        let mut local = request.clone();
        if let Some((_, r)) = &target {
            local["ref"] = json!(r);
        }
        if request["action"] != "snapshot" {
            let frame = target.as_ref().map(|(f, _)| f.as_str()).unwrap_or(&root);
            if request["action"] == "_prepare" {
                self.rpc(Some(session), "Page.bringToFront", json!({}))
                    .await?;
            }
            let mut value = self.frame_action(session, frame, &local).await?;
            if request["action"] == "_prepare" {
                value["point"] = self
                    .frame_point(session, &frames, frame, value["point"].clone(), true)
                    .await?;
                return Ok(value);
            }
            // Effects are not repeated when collecting the updated snapshot.
        }
        let offset = request["offset"].as_u64().unwrap_or(0).min(100000);
        let mut skip = offset;
        let mut total = 0;
        let mut result = json!({"elements":[],"frames":[],"text":"","headings":[],"ready":"complete","more":{"elements":false,"text":false,"frames":frames.len()==64}});
        // Internal observation correlation only; action_inner removes it from
        // the public snapshot. HTTP redirects retain the navigation loader.
        result["_loader_id"] = tree["frameTree"]["frame"]["loaderId"].clone();
        let mut full = false;
        for (frame, parent) in &frames {
            if scope.as_ref().is_some_and(|(f, _)| f != frame) {
                continue;
            }
            if parent.is_some() {
                let Some(point) = optional_frame(
                    self.frame_action(session, frame, &json!({"action":"_visibility"}))
                        .await,
                )?
                else {
                    continue;
                };
                if optional_frame(
                    self.frame_point(session, &frames, frame, point, false)
                        .await,
                )?
                .is_none()
                {
                    continue;
                }
            }
            let mut params = request.clone();
            params["action"] = json!("snapshot");
            params["offset"] = json!(skip);
            if let Some((_, ref_id)) = &scope {
                params["scope"] = json!(ref_id);
            }
            let Some(page) = optional_frame(self.frame_action(session, frame, &params).await)?
            else {
                continue;
            };
            if frame == &root {
                for key in ["url", "title", "ready"] {
                    result[key] = page[key].clone();
                }
            }
            let count = page["total"].as_u64().unwrap_or(0);
            total += count;
            skip = skip.saturating_sub(count);
            let metadata = json!({"id":format!("f{frame}"),"more":page["more"]});
            let metadata_index = result["frames"].as_array().unwrap().len();
            result["frames"].as_array_mut().unwrap().push(metadata);
            let has_metadata = result["frames"].to_string().len() <= 1200;
            if !has_metadata {
                result["frames"].as_array_mut().unwrap().pop();
                result["more"]["frames"] = json!(true);
            }
            if let Some(headings) = page["headings"].as_array() {
                for heading in headings {
                    result["headings"]
                        .as_array_mut()
                        .unwrap()
                        .push(heading.clone());
                    if result["headings"].to_string().len() > 600 || result.to_string().len() > 5500
                    {
                        result["headings"].as_array_mut().unwrap().pop();
                        break;
                    }
                }
            }
            let previous = result["text"].as_str().unwrap_or_default().to_owned();
            let text = format!("{previous} {}", page["text"].as_str().unwrap_or_default());
            result["text"] = json!(text.chars().take(1400).collect::<String>());
            if text.len() > 1400 || page["more"]["text"] == true || result.to_string().len() > 5500
            {
                result["more"]["text"] = json!(true);
                if result.to_string().len() > 5500 {
                    result["text"] = json!(previous);
                }
            }
            if let Some(elements) = page["elements"].as_array() {
                for element in elements {
                    if full {
                        if has_metadata {
                            result["frames"][metadata_index]["more"]["elements"] = json!(true);
                        }
                        break;
                    }
                    let mut element = element.clone();
                    element["ref"] = json!(format!(
                        "f{frame}:{}",
                        element["ref"].as_str().unwrap_or_default()
                    ));
                    result["elements"].as_array_mut().unwrap().push(element);
                    if result.to_string().len() > 5500
                        || result["elements"].as_array().unwrap().len() > 60
                    {
                        result["elements"].as_array_mut().unwrap().pop();
                        full = true;
                        if has_metadata {
                            result["frames"][metadata_index]["more"]["elements"] = json!(true);
                        }
                        break;
                    }
                }
            }
            if page["more"]["elements"] == true {
                full = true;
            }
        }

        let next = offset + result["elements"].as_array().unwrap().len() as u64;
        result["offset"] = json!(offset);
        result["total"] = json!(total);
        result["more"]["elements"] = json!(next < total);
        if next < total {
            result["next_offset"] = json!(next);
        }
        Ok(result)
    }

    pub async fn action(&mut self, request: &Value) -> Result<Value> {
        let result = tokio::time::timeout(Duration::from_secs(20), self.action_inner(request))
            .await
            .context(Failure::Timeout)?;
        match result {
            Err(error) if error.downcast_ref::<Refusal>().is_some() => {
                Ok(json!({"status":"refused","reason":error.downcast_ref::<Refusal>().unwrap().0}))
            }
            other => other,
        }
    }

    async fn action_inner(&mut self, request: &Value) -> Result<Value> {
        let action = request["action"]
            .as_str()
            .context("browser action required")?;
        let mut new_tab = None;
        if action == "tabs_new" {
            let url = web_url(request["url"].as_str().unwrap_or("https://example.com"))?;
            new_tab = self
                .rpc(None, "Target.createTarget", json!({"url":url}))
                .await?["targetId"]
                .as_str()
                .map(str::to_owned);
        }
        let (id, session, tabs) = self
            .target(new_tab.as_deref().or(request["tab_id"].as_str()))
            .await?;
        let mut extra = json!({});
        let mut navigation_loader = None;
        match action {
            "navigate" => {
                let navigation = self
                    .rpc(
                        Some(&session),
                        "Page.navigate",
                        json!({"url":web_url(request["url"].as_str().context("url required")?)?}),
                    )
                    .await?;
                navigation_loader = navigation["loaderId"].as_str().map(str::to_owned);
            }
            "back" | "forward" => {
                let history = self
                    .rpc(Some(&session), "Page.getNavigationHistory", json!({}))
                    .await?;
                let at = history["currentIndex"].as_i64().unwrap_or(0)
                    + if action == "back" { -1 } else { 1 };
                if let Some(entry) = at
                    .try_into()
                    .ok()
                    .and_then(|at: usize| history["entries"].get(at))
                {
                    self.rpc(
                        Some(&session),
                        "Page.navigateToHistoryEntry",
                        json!({"entryId":entry["id"]}),
                    )
                    .await?;
                }
            }
            "tabs_switch" => {
                self.rpc(None, "Target.activateTarget", json!({"targetId":id}))
                    .await?;
            }
            "tabs_close" => {
                self.rpc(None, "Target.closeTarget", json!({"targetId":id}))
                    .await?;
                self.selected = None;
                self.sessions.remove(&id);
                return Ok(
                    json!({"status":"ok","snapshot":{"closed":id,"tabs":tabs.into_iter().filter(|t|t["targetId"]!=id).collect::<Vec<_>>()}}),
                );
            }
            "evaluate" => {
                let expression = request["expression"]
                    .as_str()
                    .filter(|s| s.len() <= 16000)
                    .context("expression required (at most 16000 bytes)")?;
                let result = self
                    .rpc(
                        Some(&session),
                        "Runtime.evaluate",
                        json!({"expression":expression,"awaitPromise":true,"returnByValue":true}),
                    )
                    .await?;
                extra["evaluation"] = result;
            }
            "console" => extra["console"] = json!(self.console),
            "network" => extra["network"] = json!(self.network),
            "screenshot" => {
                let result = self
                    .rpc(
                        Some(&session),
                        "Page.captureScreenshot",
                        json!({"format":"jpeg","quality":75}),
                    )
                    .await?;
                extra["content"] =
                    json!([{"type":"image","mimeType":"image/jpeg","data":result["data"]}]);
            }
            _ => {}
        }
        let navigation = matches!(action, "navigate" | "back" | "forward" | "tabs_new");
        let content_action = if matches!(
            action,
            "navigate"
                | "back"
                | "forward"
                | "tabs"
                | "tabs_new"
                | "tabs_switch"
                | "wait"
                | "evaluate"
                | "console"
                | "network"
                | "screenshot"
        ) {
            "snapshot"
        } else {
            action
        };
        let mut parameters = request.clone();
        parameters["action"] = json!(content_action);
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(
                request["timeout_ms"]
                    .as_u64()
                    .unwrap_or(5000)
                    .clamp(100, 15000),
            );
        let mut snapshot = loop {
            match self.page_action(&session, &parameters).await {
                Ok(value)
                    if (!navigation || value["ready"] == "complete")
                        && (action != "navigate"
                            || (value["url"] != "about:blank"
                                && navigation_loader.as_ref().map_or_else(
                                    || value["url"] == request["url"],
                                    |loader| value["_loader_id"] == *loader,
                                )))
                        && (action != "wait"
                            || (value["ready"] == "complete"
                                && request["text"].as_str().is_none_or(|t| {
                                    value["text"].as_str().is_some_and(|s| s.contains(t))
                                }))) =>
                {
                    break value;
                }
                Ok(_) if navigation && tokio::time::Instant::now() >= deadline => {
                    return Err(Failure::Timeout.into());
                }
                Ok(value) if tokio::time::Instant::now() >= deadline => break value,
                Err(error) if error.downcast_ref::<Failure>().is_some() => return Err(error),
                Err(error)
                    if !navigation && action != "wait"
                        || tokio::time::Instant::now() >= deadline =>
                {
                    return Err(error);
                }
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        if let Some(snapshot) = snapshot.as_object_mut() {
            snapshot.remove("_loader_id");
        }
        extra["status"] = json!("ok");
        extra["snapshot"] = snapshot;
        extra["snapshot"]["tab_id"] = json!(id);
        let mut tab_rows = Vec::new();
        for tab in tabs {
            tab_rows.push(json!({"id":tab["targetId"], "title":tab["title"].as_str().unwrap_or_default().chars().take(80).collect::<String>(), "url":tab["url"].as_str().unwrap_or_default().chars().take(240).collect::<String>()}));
            if serde_json::to_vec(&tab_rows)?.len() > 1500 {
                tab_rows.pop();
                break;
            }
        }
        extra["snapshot"]["tabs"] = json!(tab_rows);
        if matches!(action, "click" | "type" | "select" | "press") {
            extra["input_mode"] = json!("dom_fallback");
        }
        Ok(extra)
    }
}

fn select_target_id(
    requested: Option<&str>,
    selected: Option<&str>,
    tabs: &[Value],
) -> Result<String> {
    if let Some(requested) = requested
        && !tabs.iter().any(|tab| tab["targetId"] == requested)
    {
        return Err(Refusal("stale_tab").into());
    }
    if requested.is_none()
        && let Some(selected) = selected
        && !tabs.iter().any(|tab| tab["targetId"] == selected)
    {
        return Err(Refusal("stale_tab").into());
    }
    requested
        .or(selected)
        .and_then(|id| tabs.iter().find(|tab| tab["targetId"] == id))
        .or_else(|| tabs.first())
        .context("developer browser has no tabs")?["targetId"]
        .as_str()
        .context("invalid tab")
        .map(str::to_owned)
}

// Detached/navigating frames can be skipped, but a dead pipe must reach the
// caller as browser_unavailable, never as a successful empty snapshot.
fn optional_frame<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.downcast_ref::<Failure>().is_some() => Err(error),
        Err(_) => Ok(None),
    }
}

fn spawn(command: &mut Command) -> Result<Child> {
    command.spawn().context(Failure::Spawn)
}

#[cfg(target_os = "linux")]
async fn clear_legacy_authority(profile: &Path, identity: &Identity) -> Result<()> {
    let mut command = Command::new("/bin/rm");
    identity.prepare(&mut command).context(Failure::Identity)?;
    let status = command
        .args(["-f", "--"])
        .arg(profile.join("Xauthority"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .context(Failure::ProfileDirectory)?;
    if !status.success() {
        browser::record_exit("developer", "legacy_display_cookie_cleanup", status);
        return Err(Failure::ProfileDirectory.into());
    }
    Ok(())
}

fn web_url(text: &str) -> Result<String> {
    let url = url::Url::parse(text)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("browser navigation requires an HTTP(S) URL without credentials");
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A CDP peer can acknowledge navigation while continuing to report an old
    // document, or disconnect after receiving it. This exercises the real
    // action/transport boundary without requiring a desktop or Chromium.
    fn simulated_browser(
        navigation_reply: &'static str,
    ) -> (
        DevBrowser,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
        tokio::task::JoinHandle<()>,
    ) {
        let (parent, peer) = UnixStream::pair().unwrap();
        let mut command = Command::new("/bin/sleep");
        Identity::resolve(None)
            .unwrap()
            .prepare(&mut command)
            .unwrap();
        let child = command.arg("30").spawn().unwrap();
        let browser = DevBrowser {
            _lock: tempfile::tempfile().unwrap(),
            child,
            pipe: BufReader::new(parent),
            next_id: 0,
            selected: None,
            sessions: HashMap::new(),
            console: VecDeque::new(),
            network: VecDeque::new(),
        };
        let navigations = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let count = navigations.clone();
        let task = tokio::spawn(async move {
            let mut peer = BufReader::new(peer);
            loop {
                let mut bytes = Vec::new();
                if peer.read_until(0, &mut bytes).await.unwrap() == 0 {
                    break;
                }
                bytes.pop();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "Target.getTargets" => {
                        json!({"targetInfos":[{"targetId":"current-tab","type":"page"}]})
                    }
                    "Target.attachToTarget" => json!({"sessionId":"current-session"}),
                    "Page.getFrameTree" => {
                        json!({"frameTree":{"frame":{"id":"current-frame","loaderId":if navigation_reply == "redirected" { "requested-loader" } else { "old-loader" }}}})
                    }
                    "Page.createIsolatedWorld" => json!({"executionContextId":1}),
                    "Runtime.evaluate" => {
                        json!({"result":{"value":{"url":match navigation_reply { "redirected" => "https://example.test/redirected", "old_document" => "https://example.test/old", _ => "about:blank" },"ready":"complete","elements":[],"headings":[],"text":"","total":0,"more":{}}}})
                    }
                    "Page.navigate" => {
                        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        match navigation_reply {
                            "disconnect" => break,
                            "pending" => continue,
                            _ => json!({"loaderId":"requested-loader"}),
                        }
                    }
                    _ => json!({}),
                };
                let mut response =
                    serde_json::to_vec(&json!({"id":request["id"],"result":result})).unwrap();
                response.push(0);
                peer.get_mut().write_all(&response).await.unwrap();
            }
        });
        (browser, navigations, task)
    }

    #[tokio::test]
    async fn navigation_never_reports_the_default_document_as_success() {
        for observation in ["stale_document", "old_document"] {
            let (mut browser, navigations, peer) = simulated_browser(observation);
            let result = browser
                .action(&json!({
                    "action":"navigate", "url":"https://example.test/requested", "timeout_ms":100
                }))
                .await;
            drop(browser);
            peer.await.unwrap();
            assert!(matches!(
                result.unwrap_err().downcast_ref::<Failure>(),
                Some(Failure::Timeout)
            ));
            assert_eq!(navigations.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
    }

    #[tokio::test]
    async fn navigation_accepts_a_redirect_from_its_own_document() {
        let (mut browser, navigations, peer) = simulated_browser("redirected");
        let result = browser
            .action(&json!({"action":"navigate","url":"https://example.test/requested"}))
            .await
            .unwrap();
        drop(browser);
        peer.await.unwrap();
        assert_eq!(result["status"], "ok");
        assert_eq!(result["snapshot"]["url"], "https://example.test/redirected");
        assert!(result["snapshot"].get("_loader_id").is_none());
        assert_eq!(navigations.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn scoped_snapshot_rejects_a_reference_from_an_old_browser() {
        let (mut browser, _, peer) = simulated_browser("stale_document");
        let result = browser
            .action(&json!({"action":"snapshot","scope":"fold-frame:r1"}))
            .await
            .unwrap();
        drop(browser);
        peer.await.unwrap();
        assert_eq!(result["status"], "refused");
        assert_eq!(result["reason"], "stale_ref");
    }

    #[tokio::test]
    async fn disconnect_after_navigation_submission_never_replays_it() {
        let (mut browser, navigations, peer) = simulated_browser("disconnect");
        let result = browser
            .action(&json!({"action":"navigate","url":"https://example.test/requested"}))
            .await;
        drop(browser);
        peer.await.unwrap();
        assert!(matches!(
            result.unwrap_err().downcast_ref::<Failure>(),
            Some(Failure::PipeClosed)
        ));
        assert_eq!(navigations.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn cancellation_discards_and_reaps_a_submitted_browser_action() {
        let (mut browser, navigations, peer) = simulated_browser("pending");
        let pid = browser.child.id().unwrap();
        let action = tokio::spawn(async move {
            browser
                .action(&json!({"action":"navigate","url":"https://example.test/requested"}))
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while navigations.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        action.abort();
        assert!(action.await.unwrap_err().is_cancelled());
        peer.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while unsafe { libc::kill(pid as i32, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("cancelled browser child must be reaped");
        assert_eq!(navigations.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires disposable hardened Linux supervisor"]
    async fn legacy_display_cookie_cleanup_uses_the_developer_identity() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
        let profile = root.path().join("dev-browser-profile");
        std::fs::create_dir(&profile).unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cookie = profile.join("Xauthority");
        std::fs::write(&cookie, b"obsolete-fixture-cookie").unwrap();
        std::os::unix::fs::chown(&profile, Some(1002), Some(1002)).unwrap();
        assert_eq!(
            std::fs::remove_file(&cookie).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let identity = Identity::resolve(Some("devbrowser")).unwrap();
        clear_legacy_authority(&profile, &identity).await.unwrap();
        clear_legacy_authority(&profile, &identity).await.unwrap();
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(&profile).unwrap();
        assert_eq!(
            (meta.uid(), meta.gid(), meta.mode() & 0o7777),
            (1002, 1002, 0o700)
        );
        std::os::unix::fs::chown(&profile, Some(0), Some(0)).unwrap();
        assert!(!cookie.exists());
    }

    fn assert_unavailable(error: anyhow::Error) {
        let error = super::super::MachineError::from(error);
        assert_eq!(error.public().0, 12413);
        assert!(error.public().1.starts_with("machine_browser_unavailable:"));
        assert!(!error.public().1.contains("secret-fixture"));
    }

    #[test]
    fn browser_failures_are_typed_and_do_not_expose_sources() {
        for failure in [
            Failure::Setup,
            Failure::RuntimeDirectory,
            Failure::ProfileDirectory,
            Failure::AlreadyRunning,
            Failure::Identity,
            Failure::Spawn,
            Failure::Exited,
            Failure::PipeRead,
            Failure::PipeWrite,
            Failure::PipeClosed,
            Failure::Protocol,
            Failure::Timeout,
        ] {
            assert_unavailable(
                anyhow::anyhow!("https://secret-fixture cookie=secret-fixture").context(failure),
            );
        }
        assert!(
            optional_frame::<()>(Err(anyhow::anyhow!("frame navigated")))
                .unwrap()
                .is_none()
        );
        assert_unavailable(optional_frame::<()>(Err(Failure::PipeRead.into())).unwrap_err());
    }

    #[tokio::test]
    async fn missing_browser_executable_is_unavailable() {
        let root = tempfile::tempdir().unwrap();
        assert_unavailable(
            spawn(&mut Command::new(root.path().join("missing-browser"))).unwrap_err(),
        );
    }

    #[tokio::test]
    async fn browser_pipe_failures_and_exited_child_are_unavailable() {
        for fault in ["write", "eof", "decode", "exited"] {
            let (parent, mut peer) = UnixStream::pair().unwrap();
            let mut command = Command::new("/bin/sleep");
            Identity::resolve(None)
                .unwrap()
                .prepare(&mut command)
                .unwrap();
            let child = command.arg("30").spawn().unwrap();
            let mut browser = DevBrowser {
                _lock: tempfile::tempfile().unwrap(),
                child,
                pipe: BufReader::new(parent),
                next_id: 0,
                selected: None,
                sessions: HashMap::new(),
                console: VecDeque::new(),
                network: VecDeque::new(),
            };
            match fault {
                "write" => drop(peer),
                "eof" => {
                    peer.shutdown().await.unwrap();
                    // Keep the read half open so the request write succeeds.
                    let result = browser.rpc(None, "Browser.getVersion", json!({})).await;
                    assert!(matches!(
                        result.as_ref().unwrap_err().downcast_ref::<Failure>(),
                        Some(Failure::PipeClosed)
                    ));
                    assert_unavailable(result.unwrap_err());
                    continue;
                }
                "decode" => peer.write_all(b"secret-fixture\0").await.unwrap(),
                "exited" => browser.child.kill().await.unwrap(),
                _ => unreachable!(),
            }
            let result = browser.rpc(None, "Browser.getVersion", json!({})).await;
            assert_unavailable(result.unwrap_err());
        }
    }

    #[test]
    fn explicit_tab_reference_is_rejected_when_the_tab_is_gone() {
        let tabs = vec![json!({"targetId":"current","type":"page"})];
        let error = select_target_id(Some("stale"), None, &tabs).unwrap_err();
        assert_eq!(
            error.downcast_ref::<Refusal>().map(|refusal| refusal.0),
            Some("stale_tab")
        );
    }

    #[test]
    fn stale_selected_tab_is_rejected_instead_of_remapping() {
        let tabs = vec![json!({"targetId":"current","type":"page"})];
        let error = select_target_id(None, Some("closed"), &tabs).unwrap_err();
        assert_eq!(
            error.downcast_ref::<Refusal>().map(|refusal| refusal.0),
            Some("stale_tab")
        );
    }

    #[test]
    fn absent_tab_reference_still_uses_the_selected_live_tab() {
        let tabs = vec![json!({"targetId":"current","type":"page"})];
        assert_eq!(
            select_target_id(None, Some("current"), &tabs).unwrap(),
            "current"
        );
    }

    #[test]
    fn navigation_refuses_local_files_and_credentials() {
        for url in [
            "file:///etc/passwd",
            "javascript:1",
            "https://user:pass@example.com",
        ] {
            assert!(web_url(url).is_err());
        }
        assert!(web_url("http://127.0.0.1:3000/test").is_ok());
        assert_eq!(
            browser::dev_policy()["NativeMessagingBlocklist"],
            json!(["*"])
        );
        assert!(
            browser::dev_policy()
                .get("ExtensionInstallForcelist")
                .is_none()
        );
    }
}
