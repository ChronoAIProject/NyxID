mod authority;
pub mod browser;
mod browser_input;
mod cancellation;
#[cfg(target_os = "linux")]
mod context_dispatch;
#[cfg(target_os = "linux")]
mod context_policy;
#[cfg(target_os = "linux")]
pub mod context_runtime;
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
    collections::VecDeque,
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
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

const OPERATION_RECEIPT_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const OPERATION_RECEIPT_RUNNING_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const OPERATION_RECEIPT_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
// A desktop stream can legitimately issue thousands of signed input requests
// in one turn. Complete receipts are FIFO-evicted; only in-flight operations
// can refuse admission.
const OPERATION_RECEIPT_LIMIT: usize = 4096;
const OPERATION_RECEIPT_RUNNING_LIMIT: usize = 4096;
const OPERATION_RECEIPT_RESULT_LIMIT: usize = 256 * 1024;
const OPERATION_RECEIPT_RESULT_BUDGET: usize = 32 * 1024 * 1024;

enum ReceiptState {
    Running(Arc<tokio::sync::Notify>),
    Complete { result: Value, bytes: usize },
}

struct OperationReceipt {
    fingerprint: [u8; 32],
    state: ReceiptState,
    expires_at: std::time::Instant,
}

#[derive(Default)]
struct OperationReceipts {
    entries: Mutex<ReceiptLedger>,
    replay_deliveries: Mutex<std::collections::HashMap<String, usize>>,
}

#[derive(Default)]
struct ReceiptLedger {
    entries: std::collections::HashMap<String, OperationReceipt>,
    completed: VecDeque<String>,
    result_bytes: usize,
}

enum ReceiptDecision {
    Execute,
    Wait(Arc<tokio::sync::Notify>),
    Complete(Value),
    Conflict,
    Capacity,
}

enum ReceiptWaitOutcome {
    Complete(Value),
    Conflict,
    Pending,
}

fn unknown_receipt_result() -> Value {
    json!({"error":{"code":12407,"message":"machine operation outcome unknown; observe before retrying"}})
}

fn bounded_receipt_result(result: Value) -> (Value, usize) {
    let bytes = serde_json::to_vec(&result).map_or(usize::MAX, |bytes| bytes.len());
    if bytes <= OPERATION_RECEIPT_RESULT_LIMIT {
        return (result, bytes);
    }
    let result = json!({"error":{"code":12407,"message":"machine operation completed; response exceeded the replay limit; observe before retrying"}});
    let bytes = serde_json::to_vec(&result).map_or(usize::MAX, |bytes| bytes.len());
    (result, bytes)
}

impl OperationReceipts {
    async fn begin(&self, request_id: &str, fingerprint: [u8; 32]) -> ReceiptDecision {
        let mut ledger = self.entries.lock().await;
        let now = std::time::Instant::now();
        ledger.sweep(now);
        match ledger.entries.get(request_id) {
            Some(receipt) if receipt.fingerprint != fingerprint => ReceiptDecision::Conflict,
            Some(OperationReceipt {
                state: ReceiptState::Complete { result, .. },
                ..
            }) => ReceiptDecision::Complete(result.clone()),
            Some(OperationReceipt {
                state: ReceiptState::Running(notify),
                ..
            }) => ReceiptDecision::Wait(notify.clone()),
            None if ledger.running_count() >= OPERATION_RECEIPT_RUNNING_LIMIT => {
                ReceiptDecision::Capacity
            }
            None => {
                ledger.evict_completed_until_admitted();
                if ledger.entries.len() >= OPERATION_RECEIPT_LIMIT {
                    ReceiptDecision::Capacity
                } else {
                    ledger.entries.insert(
                        request_id.to_owned(),
                        OperationReceipt {
                            fingerprint,
                            state: ReceiptState::Running(Arc::new(tokio::sync::Notify::new())),
                            expires_at: now + OPERATION_RECEIPT_RUNNING_TTL,
                        },
                    );
                    ReceiptDecision::Execute
                }
            }
        }
    }

    async fn complete(&self, request_id: &str, result: Value) {
        let notify = {
            let mut ledger = self.entries.lock().await;
            ledger.sweep(std::time::Instant::now());
            let Some(receipt) = ledger.entries.get_mut(request_id) else {
                return;
            };
            let ReceiptState::Running(notify) = &receipt.state else {
                return;
            };
            let notify = notify.clone();
            let (result, bytes) = bounded_receipt_result(result);
            receipt.state = ReceiptState::Complete { result, bytes };
            receipt.expires_at = std::time::Instant::now() + OPERATION_RECEIPT_TTL;
            ledger.result_bytes = ledger.result_bytes.saturating_add(bytes);
            ledger.completed.push_back(request_id.to_owned());
            ledger.evict_completed_for_budget(Some(request_id));
            notify
        };
        notify.notify_waiters();
    }

    async fn wait(
        &self,
        request_id: &str,
        fingerprint: [u8; 32],
        notify: Arc<tokio::sync::Notify>,
    ) -> ReceiptWaitOutcome {
        let deadline = tokio::time::Instant::now() + OPERATION_RECEIPT_WAIT_TIMEOUT;
        loop {
            // Register before checking state. If completion races the check,
            // Notify retains a permit and the waiter cannot miss the wakeup.
            let notified = notify.notified();
            let state = {
                let mut ledger = self.entries.lock().await;
                ledger.sweep(std::time::Instant::now());
                match ledger.entries.get(request_id) {
                    Some(receipt) if receipt.fingerprint != fingerprint => {
                        ReceiptWaitOutcome::Conflict
                    }
                    Some(OperationReceipt {
                        state: ReceiptState::Complete { result, .. },
                        ..
                    }) => ReceiptWaitOutcome::Complete(result.clone()),
                    Some(OperationReceipt {
                        state: ReceiptState::Running(_),
                        ..
                    }) => ReceiptWaitOutcome::Pending,
                    None => ReceiptWaitOutcome::Complete(unknown_receipt_result()),
                }
            };
            match state {
                ReceiptWaitOutcome::Pending => {}
                outcome => return outcome,
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return ReceiptWaitOutcome::Complete(unknown_receipt_result());
            }
        }
    }

    #[cfg(test)]
    async fn result_bytes(&self) -> usize {
        self.entries.lock().await.result_bytes
    }

    async fn mark_replay(&self, request_id: &str) {
        let mut deliveries = self.replay_deliveries.lock().await;
        let count = deliveries.entry(request_id.to_owned()).or_default();
        *count = count.saturating_add(1);
    }

    async fn take_replay(&self, request_id: &str) -> bool {
        let mut deliveries = self.replay_deliveries.lock().await;
        let Some(count) = deliveries.get_mut(request_id) else {
            return false;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            deliveries.remove(request_id);
        }
        true
    }
}

impl ReceiptLedger {
    fn running_count(&self) -> usize {
        self.entries
            .values()
            .filter(|receipt| matches!(receipt.state, ReceiptState::Running(_)))
            .count()
    }

    fn sweep(&mut self, now: std::time::Instant) {
        let mut expired_running = Vec::new();
        let mut expired_complete = Vec::new();
        for (request_id, receipt) in &self.entries {
            if receipt.expires_at > now {
                continue;
            }
            if matches!(receipt.state, ReceiptState::Running(_)) {
                expired_running.push(request_id.clone());
            } else {
                expired_complete.push(request_id.clone());
            }
        }
        for request_id in expired_running {
            let notify = match self.entries.get_mut(&request_id) {
                Some(receipt) => {
                    let ReceiptState::Running(notify) = &receipt.state else {
                        continue;
                    };
                    let notify = notify.clone();
                    let (result, bytes) = bounded_receipt_result(unknown_receipt_result());
                    receipt.state = ReceiptState::Complete { result, bytes };
                    receipt.expires_at = now + OPERATION_RECEIPT_TTL;
                    self.result_bytes = self.result_bytes.saturating_add(bytes);
                    self.completed.push_back(request_id.clone());
                    Some(notify)
                }
                None => None,
            };
            if let Some(notify) = notify {
                notify.notify_waiters();
            }
        }
        for request_id in expired_complete {
            self.remove_complete(&request_id);
        }
        self.evict_completed_for_budget(None);
    }

