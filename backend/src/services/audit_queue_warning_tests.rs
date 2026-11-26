use super::*;
use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::instrument::WithSubscriber;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Capture {
    fn lines(&self) -> Vec<String> {
        String::from_utf8(self.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn entry() -> AuditLog {
    AuditLog {
        id: "placeholder-audit-id".into(),
        user_id: None,
        event_type: "placeholder-private-content".into(),
        event_data: None,
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
        seq: None,
        prev_hash: None,
        entry_hash: None,
        created_at: Utc::now(),
    }
}

#[tokio::test(start_paused = true)]
async fn queued_audit_warning_is_live_bounded_and_content_free() {
    let appender = Arc::new(OrdinaryAppender::new());
    let holder = appender.gate.lock().await;
    let options = mongodb::options::ClientOptions::builder()
        .hosts(vec![mongodb::options::ServerAddress::Tcp {
            host: "127.0.0.1".into(),
            port: Some(1),
        }])
        .build();
    let client = mongodb::Client::with_options(options).unwrap();
    let db = client.database("unused_queue_warning");
    let captured = Capture::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let dispatch = tracing::Dispatch::new(subscriber);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    let mut waiters = Vec::new();
    for _ in 0..2 {
        let appender = appender.clone();
        let db = db.clone();
        waiters.push(tokio::spawn(
            async move {
                appender
                    .append_until(&db, entry(), &[7; 32], deadline)
                    .await
            }
            .with_subscriber(dispatch.clone()),
        ));
    }
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(999)).await;
    tokio::task::yield_now().await;
    assert!(captured.lines().is_empty());
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    let lines = captured.lines();
    assert_eq!(
        lines.len(),
        1,
        "one warning while the gate is still held: {lines:?}"
    );
    assert!(lines[0].contains("queue_wait_ms=1000"), "{lines:?}");
    assert!(lines[0].contains("waiting=2"), "{lines:?}");
    assert!(!lines[0].contains("placeholder"), "{lines:?}");
    assert!(waiters.iter().all(|waiter| !waiter.is_finished()));
    waiters.pop().unwrap().abort();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(29)).await;
    tokio::task::yield_now().await;
    assert_eq!(captured.lines().len(), 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let lines = captured.lines();
    assert_eq!(lines.len(), 2, "one warning per 30 seconds across waiters");
    assert!(lines[1].contains("queue_wait_ms=31000"), "{lines:?}");
    assert!(
        lines[1].contains("waiting=1"),
        "cancelled waiter must disappear: {lines:?}"
    );
    tokio::time::advance(Duration::from_secs(9)).await;
    let error = waiters.pop().unwrap().await.unwrap().unwrap_err();
    assert_eq!(
        error.get_custom::<&str>(),
        Some(&"audit chain append deadline exceeded")
    );
    assert_eq!(captured.lines().len(), 2);
    drop(holder);
}
