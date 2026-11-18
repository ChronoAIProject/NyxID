//! Keep one receiver set from server entry through startup and the complete drain.
use chrono::{DateTime, Utc};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub struct Received {
    pub signal: &'static str,
    pub at: Instant,
    pub at_utc: DateTime<Utc>,
}

pub struct Signals {
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    pending: Option<&'static str>,
}

impl Signals {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            #[cfg(unix)]
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            pending: None,
        })
    }

    pub async fn next(&mut self) -> &'static str {
        if let Some(signal) = self.pending.take() {
            return signal;
        }
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.terminate.recv() => "SIGTERM",
                _ = self.interrupt.recv() => "SIGINT",
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            "ctrl-c"
        }
    }

    pub fn watch_startup(mut self) -> StartupSignals {
        let (control, mut requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut first = None;
            let mut second = None;
            loop {
                tokio::select! {
                    biased;
                    signal = self.next(), if second.is_none() => {
                        let received = Received { signal, at: Instant::now(), at_utc: Utc::now() };
                        let ordinal = if first.is_none() {
                            first = Some(received);
                            1
                        } else {
                            second = Some(signal);
                            2
                        };
                        #[cfg(all(test, unix))]
                        crate::startup_signal_tests::received_signal(signal, received.at_utc, ordinal);
                        tracing::info!(target: "nyxid::shutdown", signal, ordinal,
                            "Stop signal received during startup; finishing startup to the next safe point");
                    }
                    request = requests.recv() => match request {
                        Some(Request::Check(reply)) => { let _ = reply.send(first); }
                        Some(Request::Finish) | None => {
                            self.pending = second;
                            return (self, first);
                        }
                    }
                }
            }
        });
        StartupSignals {
            control,
            task: Some(task),
        }
    }
}

#[derive(Debug)]
enum Request {
    Check(oneshot::Sender<Option<Received>>),
    Finish,
}

/// Observes signals independently of startup IO, without cancelling any step.
/// The bind checkpoint joins it and takes back the original receivers.
pub struct StartupSignals {
    control: mpsc::UnboundedSender<Request>,
    task: Option<JoinHandle<(Signals, Option<Received>)>>,
}

impl StartupSignals {
    pub async fn stop_requested(&self, safe_point: &'static str) -> bool {
        let (reply, response) = oneshot::channel();
        self.control
            .send(Request::Check(reply))
            .expect("startup signal observer stopped");
        if let Some(received) = response.await.expect("startup signal observer stopped") {
            tracing::info!(target: "nyxid::shutdown", signal = received.signal, safe_point,
                "Stopping startup before database writes and application workers");
            true
        } else {
            false
        }
    }

    pub async fn finish(mut self) -> (Signals, Option<Received>) {
        self.control
            .send(Request::Finish)
            .expect("startup signal observer stopped");
        self.task
            .take()
            .expect("startup signal observer missing")
            .await
            .expect("startup signal observer failed")
    }
}

impl Drop for StartupSignals {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            // Early safe-point return or failed startup: this task owns only
            // signal receivers, never database work or application claims.
            task.abort();
        }
    }
}
