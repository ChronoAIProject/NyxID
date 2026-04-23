//! Backend telemetry — vendor-neutral public API, PostHog-specific impl.
//!
//! Public surface: [`TelemetryClient`], [`TelemetryContext`],
//! [`TelemetryEvent`], [`emit_event`]. Nothing under the vendor namespace
//! (`posthog-rs` / `$identify` / `/capture/` endpoint shape) leaks across
//! the module boundary — callers never see it. See
//! `docs/TELEMETRY.md` §5.0 (hot-swap contract).
//!
//! Today's implementation POSTs to PostHog's `/capture/` endpoint with
//! `reqwest`. Swapping vendors means rewriting this file (and maybe
//! `schema.rs` event-name mapping). Callers don't move.

pub mod context;
pub mod schema;
pub mod scrub;

pub use context::{TelemetryContext, emit_event};
pub use schema::TelemetryEvent;

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc::{self, Sender};

use crate::config::AppConfig;

/// Compiled-in public DSN used when `NYXID_SHARE_ANALYTICS=true` is set
/// (self-hoster community contribution). Points at a separate PostHog
/// project from production so abuse / poisoning can't leak into the
/// data we use to drive product decisions. Safe to publish: PostHog
/// ingest keys cannot read or delete.
///
/// Update both this constant and the CLI's constant together if the
/// share-back project is ever migrated.
const NYXID_PUBLIC_TELEMETRY_DSN: &str = "phc_pHHMZRXY8ymzBy9uwiGmAVDtGvGpDTiyXH2zs7bQWEgM";
const NYXID_PUBLIC_TELEMETRY_HOST: &str = "https://us.i.posthog.com";

const DEFAULT_HOST: &str = "https://us.i.posthog.com";
const CHANNEL_CAPACITY: usize = 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// Vendor-neutral telemetry client.
///
/// Constructed via [`TelemetryClient::from_config`]; returns `None` when
/// no DSN resolves (the default "hard off" state). Hot-swappable — a
/// vendor migration replaces the internals of this file with no ripple
/// through the rest of the codebase.
#[derive(Clone)]
pub struct TelemetryClient {
    dsn: String,
    host: String,
    environment: &'static str,
    app_version: &'static str,
    tx: Sender<CaptureJob>,
}

#[derive(Debug)]
struct CaptureJob {
    distinct_id: String,
    event_name: &'static str,
    properties: serde_json::Value,
    timestamp: chrono::DateTime<chrono::Utc>,
}

impl TelemetryClient {
    /// Construct from `AppConfig`. Returns `None` when no DSN is
    /// configured — the default, and the contract for "hard off" per
    /// `docs/TELEMETRY.md` §3.
    ///
    /// Precedence (first match wins):
    /// 1. `NYXID_TELEMETRY_DSN` env var (self-hoster's own PostHog, or
    ///    the NyxID production DSN on the hosted deploy)
    /// 2. `NYXID_SHARE_ANALYTICS=true` → compiled-in public DSN
    ///    pointing at the share-back project
    /// 3. Neither → `None`
    pub fn from_config(cfg: &AppConfig) -> Option<Arc<Self>> {
        let (dsn, host) = if let Some(dsn) = cfg
            .telemetry_dsn
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .cloned()
        {
            let host = cfg
                .telemetry_host
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_HOST.to_string());
            (dsn, host)
        } else if cfg.share_analytics && !NYXID_PUBLIC_TELEMETRY_DSN.is_empty() {
            (
                NYXID_PUBLIC_TELEMETRY_DSN.to_string(),
                NYXID_PUBLIC_TELEMETRY_HOST.to_string(),
            )
        } else {
            return None;
        };

        // Normalize host: strip any trailing "/capture/" users paste by
        // mistake, and strip trailing slashes.
        let host = host
            .trim_end_matches('/')
            .trim_end_matches("/capture")
            .trim_end_matches('/')
            .to_string();

