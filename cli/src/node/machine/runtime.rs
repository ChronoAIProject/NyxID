pub mod browser;
mod browser_input;
mod cancellation;
pub mod cua;
mod desktop;
#[cfg(all(test, target_os = "macos"))]
mod desktop_bench;
mod dev_browser;
#[cfg(target_os = "linux")]
mod dev_display;
mod files;
mod gateway;
mod jobs;
#[cfg(target_os = "macos")]
mod memory_capture;
mod native_desktop;
mod process;
pub mod transfer;
pub mod update;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use nyxid_machine::{
    MachineProfile, Operation, Request, config::Config, signing::ReplayGuard, text::Redactor,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

#[derive(Debug, thiserror::Error)]
enum MachineError {
    #[error("owner_in_control")]
    OwnerInControl,
    #[error("machine_turn_stopped")]
    TurnStopped,
    #[error("path_outside_roots")]
    PathOutsideRoots,
    #[error("job_not_found")]
    JobNotFound,
    #[error("computer unavailable")]
    Computer,
    #[error(transparent)]
    Driver(#[from] cua::DriverError),
    #[error("managed browser unavailable")]
    Browser,
    #[error("saved logins require the secure browser")]
    SecureBrowserRequired,
    #[error("machine operation refused")]
    Operation,
}

impl From<anyhow::Error> for MachineError {
    fn from(error: anyhow::Error) -> Self {
        if let Some(driver) = error.downcast_ref::<cua::DriverError>() {
            return Self::Driver(driver.clone());
        }
        error.downcast::<Self>().unwrap_or(Self::Operation)
    }
}

impl MachineError {
    fn public(&self) -> (u32, &'static str) {
        match self {
            Self::TurnStopped => (
                12418,
                "machine_turn_stopped: the owner stopped this turn; wait for a new turn",
            ),
            Self::OwnerInControl => (
                12408,
                "owner_in_control: wait for the owner to hand back control",
            ),
            Self::PathOutsideRoots => (12402, "path_outside_roots: choose a configured workspace"),
            Self::JobNotFound => (
                12403,
                "job_not_found: jobs expire after one hour or a daemon restart",
            ),
            Self::Computer => (
                12406,
                "computer unavailable or action refused; check cua permissions and mode",
            ),
            Self::Driver(cua::DriverError::Restarting { .. }) => (
                12414,
                "driver_restarting: retry after retry_after_ms; observe before repeating a changing action",
            ),
            Self::Driver(cua::DriverError::PermissionMissing) => (
                12415,
                "computer_permission_missing: enable Accessibility and Screen Recording for the node",
            ),
            Self::Driver(cua::DriverError::ToolUnsupported) => (
                12416,
                "computer_tool_not_supported: choose an advertised tool or the browser tools",
            ),
            Self::Driver(cua::DriverError::DisplayUnavailable) => (
                12417,
                "display_unavailable: start the machine desktop session",
            ),
            Self::Browser => (
                12413,
                "managed browser unavailable; complete browser policy setup and restart the node",
            ),
            Self::SecureBrowserRequired => (
                12413,
                "saved logins require the secure browser; use browser=secure",
            ),
            Self::Operation => (
                12407,
                "machine operation refused; check capability, path, input, limits and expected SHA-256",
            ),
        }
    }
}

pub struct Runtime {
    update_directory: PathBuf,
    upgrading: std::sync::atomic::AtomicBool,
    operation_admission: tokio::sync::RwLock<()>,
    config: Config,
    node_id: String,
    runtime_id: String,
    excluded: Vec<PathBuf>,
    roots: files::Roots,
    identity: process::Identity,
    jobs: Arc<jobs::Jobs>,
    gateway: tokio::sync::OnceCell<Arc<gateway::Gateway>>,
    driver: Option<cua::Driver>,
    owner_driver: Option<cua::Driver>,
    desktop: desktop::Desktop,
    dev_desktop: desktop::Desktop,
    desktop_secret: Mutex<Option<zeroize::Zeroizing<Vec<u8>>>>,
    browser: Mutex<Option<browser::Browser>>,
    dev_browser: Mutex<Option<dev_browser::DevBrowser>>,
    clipboard_files: Mutex<Vec<tempfile::NamedTempFile>>,
    replay: Mutex<ReplayGuard>,
    redactor: Arc<Mutex<Redactor>>,
    owner_control: tokio::sync::watch::Sender<u64>,
    dev_owner_control: tokio::sync::watch::Sender<u64>,
    turns: cancellation::Turns,
}

impl Runtime {
    pub fn new(config: &Config, node_id: &str, config_dir: &Path) -> Result<Arc<Self>> {
        config.validate().map_err(anyhow::Error::msg)?;
        let identity = process::Identity::resolve(config.agent_user.as_deref())?;
        let browser = process::Identity::resolve(config.browser_user.as_deref())?;
        if !config.allow_root
            && ((config.shell && identity.uid == 0) || (config.computer && browser.uid == 0))
        {
            bail!("machine shell/computer as root requires --allow-root");
        }
        let excluded = vec![
            config_dir.to_owned(),
            dirs::home_dir()
                .context("home unavailable")?
                .join(".nyxid-node"),
        ];
        let roots = files::Roots::new(&config.roots, &excluded)?;
        let driver = config
            .cua_driver
            .as_ref()
            .filter(|_| config.computer)
            .map(|path| cua::Driver::new(path.clone(), browser.clone(), config.computer_mode));
        let owner_driver = config
            .cua_driver
            .as_ref()
            .filter(|_| config.computer)
            .map(|path| cua::Driver::new(path.clone(), browser, config.computer_mode).for_human());
        let redactor = Arc::new(Mutex::new(Redactor::default()));
        Ok(Arc::new(Self {
            update_directory: update::directory(config, config_dir),
            upgrading: std::sync::atomic::AtomicBool::new(false),
            operation_admission: tokio::sync::RwLock::new(()),
            config: config.clone(),
            node_id: node_id.into(),
            runtime_id: uuid::Uuid::new_v4().to_string(),
            excluded,
            roots,
            identity,
            jobs: Arc::new(jobs::Jobs::new(config, redactor.clone())),
            gateway: tokio::sync::OnceCell::new(),
            driver,
            owner_driver,
            desktop: desktop::Desktop::default(),
            dev_desktop: desktop::Desktop::default(),
            desktop_secret: Mutex::new(None),
            browser: Mutex::new(None),
            dev_browser: Mutex::new(None),
            clipboard_files: Mutex::new(Vec::new()),
            replay: Mutex::new(ReplayGuard::default()),
            redactor,
            owner_control: tokio::sync::watch::channel(0).0,
            dev_owner_control: tokio::sync::watch::channel(0).0,
            turns: cancellation::Turns::default(),
        }))
    }