    fn evict_completed_until_admitted(&mut self) {
        while self.entries.len() >= OPERATION_RECEIPT_LIMIT {
            let Some(request_id) = self.completed.pop_front() else {
                break;
            };
            self.remove_complete(&request_id);
        }
    }

    fn evict_completed_for_budget(&mut self, protected: Option<&str>) {
        let mut inspected = 0;
        while self.result_bytes > OPERATION_RECEIPT_RESULT_BUDGET {
            let Some(request_id) = self.completed.pop_front() else {
                break;
            };
            if protected == Some(request_id.as_str()) {
                self.completed.push_back(request_id);
                inspected += 1;
                if inspected >= self.completed.len() {
                    break;
                }
                continue;
            }
            inspected = 0;
            self.remove_complete(&request_id);
        }
    }

    fn remove_complete(&mut self, request_id: &str) {
        let Some(receipt) = self.entries.remove(request_id) else {
            return;
        };
        if let ReceiptState::Complete { bytes, .. } = receipt.state {
            self.result_bytes = self.result_bytes.saturating_sub(bytes);
        } else {
            self.entries.insert(request_id.to_owned(), receipt);
        }
    }
}

struct ReceiptExecutionGuard {
    receipts: Arc<OperationReceipts>,
    request_id: String,
    finished: bool,
}

impl ReceiptExecutionGuard {
    fn new(receipts: Arc<OperationReceipts>, request_id: String) -> Self {
        Self {
            receipts,
            request_id,
            finished: false,
        }
    }

    async fn finish(mut self, result: Value) {
        self.receipts.complete(&self.request_id, result).await;
        self.finished = true;
    }
}

impl Drop for ReceiptExecutionGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let receipts = self.receipts.clone();
        let request_id = self.request_id.clone();
        let result = unknown_receipt_result();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                receipts.complete(&request_id, result).await;
            });
        }
    }
}

#[cfg(test)]
mod operation_receipt_tests {
    use super::*;

    async fn wait_until_complete(receipts: &OperationReceipts, request_id: &str) -> Value {
        for _ in 0..100 {
            if let ReceiptDecision::Complete(result) = receipts.begin(request_id, [7; 32]).await {
                return result;
            }
            tokio::task::yield_now().await;
        }
        panic!("receipt did not complete");
    }

    #[tokio::test]
    async fn same_request_id_replays_the_completed_result_without_reexecution() {
        let receipts = OperationReceipts::default();
        let fingerprint = [7; 32];
        assert!(matches!(
            receipts.begin("write-once", fingerprint).await,
            ReceiptDecision::Execute
        ));

        let waiter = receipts.begin("write-once", fingerprint).await;
        let ReceiptDecision::Wait(notify) = waiter else {
            panic!("duplicate must wait for the original operation");
        };
        let notified = tokio::spawn(async move {
            notify.notified().await;
        });
        tokio::task::yield_now().await;
        receipts
            .complete("write-once", json!({"sha256":"result","bytes":4}))
            .await;
        notified.await.unwrap();

        assert!(matches!(
            receipts.begin("write-once", fingerprint).await,
            ReceiptDecision::Complete(result) if result["bytes"] == 4
        ));
        assert!(matches!(
            receipts.begin("write-once", [8; 32]).await,
            ReceiptDecision::Conflict
        ));
        receipts.mark_replay("write-once").await;
        assert!(receipts.take_replay("write-once").await);
        assert!(!receipts.take_replay("write-once").await);
    }

    #[tokio::test]
    async fn completion_before_wait_registration_is_not_lost() {
        let receipts = OperationReceipts::default();
        assert!(matches!(
            receipts.begin("race", [7; 32]).await,
            ReceiptDecision::Execute
        ));
        let ReceiptDecision::Wait(notify) = receipts.begin("race", [7; 32]).await else {
            panic!("duplicate must wait");
        };
        receipts.complete("race", json!({"ok":true})).await;
        assert!(matches!(
            receipts.wait("race", [7; 32], notify).await,
            ReceiptWaitOutcome::Complete(result) if result["ok"] == true
        ));
    }

    #[tokio::test]
    async fn dropped_execution_records_a_deterministic_outcome() {
        let receipts = Arc::new(OperationReceipts::default());
        assert!(matches!(
            receipts.begin("aborted", [7; 32]).await,
            ReceiptDecision::Execute
        ));
        drop(ReceiptExecutionGuard::new(
            receipts.clone(),
            "aborted".into(),
        ));
        let result = wait_until_complete(&receipts, "aborted").await;
        assert_eq!(result["error"]["code"], 12407);
        assert_eq!(
            result["error"]["message"],
            "machine operation outcome unknown; observe before retrying"
        );
    }

    #[tokio::test]
    async fn sequential_desktop_burst_evicts_completed_receipts() {
        let receipts = OperationReceipts::default();
        for index in 0..10_000 {
            let request_id = format!("input-{index}");
            assert!(matches!(
                receipts.begin(&request_id, [7; 32]).await,
                ReceiptDecision::Execute
            ));
            receipts
                .complete(&request_id, json!({"accepted":true}))
                .await;
        }
        assert!(receipts.result_bytes().await <= OPERATION_RECEIPT_RESULT_BUDGET);
    }

    #[tokio::test]
    async fn completed_results_are_evicted_at_the_aggregate_byte_budget() {
        let receipts = OperationReceipts::default();
        let payload = "x".repeat(OPERATION_RECEIPT_RESULT_LIMIT - 128);
        for index in 0..200 {
            let request_id = format!("large-{index}");
            assert!(matches!(
                receipts.begin(&request_id, [7; 32]).await,
                ReceiptDecision::Execute
            ));
            receipts
                .complete(&request_id, json!({"payload":payload.as_str()}))
                .await;
        }
        assert!(receipts.result_bytes().await <= OPERATION_RECEIPT_RESULT_BUDGET);
    }
}

fn receipt_fingerprint(request: &Request) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(&request.operation).unwrap_or_default());
    digest.update(serde_json::to_vec(&request.authority).unwrap_or_default());
    digest.update(serde_json::to_vec(&request.parameters).unwrap_or_default());
    digest.finalize().into()
}

fn receipt_operation(operation: Operation) -> bool {
    matches!(
        operation,
        Operation::Exec
            | Operation::JobCancel
            | Operation::WriteFile
            | Operation::EditFile
            | Operation::SaveAttachment
            | Operation::ServiceCall
            | Operation::Browser
            | Operation::FillLogin
            | Operation::Computer
            | Operation::DesktopOpen
            | Operation::DesktopClose
            | Operation::DesktopControl
            | Operation::DesktopInput
            | Operation::Upgrade
            | Operation::ContainerMigrate
    )
}

