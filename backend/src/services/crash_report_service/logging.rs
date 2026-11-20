//! Closed diagnostic events. Resolve filtering against the current dispatcher,
//! rather than the process-wide callsite interest cache: independent request/test
//! subscribers must observe the same production fields under parallel execution.
use tracing::{Level, callsite::Callsite, field::Value};

fn emit(metadata: &'static tracing::Metadata<'static>, fields: &[Option<&dyn Value>]) {
    tracing::dispatcher::get_default(|dispatcher| {
        if dispatcher.enabled(metadata) {
            let values = metadata.fields().value_set_all(fields);
            dispatcher.event(&tracing::Event::new(metadata, &values));
        }
    });
}

pub(super) fn ingestion(outcome: &'static str, platform: &'static str) {
    let site = tracing::callsite! {
        name: "crash_ingestion_outcome", kind: tracing::metadata::Kind::EVENT,
        level: Level::DEBUG, fields: message, outcome, platform
    };
    emit(
        site.metadata(),
        &[
            Some(&"Crash ingestion outcome"),
            Some(&outcome),
            Some(&platform),
        ],
    );
}

pub(super) fn counter_unavailable() {
    let site = tracing::callsite! {
        name: "crash_ingestion_health", kind: tracing::metadata::Kind::EVENT,
        level: Level::WARN, fields: message, outcome
    };
    emit(
        site.metadata(),
        &[
            Some(&"Crash ingestion health"),
            Some(&"counter_unavailable"),
        ],
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ManifestFailure {
    Transport,
    Status,
    Oversize,
    InvalidJson,
    Mismatch,
}
impl ManifestFailure {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Status => "status",
            Self::Oversize => "oversize",
            Self::InvalidJson => "invalid_json",
            Self::Mismatch => "mismatch",
        }
    }
}
pub(super) fn manifest_failure(reason: ManifestFailure, host: &str) {
    let site = tracing::callsite! {
        name: "crash_manifest_fetch_failed", kind: tracing::metadata::Kind::EVENT,
        level: Level::WARN, fields: message, reason, host
    };
    emit(
        site.metadata(),
        &[
            Some(&"Crash release manifest fetch failed"),
            Some(&reason.as_str()),
            Some(&host),
        ],
    );
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[derive(Clone, Default)]
    pub(crate) struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Capture {
        pub(crate) fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
            let writer = self.clone();
            tracing_subscriber::fmt()
                .json()
                .with_ansi(false)
                .without_time()
                .with_max_level(Level::DEBUG)
                .with_writer(move || writer.clone())
                .finish()
        }
        pub(crate) fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
        pub(crate) fn events(&self) -> Vec<serde_json::Value> {
            self.text()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }
    #[test]
    fn crash_logging_observes_current_dispatcher_despite_parallel_interest_changes() {
        std::thread::scope(|scope| {
            for _ in 0..32 {
                scope.spawn(|| {
                    // Prime these exact production callsites as uninterested.
                    tracing::dispatcher::with_default(&tracing::Dispatch::none(), || {
                        ingestion("invalid", "unknown"); counter_unavailable();
                        manifest_failure(ManifestFailure::Status, "frontend.example.test");
                    });
                    let capture = Capture::default();
                    tracing::subscriber::with_default(capture.subscriber(), || {
                        ingestion("accepted", "web"); counter_unavailable();
                        manifest_failure(ManifestFailure::Status, "frontend.example.test");
                    });
                    let text = capture.text();
                    assert_eq!(text.lines().count(), 3);
                    let events = capture.events();
                    assert_eq!(events[0]["fields"], serde_json::json!({"message":"Crash ingestion outcome", "outcome":"accepted", "platform":"web"}));
                    assert_eq!(events[1]["fields"], serde_json::json!({"message":"Crash ingestion health", "outcome":"counter_unavailable"}));
                    assert_eq!(events[2]["fields"], serde_json::json!({"message":"Crash release manifest fetch failed", "reason":"status", "host":"frontend.example.test"}));
                });
            }
        });
    }
}