    pub async fn connect(
        self: &Arc<Self>,
        sender: tokio::sync::mpsc::Sender<crate::node::ws_client::NodeWsMessage>,
        signing_secret: &[u8],
    ) -> Result<()> {
        let gateway = self
            .gateway
            .get_or_try_init(|| {
                gateway::Gateway::start(
                    Arc::downgrade(&self.jobs),
                    self.node_id.clone(),
                    self.runtime_id.clone(),
                    zeroize::Zeroizing::new(signing_secret.to_vec()),
                )
            })
            .await?;
        gateway.connect(sender.clone()).await;
        *self.desktop.sender.lock().await = Some(sender.clone());
        *self.dev_desktop.sender.lock().await = Some(sender);
        *self.desktop_secret.lock().await = Some(zeroize::Zeroizing::new(signing_secret.to_vec()));
        if self.config.computer {
            self.start_capture(nyxid_machine::desktop::Display::Secure);
            self.start_capture(nyxid_machine::desktop::Display::Dev);
        }
        Ok(())
    }
    pub async fn disconnect(&self) {
        *self.desktop.sender.lock().await = None;
        *self.dev_desktop.sender.lock().await = None;
        if let Some(gateway) = self.gateway.get() {
            gateway.disconnect().await;
        }
    }
    pub async fn gateway_response(&self, id: &str, value: Value) {
        if let Some(gateway) = self.gateway.get() {
            gateway.response(id, value).await;
        }
    }
    pub async fn job_finished_ack(&self, request_id: &str) {
        if let Some(gateway) = self.gateway.get() {
            gateway.finished_ack(request_id).await;
        }
    }
    pub async fn binary(self: &Arc<Self>, bytes: &[u8]) {
        if let Ok(frame) = nyxid_machine::binary::Frame::decode(bytes) {
            if frame.kind == nyxid_machine::binary::Kind::Input {
                self.input_frame(frame).await;
            } else if let Some(gateway) = self.gateway.get() {
                gateway.chunk(frame).await;
            }
        }
    }

    pub async fn profile(&self) -> MachineProfile {
        let tools = if let Some(driver) = &self.driver {
            tokio::time::timeout(std::time::Duration::from_secs(20), driver.tools())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_else(|| driver.advertised_tools())
        } else {
            Vec::new()
        };
        #[cfg(target_os = "macos")]
        let computer_permissions = match &self.driver {
            Some(driver) => driver
                .permissions()
                .await
                .ok()
                .or_else(|| driver.last_permissions()),
            None => None,
        };
        #[cfg(not(target_os = "macos"))]
        let computer_permissions: Option<nyxid_machine::ComputerPermissions> = None;
        let computer_ready = computer_ready(&tools, computer_permissions.as_ref());
        let browser_identity = process::Identity::resolve(self.config.browser_user.as_deref());
        let isolated = browser_identity.is_ok_and(|browser| {
            self.identity.uid != 0
                && browser.uid != 0
                && self.identity.uid != browser.uid
                && self.identity.gid != browser.gid
                && unsafe { libc::geteuid() } == 0
        });
        let saved_login_ready = if self.config.computer && self.config.managed_browser.is_some() {
            self.ensure_browser().await.is_ok()
                && match self.browser.lock().await.as_ref() {
                    Some(browser) => browser.ready().await,
                    None => false,
                }
        } else {
            false
        };
        MachineProfile {
            installation: Some(update::installation(&self.config)),
            updater_ready: update::ready(&self.update_directory),
            version: nyxid_machine::PROTOCOL_VERSION,
            runtime_id: self.runtime_id.clone(),
            shell: self.config.shell,
            files: self.config.files,
            computer: self.config.computer,
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            roots: self
                .config
                .roots
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            computer_mode: self.config.computer_mode,
            cua_version: self.driver.as_ref().map(|_| cua::VERSION.into()),
            computer_ready,
            computer_permissions,
            computer_tools: tools,
            browser_isolated: isolated,
            commands_isolated: Some(self.identity.commands_isolated(&self.excluded).await),
            saved_login_ready,
            browser_tools: self.config.computer,
        }
    }

    fn desktop_for(&self, display: nyxid_machine::desktop::Display) -> &desktop::Desktop {
        match display {
            nyxid_machine::desktop::Display::Secure => &self.desktop,
            nyxid_machine::desktop::Display::Dev => &self.dev_desktop,
        }
    }
    fn control_for(
        &self,
        display: nyxid_machine::desktop::Display,
    ) -> &tokio::sync::watch::Sender<u64> {
        #[cfg(target_os = "macos")]
        {
            let _ = display;
            &self.owner_control
        }
        #[cfg(target_os = "linux")]
        match display {
            nyxid_machine::desktop::Display::Secure => &self.owner_control,
            nyxid_machine::desktop::Display::Dev => &self.dev_owner_control,
        }
    }
    fn control_channels(
        &self,
        operation: Operation,
        parameters: &Value,
    ) -> (
        &tokio::sync::watch::Sender<u64>,
        &tokio::sync::watch::Sender<u64>,
    ) {
        #[cfg(target_os = "macos")]
        {
            let _ = (operation, parameters);
            (&self.owner_control, &self.owner_control)
        }
        #[cfg(target_os = "linux")]
        match operation {
            Operation::Browser if parameters["browser"] == "dev" => {
                (&self.dev_owner_control, &self.dev_owner_control)
            }
            Operation::Browser | Operation::Computer | Operation::FillLogin => {
                (&self.owner_control, &self.owner_control)
            }
            _ => (&self.owner_control, &self.dev_owner_control),
        }
    }
    pub fn control_revision(&self, operation: Operation, parameters: &Value) -> u64 {
        let (a, b) = self.control_channels(operation, parameters);
        (*a.borrow() << 32) | *b.borrow()
    }

