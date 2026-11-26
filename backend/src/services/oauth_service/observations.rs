use std::{collections::HashMap, hash::Hash, sync::Mutex, time::Duration};
use tracing::instrument::WithSubscriber;

/// Emit the first request immediately, then count additional observations once per minute.
pub(crate) struct Windows<K>(Mutex<HashMap<K, u64>>);

impl<K> Default for Windows<K> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Windows<K> {
    pub(crate) fn record(&'static self, mut key: K, coalesce: fn(&mut K), emit: fn(&K, u64)) {
        {
            let mut pending = self.0.lock().unwrap_or_else(|err| err.into_inner());
            // Bound windows created by request-supplied scheme names. Keep registered client attribution.
            if pending.len() >= 4096 && !pending.contains_key(&key) {
                coalesce(&mut key);
            }
            if let Some(count) = pending.get_mut(&key) {
                *count = count.saturating_add(1);
                return;
            }
            pending.insert(key.clone(), 0);
        }
        emit(&key, 1);
        crate::services::shutdown::process().spawn_worker(
            async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    let count = {
                        let mut pending = self.0.lock().unwrap_or_else(|err| err.into_inner());
                        let count = pending.get_mut(&key).expect("active observation window");
                        if *count == 0 {
                            pending.remove(&key);
                            return;
                        }
                        std::mem::take(count)
                    };
                    emit(&key, count);
                }
            }
            .with_current_subscriber(),
        );
    }
}

#[cfg(test)]
pub(crate) fn capture() -> (
    tempfile::NamedTempFile,
    impl tracing::Subscriber + Send + Sync,
) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let writer = file.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.try_clone().unwrap())
        .finish();
    (file, subscriber)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    #[tokio::test(start_paused = true)]
    async fn observation_windows_flush_final_count_and_expire() {
        static WINDOWS: LazyLock<Windows<u8>> = LazyLock::new(Default::default);
        static COUNTS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
        for _ in 0..3 {
            WINDOWS.record(1, |_| {}, |_, count| COUNTS.lock().unwrap().push(count));
        }
        assert_eq!(*COUNTS.lock().unwrap(), [1]);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(*COUNTS.lock().unwrap(), [1, 2]);
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(WINDOWS.0.lock().unwrap().is_empty());
    }
}