        let environment: &'static str = match cfg.environment.as_str() {
            "production" => "production",
            "staging" => "staging",
            _ => "development",
        };

        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("nyxid-backend/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;

        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let dsn_for_loop = dsn.clone();
        let host_for_loop = host.clone();
        tracing::info!(host = %host, env = environment, "telemetry client initialized");
        tokio::spawn(drain_loop(rx, http, dsn_for_loop, host_for_loop));

        Some(Arc::new(Self {
            dsn,
            host,
            environment,
            app_version: env!("CARGO_PKG_VERSION"),
            tx,
        }))
    }

    /// Enqueue a capture job. Fire-and-forget: returns immediately and
    /// drops silently if the bounded channel is full (1024-deep). Under
    /// burst, dropping is preferable to blocking request handlers.
    pub fn track(
        &self,
        distinct_id: &str,
        event: TelemetryEvent,
        ctx: &TelemetryContext,
        api_key_id: Option<&str>,
    ) {
        let event_name = event.name();
        let mut props = event.properties();

        // Merge common props (surface, app_version, environment, optional
        // client_version, optional api_key_id) into the scrubbed props.
        if let Some(obj) = props.as_object_mut() {
            obj.insert("surface".into(), json!(ctx.surface));
            obj.insert("app_version".into(), json!(self.app_version));
            obj.insert("environment".into(), json!(self.environment));
            if let Some(v) = &ctx.client_version {
                obj.insert("client_version".into(), json!(v));
            }
            if let Some(id) = api_key_id {
                obj.insert("api_key_id".into(), json!(id));
            }
        }

        let job = CaptureJob {
            distinct_id: distinct_id.to_string(),
            event_name,
            properties: props,
            timestamp: chrono::Utc::now(),
        };

        // `try_send` is the "drop on full" path we want. Failure to send
        // is intentionally silent — telemetry is never allowed to surface
        // errors to users or block the request path.
        let _ = self.tx.try_send(job);
    }

    /// Ask the vendor to delete the person identified by `distinct_id`
    /// and all aliased distinct_ids. Used by the erasure worker.
    ///
    /// Returns `Ok(())` on 2xx responses; surfaces errors so the worker
    /// can retry + dead-letter per `services::telemetry_erasure_service`.
    pub async fn delete_person(&self, distinct_id: &str) -> Result<(), reqwest::Error> {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("nyxid-backend/", env!("CARGO_PKG_VERSION")))
            .build()?;

        // PostHog: `DELETE /api/projects/@current/persons/?distinct_id=…`
        // requires the personal API key, not the ingest key. We document
        // this and treat unconfigured-personal-key as "non-retryable skip"
        // at the service layer: the hosted deploy sets both; self-hosters
        // with only an ingest DSN skip the delete.
        //
        // For the first ship, we use the bulk-delete capture API on the
        // ingest DSN, which is vendor-supported for user-data deletion
        // requests. See PostHog docs: `$delete_person` event.
        let url = format!("{host}/capture/", host = self.host);
        let body = json!({
            "api_key": self.dsn,
            "event": "$delete_person",
            "distinct_id": distinct_id,
            "properties": {
                "surface": "backend",
                "app_version": self.app_version,
                "environment": self.environment,
            },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        http.post(&url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

/// Drain loop — owns the `reqwest` client and POSTs each job to the
/// vendor's capture endpoint. Spawned once per [`TelemetryClient`] at
/// construction time.
async fn drain_loop(mut rx: mpsc::Receiver<CaptureJob>, http: Client, dsn: String, host: String) {
    let url = format!("{host}/capture/");
    while let Some(job) = rx.recv().await {
        let body = json!({
            "api_key": dsn,
            "event": job.event_name,
            "distinct_id": job.distinct_id,
            "properties": job.properties,
            "timestamp": job.timestamp.to_rfc3339(),
        });

        // Best-effort. 4xx = shape bug, don't retry. 5xx = transient,
        // don't block other events either; next event will retry the
        // underlying connection implicitly.
        match http.post(&url).json(&body).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    tracing::warn!(
                        status = %resp.status(),
                        event = %job.event_name,
                        "telemetry capture returned non-2xx"
                    );
                } else {
                    tracing::debug!(event = %job.event_name, "telemetry capture sent");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, event = %job.event_name, "telemetry capture failed");
            }
        }
    }
}