    pub async fn send_result(
        &self,
        sender: &tokio::sync::mpsc::Sender<crate::node::ws_client::NodeWsMessage>,
        request_id: &str,
        operation: Operation,
        revision: u64,
        parameters: &Value,
        mut result: Value,
    ) {
        let Ok(permit) = sender.reserve().await else {
            return;
        };
        // Reserve first: a full socket queue must not let an old result escape
        // after takeover. This short read guard spans only serialization and
        // nonblocking enqueue, never socket I/O.
        let scope = serde_json::from_value(parameters.clone()).unwrap_or_default();
        let stopped = self.turns.subscribe(&scope);
        let cancelled = stopped.borrow();
        let (a, b) = self.control_channels(operation, parameters);
        let a = a.borrow();
        let b = b.borrow();
        let current = (*a << 32) | *b;
        if !matches!(
            operation,
            Operation::Cancel
                | Operation::Upgrade
                | Operation::UpgradeStatus
                | Operation::DesktopControl
                | Operation::DesktopClose
                | Operation::DesktopOpen
                | Operation::DesktopInput
        ) && (current != revision || *cancelled)
        {
            let error = if *cancelled {
                MachineError::TurnStopped
            } else {
                MachineError::OwnerInControl
            };
            let (code, message) = error.public();
            result = json!({"error":{"code":code,"message":message}});
        }
        permit.send(crate::node::ws_client::NodeWsMessage::Text(
            json!({
                "type": "machine_result", "request_id": request_id, "result": result,
            })
            .to_string(),
        ));
    }

    pub fn updater_ready(&self) -> bool {
        update::ready(&self.update_directory)
    }

    pub fn report_connected(&self) {
        if update::connected(&self.update_directory).is_err() {
            tracing::warn!("Machine updater reconnect marker could not be written");
        }
    }

    pub async fn handle(&self, request: Request, signing_secret: &[u8]) -> Value {
        if self
            .replay
            .lock()
            .await
            .verify(
                &request,
                &self.node_id,
                signing_secret,
                chrono::Utc::now().timestamp(),
            )
            .is_err()
        {
            return json!({"error":{"code":12401,"message":"machine signature or replay check failed"}});
        }
        let agent_operation = !matches!(
            request.operation,
            Operation::Cancel
                | Operation::Upgrade
                | Operation::UpgradeStatus
                | Operation::DesktopControl
                | Operation::DesktopClose
                | Operation::DesktopOpen
                | Operation::DesktopInput
        );
        let (first, second) = self.control_channels(request.operation, &request.parameters);
        let revision = (*first.borrow(), *second.borrow());
        let result = self.execute(request.operation, request.parameters).await;
        match result {
            Ok(mut value) => {
                scrub_value(&mut value, &*self.redactor.lock().await);
                if agent_operation && (*first.borrow(), *second.borrow()) != revision {
                    let (code, message) = MachineError::OwnerInControl.public();
                    return json!({"error":{"code":code,"message":message}});
                }
                value
            }
            Err(error) => {
                let (code, message) = error.public();
                let mut result = json!({"error":{"code":code,"message":message}});
                if let MachineError::Driver(cua::DriverError::Restarting { retry_after_ms }) = error
                {
                    result["error"]["retry_after_ms"] = json!(retry_after_ms);
                }
                result
            }
        }
    }

    async fn execute(
        &self,
        operation: Operation,
        parameters: Value,
    ) -> std::result::Result<Value, MachineError> {
        let human = matches!(
            operation,
            Operation::Cancel
                | Operation::Upgrade
                | Operation::UpgradeStatus
                | Operation::DesktopControl
                | Operation::DesktopClose
                | Operation::DesktopOpen
                | Operation::DesktopInput
        );
        if human {
            return self
                .execute_inner(operation, parameters)
                .await
                .map_err(MachineError::from);
        }
        let _admission = self.operation_admission.read().await;
        if self.upgrading.load(std::sync::atomic::Ordering::Acquire) {
            return Err(MachineError::TurnStopped);
        }
        let scope: cancellation::Scope =
            serde_json::from_value(parameters.clone()).map_err(|_| MachineError::Operation)?;
        let mut stopped = self.turns.subscribe(&scope);
        if *stopped.borrow_and_update() {
            return Err(MachineError::TurnStopped);
        }
        let (first, second) = self.control_channels(operation, &parameters);
        let mut control = first.subscribe();
        let mut other_control = second.subscribe();
        let other_revision = *other_control.borrow_and_update();
        let revision = *control.borrow_and_update();
        if revision & 1 != 0 || other_revision & 1 != 0 {
            return Err(MachineError::OwnerInControl);
        }
        let result = tokio::select! {
            biased;
            _ = stopped.changed() => Err(MachineError::TurnStopped),
            _ = control.changed() => Err(MachineError::OwnerInControl),
            _ = other_control.changed() => Err(MachineError::OwnerInControl),
            result = self.execute_inner(operation, parameters) => result.map_err(MachineError::from),
        };
        if *stopped.borrow() {
            return Err(MachineError::TurnStopped);
        }
        // A completed operation may race takeover. Never deliver its late result.
        if *control.borrow() != revision || *other_control.borrow() != other_revision {
            return Err(MachineError::OwnerInControl);
        }
        result
    }

