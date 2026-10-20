//! Mongo's failCommand is server-global, including its appName selector.
//! A Tokio mutex alone cannot protect scripts in separate nextest processes.
use mongodb::{
    bson::{Bson, Document, doc},
    event::{EventHandler, command::CommandEvent},
};
use std::{
    fs::{File, OpenOptions},
    sync::{Arc, Mutex, mpsc},
};
use zeroize::Zeroizing;

pub(crate) static PROCESS_LOCK: ProcessLock = ProcessLock::new();

/// Optional overrides for the monitored client only. Defaults retain the URI's
/// driver policy; selecting false permits exact uncertain-write assertions.
#[derive(Default)]
pub(crate) struct GuardOptions {
    pub(crate) retry_writes: Option<bool>,
}

/// One fault script, with one private client and server-global ownership.
/// Clone database() for the operations under test; unrelated clients can never
/// consume this script's injections. Keep the guard through all assertions.
pub(crate) struct Guard {
    db: mongodb::Database,
    controller: FailCommandController,
    worker: Option<std::thread::JoinHandle<()>>,
    _ownership: ProcessGuard<'static>,
}

/// Blocking control for synchronous driver observers. The independent control
/// runtime/client cannot recurse into the monitored client's observer or pool.
/// A clone does not extend guard ownership: every call after Drop is an error.
#[derive(Clone)]
pub(crate) struct FailCommandController {
    inner: Arc<Control>,
}
struct Control {
    app_name: String,
    state: Mutex<ControlState>,
    requests: tokio::sync::mpsc::UnboundedSender<Request>,
}
struct ControlState {
    alive: bool,
    armed: bool,
}
enum Request {
    Command(Document, mpsc::SyncSender<mongodb::error::Result<Document>>),
    Stop,
}

fn control_error(message: &'static str) -> mongodb::error::Error {
    mongodb::error::Error::custom(message)
}

async fn control_client(uri: &str) -> Option<mongodb::Client> {
    let Some(Ok(mut options)) = super::test_db_cleanup_operation_with_timeout(
        super::TEST_DB_CLEANUP_CLIENT_PARSE_TIMEOUT,
        async { mongodb::options::ClientOptions::parse(uri).await },
    )
    .await
    else {
        return None;
    };
    super::local_test_connection(&mut options);
    options.max_pool_size = Some(1);
    options.app_name = Some(format!("nyxid-fault-control-{}", uuid::Uuid::new_v4()));
    options.server_selection_timeout = Some(super::STALE_TEST_DB_DROP_TIMEOUT);
    options.connect_timeout = Some(super::STALE_TEST_DB_DROP_TIMEOUT);
    mongodb::Client::with_options(options).ok()
}

impl FailCommandController {
    pub(crate) fn arm(
        &self,
        mode: impl Into<Bson>,
        mut data: Document,
    ) -> mongodb::error::Result<Document> {
        assert!(!data.contains_key("appName"), "the guard owns fault scope");
        assert!(
            data.get_array("failCommands").is_ok_and(|commands| {
                !commands.is_empty()
                    && commands.iter().all(|command| {
                        command
                            .as_str()
                            .is_some_and(|name| name != "configureFailPoint")
                    })
            }),
            "a script cannot disable its own administrative reset"
        );
        let mut state = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state.alive {
            return Err(control_error("Mongo fault guard is closed"));
        }
        data.insert("appName", &self.inner.app_name);
        // Track possible server-side success before waiting for its acknowledgment.
        state.armed = true;
        self.command(doc! {
            "configureFailPoint": "failCommand", "mode": mode.into(), "data": data,
        })
    }

    pub(crate) fn disarm(&self) -> mongodb::error::Result<()> {
        let mut state = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state.alive {
            return Err(control_error("Mongo fault guard is closed"));
        }
        self.command(doc! {"configureFailPoint": "failCommand", "mode": "off"})?;
        state.armed = false;
        Ok(())
    }

