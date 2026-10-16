//! Process identity and content-free panic containment for detached engine turns.
use futures::FutureExt;
use std::{
    cell::RefCell,
    future::Future,
    panic::{self, AssertUnwindSafe},
    sync::{LazyLock, Once},
};
use uuid::Uuid;

use super::assistant_nyxagent::TurnError;
use crate::models::assistant_conversation::{AssistantConversation, EngineWorkerInstance};

static WORKER: LazyLock<EngineWorkerInstance> = LazyLock::new(|| EngineWorkerInstance {
    id: Uuid::new_v4().to_string(),
    hostname: hostname(std::env::var("HOSTNAME").ok()),
});
static PANIC_HOOK: Once = Once::new();
static STARTED: Once = Once::new();

struct PanicLocation {
    file: String,
    line: u32,
    column: u32,
}

tokio::task_local! {
    static PANIC_LOCATION: RefCell<Option<PanicLocation>>;
}

fn hostname(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        !value.is_empty()
            && value.len() <= super::assistant_engine_policy::WORKER_HOSTNAME_MAX_BYTES
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    })
}

pub fn instance() -> &'static EngineWorkerInstance {
    &WORKER
}

pub fn initialize() {
    install_panic_hook();
    STARTED.call_once(|| {
        let worker = instance();
        tracing::info!(target: "nyxid::assistant_engine", worker_instance = %worker.id,
            worker_hostname = worker.hostname.as_deref(), version = env!("CARGO_PKG_VERSION"),
            "NyxID engine worker started");
    });
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            // The default hook prints the payload before catch_unwind runs.
            // Suppress it only while polling a contained engine future.
            if PANIC_LOCATION
                .try_with(|slot| {
                    if let Ok(mut slot) = slot.try_borrow_mut() {
                        *slot = info.location().map(|at| PanicLocation {
                            file: at.file().to_owned(),
                            line: at.line(),
                            column: at.column(),
                        });
                    }
                })
                .is_err()
            {
                previous(info);
            }
        }));
    });
}

pub async fn contain<T>(
    row: &AssistantConversation,
    execution: impl Future<Output = Result<T, TurnError>>,
) -> Result<T, TurnError> {
    install_panic_hook();
    PANIC_LOCATION
        .scope(RefCell::new(None), async {
            match AssertUnwindSafe(execution).catch_unwind().await {
                Ok(result) => result,
                Err(payload) => {
                    let payload_type = if payload.is::<&'static str>() {
                        "static_str"
                    } else if payload.is::<String>() {
                        "string"
                    } else {
                        "opaque"
                    };
                    let at = PANIC_LOCATION.with(|slot| slot.borrow_mut().take());
                    let worker = instance();
                    tracing::error!(target: "nyxid::assistant_engine", conversation_id = %row.id,
                        turn_id = row.active_turn.as_ref().map(|turn| turn.turn_id.as_str()),
                        worker_instance = %worker.id, worker_hostname = worker.hostname.as_deref(),
                        code = "internal_error", stage = "panic", payload_type,
                        panic_file = at.as_ref().map(|at| at.file.as_str()),
                        panic_line = at.as_ref().map(|at| at.line),
                        panic_column = at.as_ref().map(|at| at.column),
                        "Assistant turn execution panicked");
                    Err(TurnError::internal("internal_error", "panic"))
                }
            }
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_logs_one_process_identity_and_version() {
        let capture = tempfile::NamedTempFile::new().unwrap();
        let writer = capture.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            initialize();
            initialize();
        });
        let logs = std::fs::read_to_string(capture.path()).unwrap();
        assert_eq!(logs.matches("NyxID engine worker started").count(), 1);
        assert!(logs.contains(&instance().id));
        assert!(logs.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn worker_identity_is_stable_and_hostname_is_bounded_metadata() {
        assert_eq!(instance(), instance());
        assert_eq!(
            Uuid::parse_str(&instance().id).unwrap().get_version_num(),
            4
        );
        assert_eq!(
            hostname(Some("nyxid-abc_1.example".into())).as_deref(),
            Some("nyxid-abc_1.example")
        );
        for value in [
            String::new(),
            "secret\nprompt".into(),
            "bad host".into(),
            "x".repeat(super::super::assistant_engine_policy::WORKER_HOSTNAME_MAX_BYTES + 1),
        ] {
            assert!(hostname(Some(value)).is_none());
        }
        assert!(hostname(None).is_none());
    }
}
