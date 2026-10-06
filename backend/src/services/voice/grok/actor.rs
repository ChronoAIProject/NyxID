//! Cancellation-safe provider actor; timer ticks cannot abandon admitted receipts.
use super::super::transcript::Segment;
use super::relay::Grok;
use crate::errors::{AppError, AppResult};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use tokio::sync::{mpsc, oneshot};

enum Command {
    Send(Value, oneshot::Sender<AppResult<()>>),
    Accept(Segment, oneshot::Sender<AppResult<bool>>),
    Close(oneshot::Sender<AppResult<()>>),
}
pub struct Handle {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<AppResult<Value>>,
    task: tokio::task::JoinHandle<()>,
    clock: tokio::time::Instant,
    closed_ms: Arc<AtomicI64>,
}
impl Handle {
    pub fn spawn(mut grok: Grok) -> Self {
        let (commands, mut receiver) = mpsc::channel(16);
        let (events, output) = mpsc::channel(64);
        let clock = grok.clock;
        let closed_ms = grok.closed_ms.clone();
        let marker = closed_ms.clone();
        let task = tokio::spawn(Box::pin(async move {
            let work = async {
                loop {
                    while let Some(event) = grok.events.pop_front() {
                        tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            events.send(Ok(event)),
                        )
                        .await
                        .map_err(|_| AppError::ClientDisconnected)?
                        .map_err(|_| AppError::ClientDisconnected)?;
                    }
                    grok.pump().await?;
                    tokio::select! {
                        command = receiver.recv() => match command {
                            Some(Command::Send(event, reply)) => {
                                let result=grok.send(event).await;
                                let failed=result.is_err(); let _=reply.send(result);
                                if failed {return Err(AppError::VoiceProviderUnavailable);}
                            }
                            Some(Command::Accept(segment, reply)) => {
                                let result=grok.accept_input(&segment).await;
                                let failed=result.is_err(); let _=reply.send(result);
                                if failed {return Err(AppError::VoiceProviderUnavailable);}
                            }
                            Some(Command::Close(reply)) => {
                                let result=grok.close().await;
                                marker.store(grok.measured_ms(),Ordering::Release);
                                let _=reply.send(result);
                                return Ok(());
                            }
                            None=>return Ok(()),
                        },
                        input = grok.input.recv() => {
                            let Some(input)=input else {return Ok(());};
                            grok.client(input).await?;
                        }
                        event = super::super::openai::receive(&mut grok.socket) => {
                            let Some(event)=event? else {return Ok(());};
                            grok.provider(event).await?;
                        }
                    }
                }
            };
            let result: AppResult<()> = work.await;
            if marker.load(Ordering::Acquire) < 0 {
                let _ = grok.close().await;
                marker.store(grok.measured_ms(), Ordering::Release);
            }
            if let Err(error) = result {
                let _ = events.try_send(Err(error));
            }
        }));
        Self {
            task,
            commands,
            events: output,
            clock,
            closed_ms,
        }
    }
    pub async fn receive(&mut self) -> AppResult<Option<Value>> {
        self.events.recv().await.transpose()
    }
    /// After close, preserve normalized events already received by the relay.
    /// The coordinator persists transcripts only; shutdown never executes them.
    pub fn take_pending(&mut self) -> Vec<Value> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if let Ok(event) = event {
                events.push(event);
            }
        }
        events
    }
    pub async fn send(&mut self, event: Value) -> AppResult<()> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Send(event, tx))
            .await
            .map_err(|_| AppError::VoiceProviderUnavailable)?;
        rx.await.map_err(|_| AppError::VoiceProviderUnavailable)?
    }
    pub async fn accept_input(&mut self, segment: &Segment) -> AppResult<bool> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Accept(segment.clone(), tx))
            .await
            .map_err(|_| AppError::VoiceProviderUnavailable)?;
        rx.await.map_err(|_| AppError::VoiceProviderUnavailable)?
    }
    pub fn measured_ms(&self) -> i64 {
        let closed = self.closed_ms.load(Ordering::Acquire);
        if closed >= 0 {
            closed
        } else {
            self.clock.elapsed().as_millis().min(1_800_000) as i64
        }
    }
    pub async fn close(&mut self) -> AppResult<()> {
        if self.task.is_finished() && self.closed_ms.load(Ordering::Acquire) >= 0 {
            return Ok(());
        }
        let close = async {
            let (tx, rx) = oneshot::channel();
            let reply = if self.commands.send(Command::Close(tx)).await.is_ok() {
                rx.await.ok()
            } else {
                None
            };
            if let Some(result) = reply {
                return result;
            }
            // Browser disconnect can win the actor's select against Close.
            // Wait for that shutdown to drop the sole provider socket instead
            // of treating a dropped reply as an orphaned, still-live call.
            (&mut self.task)
                .await
                .map_err(|_| AppError::VoiceProviderUnavailable)
        };
        match tokio::time::timeout(std::time::Duration::from_secs(5), close).await {
            Ok(result) => result,
            Err(_) => {
                // No actor survives a stalled close or a dropped owning coordinator.
                self.task.abort();
                let _ = (&mut self.task).await;
                self.closed_ms
                    .fetch_max(self.measured_ms(), Ordering::AcqRel);
                Err(AppError::VoiceProviderUnavailable)
            }
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.task.abort();
    }
}