    async fn execute_inner(&self, operation: Operation, mut parameters: Value) -> Result<Value> {
        let profile = MachineProfile {
            version: nyxid_machine::PROTOCOL_VERSION,
            shell: self.config.shell,
            files: self.config.files,
            computer: self.config.computer,
            ..Default::default()
        };
        if !operation.allowed(&profile) {
            bail!("machine capability disabled locally");
        }
        match operation {
            Operation::ContainerInspect | Operation::ContainerMigrate => {
                update::container_operation(
                    &self.identity,
                    &self.config,
                    &parameters,
                    operation == Operation::ContainerMigrate,
                )
                .await
            }
            Operation::UpgradeStatus => {
                let progress = update::progress(&self.update_directory);
                if progress.as_ref().is_some_and(|p| p.phase.terminal()) {
                    self.upgrading
                        .store(false, std::sync::atomic::Ordering::Release);
                }
                Ok(
                    json!({"progress":progress,"updater_ready":update::ready(&self.update_directory)}),
                )
            }
            Operation::Upgrade => {
                let target = string(&parameters, "version")?;
                if self
                    .upgrading
                    .swap(true, std::sync::atomic::Ordering::AcqRel)
                {
                    return Ok(json!({"accepted":false,"reason":"update_in_progress"}));
                }
                let automatic = parameters["automatic"] == true;
                if automatic
                    && (self.owner_in_control()
                        || self.desktop.session.lock().await.is_some()
                        || self.dev_desktop.session.lock().await.is_some()
                        || self.jobs.any_running().await
                        || self.operation_admission.try_write().is_err())
                {
                    self.upgrading
                        .store(false, std::sync::atomic::Ordering::Release);
                    return Ok(json!({"accepted":false,"reason":"machine_busy"}));
                }
                if !automatic {
                    self.turns.stop(None);
                    if let Some(driver) = &self.driver {
                        driver.stop().await;
                    }
                    let ids = self.jobs.preempt_scope(None).await;
                    if let Some(gateway) = self.gateway.get() {
                        gateway.cancel_jobs(&ids).await;
                    }
                }
                let result = update::request(
                    &self.update_directory,
                    target,
                    parameters["owner_rollback"] == true,
                );
                if result.is_err() {
                    self.upgrading
                        .store(false, std::sync::atomic::Ordering::Release);
                    return Ok(
                        json!({"accepted":false,"reason":"updater_unavailable","note":"Open Assistant → Machines to install or repair the updater. No update was started."}),
                    );
                }
                Ok(json!({"accepted":true,"progress":update::progress(&self.update_directory)}))
            }
            Operation::Exec => {
                if string(&parameters, "runtime_id")? != self.runtime_id {
                    bail!("machine runtime changed; refresh machine list");
                }
                let gateway = self.gateway.get().context("machine gateway unavailable")?;
                let environment_spec: nyxid_machine::gateway::Environment = serde_json::from_value(
                    parameters.get("environment").cloned().unwrap_or(json!({})),
                )?;
                let environment = gateway
                    .environment(
                        string(&parameters, "job_id")?,
                        string(&parameters, "conversation_id")?,
                        &environment_spec,
                    )
                    .await?;
                let mut request: jobs::Exec = serde_json::from_value(parameters)?;
                let background = request.background;
                let id = request.job_id.clone();
                let timeout = request.timeout_secs.unwrap_or(120);
                request.background = true;
                let result = self
                    .jobs
                    .start(request, &self.identity, &self.roots, &environment)
                    .await;
                if result.is_err() {
                    gateway.remove_job(&id).await;
                }
                let result = result?;
                if background {
                    Ok(result)
                } else {
                    let result = self.jobs.foreground_result(&id, timeout + 5).await?;
                    if self.owner_in_control() {
                        return Err(MachineError::OwnerInControl.into());
                    }
                    Ok(result)
                }
            }
            Operation::ProxyUpload | Operation::ServiceCall | Operation::JobFinished => {
                bail!("service events are node initiated only")
            }
            Operation::Job => {
                let result = self
                    .jobs
                    .result(
                        string(&parameters, "job_id")?,
                        parameters["wait_secs"].as_u64().unwrap_or(0).min(60),
                        parameters["output_offset"].as_u64().unwrap_or(0),
                        parameters["stderr_offset"].as_u64().unwrap_or(0),
                    )
                    .await?;
                if self.owner_in_control() {
                    return Err(MachineError::OwnerInControl.into());
                }
                Ok(result)
            }
            Operation::JobCancel => self.jobs.cancel(string(&parameters, "job_id")?).await,
            Operation::ListFiles
            | Operation::ReadFile
            | Operation::WriteFile
            | Operation::EditFile => self.file_operation(operation, parameters).await,
            Operation::Computer => {
                self.ensure_browser().await?;
                let clipboard = parameters["tool"] == "clipboard_write";
                let window_state = parameters["tool"] == "get_window_state";
                let mut staged = Vec::new();
                // cua's output-file option bypasses NyxID's memory-only output path.
                if let Some(args) = parameters
                    .get_mut("arguments")
                    .and_then(Value::as_object_mut)
                {
                    args.remove("screenshot_out_file");
                    if window_state {
                        args.entry("include_screenshot").or_insert(json!(false));
                        args.entry("max_elements").or_insert(json!(800));
                        args.entry("timeout_ms").or_insert(json!(1200));
                    }
                    args.insert("session".into(), json!("nyxid-agent"));
                    if clipboard {
                        for key in ["file_path", "image_path"] {
                            if let Some(path) = args.get(key).and_then(Value::as_str) {
                                let file = self.stage_clipboard_file(path).await?;
                                args.insert(key.into(), json!(file.path()));
                                staged.push(file);
                            }
                        }
                    }
                }
                let result = self
                    .driver
                    .as_ref()
                    .context(MachineError::Computer)?
                    .call(
                        string(&parameters, "tool")?,
                        parameters.get("arguments").cloned().unwrap_or(json!({})),
                    )
                    .await?;
                if clipboard && result["isError"] != true {
                    // File clipboards may reference the path until the next copy.
                    *self.clipboard_files.lock().await = staged;
                }
                self.desktop_activity(
                    string(&parameters, "tool")?,
                    nyxid_machine::desktop::Display::Secure,
                )
                .await;
                Ok(if window_state {
                    cua::compact_window_state(result)
                } else {
                    result
                })
            }
            Operation::Browser => {
                let result = match parameters["browser"].as_str().unwrap_or("secure") {
                    "secure" => {
                        if matches!(
                            parameters["action"].as_str(),
                            Some("evaluate" | "console" | "network" | "screenshot")
                        ) {
                            bail!(
                                "This action requires browser=dev; secure never exposes DevTools"
                            );
                        }
                        self.ensure_browser().await.context(MachineError::Browser)?;
                        self.secure_browser_action(&parameters).await?
                    }
                    "dev" => {
                        let config = self
                            .config
                            .managed_browser
                            .as_ref()
                            .context(MachineError::Browser)?;
                        let mut guard = self.dev_browser.lock().await;
                        // Cancellation drops the pipe and process. A following turn
                        // starts a fresh session, never consumes a stale CDP reply.
                        let mut browser = match guard.take() {
                            Some(browser) => browser,
                            None => {
                                dev_browser::DevBrowser::launch(
                                    &config.data_dir,
                                    &process::Identity::resolve(
                                        self.config.effective_dev_browser_user(),
                                    )?,
                                    &config.binary,
                                )
                                .await?
                            }
                        };
                        let result = async {
                            let result = if matches!(
                                parameters["action"].as_str(),
                                Some("click" | "type" | "select" | "press")
                            ) && parameters["input_mode"] != "dom_fallback"
                            {
                                let mut probe = parameters.clone();
                                probe["input_action"] = parameters["action"].clone();
                                probe["action"] = json!("_prepare");
                                let prepared = browser.action(&probe).await?;
                                if prepared["status"] == "refused" {
                                    return Ok::<_, anyhow::Error>(prepared);
                                }
                                self.browser_native_input(
                                    &parameters,
                                    &prepared["snapshot"],
                                    nyxid_machine::desktop::Display::Dev,
                                )
                                .await?;
                                tokio::time::sleep(std::time::Duration::from_millis(35)).await;
                                let mut observe = parameters.clone();
                                observe["action"] = json!("snapshot");
                                let mut result = browser.action(&observe).await?;
                                result["input_mode"] = json!("trusted");
                                result
                            } else {
                                browser.action(&parameters).await?
                            };
                            Ok::<_, anyhow::Error>(result)
                        }
                        .await;
                        if result.is_ok() {
                            *guard = Some(browser);
                        }
                        result?
                    }
                    _ => bail!("Choose secure or dev browser"),
                };
                self.desktop_activity(
                    parameters["action"].as_str().unwrap_or("browser"),
                    if parameters["browser"] == "dev" {
                        nyxid_machine::desktop::Display::Dev
                    } else {
                        nyxid_machine::desktop::Display::Secure
                    },
                )
                .await;
                Ok(result)
            }
            Operation::Cancel => {
                let scope = if parameters["all"] == true {
                    None
                } else {
                    Some(serde_json::from_value::<cancellation::Scope>(
                        parameters.clone(),
                    )?)
                };
                if scope.is_none() {
                    // Include turns admitted by the server whose first signed
                    // operation is still in transit when the owner presses Stop.
                    if let Some(scopes) = parameters.get("scopes").and_then(Value::as_array) {
                        for value in scopes.iter().take(1024) {
                            let late: cancellation::Scope = serde_json::from_value(value.clone())?;
                            self.turns.stop(Some(&late));
                        }
                    }
                }
                self.turns.stop(scope.as_ref());
                // Driver stop is lock-free; jobs receive SIGKILL immediately.
                if scope.is_none()
                    && let Some(driver) = &self.driver
                {
                    driver.stop().await;
                }
                let jobs_scope = scope.as_ref().map(|s| cancellation::Scope {
                    conversation_id: s.conversation_id.clone(),
                    turn_id: String::new(),
                });
                let ids = self.jobs.preempt_scope(jobs_scope.as_ref()).await;
                if let Some(gateway) = self.gateway.get() {
                    gateway.cancel_jobs(&ids).await;
                }
                Ok(json!({"stopped":true}))
            }
            Operation::DesktopControl => {
                let display = nyxid_machine::desktop::Display::from_parameters(&parameters)
                    .map_err(anyhow::Error::msg)?;
                let desktop = self.desktop_for(display);
                let control = self.control_for(display);
                let owner = parameters["owner"]
                    .as_bool()
                    .context("invalid controller")?;
                #[cfg(target_os = "macos")]
                if owner {
                    let other =
                        self.desktop_for(if display == nyxid_machine::desktop::Display::Secure {
                            nyxid_machine::desktop::Display::Dev
                        } else {
                            nyxid_machine::desktop::Display::Secure
                        });
                    if other
                        .session
                        .lock()
                        .await
                        .as_ref()
                        .is_some_and(|s| s.controller.is_some())
                    {
                        return Err(MachineError::OwnerInControl.into());
                    }
                }
                let mut session_guard = desktop.session.lock().await;
                let session = session_guard.as_mut().context("desktop session closed")?;
                if parameters["session_id"] != session.id.to_string() {
                    bail!("desktop session mismatch");
                }
                let revision = parameters["revision"]
                    .as_u64()
                    .context("controller revision missing")?;
                if revision <= session.control_revision {
                    bail!("stale desktop controller");
                }
                if !owner && session.controller.as_deref() != parameters["viewer_id"].as_str() {
                    bail!("desktop controller changed");
                }
                let viewer = if owner {
                    Some(string(&parameters, "viewer_id")?.to_owned())
                } else {
                    None
                };
                // Flip admission and cancellation before any work that can wait.
                control.send_modify(|epoch| *epoch = ((*epoch >> 1) + 1) * 2 + 1);
                session.controller = viewer;
                session.control_revision = revision;
                session.budget.reset();
                session.frame_revision += 1;
                let text = std::mem::take(&mut session.owner_text);
                drop(session_guard);
                if owner {
                    if (cfg!(target_os = "macos")
                        || display == nyxid_machine::desktop::Display::Secure)
                        && let Some(driver) = &self.driver
                    {
                        driver.stop().await;
                    }
                    self.jobs.preempt().await;
                }
                if !text.is_empty() {
                    self.redactor
                        .lock()
                        .await
                        .register(&text)
                        .map_err(anyhow::Error::msg)?;
                }
                if !owner {
                    if (cfg!(target_os = "macos")
                        || display == nyxid_machine::desktop::Display::Secure)
                        && let Some(driver) = &self.owner_driver
                    {
                        driver.stop().await;
                    }
                    let session = desktop.session.lock().await;
                    if session.as_ref().is_none_or(|active| {
                        active.control_revision != revision || active.controller.is_some()
                    }) {
                        bail!("desktop controller changed during hand-back");
                    }
                    control.send_modify(|epoch| *epoch = ((*epoch >> 1) + 1) * 2);
                }
                Ok(json!({"controller":if owner{"owner"}else{"agent"}}))
            }
            Operation::DesktopClose => {
                let display = nyxid_machine::desktop::Display::from_parameters(&parameters)
                    .map_err(anyhow::Error::msg)?;
                let desktop = self.desktop_for(display);
                if *self.control_for(display).borrow() & 1 != 0 {
                    return Err(MachineError::OwnerInControl.into());
                }
                *desktop.session.lock().await = None;
                Ok(json!({"closed":true}))
            }
            Operation::DesktopOpen => self.desktop_open(&parameters).await,
            Operation::DesktopInput => self.desktop_input(&parameters).await,
            Operation::SaveAttachment => {
                self.file_operation(Operation::WriteFile, parameters).await
            }
            Operation::ShareFile => self.file_operation(Operation::ReadFile, parameters).await,
            Operation::FillLogin => {
                if parameters.get("browser").is_some_and(|v| v != "secure") {
                    return Err(MachineError::SecureBrowserRequired.into());
                }
                let value = match parameters["value"].take() {
                    Value::String(value) => zeroize::Zeroizing::new(value),
                    _ => bail!("missing saved-login value"),
                };
                let origins: Vec<String> =
                    serde_json::from_value(parameters["allowed_origins"].clone())?;
                let field = string(&parameters, "field")?;
                self.ensure_browser().await.context(MachineError::Browser)?;
                let browser = self.browser.lock().await;
                self.redactor
                    .lock()
                    .await
                    .register(&value)
                    .map_err(anyhow::Error::msg)?;
                browser
                    .as_ref()
                    .context(MachineError::Browser)?
                    .fill(field, &origins, &value)
                    .await
                    .context(MachineError::Browser)
            }
        }
    }

