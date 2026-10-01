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
                tracing::warn!(reason = ?failure, "developer browser launch failed");
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
        browser::protected_runtime_parent(directory)?;
        let profile = directory.join("dev-browser-profile");
        browser::runtime_directory(&profile, identity.uid, identity.gid, 0o700)?;
        let (parent, child_pipe) = std::os::unix::net::UnixStream::pair()?;
        parent.set_nonblocking(true)?;
        // Reserve a descriptor above Chromium's fixed pipe slots before pre_exec.
        let fd = unsafe { libc::fcntl(child_pipe.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        #[cfg(target_os = "linux")]
        let mut command = if container {
            // Container policy files are readable only by their browser group.
            // Docker/AppArmor already supplies the mount boundary; no bwrap mount
            // is needed or permitted. Chromium still establishes its own sandbox.
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
                "/etc/chromium/policies/managed",
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
        command
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env_remove("AT_SPI_BUS_ADDRESS");
        #[cfg(target_os = "linux")]
        {
            super::dev_display::ensure(identity)
                .await
                .context(Failure::Display)?;
            let (display, authority) = super::dev_display::endpoint().context(Failure::Display)?;
            command.env("DISPLAY", display).env("XAUTHORITY", authority);
            // Remove the cookie copied by older versions. Never share X11 trust.
            match std::fs::remove_file(profile.join("Xauthority")) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
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
        unsafe {
            command.pre_exec(move || {
                // Bubblewrap preserves stdio, but closes other descriptors.
                // Its fixed wrapper moves these to Chromium's CDP slots after
                // the namespace is established. Neither is exposed to jobs.
                #[cfg(target_os = "linux")]
                let (read_fd, write_fd) = if container { (3, 4) } else { (0, 1) };
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
        let id = requested
            .or(self.selected.as_deref())
            .and_then(|id| tabs.iter().find(|t| t["targetId"] == id))
            .or_else(|| tabs.first())
            .context("developer browser has no tabs")?["targetId"]
            .as_str()
            .context("invalid tab")?
            .to_owned();
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
                .map(|(f, r)| (f.to_owned(), r.to_owned()))
        };
        let target = request["ref"].as_str().and_then(parse);
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
        let scope = request["scope"].as_str().and_then(parse);
        let offset = request["offset"].as_u64().unwrap_or(0).min(100000);
        let mut skip = offset;
        let mut total = 0;
        let mut result = json!({"elements":[],"frames":[],"text":"","headings":[],"ready":"complete","more":{"elements":false,"text":false,"frames":frames.len()==64}});
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
        match action {
            "navigate" => {
                self.rpc(
                    Some(&session),
                    "Page.navigate",
                    json!({"url":web_url(request["url"].as_str().context("url required")?)?}),
                )
                .await?;
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
        let snapshot = loop {
            match self.page_action(&session, &parameters).await {
                Ok(value)
                    if (!navigation || value["ready"] == "complete")
                        && (action != "navigate" || value["url"] == request["url"])
                        && (action != "wait"
                            || (value["ready"] == "complete"
                                && request["text"].as_str().is_none_or(|t| {
                                    value["text"].as_str().is_some_and(|s| s.contains(t))
                                }))) =>
                {
                    break value;
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
