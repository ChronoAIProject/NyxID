//! CONNECT to the already selected service origin, never an arbitrary host.
use super::*;
use crate::models::assistant_sharing::SharedExecution;
use crate::services::shared_agent_transport as authority;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) async fn start(
    state: &AppState,
    client: hyper::upgrade::OnUpgrade,
    upstream: reqwest::Response,
    binding: SharedExecution,
    meter: crate::services::billing::MeteredProxyContext,
    lease: Option<crate::services::service_concurrency_service::Lease>,
) -> AppResult<Response> {
    let db = state.db.clone();
    let billing = state.billing.clone();
    tokio::spawn(async move {
        let mut stats = ConnectionUsageStats::default();
        let started = std::time::Instant::now();
        let opened = async {
            let (client, upstream) = tokio::try_join!(
                async {
                    client
                        .await
                        .map(hyper_util::rt::TokioIo::new)
                        .map_err(|_| AppError::ClientDisconnected)
                },
                async {
                    upstream.upgrade().await.map_err(|_| {
                        AppError::WsProxyDownstream("Service tunnel unavailable".into())
                    })
                },
            )?;
            authority::revalidate(&db, &binding).await?;
            Ok::<_, AppError>((client, upstream))
        };
        if let Ok(Ok((mut client, mut upstream))) =
            tokio::time::timeout(std::time::Duration::from_secs(10), opened).await
        {
            let work = relay(&db, &binding, &mut client, &mut upstream, &mut stats);
            tokio::select! {
                _ = work => {},
                _ = async { if let Some(lease) = &lease { lease.cancelled().await } else { std::future::pending().await } } => {},
            }
            let _ = client.shutdown().await;
            let _ = upstream.shutdown().await;
        }
        stats.duration = started.elapsed();
        let mut event = serde_json::json!({"connection_id":binding.connection, "duration_secs": stats.duration.as_secs(),
            "bytes_in":stats.bytes_in, "bytes_out":stats.bytes_out});
        authority::annotate(&mut event, Some(&binding));
        audit_service::log_async(
            db,
            Some(binding.actor.clone()),
            "proxy_connect_closed".into(),
            Some(event),
            None,
            None,
            None,
            None,
        );
        let usage = websocket_platform_usage(&stats);
        let resale = websocket_resale_usage(&meter, &stats);
        settle_meter_async(billing, meter, usage, resale, None).await;
        drop(lease);
    });
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::empty())
        .expect("static tunnel response"))
}

async fn relay<C, U>(
    db: &mongodb::Database,
    binding: &SharedExecution,
    client: &mut C,
    upstream: &mut U,
    stats: &mut ConnectionUsageStats,
) -> AppResult<()>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut ur, mut uw) = tokio::io::split(upstream);
    let mut incoming = [0u8; 16 * 1024];
    let mut outgoing = [0u8; 16 * 1024];
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    let lifetime = tokio::time::sleep(std::time::Duration::from_secs(1800));
    tokio::pin!(lifetime);
    loop {
        tokio::select! {
            _ = &mut lifetime => return Ok(()),
            _ = tick.tick() => authority::revalidate(db, binding).await?,
            read = cr.read(&mut incoming) => {
                let n = read.map_err(|_| AppError::ClientDisconnected)?;
                if n == 0 { return Ok(()); }
                authority::revalidate(db, binding).await?;
                tokio::time::timeout(std::time::Duration::from_secs(10), uw.write_all(&incoming[..n])).await
                    .map_err(|_| AppError::ClientDisconnected)?.map_err(|_| AppError::ClientDisconnected)?;
                stats.bytes_in += n as i64;
            },
            read = ur.read(&mut outgoing) => {
                let n = read.map_err(|_| AppError::ClientDisconnected)?;
                if n == 0 { return Ok(()); }
                authority::revalidate(db, binding).await?;
                tokio::time::timeout(std::time::Duration::from_secs(10), cw.write_all(&outgoing[..n])).await
                    .map_err(|_| AppError::ClientDisconnected)?.map_err(|_| AppError::ClientDisconnected)?;
                stats.bytes_out += n as i64;
            },
        }
    }
}