    fn owner_in_control(&self) -> bool {
        *self.owner_control.borrow() & 1 != 0 || *self.dev_owner_control.borrow() & 1 != 0
    }

    async fn ensure_dev_browser(&self) -> Result<()> {
        let config = self
            .config
            .managed_browser
            .as_ref()
            .context(MachineError::Browser)?;
        let mut browser = self.dev_browser.lock().await;
        if browser.is_none() {
            *browser = Some(
                dev_browser::DevBrowser::launch(
                    &config.data_dir,
                    &process::Identity::resolve(self.config.effective_dev_browser_user())?,
                    &config.binary,
                )
                .await?,
            );
        }
        Ok(())
    }

    async fn ensure_browser(&self) -> Result<()> {
        let Some(config) = self.config.managed_browser.as_ref() else {
            return Ok(());
        };
        let mut browser = self.browser.lock().await;
        if let Some(active) = browser.as_ref()
            && !active.alive().await
        {
            *browser = None;
        }
        if browser.is_none() {
            *browser = Some(
                browser::Browser::launch(
                    &config.data_dir,
                    &process::Identity::resolve(self.config.browser_user.as_deref())?,
                    &config.binary,
                    config.update_port,
                    config.container,
                )
                .await?,
            );
        }
        Ok(())
    }