#[derive(Debug, thiserror::Error)]
enum MachineError {
    #[error("owner_in_control")]
    OwnerInControl,
    #[error("machine_turn_stopped")]
    TurnStopped,
    #[error("machine_authority_stale")]
    AuthorityStale,
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
    #[error(transparent)]
    DevBrowser(#[from] dev_browser::Failure),
    #[error(transparent)]
    SecureBrowser(#[from] browser::Failure),
    #[error("saved logins require the secure browser")]
    SecureBrowserRequired,
    #[error("machine operation refused")]
    File(FileFailure),
    #[error("machine operation refused")]
    Operation,
}

impl From<anyhow::Error> for MachineError {
    fn from(error: anyhow::Error) -> Self {
        if let Some(driver) = error.downcast_ref::<cua::DriverError>() {
            return Self::Driver(driver.clone());
        }
        if let Some(browser) = error.downcast_ref::<browser::Failure>() {
            return Self::SecureBrowser(*browser);
        }
        if let Some(browser) = error.downcast_ref::<dev_browser::Failure>() {
            return Self::DevBrowser(*browser);
        }
        error.downcast::<Self>().unwrap_or(Self::Operation)
    }
}

impl MachineError {
    fn public(&self) -> (u32, &'static str) {
        match self {
            Self::AuthorityStale => (12421, "machine_authority_stale: acquire fresh authority"),
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
            Self::Computer | Self::Driver(cua::DriverError::Refused) => (
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
            Self::DevBrowser(error) => (12413, error.message()),
            Self::SecureBrowser(error) => (12413, error.message()),
            Self::SecureBrowserRequired => (
                12413,
                "saved logins require the secure browser; use browser=secure",
            ),
            Self::File(_) => (
                12407,
                "machine operation refused; check capability, path, input, limits and expected SHA-256",
            ),
            Self::Operation => (
                12407,
                "machine operation refused; check capability, path, input, limits and expected SHA-256",
            ),
        }
    }
}

fn cache_status(
    directory: &std::path::Path,
    profile: &nyxid_machine::MachineProfile,
) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    let mut value = serde_json::to_value(profile)?;
    value["observed_at_ms"] = serde_json::json!(chrono::Utc::now().timestamp_millis());
    file.write_all(&serde_json::to_vec(&value)?)?;
    file.persist(directory.join("machine-status.json"))?;
    Ok(())
}

pub struct Runtime {
    update_directory: PathBuf,
    status_directory: PathBuf,
    upgrading: Arc<std::sync::atomic::AtomicBool>,
    operation_admission: Arc<tokio::sync::RwLock<()>>,
    config: Config,
    node_id: String,
    runtime_id: String,
    excluded: Vec<PathBuf>,
    roots: files::Roots,
    identity: process::Identity,
    browser_identity: process::Identity,
    dev_identity: Option<process::Identity>,
    #[cfg(target_os = "linux")]
    contexts: Mutex<context_dispatch::Contexts>,
    #[cfg(target_os = "linux")]
    context_resources: Option<context_runtime::BrowserResources>,
    #[cfg(target_os = "linux")]
    context_binding: Option<nyxid_machine::authority::Authority>,
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
    operation_receipts: Arc<OperationReceipts>,
    replay: Mutex<ReplayGuard>,
    redactor: Arc<Mutex<Redactor>>,
    owner_control: tokio::sync::watch::Sender<u64>,
    dev_owner_control: tokio::sync::watch::Sender<u64>,
    turns: Arc<cancellation::Turns>,
    authority: authority::Fences,
    authority_watch: std::sync::atomic::AtomicBool,
}

impl Runtime {
    pub fn new(config: &Config, node_id: &str, config_dir: &Path) -> Result<Arc<Self>> {
        config.validate().map_err(anyhow::Error::msg)?;
        let identity = process::Identity::resolve(config.agent_user.as_deref())?;
        let browser = process::Identity::resolve(config.browser_user.as_deref())?;
        if !config.allow_root
            && ((config.shell && identity.uid == 0)
                || ((config.computer || config.browser_enabled()) && browser.uid == 0))
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
            .filter(|_| config.computer || config.browser_enabled())
            .map(|path| cua::Driver::new(path.clone(), browser.clone(), config.computer_mode));
        let owner_driver = config
            .cua_driver
            .as_ref()
            .filter(|_| config.computer || config.browser_enabled())
            .map(|path| {
                cua::Driver::new(path.clone(), browser.clone(), config.computer_mode).for_human()
            });
        let redactor = Arc::new(Mutex::new(Redactor::default()));
        let runtime = Arc::new(Self {
            update_directory: update::directory(config, config_dir),
            status_directory: config_dir.to_owned(),
            upgrading: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            operation_admission: Arc::new(tokio::sync::RwLock::new(())),
            config: config.clone(),
            node_id: node_id.into(),
            runtime_id: uuid::Uuid::new_v4().to_string(),
            excluded,
            roots,
            identity,
            browser_identity: browser,
            dev_identity: process::Identity::resolve(config.effective_dev_browser_user()).ok(),
            #[cfg(target_os = "linux")]
            contexts: Mutex::new(context_dispatch::Contexts::default()),
            #[cfg(target_os = "linux")]
            context_resources: None,
            #[cfg(target_os = "linux")]
            context_binding: None,
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
            operation_receipts: Arc::new(OperationReceipts::default()),
            replay: Mutex::new(ReplayGuard::default()),
            redactor,
            owner_control: tokio::sync::watch::channel(0).0,
            dev_owner_control: tokio::sync::watch::channel(0).0,
            turns: Arc::new(cancellation::Turns::default()),
            authority: authority::Fences::open(config_dir)?,
            authority_watch: std::sync::atomic::AtomicBool::new(false),
        });
        Ok(runtime)
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
        *self.dev_desktop.sender.lock().await = Some(sender.clone());
        #[cfg(target_os = "linux")]
        for child in self.context_children().await {
            *child.desktop.sender.lock().await = Some(sender.clone());
            *child.dev_desktop.sender.lock().await = Some(sender.clone());
            *child.desktop_secret.lock().await =
                Some(zeroize::Zeroizing::new(signing_secret.to_vec()));
        }
        *self.desktop_secret.lock().await = Some(zeroize::Zeroizing::new(signing_secret.to_vec()));
        if self.config.computer || self.config.browser_enabled() {
            self.start_capture(nyxid_machine::desktop::Display::Secure);
            self.start_capture(nyxid_machine::desktop::Display::Dev);
        }
        Ok(())
    }
    pub async fn disconnect(&self) {
        #[cfg(target_os = "linux")]
        for child in self.context_children().await {
            *child.desktop.sender.lock().await = None;
            *child.dev_desktop.sender.lock().await = None;
        }
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
                .and_then(|result| result.ok())
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
        let saved_login_ready =
            if self.config.browser_enabled() && self.config.managed_browser.is_some() {
                self.ensure_browser().await.is_ok()
                    && match self.browser.lock().await.as_ref() {
                        Some(browser) => browser.ready().await,
                        None => false,
                    }
            } else {
                false
            };
        let profile = MachineProfile {
            installation: Some(update::installation(&self.config)),
            updater_ready: update::ready(&self.update_directory),
            updater: self.updater_status(),
            version: nyxid_machine::PROTOCOL_VERSION,
            runtime_id: self.runtime_id.clone(),
            shell: self.config.shell,
            files: self.config.files,
            computer: self.config.computer,
            browser: self.config.browser,
            authority_versions: vec![2],
            separated: Some(self.context_support().await),
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
            browser_tools: self.config.browser_enabled(),
        };
        if cache_status(&self.status_directory, &profile).is_err() {
            tracing::warn!("machine_status_snapshot_unavailable");
        }
        profile
    }

    fn developer_identity(&self) -> Result<std::borrow::Cow<'_, process::Identity>> {
        if let Some(identity) = &self.dev_identity {
            return Ok(std::borrow::Cow::Borrowed(identity));
        }
        // Shared installations may create the developer user after the daemon
        // starts. Retry that lookup at launch, retaining its typed diagnostic.
        // A separated context must use its supervisor-provisioned identity.
        #[cfg(target_os = "linux")]
        anyhow::ensure!(
            self.context_binding.is_none(),
            dev_browser::Failure::Identity
        );
        anyhow::ensure!(
            self.browser_identity.desktop.is_none(),
            dev_browser::Failure::Identity
        );
        process::Identity::resolve(self.config.effective_dev_browser_user())
            .map(std::borrow::Cow::Owned)
            .context(dev_browser::Failure::Identity)
    }

    fn dev_directory(&self, config: &nyxid_machine::config::ManagedBrowserConfig) -> PathBuf {
        if self.browser_identity.desktop.is_some() {
            self.dev_identity
                .as_ref()
                .and_then(|i| i.home.parent())
                .unwrap_or(&config.data_dir)
                .to_owned()
        } else {
            config.data_dir.clone()
        }
    }
    #[cfg(target_os = "linux")]
    fn display_endpoint(
        &self,
        display: nyxid_machine::desktop::Display,
    ) -> Option<(String, PathBuf)> {
        let identity = match display {
            nyxid_machine::desktop::Display::Secure => Some(&self.browser_identity),
            nyxid_machine::desktop::Display::Dev => self.dev_identity.as_ref(),
        };
        identity
            .and_then(|i| i.desktop.as_ref())
            .map(|d| (d.display.clone(), d.authority.clone()))
    }
    #[cfg(not(target_os = "linux"))]
    async fn context_support(&self) -> nyxid_machine::context::Support {
        nyxid_machine::context::Support {
            available: false,
            landlock_abi: None,
            reason: Some("separated_requires_linux".into()),
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
    pub async fn control_revision(&self, operation: Operation, parameters: &Value) -> u64 {
        #[cfg(target_os = "linux")]
        let child = self.result_context(operation, parameters).await;
        #[cfg(target_os = "linux")]
        if child.is_none() && parameters["_signed_authority"]["mode"] == "separated" {
            // An unprovisioned context starts with its own revision zero. The
            // legacy display's history is unrelated to its first operation.
            return 0;
        }
        #[cfg(target_os = "linux")]
        let runtime = child.as_deref().unwrap_or(self);
        #[cfg(not(target_os = "linux"))]
        let runtime = self;
        let (a, b) = runtime.control_channels(operation, parameters);
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
        let receipt_replay = if receipt_operation(operation) {
            self.operation_receipts.take_replay(request_id).await
        } else {
            false
        };
        let Ok(permit) = sender.reserve().await else {
            return;
        };
        // Reserve first: a full socket queue must not let an old result escape
        // after takeover. This short read guard spans only serialization and
        // nonblocking enqueue, never socket I/O.
        #[cfg(target_os = "linux")]
        let child = self.result_context(operation, parameters).await;
        #[cfg(target_os = "linux")]
        let unprovisioned =
            child.is_none() && parameters["_signed_authority"]["mode"] == "separated";
        #[cfg(not(target_os = "linux"))]
        let unprovisioned = false;
        #[cfg(target_os = "linux")]
        let runtime = child.as_deref().unwrap_or(self);
        #[cfg(not(target_os = "linux"))]
        let runtime = self;
        let scope = serde_json::from_value(parameters.clone()).unwrap_or_default();
        let stopped = self.turns.subscribe(&scope);
        let cancelled = stopped.borrow();
        let (a, b) = runtime.control_channels(operation, parameters);
        let a = a.borrow();
        let b = b.borrow();
        let current = if unprovisioned { 0 } else { (*a << 32) | *b };
        if !matches!(
            operation,
            Operation::AuthorityRenew
                | Operation::AuthorityRevoke
                | Operation::Cancel
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
        if let Ok(Some(authority)) = serde_json::from_value::<
            Option<Box<nyxid_machine::authority::Authority>>,
        >(parameters["_signed_authority"].clone())
        {
            if result.get("error").is_none()
                && !receipt_replay
                && !self.authority.live(&authority.lease_id)
                && !matches!(
                    operation,
                    Operation::AuthorityRenew | Operation::AuthorityRevoke
                )
            {
                result = json!({"error":{"code":12421,"message":"machine_authority_stale: acquire fresh authority"}});
            }
            // A renewal's acknowledgement is not the operation's completion.
            // Rejected signatures/envelopes may not mutate an existing lease.
            if !receipt_replay
                && !matches!(
                    operation,
                    Operation::AuthorityRenew | Operation::AuthorityRevoke
                )
                && !matches!(
                    result["error"]["code"].as_u64(),
                    Some(12401 | 12420 | 12421)
                )
            {
                self.authority.finish(&authority.lease_id);
            }
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

    pub fn updater_status(&self) -> Option<nyxid_machine::update::CompanionStatus> {
        update::reported_companion(&self.update_directory, update::installation(&self.config))
    }

    pub fn report_connected(&self) {
        if update::connected(&self.update_directory).is_err() {
            tracing::warn!("Machine updater reconnect marker could not be written");
        }
    }

    fn ensure_authority_watch(self: &Arc<Self>) {
        if !self
            .authority_watch
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
                loop {
                    interval.tick().await;
                    let Some(runtime) = weak.upgrade() else { break };
                    let jobs = runtime.authority.expired_jobs();
                    runtime.jobs.preempt_ids(&jobs).await;
                    if let Some(gateway) = runtime.gateway.get() {
                        gateway.cancel_jobs(&jobs).await;
                    }
                }
            });
        }
    }
    pub async fn handle(self: &Arc<Self>, request: Request, signing_secret: &[u8]) -> Value {
        if receipt_operation(request.operation) {
            let request_id = request.request_id.clone();
            let fingerprint = receipt_fingerprint(&request);
            // Validate the signed identity and timestamp before consulting the
            // receipt cache. Duplicate nonce consumption is deliberately left
            // to the full replay check below; a cached result still requires a
            // valid HMAC and a fresh request.
            if self
                .replay
                .lock()
                .await
                .verify_signature_and_freshness(
                    &request,
                    &self.node_id,
                    signing_secret,
                    chrono::Utc::now().timestamp(),
                )
                .is_err()
            {
                return json!({"error":{"code":12401,"message":"machine signature or replay check failed"}});
            }
            match self
                .operation_receipts
                .begin(&request_id, fingerprint)
                .await
            {
                ReceiptDecision::Complete(result) => {
                    self.operation_receipts.mark_replay(&request_id).await;
                    tracing::info!(
                        request_id = %request_id,
                        operation = ?request.operation,
                        "machine operation receipt replayed"
                    );
                    return result;
                }
                ReceiptDecision::Conflict => {
                    tracing::warn!(
                        request_id = %request_id,
                        operation = ?request.operation,
                        "machine operation request identity conflict"
                    );
                    return json!({"error":{"code":12401,"message":"machine request identity conflict; retry with a new request"}});
                }
                ReceiptDecision::Capacity => {
                    self.operation_receipts.mark_replay(&request_id).await;
                    return json!({"error":{"code":12407,"message":"machine operation receipt capacity reached; retry later"}});
                }
                ReceiptDecision::Wait(notify) => {
                    tracing::info!(
                        request_id = %request_id,
                        operation = ?request.operation,
                        "machine operation receipt awaiting original result"
                    );
                    return match self
                        .operation_receipts
                        .wait(&request_id, fingerprint, notify)
                        .await
                    {
                        ReceiptWaitOutcome::Complete(result) => {
                            self.operation_receipts.mark_replay(&request_id).await;
                            result
                        }
                        ReceiptWaitOutcome::Conflict => {
                            json!({"error":{"code":12401,"message":"machine request identity conflict; retry with a new request"}})
                        }
                        ReceiptWaitOutcome::Pending => {
                            self.operation_receipts.mark_replay(&request_id).await;
                            unknown_receipt_result()
                        }
                    };
                }
                ReceiptDecision::Execute => {}
            }
            let verification = self.replay.lock().await.verify(
                &request,
                &self.node_id,
                signing_secret,
                chrono::Utc::now().timestamp(),
            );
            if verification.is_err() {
                let result = json!({"error":{"code":12401,"message":"machine signature or replay check failed"}});
                self.operation_receipts
                    .complete(&request_id, result.clone())
                    .await;
                return result;
            }
            let receipt_guard =
                ReceiptExecutionGuard::new(self.operation_receipts.clone(), request_id.clone());
            let result = self.handle_uncached(request, signing_secret, true).await;
            receipt_guard.finish(result.clone()).await;
            return result;
        }
        self.handle_uncached(request, signing_secret, false).await
    }

    async fn handle_uncached(
        self: &Arc<Self>,
        mut request: Request,
        signing_secret: &[u8],
        already_verified: bool,
    ) -> Value {
        if request.version == 2 {
            self.ensure_authority_watch();
        }
        if !already_verified
            && self
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
        let mut authority_stop = None;
        if let Some(authority) = request.authority.as_ref() {
            let result = match request.operation {
                Operation::AuthorityRenew => self.authority.renew(authority, &self.runtime_id),
                Operation::AuthorityRevoke => self.authority.revoke(authority, &self.runtime_id),
                _ => {
                    if !authority
                        .capabilities
                        .allows(request.operation, &request.parameters)
                        || request.parameters["conversation_id"] != authority.conversation_id
                        || request.parameters["turn_id"] != authority.turn_id
                    {
                        return json!({"error":{"code":12420,"message":"machine_permission_revoked: capability or task not authorized"}});
                    }
                    self.authority
                        .admit(
                            authority,
                            &self.runtime_id,
                            (request.operation == Operation::Exec)
                                .then(|| request.parameters["job_id"].as_str().map(str::to_owned))
                                .flatten(),
                        )
                        .map(|stop| authority_stop = Some(stop))
                }
            };
            if result.is_err() {
                return json!({"error":{"code":12421,"message":"machine_authority_stale: acquire fresh authority"}});
            }
            if matches!(
                request.operation,
                Operation::AuthorityRenew | Operation::AuthorityRevoke
            ) {
                #[cfg(target_os = "linux")]
                if request.operation == Operation::AuthorityRevoke
                    && request.parameters["quarantine_profiles"] == true
                    && self
                        .quarantine_profiles(&authority.agent_id, authority.revision)
                        .await
                        .is_err()
                {
                    return json!({"error":{"code":12421,"message":"context_quarantine_pending: retry revocation"}});
                }
                return json!({"accepted":true});
            }
        } else if self.authority.enrolled()
            && !matches!(
                request.operation,
                Operation::Cancel
                    | Operation::Upgrade
                    | Operation::UpgradeStatus
                    | Operation::ContainerInspect
                    | Operation::ContainerMigrate
                    | Operation::DesktopOpen
                    | Operation::DesktopClose
                    | Operation::DesktopControl
                    | Operation::DesktopInput
            )
        {
            return json!({"error":{"code":12419,"message":"machine_authority_unsupported: v2 required for agent work"}});
        }
        #[cfg(target_os = "linux")]
        let context_runtime = if let Some(authority) =
            request.authority.as_ref().filter(|a| a.mode == "separated")
        {
            match Box::pin(self.context_instance(authority)).await {
                Ok(runtime) => Some(runtime),
                Err(error) => {
                    let stage = context_dispatch::failure_stage(&error);
                    tracing::warn!(
                        stage,
                        reason = "context_preparation_failed",
                        "separated context unavailable"
                    );
                    return json!({"error":{"code":12419,"stage":stage,"message":"separated_context_unavailable: verify Linux Landlock ABI 6, separate users, private legacy roots and browser setup; shared fallback is disabled"}});
                }
            }
        } else if let Some(id) = request.parameters["context_id"].as_str().filter(|_| {
            matches!(
                request.operation,
                Operation::DesktopOpen
                    | Operation::DesktopClose
                    | Operation::DesktopControl
                    | Operation::DesktopInput
            )
        }) {
            match self.contexts.lock().await.runtimes.get(id).cloned() {
                Some(runtime) => Some(runtime),
                None => {
                    return json!({"error":{"code":12419,"message":"separated_context_unavailable: context is not running"}});
                }
            }
        } else {
            None
        };
        #[cfg(target_os = "linux")]
        let executor = context_runtime.as_deref().unwrap_or(self);
        #[cfg(not(target_os = "linux"))]
        let executor = {
            if request
                .authority
                .as_ref()
                .is_some_and(|a| a.mode == "separated")
            {
                return json!({"error":{"code":12419,"message":"separated_requires_linux: use a separate machine container or VM"}});
            }
            self.as_ref()
        };
        request.parameters["_machine_authority"] =
            serde_json::to_value(&request.authority).unwrap_or(Value::Null);
        request.parameters["_machine_request_id"] = json!(request.request_id);
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
        let (first, second) = executor.control_channels(request.operation, &request.parameters);
        let revision = (*first.borrow(), *second.borrow());
        let result = if let Some(mut stopped) = authority_stop {
            if *stopped.borrow_and_update() {
                return json!({"error":{"code":12421,"message":"machine_authority_stale: acquire fresh authority"}});
            }
            tokio::select! {
                biased;
                _=stopped.changed()=>Err(MachineError::AuthorityStale),
                value=executor.execute(request.operation,request.parameters)=>value,
            }
        } else {
            executor
                .execute(request.operation, request.parameters)
                .await
        };
        if let Some(authority) = &request.authority
            && !self.authority.live(&authority.lease_id)
        {
            return json!({"error":{"code":12421,"message":"machine_authority_stale: acquire fresh authority"}});
        }
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
                if let MachineError::File(failure) = &error {
                    result["error"]["reason"] = json!(failure.reason);
                    if let Some(exit_code) = failure.exit_code {
                        result["error"]["exit_code"] = json!(exit_code);
                    }
                    if let Some(signal) = failure.signal {
                        result["error"]["signal"] = json!(signal);
                    }
                }
                if matches!(
                    error,
                    MachineError::Driver(cua::DriverError::ToolUnsupported)
                ) {
                    result["error"]["computer_tools"] = json!(
                        self.driver
                            .as_ref()
                            .map(|d| d
                                .advertised_tools()
                                .into_iter()
                                .take(64)
                                .collect::<Vec<_>>())
                            .unwrap_or_default()
                    );
                    result["error"]["browser_hint"] = json!(
                        "Use nyx__machine_browser action=snapshot for page content; browser=dev action=screenshot attaches an owner screenshot."
                    );
                }
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
            browser: self.config.browser,
            authority_versions: vec![2],
            ..Default::default()
        };
        if !operation.allowed(&profile) {
            bail!("machine capability disabled locally");
        }
        match operation {
            Operation::AuthorityRenew | Operation::AuthorityRevoke => {
                bail!("authority command requires v2")
            }
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
                    json!({"progress":progress,"updater_ready":update::ready(&self.update_directory), "updater": self.updater_status()}),
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
                    && (self.contexts_busy().await
                        || self.owner_in_control()
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
                gateway
                    .bind_authority(
                        string(&parameters, "job_id")?,
                        &parameters["_machine_authority"],
                    )
                    .await;

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
                                let identity = self.developer_identity()?;
                                dev_browser::DevBrowser::launch(
                                    &self.dev_directory(config),
                                    identity.as_ref(),
                                    &config.binary,
                                    config.container,
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
                    self.jobs.preempt_identity(self.identity.uid).await;
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
            let identity = self.developer_identity()?;
            *browser = Some(
                dev_browser::DevBrowser::launch(
                    &self.dev_directory(config),
                    identity.as_ref(),
                    &config.binary,
                    config.container,
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
        for attempt in 0..2 {
            if let Some(active) = browser.as_ref()
                && !active.alive().await
            {
                active.stop().await;
                *browser = None;
            }
            if browser.is_none() {
                *browser = Some(
                    browser::Browser::launch(
                        &config.data_dir,
                        &self.browser_identity,
                        &config.binary,
                        config.update_port,
                        config.container,
                        attempt > 0,
                    )
                    .await?,
                );
            }
            let active = browser.as_ref().expect("launched");
            // Each separated browser process owns its cold-start deadline
            // (45 s from spawn), including a repair launch. After its first
            // exchange, these 12 + 4 s waits apply only to reconnection.
            // Shared legacy browsers retain their alive-first check and timing.
            let startup_ready = if active.is_context_browser() {
                active
                    .wait_startup(std::time::Duration::from_secs(12))
                    .await
            } else {
                active.ready().await
            };
            if startup_ready {
                return Ok(());
            }
            // A live browser may be reconnecting its native host. Preserve its
            // tabs for another full (at most 4 s) extension retry before repair.
            // Check liveness first: a dead browser is repaired immediately.
            if active.alive().await
                && if active.is_context_browser() {
                    active.wait_startup(std::time::Duration::from_secs(4)).await
                } else {
                    active.wait_ready(std::time::Duration::from_secs(5)).await
                }
            {
                return Ok(());
            }
            active.stop().await;
            *browser = None;
            tracing::warn!("secure_browser_restarting_extension_handshake");
        }
        if let Some(active) = browser.take() {
            active.stop().await;
        }
        tracing::warn!("secure_browser_extension_recovery_failed");
        Err(browser::Failure::ExtensionUnavailable.into())
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
        let request_id = request
            .parameters
            .get("_machine_request_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let operation = request.operation;
        let failure = |failure: FileFailure| {
            tracing::warn!(
                request_id,
                operation = ?operation,
                reason = failure.reason,
                exit_code = failure.exit_code,
                signal = failure.signal,
                "machine file worker failed"
            );
            anyhow::Error::new(MachineError::File(failure))
        };
        // The backend transport test library runs inside nyxid-server's test
        // harness, not the CLI executable. Production always uses a cancellable
        // child, even when the command user is the supervisor's own user.
        #[cfg(feature = "node-proxy-test")]
        if self.identity.uid == unsafe { libc::geteuid() } {
            return tokio::task::spawn_blocking(move || execute_file(request))
                .await
                .map_err(|_| failure(FileFailure::new("worker_exit_nonzero")))?
                .map_err(|error| failure(FileFailure::new(file_error_reason(&error))));
        }
        let mut command = tokio::process::Command::new(
            std::env::current_exe()
                .map_err(|_| failure(FileFailure::new("worker_spawn_failed")))?,
        );
        self.identity
            .prepare_agent(&mut command)
            .map_err(|_| failure(FileFailure::new("worker_spawn_failed")))?;
        command
            .args(["node", "machine-worker"])
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|_| failure(FileFailure::new("worker_spawn_failed")))?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| failure(FileFailure::new("worker_stdin_failed")))?;
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(&request)
                .map_err(|_| failure(FileFailure::new("worker_stdin_failed")))?,
        );
        input
            .write_all(&bytes)
            .await
            .map_err(|_| failure(FileFailure::new("worker_stdin_failed")))?;
        input
            .shutdown()
            .await
            .map_err(|_| failure(FileFailure::new("worker_stdin_failed")))?;
        drop(input);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failure(FileFailure::new("worker_output_invalid")))?;
        let (output, status) = collect_file_worker(&mut child, stdout)
            .await
            .map_err(&failure)?;
        if let Some(result) = worker_exit_outcome(&output, &status).map_err(&failure)? {
            // A successful response proves that execute_file reached its
            // commit and serialized the result. Preserve that result even
            // if teardown returned a non-zero status (for example, a
            // post-write stdout/close failure).
            tracing::warn!(
                request_id,
                operation = ?operation,
                reason = "worker_exit_nonzero_after_result",
                exit_code = status.code(),
                signal = worker_status_failure(&status).signal,
                "machine file worker committed before non-zero exit"
            );
            return Ok(result);
        }
        let response: Value = serde_json::from_slice(&output)
            .map_err(|_| failure(FileFailure::new("worker_output_invalid")))?;
        if response["reason"] == "path_outside_roots" {
            tracing::warn!(
                request_id,
                operation = ?operation,
                reason = "path_outside_roots",
                "machine file worker refused path"
            );
            return Err(MachineError::PathOutsideRoots.into());
        }
        if response.get("error").is_some() {
            let reason =
                worker_reason(response["reason"].as_str()).unwrap_or("worker_output_invalid");
            return Err(failure(FileFailure::new(reason)));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| failure(FileFailure::new("worker_output_invalid")))
    }

    pub async fn shutdown(&self) {
        self.jobs.cancel_all().await;
        #[cfg(target_os = "linux")]
        for child in self.context_children().await {
            child.stop_local().await;
        }
        self.stop_local().await;
    }

    async fn contexts_busy(&self) -> bool {
        #[cfg(target_os = "linux")]
        for child in self.context_children().await {
            if child.owner_in_control()
                || child.desktop.session.lock().await.is_some()
                || child.dev_desktop.session.lock().await.is_some()
            {
                return true;
            }
        }
        false
    }

    async fn stop_local(&self) {
        if let Some(task) = self.dev_desktop.capture.get() {
            task.abort();
        }
        if let Some(browser) = self.browser.lock().await.take() {
            browser.stop().await;
        }
        self.dev_browser.lock().await.take();
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

#[derive(Debug, Clone)]
struct FileFailure {
    reason: &'static str,
    exit_code: Option<i32>,
    signal: Option<i32>,
}

impl FileFailure {
    fn new(reason: &'static str) -> Self {
        Self {
            reason,
            exit_code: None,
            signal: None,
        }
    }
}

fn file_error_reason(error: &anyhow::Error) -> &'static str {
    if error
        .downcast_ref::<MachineError>()
        .is_some_and(|error| matches!(error, MachineError::PathOutsideRoots))
    {
        return "path_outside_roots";
    }
    if error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists)
    }) || error.to_string() == "file already exists"
    {
        return "file_exists";
    }
    match error.to_string().as_str() {
        "sha256 mismatch" | "file transfer checksum mismatch" => "sha256_mismatch",
        "file transfer length mismatch" => "length_mismatch",
        _ => "operation_refused",
    }
}

fn worker_reason(value: Option<&str>) -> Option<&'static str> {
    match value {
        Some("path_outside_roots") => Some("path_outside_roots"),
        Some("file_exists") => Some("file_exists"),
        Some("sha256_mismatch") => Some("sha256_mismatch"),
        Some("length_mismatch") => Some("length_mismatch"),
        Some("operation_refused") => Some("operation_refused"),
        Some("worker_output_invalid") => Some("worker_output_invalid"),
        Some("worker_timeout") => Some("worker_timeout"),
        _ => None,
    }
}

/// One deadline for reading and reaping. A broken/partial reply cannot prove
/// that a changing operation did not commit: callers must observe before retry.
async fn collect_file_worker(
    child: &mut tokio::process::Child,
    stdout: impl tokio::io::AsyncRead + Unpin,
) -> std::result::Result<(zeroize::Zeroizing<Vec<u8>>, std::process::ExitStatus), FileFailure> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut output = zeroize::Zeroizing::new(Vec::new());
        stdout
            .take(256 * 1024 + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| FileFailure::new("worker_output_invalid"))?;
        if output.len() > 256 * 1024 {
            return Err(FileFailure::new("worker_output_invalid"));
        }
        let status = child
            .wait()
            .await
            .map_err(|_| FileFailure::new("worker_wait_failed"))?;
        Ok((output, status))
    })
    .await
    .map_err(|_| FileFailure::new("worker_timeout"))?
}

