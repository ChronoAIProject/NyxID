//! Discord Gateway v10. Payloads and credentials never implement Debug or enter logs.
use std::{collections::VecDeque, future::Future, time::Duration};

use futures::{SinkExt, StreamExt, future::BoxFuture};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use zeroize::Zeroize;

pub(super) const INTENTS: u64 = 1 | (1 << 9) | (1 << 12) | (1 << 15);
const MAX_PENDING: usize = 32;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Resume {
    pub session_id: String,
    pub url: String,
    pub sequence: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Transport,
    Protocol,
    Heartbeat,
    Backpressure,
    Ingress,
    InvalidToken,
    Intents,
    Sharding,
    Identity,
    RateLimited,
    Reconnect,
    InvalidSession(bool),
}
impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidToken => "discord_gateway_invalid_token",
            Self::Intents => "discord_gateway_intents_required",
            Self::Sharding => "discord_gateway_sharding_required",
            Self::Identity => "discord_gateway_identity_mismatch",
            Self::RateLimited => "discord_gateway_rate_limited",
            Self::Heartbeat => "discord_gateway_heartbeat_timeout",
            Self::Backpressure => "discord_gateway_backpressure",
            Self::Ingress => "discord_gateway_ingress_failed",
            Self::Protocol => "discord_gateway_protocol_error",
            _ => "discord_gateway_disconnected",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::InvalidToken | Self::Intents | Self::Sharding | Self::Identity
        )
    }
}

pub(super) fn gateway_url(input: &str) -> Result<String, Failure> {
    let mut url = url::Url::parse(input).map_err(|_| Failure::Protocol)?;
    let host = url.host_str().ok_or(Failure::Protocol)?;
    let trusted = url.scheme() == "wss"
        && (host == "gateway.discord.gg" || host.ends_with(".discord.gg"))
        && url.port().is_none_or(|port| port == 443);
    #[cfg(test)]
    let trusted = trusted || (url.scheme() == "ws" && host == "127.0.0.1");
    if !trusted
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.fragment().is_some()
    {
        return Err(Failure::Protocol);
    }
    url.set_query(Some("v=10&encoding=json"));
    Ok(url.into())
}

fn close_failure(code: u16) -> Failure {
    match code {
        4004 => Failure::InvalidToken,
        4013 | 4014 => Failure::Intents,
        4010 | 4011 => Failure::Sharding,
        4007 | 4009 => Failure::InvalidSession(false),
        4008 => Failure::RateLimited,
        _ => Failure::Transport,
    }
}