    async fn file_operation(&self, operation: Operation, parameters: Value) -> Result<Value> {
        if operation == Operation::ReadFile {
            let mut parameters = parameters;
            parameters["scrub_context"] =
                serde_json::json!(self.redactor.lock().await.max_pattern_bytes());
            let request = FileRequest {
                roots: self.config.roots.clone(),
                excluded: self.excluded.clone(),
                operation,
                parameters: parameters.clone(),
            };
            let page = self.execute_file_worker(request).await?;
            let bytes = zeroize::Zeroizing::new(
                STANDARD.decode(
                    page["context"]
                        .as_str()
                        .context("file context unavailable")?,
                )?,
            );
            let skip = page["skip"].as_u64().unwrap_or(0) as usize;
            let count = page["count"].as_u64().unwrap_or(0) as usize;
            let clean = self
                .redactor
                .lock()
                .await
                .redact_window(&bytes, skip, skip + count);
            let encoding = parameters["encoding"].as_str().unwrap_or("text");
            let content = match encoding {
                "base64" => STANDARD.encode(&clean),
                "text" => String::from_utf8_lossy(&clean).into_owned(),
                _ => bail!("invalid encoding"),
            };
            return Ok(
                json!({"content":content,"encoding":encoding,"offset":page["offset"],"size":page["size"],"has_more":page["has_more"]}),
            );
        }
        let request = FileRequest {
            roots: self.config.roots.clone(),
            excluded: self.excluded.clone(),
            operation,
            parameters,
        };
        self.execute_file_worker(request).await
    }