fn worker_status_failure(status: &std::process::ExitStatus) -> FileFailure {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(status);
    #[cfg(not(unix))]
    let signal = None;
    FileFailure {
        reason: "worker_exit_nonzero",
        exit_code: status.code(),
        signal,
    }
}

fn committed_worker_result(output: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(output)
        .ok()
        .and_then(|response| {
            (response.as_object()?.len() == 1)
                .then(|| response.get("result").cloned())
                .flatten()
        })
}

fn worker_exit_outcome(
    output: &[u8],
    status: &std::process::ExitStatus,
) -> std::result::Result<Option<Value>, FileFailure> {
    if status.success() {
        return Ok(None);
    }
    committed_worker_result(output)
        .map(Some)
        .ok_or_else(|| worker_status_failure(status))
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
            let reason = file_error_reason(&error);
            json!({"error":"operation_refused","reason":reason})
        }
    };
    // This dedicated child already performs synchronous file operations. Finish
    // its bounded reply with an explicit write and flush so an EPIPE or other
    // output failure reaches the supervisor as a non-zero worker exit.
    use std::io::Write;
    let output = zeroize::Zeroizing::new(serde_json::to_vec(&result)?);
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&output)?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod file_worker_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn committed_write_survives_nonzero_worker_teardown() {
        let root = tempfile::tempdir().unwrap();
        let roots = files::Roots::new(&[root.path().to_owned()], &[]).unwrap();
        let hash = roots
            .write("private.txt", b"committed", "create", None)
            .unwrap();
        let output = serde_json::to_vec(&json!({
            "result": {"sha256": hash, "bytes": 9}
        }))
        .unwrap();
        let status = std::process::ExitStatus::from_raw(17 << 8);
        let result = worker_exit_outcome(&output, &status).unwrap().unwrap();
        assert_eq!(result["bytes"], 9);
        assert_eq!(
            std::fs::read(root.path().join("private.txt")).unwrap(),
            b"committed"
        );
    }

    #[test]
    fn nonzero_worker_without_result_has_fixed_status_reason() {
        let status = std::process::ExitStatus::from_raw(23 << 8);
        let failure = worker_exit_outcome(b"", &status).unwrap_err();
        assert_eq!(failure.reason, "worker_exit_nonzero");
        assert_eq!(failure.exit_code, Some(23));
        assert_eq!(failure.signal, None);
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[tokio::test]
    async fn developer_identity_retries_shared_lookup_but_never_replaces_context_identity() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(
            &Config {
                roots: vec![root.path().into()],
                ..Default::default()
            },
            "node",
            &root.path().join("node"),
        )
        .unwrap();
        let runtime = Arc::get_mut(&mut runtime).unwrap();
        let expected = runtime.dev_identity.take().unwrap();
        let resolved = runtime.developer_identity().unwrap();
        assert_eq!(resolved.uid, expected.uid);
        assert!(matches!(resolved, std::borrow::Cow::Owned(_)));
        drop(resolved);
        runtime.config.dev_browser_user = Some("nyxid-no-such-developer-user".into());
        assert!(
            runtime
                .developer_identity()
                .err()
                .unwrap()
                .is::<dev_browser::Failure>()
        );
        runtime.dev_identity = Some(expected.clone());
        assert!(matches!(
            runtime.developer_identity().unwrap(),
            std::borrow::Cow::Borrowed(_)
        ));
        runtime.dev_identity = None;
        runtime.config.dev_browser_user = None;
        runtime.browser_identity.desktop = Some(process::DesktopEnvironment {
            display: ":123".into(),
            authority: root.path().join("authority"),
            bus: "private".into(),
            runtime: root.path().into(),
        });
        assert!(
            runtime
                .developer_identity()
                .err()
                .unwrap()
                .is::<dev_browser::Failure>()
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn context_shared_developer_user_created_after_daemon_start_is_resolved() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let user = format!(
            "nyxlate{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        );
        let runtime = Runtime::new(
            &Config {
                roots: vec![root.path().into()],
                dev_browser_user: Some(user.clone()),
                ..Default::default()
            },
            "node",
            &root.path().join("node"),
        )
        .unwrap();
        assert!(runtime.dev_identity.is_none());
        assert!(runtime.developer_identity().is_err());
        assert!(
            std::process::Command::new("useradd")
                .args([
                    "--system",
                    "--user-group",
                    "--no-create-home",
                    "--no-log-init",
                    &user
                ])
                .status()
                .unwrap()
                .success()
        );
        struct RemoveUser(String);
        impl Drop for RemoveUser {
            fn drop(&mut self) {
                let _ = std::process::Command::new("userdel").arg(&self.0).status();
            }
        }
        let _cleanup = RemoveUser(user.clone());
        let resolved = runtime.developer_identity().unwrap();
        assert_eq!(resolved.name, user);
        assert_ne!(resolved.uid, 0);
        assert!(matches!(resolved, std::borrow::Cow::Owned(_)));
        assert!(
            runtime.dev_identity.is_none(),
            "no daemon restart or cached identity was needed"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn new_context_ignores_legacy_control_history_but_fences_its_own_takeover() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            roots: vec![root.path().into()],
            ..Default::default()
        };
        let runtime = Runtime::new(&config, "node", &root.path().join("node")).unwrap();
        runtime.owner_control.send_replace(4);
        let context = uuid::Uuid::new_v4().to_string();
        let parameters = json!({"_signed_authority":{"mode":"separated","context_id":context}});
        let revision = runtime
            .control_revision(Operation::ReadFile, &parameters)
            .await;
        assert_eq!(revision, 0);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        // A preparation refusal must also retain its real error code.
        runtime
            .send_result(
                &sender,
                "request",
                Operation::ReadFile,
                revision,
                &parameters,
                json!({"error":{"code":12419}}),
            )
            .await;
        let crate::node::ws_client::NodeWsMessage::Text(message) = receiver.recv().await.unwrap()
        else {
            panic!("result")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&message).unwrap()["result"]["error"]["code"],
            12419
        );
        let child = Runtime::new(&config, "node", &root.path().join("child")).unwrap();
        runtime
            .contexts
            .lock()
            .await
            .runtimes
            .insert(context.clone(), child.clone());
        runtime.owner_control.send_replace(7);
        let shared = json!({"_signed_authority":{"mode":"shared_legacy","context_id":context}});
        assert_eq!(
            runtime.control_revision(Operation::ReadFile, &shared).await,
            7 << 32
        );
        let untrusted = json!({"context_id":context});
        assert_eq!(
            runtime
                .control_revision(Operation::ReadFile, &untrusted)
                .await,
            7 << 32
        );
        assert_eq!(
            runtime
                .control_revision(Operation::DesktopOpen, &untrusted)
                .await,
            0
        );
        for takeover in [false, true] {
            if takeover {
                child.owner_control.send_replace(1);
            }
            runtime
                .send_result(
                    &sender,
                    "request",
                    Operation::ReadFile,
                    revision,
                    &parameters,
                    json!({"content":"context result"}),
                )
                .await;
            let crate::node::ws_client::NodeWsMessage::Text(message) =
                receiver.recv().await.unwrap()
            else {
                panic!("result")
            };
            let result: Value = serde_json::from_str(&message).unwrap();
            if takeover {
                assert_eq!(result["result"]["error"]["code"], 12408);
                assert!(!message.contains("context result"));
            } else {
                assert_eq!(result["result"]["content"], "context result");
            }
        }
    }

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
            version: 1,
            authority: None,
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

#[cfg(test)]
mod authority_runtime_tests {
    use super::*;
    fn signed(
        runtime: &Runtime,
        operation: Operation,
        parameters: Value,
        authority: Option<nyxid_machine::authority::Authority>,
    ) -> Request {
        let mut request = Request {
            version: if authority.is_some() { 2 } else { 1 },
            authority: authority.map(Box::new),
            request_id: uuid::Uuid::new_v4().to_string(),
            node_id: runtime.node_id.clone(),
            operation,
            parameters,
            timestamp: chrono::Utc::now().timestamp(),
            nonce: uuid::Uuid::new_v4().to_string(),
            signature: String::new(),
        };
        request.signature = nyxid_machine::signing::sign(&request, b"authority test");
        request
    }
    async fn fixture() -> (
        tempfile::TempDir,
        Arc<Runtime>,
        nyxid_machine::authority::Authority,
        tokio::sync::mpsc::Receiver<crate::node::ws_client::NodeWsMessage>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("node");
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        let runtime = Runtime::new(
            &Config {
                shell: true,
                files: true,
                allow_root: true,
                roots: vec![workspace],
                ..Default::default()
            },
            &uuid::Uuid::new_v4().to_string(),
            &config,
        )
        .unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(100);
        runtime.connect(sender, b"authority test").await.unwrap();
        let id = || uuid::Uuid::new_v4().to_string();
        let a = nyxid_machine::authority::Authority {
            require_v2: true,
            context_id: id(),
            generation: 1,
            mode: "shared_legacy".into(),
            agent_id: id(),
            owner_id: id(),
            actor_id: id(),
            group_id: None,
            runtime_id: runtime.runtime_id.clone(),
            conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
            turn_id: id(),
            lease_id: id(),
            revision: 1,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 4_000,
            capabilities: nyxid_machine::authority::Capabilities {
                shell: true,
                files: true,
                ..Default::default()
            },
        };
        (root, runtime, a, receiver)
    }
    fn exec(a: &nyxid_machine::authority::Authority, job: &str, background: bool) -> Value {
        json!({"conversation_id":a.conversation_id,"turn_id":a.turn_id,"runtime_id":a.runtime_id,"job_id":job,"command":"sleep 30","background":background})
    }
    #[tokio::test]
    async fn renewal_result_does_not_finish_foreground_lease_and_denials_keep_their_code() {
        let (_directory, runtime, a, _receiver) = fixture().await;
        runtime.authority.admit(&a, &a.runtime_id, None).unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
        runtime
            .send_result(
                &sender,
                "renew",
                Operation::AuthorityRenew,
                0,
                &json!({"_signed_authority":a}),
                json!({"accepted":true}),
            )
            .await;
        assert!(runtime.authority.live(&a.lease_id));
        receiver.recv().await.unwrap();
        let mut denied = a.clone();
        denied.lease_id = uuid::Uuid::new_v4().to_string();
        runtime
            .send_result(
                &sender,
                "denied",
                Operation::ReadFile,
                0,
                &json!({"_signed_authority":denied}),
                json!({"error":{"code":12420}}),
            )
            .await;
        let crate::node::ws_client::NodeWsMessage::Text(message) = receiver.recv().await.unwrap()
        else {
            panic!("text result");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&message).unwrap()["result"]["error"]["code"],
            12420
        );
        runtime
            .send_result(
                &sender,
                "forged",
                Operation::ReadFile,
                0,
                &json!({"_signed_authority":a}),
                json!({"error":{"code":12401}}),
            )
            .await;
        assert!(runtime.authority.live(&a.lease_id));
    }
    #[tokio::test]
    async fn authority_revocation_cancels_foreground_and_background_work_and_next_revision_works() {
        for background in [false, true] {
            let (_directory, runtime, a, _receiver) = fixture().await;
            let job = uuid::Uuid::new_v4().to_string();
            let request = signed(
                &runtime,
                Operation::Exec,
                exec(&a, &job, background),
                Some(a.clone()),
            );
            let copy = runtime.clone();
            let task = tokio::spawn(async move { copy.handle(request, b"authority test").await });
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !runtime.jobs.running(&job).await {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let mut revoked = a.clone();
            revoked.revision = 2;
            assert_eq!(
                runtime
                    .handle(
                        signed(
                            &runtime,
                            Operation::AuthorityRevoke,
                            json!({}),
                            Some(revoked.clone())
                        ),
                        b"authority test"
                    )
                    .await["accepted"],
                true
            );
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while runtime.jobs.running(&job).await {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let result = task.await.unwrap();
            if !background {
                assert_eq!(result["error"]["code"], 12421);
            }
            assert_eq!(
                runtime
                    .handle(
                        signed(&runtime, Operation::Exec, exec(&a, &job, true), None),
                        b"authority test"
                    )
                    .await["error"]["code"],
                12419
            );
            revoked.lease_id = uuid::Uuid::new_v4().to_string();
            revoked.expires_at_ms = chrono::Utc::now().timestamp_millis() + 4_000;
            let mut params = exec(&revoked, &uuid::Uuid::new_v4().to_string(), false);
            params["command"] = json!("true");
            let result = runtime
                .handle(
                    signed(&runtime, Operation::Exec, params, Some(revoked)),
                    b"authority test",
                )
                .await;
            assert!(result.get("error").is_none(), "{result}");
        }
    }
    #[tokio::test]
    async fn authority_grace_keeps_a_job_alive_through_a_seven_second_disconnect() {
        let (_directory, runtime, mut a, _receiver) = fixture().await;
        a.expires_at_ms =
            chrono::Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS;
        let job = uuid::Uuid::new_v4().to_string();
        let result = runtime
            .handle(
                signed(
                    &runtime,
                    Operation::Exec,
                    exec(&a, &job, true),
                    Some(a.clone()),
                ),
                b"authority test",
            )
            .await;
        assert_eq!(result["job_id"], job, "{result}");
        runtime.disconnect().await;
        tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        assert!(runtime.jobs.running(&job).await);
        assert!(runtime.authority.live(&a.lease_id));
        let (sender, _receiver) = tokio::sync::mpsc::channel(100);
        runtime.connect(sender, b"authority test").await.unwrap();
        a.expires_at_ms =
            chrono::Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS;
        assert_eq!(
            runtime
                .handle(
                    signed(
                        &runtime,
                        Operation::AuthorityRenew,
                        json!({}),
                        Some(a.clone())
                    ),
                    b"authority test"
                )
                .await["accepted"],
            true
        );
        assert!(runtime.jobs.running(&job).await);
        a.revision += 1;
        assert_eq!(
            runtime
                .handle(
                    signed(&runtime, Operation::AuthorityRevoke, json!({}), Some(a)),
                    b"authority test"
                )
                .await["accepted"],
            true
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while runtime.jobs.running(&job).await {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        runtime.shutdown().await;
    }
    #[tokio::test]
    async fn authority_expiry_stops_background_work_without_a_socket_or_renewal() {
        let (_directory, runtime, mut a, _receiver) = fixture().await;
        let job = uuid::Uuid::new_v4().to_string();
        let result = runtime
            .handle(
                signed(
                    &runtime,
                    Operation::Exec,
                    exec(&a, &job, true),
                    Some(a.clone()),
                ),
                b"authority test",
            )
            .await;
        assert!(result.get("error").is_none(), "{result}");
        runtime.disconnect().await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while runtime.jobs.running(&job).await {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        a.expires_at_ms = chrono::Utc::now().timestamp_millis() + 4_000;
        assert_eq!(
            runtime
                .handle(
                    signed(&runtime, Operation::AuthorityRenew, json!({}), Some(a)),
                    b"authority test"
                )
                .await["error"]["code"],
            12421
        );
    }
}
