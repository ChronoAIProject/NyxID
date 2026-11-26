use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::http::Method;
use tracing::instrument::WithSubscriber;

use crate::services::shutdown::Runtime;

const INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Eq, Hash, PartialEq)]
struct ObservationKey {
    client_id: String,
    method: Method,
    route: String,
}

impl ObservationKey {
    fn emit(&self, count: u64) {
        tracing::info!(
            parent: None,
            client_id = %self.client_id,
            method = %self.method,
            route = %self.route,
            count,
            "app_token_management_refusal"
        );
    }
}

#[derive(Default)]
pub(crate) struct AppManagementObservations {
    pending: Mutex<HashMap<ObservationKey, u64>>,
}

impl AppManagementObservations {
    pub(crate) fn record(
        self: &Arc<Self>,
        shutdown: &Runtime,
        client_id: &str,
        method: &Method,
        route: &str,
    ) {
        let key = ObservationKey {
            client_id: client_id.to_owned(),
            method: method.clone(),
            route: route.to_owned(),
        };
        {
            let mut pending = self.pending.lock().unwrap_or_else(|err| err.into_inner());
            if let Some(count) = pending.get_mut(&key) {
                *count = count.saturating_add(1);
                return;
            }
            pending.insert(key.clone(), 0);
        }
        key.emit(1);
        shutdown.spawn_worker(Arc::clone(self).flush(key).with_current_subscriber());
    }

    // One timer per active tuple emits the last partial count even if the caller stops.
    async fn flush(self: Arc<Self>, key: ObservationKey) {
        loop {
            tokio::time::sleep(INTERVAL).await;
            let count = {
                let mut pending = self.pending.lock().unwrap_or_else(|err| err.into_inner());
                let count = pending.get_mut(&key).expect("active observation window");
                if *count == 0 {
                    pending.remove(&key);
                    return;
                }
                std::mem::take(count)
            };
            key.emit(count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::read_to_string;
    use tokio::time::advance;
    use tracing::instrument::WithSubscriber;

    #[tokio::test(start_paused = true)]
    async fn app_management_observations_count_each_tuple_and_release_idle_windows() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let writer = file.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        let events = || {
            read_to_string(file.path())
                .unwrap()
                .lines()
                .filter(|line| line.contains("app_token_management_refusal"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let observations = Arc::new(AppManagementObservations::default());
        let shutdown = Runtime::default();
        async {
            for _ in 0..4 {
                observations.record(&shutdown, "client-a", &Method::GET, "/sessions");
            }
            observations.record(&shutdown, "client-b", &Method::GET, "/sessions");
            observations.record(&shutdown, "client-a", &Method::DELETE, "/sessions");
            observations.record(&shutdown, "client-a", &Method::GET, "/users/me");
            assert_eq!(events().len(), 4);
            assert!(events().iter().all(|line| line.contains("count=1")));
            tokio::task::yield_now().await;
            advance(Duration::from_secs(59)).await;
            tokio::task::yield_now().await;
            assert_eq!(events().len(), 4);
            observations.record(&shutdown, "client-a", &Method::GET, "/sessions");
            advance(Duration::from_secs(1)).await;
            tokio::task::yield_now().await;
            let lines = events();
            assert_eq!(lines.len(), 5);
            for field in ["client-a", "GET", "/sessions", "count=4"] {
                assert!(lines[4].contains(field));
            }
            observations.record(&shutdown, "client-a", &Method::GET, "/sessions");
            assert_eq!(events().len(), 5);
            advance(Duration::from_secs(60)).await;
            tokio::task::yield_now().await;
            let lines = events();
            assert_eq!(lines.len(), 6);
            assert!(lines[5].contains("count=1"));
            advance(Duration::from_secs(60)).await;
            tokio::task::yield_now().await;
            assert!(observations.pending.lock().unwrap().is_empty());
            observations.record(&shutdown, "client-a", &Method::GET, "/sessions");
            assert_eq!(events().len(), 7);
        }
        .with_subscriber(subscriber)
        .await;
    }
}