    async fn execute_file_worker(&self, request: FileRequest) -> Result<Value> {
        // The backend transport test library runs inside nyxid-server's test
        // harness, not the CLI executable. Production always uses a cancellable
        // child, even when the command user is the supervisor's own user.
        #[cfg(feature = "node-proxy-test")]
        if self.identity.uid == unsafe { libc::geteuid() } {
            return tokio::task::spawn_blocking(move || execute_file(request)).await?;
        }
        {
            let mut command = tokio::process::Command::new(std::env::current_exe()?);
            self.identity.prepare_agent(&mut command)?;
            command
                .args(["node", "machine-worker"])
                .kill_on_drop(true)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            let mut child = command.spawn()?;
            let mut input = child
                .stdin
                .take()
                .context("file worker stdin unavailable")?;
            let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&request)?);
            input.write_all(&bytes).await?;
            input.shutdown().await?;
            drop(input);
            let stdout = child
                .stdout
                .take()
                .context("file worker stdout unavailable")?;
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                let mut output = zeroize::Zeroizing::new(Vec::new());
                stdout.take(256 * 1024 + 1).read_to_end(&mut output).await?;
                if output.len() > 256 * 1024 || !child.wait().await?.success() {
                    bail!("file operation refused");
                }
                let response: Value =
                    serde_json::from_slice(&output).context("invalid file worker response")?;
                if response["error"] == "path_outside_roots" {
                    return Err(MachineError::PathOutsideRoots.into());
                }
                if response.get("error").is_some() {
                    bail!("file operation refused");
                }
                Ok(response["result"].clone())
            })
            .await?
        }
    }

    pub async fn shutdown(&self) {
        self.jobs.cancel_all().await;
        if let Some(task) = self.desktop.capture.get() {
            task.abort();
        }
        if let Some(driver) = &self.driver {
            driver.stop().await;
        }
        if let Some(driver) = &self.owner_driver {
            driver.stop().await;
        }
    }
}

fn computer_ready(
    tools: &[String],
    permissions: Option<&nyxid_machine::ComputerPermissions>,
) -> bool {
    !tools.is_empty()
        && permissions
            .is_none_or(|p| p.screen_recording == Some(true) && p.accessibility == Some(true))
}