/// One connection, with a bounded serial dispatch queue. Heartbeats/read/ACK
/// continue while shared ingress awaits a callback. Resume advances only after
/// ordered dispatch completes (including its fenced checkpoint write).
pub(super) async fn connection<F, Fut>(
    initial_url: &str,
    token: &str,
    bot_id: &str,
    resume: &mut Resume,
    dispatch: F,
) -> Failure
where
    F: Fn(Resume, Value) -> Fut,
    Fut: Future<Output = Result<(), Failure>> + Send,
{
    let url = if resume.session_id.is_empty() {
        initial_url
    } else {
        &resume.url
    };
    let Ok(url) = gateway_url(url) else {
        return Failure::Protocol;
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(8 * 1024 * 1024))
        .max_frame_size(Some(8 * 1024 * 1024));
    let Ok(Ok((mut ws, _))) = timeout(
        IO_TIMEOUT,
        connect_async_with_config(&url, Some(config), false),
    )
    .await
    else {
        return Failure::Transport;
    };
    let hello = match timeout(IO_TIMEOUT, ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Value>(&text).ok(),
        _ => None,
    };
    let Some(hello) = hello.filter(|v| v["op"] == 10) else {
        return Failure::Protocol;
    };
    let Some(ms) = hello["d"]["heartbeat_interval"]
        .as_u64()
        .filter(|ms| (100..=120_000).contains(ms))
    else {
        return Failure::Protocol;
    };
    let mut identify = if resume.session_id.is_empty() {
        json!({"op":2,"d":{"token":token,"intents":INTENTS,"properties":{"os":std::env::consts::OS,"browser":"NyxID","device":"NyxID"}}})
    } else {
        json!({"op":6,"d":{"token":token,"session_id":resume.session_id,"seq":resume.sequence}})
    };
    let sent = matches!(
        timeout(
            IO_TIMEOUT,
            ws.send(Message::Text(identify.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    );
    if let Some(Value::String(secret)) = identify.pointer_mut("/d/token") {
        secret.zeroize();
    }
    drop(identify);
    if !sent {
        return Failure::Transport;
    }
    let heartbeat_interval = Duration::from_millis(ms);
    let mut next_heartbeat =
        Instant::now() + Duration::from_millis(rand::thread_rng().gen_range(0..ms));
    let mut ack_pending = false;
    let mut received = resume.sequence;
    let mut pending = VecDeque::<(Resume, Value)>::new();
    let mut processing: Option<BoxFuture<'_, (Resume, Result<(), Failure>)>> = None;
    let mut failure = None;
    let mut closed = false;
    let mut authenticated = false;
    let auth_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if processing.is_none()
            && let Some((cursor, event)) = pending.pop_front()
        {
            let work = dispatch(cursor.clone(), event);
            processing = Some(Box::pin(async move { (cursor, work.await) }));
        }
        if failure.is_some() && !closed {
            // A normal (1000/1001) close invalidates Discord's resumable
            // session. Stop this socket immediately, then drain admitted work.
            let _ = timeout(
                Duration::from_secs(1),
                ws.close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code: 4000.into(),
                    reason: "reconnect".into(),
                })),
            )
            .await;
            closed = true;
        }
        if failure.is_some() && processing.is_none() && pending.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(auth_deadline), if !authenticated && failure.is_none() => {
                failure = Some(Failure::Protocol);
            }
            _ = tokio::time::sleep_until(next_heartbeat), if failure.is_none() => {
                if ack_pending {
                    failure = Some(Failure::Heartbeat);
                    continue;
                }
                let heartbeat = json!({"op":1,"d":received});
                if !matches!(timeout(IO_TIMEOUT, ws.send(Message::Text(heartbeat.to_string().into()))).await, Ok(Ok(()))) {
                    failure = Some(Failure::Transport);
                }
                ack_pending = true;
                next_heartbeat = Instant::now() + heartbeat_interval;
            }
            result = async { processing.as_mut().expect("guarded").await }, if processing.is_some() => {
                processing = None;
                match result {
                    (cursor, Ok(())) => *resume = cursor,
                    (_, Err(reason)) => {
                        failure = Some(reason);
                        pending.clear();
                    }
                }
            }
            frame = ws.next(), if failure.is_none() => {
                let value = match frame {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<Value>(&text) {
                        Ok(value) => value,
                        Err(_) => { failure = Some(Failure::Protocol); continue; }
                    },
                    Some(Ok(Message::Close(frame))) => {
                        failure = Some(frame.map_or(Failure::Transport, |f| close_failure(f.code.into())));
                        continue;
                    }
                    Some(Ok(Message::Ping(bytes))) => {
                        if !matches!(timeout(IO_TIMEOUT, ws.send(Message::Pong(bytes))).await, Ok(Ok(()))) { failure = Some(Failure::Transport); }
                        continue;
                    }
                    Some(Ok(Message::Pong(_))) => continue,
                    _ => { failure = Some(Failure::Transport); continue; }
                };
                match value["op"].as_u64() {
                    Some(11) => ack_pending = false,
                    Some(1) => {
                        if !matches!(timeout(IO_TIMEOUT, ws.send(Message::Text(json!({"op":1,"d":received}).to_string().into()))).await, Ok(Ok(()))) { failure = Some(Failure::Transport); }
                    }
                    Some(7) => failure = Some(Failure::Reconnect),
                    Some(9) => failure = Some(Failure::InvalidSession(value["d"].as_bool().unwrap_or(false))),
                    Some(0) => {
                        let Some(sequence) = value["s"].as_u64() else { failure = Some(Failure::Protocol); continue; };
                        received = Some(sequence);
                        // The socket has its own session identity even while old
                        // queued checkpoints are completing.
                        if value["t"] == "READY" {
                            if value["d"]["user"]["id"].as_str() != Some(bot_id) {
                                failure = Some(Failure::Identity); continue;
                            }
                            let (Some(id), Some(url)) = (value["d"]["session_id"].as_str(), value["d"]["resume_gateway_url"].as_str()) else { failure = Some(Failure::Protocol); continue; };
                            if id.is_empty() || id.len() > 256 || gateway_url(url).is_err() { failure = Some(Failure::Protocol); continue; }
                            resume.session_id = id.into();
                            resume.url = url.into();
                            authenticated = true;
                        } else if value["t"] == "RESUMED" {
                            authenticated = true;
                        } else if !authenticated && resume.session_id.is_empty() {
                            // Replay dispatches can precede RESUMED only for
                            // an already authenticated retained session.
                            failure = Some(Failure::Protocol); continue;
                        }
                        if pending.len() >= MAX_PENDING { failure = Some(Failure::Backpressure); continue; }
                        let mut cursor = resume.clone();
                        cursor.sequence = Some(sequence);
                        let event = if value["t"] == "MESSAGE_CREATE" {
                            if value.to_string().len() > 256 * 1024 { failure = Some(Failure::Backpressure); continue; }
                            value
                        } else { json!({"t":value["t"],"s":sequence}) };
                        pending.push_back((cursor, event));
                    }
                    _ => {}
                }
            }
        }
    }
    let reason = failure.unwrap_or(Failure::Transport);
    if matches!(reason, Failure::InvalidSession(false)) {
        *resume = Resume::default();
    }
    reason
}

pub(super) fn retry_delay(attempt: u32, failure: Failure) -> Duration {
    if failure.terminal() {
        return Duration::from_secs(300);
    }
    if matches!(failure, Failure::InvalidSession(_)) {
        return Duration::from_millis(rand::thread_rng().gen_range(1_000..=5_000));
    }
    let cap = (1_u64 << attempt.min(6)).min(60) * 1_000;
    Duration::from_millis(rand::thread_rng().gen_range(cap / 2..=cap))
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