    // The state lock remains held by the caller through the acknowledgment.
    // Drop cannot release process ownership while an observer is still arming.
    fn command(&self, command: Document) -> mongodb::error::Result<Document> {
        let (send, receive) = mpsc::sync_channel(1);
        self.inner
            .requests
            .send(Request::Command(command, send))
            .map_err(|_| control_error("Mongo fault control worker stopped"))?;
        receive
            .recv()
            .map_err(|_| control_error("Mongo fault control acknowledgment lost"))?
    }
}

impl Guard {
    pub(crate) async fn acquire(
        base: &mongodb::Database,
        handler: Option<EventHandler<CommandEvent>>,
    ) -> Self {
        Self::acquire_with(base, handler, GuardOptions::default()).await
    }

    pub(crate) async fn acquire_with(
        base: &mongodb::Database,
        handler: Option<EventHandler<CommandEvent>>,
        guard_options: GuardOptions,
    ) -> Self {
        let ownership = PROCESS_LOCK.lock().await;
        let uri = Zeroizing::new(
            super::TEST_DB_PINNED_URI
                .get()
                .cloned()
                .or_else(|| std::env::var(super::TEST_DATABASE_URL_ENV).ok())
                .expect("fault script requires the fixture's Mongo URI"),
        );
        let mut options = mongodb::options::ClientOptions::parse(uri.as_str())
            .await
            .unwrap();
        super::local_test_connection(&mut options);
        if let Some(retry_writes) = guard_options.retry_writes {
            options.retry_writes = Some(retry_writes);
        }
        // A Started observer owns the only command connection until that
        // command completes. Another operation cannot consume its new fault.
        options.max_pool_size = Some(1);
        let app_name = format!("nyxid-fault-{}", uuid::Uuid::new_v4());
        options.app_name = Some(app_name.clone());
        options.command_event_handler = handler;
        let client = mongodb::Client::with_options(options).unwrap();
        let (requests, mut receive) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::Builder::new()
            .name("nyxid-fault-control".into())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async move {
                        // Separate appName and a single connection; no monitored
                        // callback, fixture lifetime guard, or shared pool.
                        let client = control_client(&uri).await;
                        while let Some(request) = receive.recv().await {
                            match request {
                                Request::Stop => break,
                                Request::Command(command, reply) => {
                                    let result = match &client {
                                        Some(client) => match tokio::time::timeout(
                                            super::STALE_TEST_DB_DROP_TIMEOUT,
                                            client.database("admin").run_command(command),
                                        )
                                        .await
                                        {
                                            Ok(result) => result,
                                            Err(_) => {
                                                Err(control_error("Mongo fault control timed out"))
                                            }
                                        },
                                        None => {
                                            Err(control_error("Mongo fault control unavailable"))
                                        }
                                    };
                                    let _ = reply.send(result);
                                }
                            }
                        }
                    });
            })
            .expect("start independent Mongo fault control runtime");
        Self {
            db: client.database(base.name()),
            controller: FailCommandController {
                inner: Arc::new(Control {
                    app_name,
                    state: Mutex::new(ControlState {
                        alive: true,
                        armed: false,
                    }),
                    requests,
                }),
            },
            worker: Some(worker),
            _ownership: ownership,
        }
    }

    pub(crate) fn database(&self) -> &mongodb::Database {
        &self.db
    }

    pub(crate) fn controller(&self) -> FailCommandController {
        self.controller.clone()
    }

    pub(crate) async fn arm(
        &mut self,
        mode: impl Into<Bson>,
        data: Document,
    ) -> mongodb::error::Result<Document> {
        let (control, mode) = (self.controller(), mode.into());
        tokio::task::spawn_blocking(move || control.arm(mode, data))
            .await
            .map_err(|_| control_error("Mongo fault control task failed"))?
    }

    pub(crate) async fn disarm(&mut self) -> mongodb::error::Result<()> {
        let control = self.controller();
        tokio::task::spawn_blocking(move || control.disarm())
            .await
            .map_err(|_| control_error("Mongo fault control task failed"))?
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // Joined reset runs independently of the test's Tokio runtime, including
        // during unwinding. No clone can arm after this ownership boundary.
        let mut state = self
            .controller
            .inner
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        state.alive = false;
        let reset = !state.armed
            || self
                .controller
                .command(doc! {
                    "configureFailPoint": "failCommand", "mode": "off",
                })
                .is_ok();
        let _ = self.controller.inner.requests.send(Request::Stop);
        let joined = self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_ok());
        if !reset || !joined {
            eprintln!("Mongo failCommand cleanup failed; scoped test client must not be reused");
            assert!(std::thread::panicking(), "Mongo failCommand reset failed");
        }
    }
}