fn scrub_value(value: &mut Value, redactor: &Redactor) {
    match value {
        Value::String(text) => *text = redactor.redact(text),
        Value::Array(values) => values.iter_mut().for_each(|v| scrub_value(v, redactor)),
        Value::Object(values) => values.values_mut().for_each(|v| scrub_value(v, redactor)),
        _ => {}
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().context("missing string argument")
}

#[derive(Serialize, Deserialize)]
struct FileRequest {
    roots: Vec<PathBuf>,
    excluded: Vec<PathBuf>,
    operation: Operation,
    parameters: Value,
}

fn execute_file(request: FileRequest) -> Result<Value> {
    let roots = files::Roots::new(&request.roots, &request.excluded)?;
    let p = &request.parameters;
    let path = string(p, "path")?;
    match request.operation {
        Operation::ListFiles => roots.list(
            path,
            p["depth"].as_u64().unwrap_or(0) as usize,
            p["offset"].as_u64().unwrap_or(0) as usize,
            p["glob"].as_str(),
        ),
        Operation::ReadFile => roots.read_context(
            path,
            p["offset"].as_u64().unwrap_or(0),
            p["limit"].as_u64().unwrap_or(4096) as usize,
            p["scrub_context"].as_u64().unwrap_or(0) as usize,
        ),
        Operation::WriteFile => {
            let content = string(p, "content")?;
            if content.len() > 8 * 1024 * 1024 {
                bail!("file transfer size limit exceeded");
            }
            let bytes = if p["encoding"].as_str() == Some("base64") {
                STANDARD.decode(content)?
            } else {
                content.as_bytes().to_vec()
            };
            let hash = roots.write(
                path,
                &bytes,
                p["mode"].as_str().unwrap_or("create"),
                p["expected_sha256"].as_str(),
            )?;
            Ok(json!({"sha256":hash,"bytes":bytes.len()}))
        }
        Operation::EditFile => Ok(
            json!({"sha256":roots.edit(path,string(p,"old_string")?,string(p,"new_string")?,p["replace_all"].as_bool().unwrap_or(false),p["expected_sha256"].as_str())?}),
        ),
        _ => bail!("invalid file worker operation"),
    }
}

pub async fn worker() -> Result<()> {
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    tokio::io::stdin()
        .take(9 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 9 * 1024 * 1024 {
        bail!("file worker request limit exceeded");
    }
    let result = match execute_file(serde_json::from_slice(&bytes)?) {
        Ok(value) => json!({"result":value}),
        Err(error) => {
            let kind = MachineError::from(error);
            json!({"error": if matches!(kind, MachineError::PathOutsideRoots) {"path_outside_roots"} else {"operation_refused"}})
        }
    };
    tokio::io::stdout()
        .write_all(result.to_string().as_bytes())
        .await?;
    Ok(())
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[tokio::test]
    async fn queued_agent_results_are_fenced_at_enqueue_after_takeover() {
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(
            &Config {
                roots: vec![root.path().into()],
                ..Default::default()
            },
            &uuid::Uuid::new_v4().to_string(),
            &root.path().join("node"),
        )
        .unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        sender
            .send(crate::node::ws_client::NodeWsMessage::Text(
                "occupied".into(),
            ))
            .await
            .unwrap();
        let active = runtime.clone();
        let result = tokio::spawn(async move {
            active
                .send_result(
                    &sender,
                    "request",
                    Operation::ReadFile,
                    0,
                    &json!({}),
                    json!({"content":"late result"}),
                )
                .await;
        });
        tokio::task::yield_now().await;
        runtime.owner_control.send_replace(3);
        receiver.recv().await.unwrap();
        result.await.unwrap();
        let crate::node::ws_client::NodeWsMessage::Text(message) = receiver.recv().await.unwrap()
        else {
            panic!("expected result");
        };
        let value: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(value["result"]["error"]["code"], 12408);
        assert!(!message.contains("late result"));
    }

    #[test]
    fn runtime_errors_use_types_never_incidental_words() {
        for message in [
            "cua in a filename",
            "managed browser data",
            "owner_in_control",
            "job_not_found",
        ] {
            assert_eq!(
                MachineError::from(anyhow::anyhow!(message.to_owned()))
                    .public()
                    .0,
                12407
            );
        }
        assert_eq!(
            MachineError::from(
                anyhow::anyhow!("private diagnostic").context(MachineError::Computer)
            )
            .public()
            .0,
            12406
        );
    }

    #[tokio::test]
    async fn automatic_update_waits_for_owner_control_and_inflight_operations() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(
            &Config {
                shell: true,
                allow_root: true,
                roots: vec![root.path().into()],
                ..Default::default()
            },
            "node",
            root.path(),
        )
        .unwrap();
        std::fs::create_dir_all(&runtime.update_directory).unwrap();
        std::fs::set_permissions(
            &runtime.update_directory,
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(runtime.update_directory.join("heartbeat"), []).unwrap();
        let params = json!({"version":"99.0.0","automatic":true});
        runtime.owner_control.send_replace(1);
        assert_eq!(
            runtime
                .execute(Operation::Upgrade, params.clone())
                .await
                .unwrap()["reason"],
            "machine_busy"
        );
        runtime.owner_control.send_replace(2);
        let operation = runtime.operation_admission.read().await;
        assert_eq!(
            runtime
                .execute(Operation::Upgrade, params.clone())
                .await
                .unwrap()["reason"],
            "machine_busy"
        );
        assert!(!runtime.update_directory.join("request").exists());
        drop(operation);
        assert_eq!(
            runtime.execute(Operation::Upgrade, params).await.unwrap()["accepted"],
            true
        );
        assert_eq!(
            std::fs::read(runtime.update_directory.join("request")).unwrap(),
            b"99.0.0"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn takeover_cancels_a_thirty_second_cua_action_and_discards_its_result() {
        use std::{
            os::unix::fs::PermissionsExt,
            time::{Duration, Instant},
        };
        let root = tempfile::tempdir().unwrap();
        let driver = root.path().join("driver");
        let marker = root.path().join("started");
        std::fs::write(
            &driver,
            r#"#!/usr/bin/env python3
import sys,json,time,os
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r:continue
 if r['method']=='tools/list':result={'tools':[{'name':'click'}]}
 elif r['method']=='tools/call':
  open(r['params']['arguments']['marker'],'w').write(str(os.getpid()))
  time.sleep(30)
  result={'content':[{'type':'text','text':'late agent result'}]}
 else:result={}
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':result}),flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&driver, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut runtime = Runtime::new(
            &Config {
                computer: true,
                allow_root: true,
                cua_driver: Some(driver),
                roots: vec![root.path().into()],
                ..Default::default()
            },
            "node",
            &root.path().join("config"),
        )
        .unwrap();
        Arc::get_mut(&mut runtime)
            .unwrap()
            .driver
            .as_mut()
            .unwrap()
            .without_capture_for_test();
        let id = uuid::Uuid::new_v4().to_string();
        runtime
            .execute(Operation::DesktopOpen, json!({"session_id":id}))
            .await
            .unwrap();
        let task_runtime = runtime.clone();
        let marker_arg = marker.clone();
        let action = tokio::spawn(async move {
            task_runtime
                .execute(
                    Operation::Computer,
                    json!({"tool":"click","arguments":{"marker":marker_arg}}),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let start = Instant::now();
        runtime
            .execute(
                Operation::DesktopControl,
                json!({"session_id":id,"viewer_id":"owner","owner":true,"revision":1}),
            )
            .await
            .unwrap();
        let elapsed = start.elapsed();
        let takeover_budget = Duration::from_millis(
            if std::env::var("NYXID_MACHINE_STRICT_BENCHMARK").as_deref() == Ok("1") {
                150
            } else {
                2000
            },
        );
        assert!(elapsed <= takeover_budget, "takeover: {elapsed:?}");
        assert!(matches!(
            tokio::time::timeout(takeover_budget, action)
                .await
                .unwrap()
                .unwrap(),
            Err(MachineError::OwnerInControl)
        ));
        let pid = std::fs::read_to_string(marker)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        runtime.shutdown().await;
        println!(
            "takeover while cua sleeps 30 s: {:.3} ms",
            elapsed.as_secs_f64() * 1000.
        );
    }

    #[test]
    fn computer_requires_advertised_tools_and_both_known_macos_permissions() {
        let tools = vec!["get_desktop_state".into()];
        assert!(computer_ready(&tools, None));
        assert!(!computer_ready(&[], None));
        for screen_recording in [None, Some(false), Some(true)] {
            for accessibility in [None, Some(false), Some(true)] {
                assert_eq!(
                    computer_ready(
                        &tools,
                        Some(&nyxid_machine::ComputerPermissions {
                            screen_recording,
                            accessibility,
                        })
                    ),
                    screen_recording == Some(true) && accessibility == Some(true)
                );
            }
        }
    }

    #[tokio::test]
    async fn saved_login_without_managed_browser_returns_specific_error_without_value() {
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(
            &Config {
                computer: true,
                allow_root: true,
                roots: vec![root.path().into()],
                ..Default::default()
            },
            "node",
            &root.path().join("identity"),
        )
        .unwrap();
        let mut request = Request {
            request_id: uuid::Uuid::new_v4().to_string(),
            node_id: "node".into(),
            operation: Operation::FillLogin,
            parameters: json!({"value":"must-not-escape","field":"password","allowed_origins":["https://example.test"]}),
            timestamp: chrono::Utc::now().timestamp(),
            nonce: uuid::Uuid::new_v4().to_string(),
            signature: String::new(),
        };
        request.signature = nyxid_machine::signing::sign(&request, &[7; 32]);
        let result = runtime.handle(request, &[7; 32]).await;
        assert_eq!(result["error"]["code"], 12413);
        assert!(!result.to_string().contains("must-not-escape"));

        let result = runtime
            .execute(
                Operation::FillLogin,
                json!({"browser":"dev","value":"must-not-escape","field":"password"}),
            )
            .await
            .unwrap_err();
        assert_eq!(result.public().0, 12413);
        assert!(result.public().1.contains("browser=secure"));
        assert!(!result.to_string().contains("must-not-escape"));
    }
}