pub(crate) struct ProcessLock {
    local: tokio::sync::Mutex<()>,
    path: Option<std::path::PathBuf>,
}

pub(crate) struct ProcessGuard<'a> {
    _local: tokio::sync::MutexGuard<'a, ()>,
    _process: File,
}

impl ProcessLock {
    pub(crate) const fn new() -> Self {
        Self {
            local: tokio::sync::Mutex::const_new(()),
            path: None,
        }
    }

    pub(crate) async fn lock(&self) -> ProcessGuard<'_> {
        let local = self.local.lock().await;
        let path = self
            .path
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("nyxid-mongo-fail-command.lock"));
        let file = tokio::task::spawn_blocking(move || {
            let file = open_lock(&path);
            file.lock()
                .expect("lock Mongo failCommand across processes");
            file
        })
        .await
        .expect("Mongo failCommand lock worker");
        ProcessGuard {
            _local: local,
            _process: file,
        }
    }
}

fn open_lock(path: &std::path::Path) -> File {
    // Do not unlink: replacing the inode would admit a second script while an
    // existing process still owns the first inode's lock. Kernel close/exit
    // releases ownership, including panic and process-crash boundaries.
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("open Mongo failCommand process lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audit_fail_command_retry_writes_override_surfaces_one_attempt() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let base = super::super::connect_transaction_test_database("fault_no_retry").await;
        for retries in [true, false] {
            let attempts = Arc::new(AtomicUsize::new(0));
            let observed = attempts.clone();
            let mut guard = Guard::acquire_with(
                &base,
                Some(EventHandler::callback(move |event| {
                    if let CommandEvent::Started(event) = event
                        && event.command_name == "insert"
                    {
                        observed.fetch_add(1, Ordering::SeqCst);
                    }
                })),
                GuardOptions {
                    retry_writes: Some(retries),
                },
            )
            .await;
            guard
                .arm(
                    doc! {"times": 1},
                    doc! {
                        "failCommands": ["insert"], "errorCode": 91,
                        "errorLabels": ["RetryableWriteError"],
                    },
                )
                .await
                .unwrap();
            let result = guard
                .database()
                .collection::<Document>("business")
                .insert_one(doc! {"_id": format!("retry-{retries}")})
                .await;
            guard.disarm().await.unwrap();
            assert_eq!(attempts.load(Ordering::SeqCst), if retries { 2 } else { 1 });
            assert_eq!(result.is_ok(), retries);
            assert_eq!(
                base.collection::<Document>("business")
                    .count_documents(doc! {"_id": format!("retry-{retries}")})
                    .await
                    .unwrap(),
                u64::from(retries)
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audit_fail_command_observer_reserves_the_single_connection_against_same_client_race() {
        use std::sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        let base = super::super::connect_transaction_test_database("fault_client_queue").await;
        let slot = Arc::new(OnceLock::<FailCommandController>::new());
        let observer_slot = slot.clone();
        let ready = Arc::new(tokio::sync::Notify::new());
        let observer_ready = ready.clone();
        let (release, receive) = mpsc::sync_channel(1);
        let receive = Mutex::new(receive);
        let unrelated_started = Arc::new(AtomicBool::new(false));
        let observer_unrelated = unrelated_started.clone();
        let guard = Guard::acquire(
            &base,
            Some(EventHandler::callback(move |event| {
                let CommandEvent::Started(event) = event else {
                    return;
                };
                if event.command_name != "insert" {
                    return;
                }
                let id = event
                    .command
                    .get_array("documents")
                    .ok()
                    .and_then(|docs| docs.first())
                    .and_then(Bson::as_document)
                    .and_then(|doc| doc.get_str("_id").ok());
                if id == Some("target") {
                    observer_slot
                        .get()
                        .unwrap()
                        .arm(
                            doc! {"times": 1},
                            doc! {
                                "failCommands": ["insert"], "errorCode": 112, "errorLabels": [],
                            },
                        )
                        .unwrap();
                    observer_ready.notify_one();
                    // The exact target already owns the only connection. Keep its
                    // Started observer paused while another same-client insert polls.
                    let _ = receive
                        .lock()
                        .unwrap()
                        .recv_timeout(super::super::STALE_TEST_DB_DROP_TIMEOUT);
                } else if id == Some("unrelated") {
                    observer_unrelated.store(true, Ordering::SeqCst);
                }
            })),
        )
        .await;
        assert!(slot.set(guard.controller()).is_ok());
        let db = guard.database().clone();
        let target = tokio::spawn(async move {
            db.collection::<Document>("business")
                .insert_one(doc! {"_id": "target"})
                .await
        });
        tokio::time::timeout(super::super::STALE_TEST_DB_DROP_TIMEOUT, ready.notified())
            .await
            .unwrap();
        let unrelated = async {
            guard
                .database()
                .collection::<Document>("business")
                .insert_one(doc! {"_id": "unrelated"})
                .await
        };
        tokio::pin!(unrelated);
        let waiting =
            tokio::time::timeout(std::time::Duration::from_millis(20), unrelated.as_mut()).await;
        let did_start = unrelated_started.load(Ordering::SeqCst);
        release.send(()).unwrap();
        assert!(
            waiting.is_err(),
            "the second operation must queue behind the Started target"
        );
        assert!(!did_start);
        let error = target.await.unwrap().unwrap_err();
        assert!(
            matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == 112)
        );
        unrelated.await.unwrap();
        assert!(unrelated_started.load(Ordering::SeqCst));
        assert_eq!(
            base.collection::<Document>("business")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn audit_fail_command_controller_arms_inside_started_observer_and_closes_with_guard() {
        use std::sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        let base = super::super::connect_transaction_test_database("fault_observer").await;
        let controller = Arc::new(OnceLock::<FailCommandController>::new());
        let slot = controller.clone();
        let armed = Arc::new(AtomicBool::new(false));
        let did_arm = armed.clone();
        let guard = Guard::acquire(
            &base,
            Some(EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event
                    && event.command_name == "find"
                    && !did_arm.swap(true, Ordering::SeqCst)
                {
                    // A blocking call on this test's single Tokio thread cannot
                    // rely on that runtime or the monitored client's pool.
                    slot.get()
                        .unwrap()
                        .arm(
                            doc! {"times": 1},
                            doc! {
                                "failCommands": ["insert"], "errorCode": 112, "errorLabels": [],
                            },
                        )
                        .unwrap();
                }
            })),
        )
        .await;
        assert!(controller.set(guard.controller()).is_ok());
        guard
            .database()
            .collection::<Document>("business")
            .find_one(doc! {})
            .await
            .unwrap();
        assert!(armed.load(Ordering::SeqCst));
        // The script is armed but its single insert injection is unconsumed.
        base.collection::<Document>("business")
            .insert_one(doc! {"_id": "unrelated"})
            .await
            .unwrap();
        let error = guard
            .database()
            .collection::<Document>("business")
            .insert_one(doc! {"_id": "target"})
            .await
            .unwrap_err();
        assert!(
            matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == 112)
        );
        guard
            .database()
            .collection::<Document>("business")
            .insert_one(doc! {"_id": "after-target"})
            .await
            .unwrap();
        assert!(armed.load(Ordering::SeqCst));
        assert_eq!(
            base.collection::<Document>("business")
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        let retained = guard.controller();
        drop(guard);
        assert!(
            retained
                .arm(
                    doc! {"times": 1},
                    doc! {"failCommands": ["insert"], "errorCode": 112}
                )
                .is_err()
        );
        assert!(retained.disarm().is_err());
        base.collection::<Document>("business")
            .insert_one(doc! {"_id": "after-close"})
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn audit_fail_command_scopes_injections_to_its_client_and_preserves_other_writes() {
        let base = super::super::connect_transaction_test_database("fault_scope").await;
        let mut fault = Guard::acquire(&base, None).await;
        fault
            .arm(
                doc! {"times": 2},
                doc! {
                    "failCommands": ["insert"], "errorCode": 112, "errorLabels": [],
                },
            )
            .await
            .unwrap();
        base.collection::<Document>("business")
            .insert_one(doc! {"_id": "unrelated"})
            .await
            .unwrap();
        for i in 0..2 {
            let error = fault
                .database()
                .collection::<Document>("business")
                .insert_one(doc! {"_id": format!("rejected-{i}")})
                .await
                .unwrap_err();
            assert!(matches!(
                error.kind.as_ref(),
                mongodb::error::ErrorKind::Command(command) if command.code == 112
            ));
        }
        fault
            .database()
            .collection::<Document>("business")
            .insert_one(doc! {"_id": "injections-exhausted"})
            .await
            .unwrap();
        assert_eq!(
            base.collection::<Document>("business")
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        // No explicit reset: ordinary scope exit must disarm too.
        drop(fault);
    }

    #[tokio::test]
    async fn audit_fail_command_panic_disarms_before_releasing_process_ownership() {
        let base = super::super::connect_transaction_test_database("fault_panic").await;
        let task_base = base.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut fault = Guard::acquire(&task_base, None).await;
            fault
                .arm(
                    "alwaysOn",
                    doc! {
                        "failCommands": ["insert"], "errorCode": 112, "errorLabels": [],
                    },
                )
                .await
                .unwrap();
            send.send(fault.database().clone()).unwrap();
            panic!("exercise fault guard panic cleanup");
        });
        let scoped = receive.await.unwrap();
        assert!(task.await.unwrap_err().is_panic());
        // The exact previously targeted client succeeds after unwinding; the
        // kernel lock alone would not remove an alwaysOn server failpoint.
        scoped
            .collection::<Document>("business")
            .insert_one(doc! {"_id": "after-panic"})
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn audit_failpoint_lock_excludes_other_processes_and_releases_on_drop() {
        const CHILD: &str = "NYXID_FAIL_COMMAND_LOCK_CHILD";
        if let Ok(mode) = std::env::var(CHILD) {
            let path = std::path::PathBuf::from(
                std::env::var_os("NYXID_FAIL_COMMAND_LOCK_CHILD_PATH").unwrap(),
            );
            let file = open_lock(&path);
            match (mode.as_str(), file.try_lock()) {
                ("blocked", Err(std::fs::TryLockError::WouldBlock)) | ("released", Ok(())) => (),
                (_, result) => panic!("unexpected cross-process lock result: {mode}: {result:?}"),
            }
            return;
        }
        async fn child(mode: &'static str, path: std::path::PathBuf) {
            let result = tokio::task::spawn_blocking(move || {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "test_utils::fail_command::tests::audit_failpoint_lock_excludes_other_processes_and_releases_on_drop",
                        "--nocapture",
                    ])
                    .env(CHILD, mode)
                    .env("NYXID_FAIL_COMMAND_LOCK_CHILD_PATH", path)
                    .output()
                    .unwrap()
            })
            .await
            .unwrap();
            assert!(result.status.success(), "{result:?}");
            assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
        }
        let path =
            std::env::temp_dir().join(format!("nyxid-failpoint-lock-{}", uuid::Uuid::new_v4()));
        let lock = ProcessLock {
            local: tokio::sync::Mutex::new(()),
            path: Some(path.clone()),
        };
        let guard = lock.lock().await;
        child("blocked", path.clone()).await;
        drop(guard);
        child("released", path.clone()).await;
        std::fs::remove_file(path).unwrap();
    }
}
