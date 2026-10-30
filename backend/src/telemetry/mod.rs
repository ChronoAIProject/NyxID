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

pub mod client_event;
pub mod context;
#[cfg(test)]
mod dispatch_inventory_tests;
pub mod erasure;
pub mod sampling;
pub mod schema;
pub mod scrub;

pub use context::{TelemetryContext, emit_event};
// Re-exported for downstream emit-site chunks.
#[allow(unused_imports)]
pub use sampling::{hash_short_id, should_sample_event};
pub use schema::TelemetryEvent;

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde_json::json;
use tokio::sync::{
    Semaphore,
    mpsc::{self, Sender},
};

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
    admission_slots: Arc<Semaphore>,
    db: Option<mongodb::Database>,
    #[cfg(test)]
    dsn: String,
    #[cfg(test)]
    host: String,
    environment: &'static str,
    app_version: &'static str,
    tx: Sender<CaptureJob>,
}

#[derive(Debug)]
struct CaptureJob {
    consent_revision: String,
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
    pub fn from_config(cfg: &AppConfig, db: Option<mongodb::Database>) -> Option<Arc<Self>> {
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
        tokio::spawn(drain_loop(
            rx,
            http,
            dsn_for_loop,
            host_for_loop,
            db.clone(),
        ));

        Some(Arc::new(Self {
            admission_slots: Arc::new(Semaphore::new(CHANNEL_CAPACITY)),
            db,
            #[cfg(test)]
            dsn,
            #[cfg(test)]
            host,
            environment,
            app_version: env!("CARGO_PKG_VERSION"),
            tx,
        }))
    }

    /// Admit a job after an at-most-250ms account-permission read. Concurrent
    /// admission and the dispatch queue are bounded; saturation drops analytics.
    /// Vendor delivery runs separately and never blocks a product request.
    pub async fn track(
        &self,
        distinct_id: &str,
        event: TelemetryEvent,
        ctx: &TelemetryContext,
        api_key_id: Option<&str>,
    ) {
        let Ok(_admission_slot) = self.admission_slots.try_acquire() else {
            return;
        };
        if !ctx.analytics_allowed
            || ctx.analytics_account.as_deref() != Some(distinct_id)
            || self.tx.is_closed()
            || self.tx.capacity() == 0
        {
            return;
        }
        let Some(db) = self.db.as_ref() else {
            return;
        };
        let Some(consent_revision) =
            crate::services::analytics_consent_service::permitted_revision(db, distinct_id).await
        else {
            return;
        };
        if let TelemetryEvent::ClientOriginated { revision, .. } = &event
            && revision != &consent_revision
        {
            return;
        }
        let event_name = event.name();
        let mut props = event.properties();

        // Merge common props (surface, app_version, environment, optional
        // client_version, optional api_key_id) into the scrubbed props.
        //
        // `event.properties()` runs the egress scrubber over every field
        // the event itself carries. Common props are added AFTER that
        // pass, so `client_version` (from `X-NyxID-Client-Version`,
        // attacker-controllable) and `api_key_id` (raw UUID) would
        // otherwise bypass the scrubber. Re-run scrubbing on each
        // common prop we actually insert. Fixed values
        // (`surface`, `app_version`, `environment`) are short enum
        // strings and do not need scrubbing.
        if let Some(obj) = props.as_object_mut() {
            obj.insert("surface".into(), json!(ctx.surface));
            obj.insert("app_version".into(), json!(self.app_version));
            obj.insert("environment".into(), json!(self.environment));
            if let Some(v) = &ctx.client_version {
                let scrubbed = scrub::scrub_string(v).into_owned();
                obj.insert("client_version".into(), json!(scrubbed));
            }
            if let Some(id) = api_key_id {
                // Raw api_key_id would be a UUID that the scrubber
                // collapses to `[UUID_REDACTED]`; hash for stable
                // per-agent attribution the same way emit sites do.
                obj.insert("api_key_id".into(), json!(sampling::hash_short_id(id)));
            }
        }

        let job = CaptureJob {
            consent_revision,
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
}

/// Drain loop — owns the `reqwest` client and POSTs each job to the
/// vendor's capture endpoint. Spawned once per [`TelemetryClient`] at
/// construction time.
async fn drain_loop(
    mut rx: mpsc::Receiver<CaptureJob>,
    http: Client,
    dsn: String,
    host: String,
    db: Option<mongodb::Database>,
) {
    let url = format!("{host}/capture/");
    while let Some(job) = rx.recv().await {
        let Some(db) = db.as_ref() else {
            continue;
        };
        if !crate::services::analytics_consent_service::permits_dispatch(
            db,
            &job.distinct_id,
            &job.consent_revision,
        )
        .await
        {
            continue;
        }
        let Some(dispatch_id) = crate::services::analytics_dispatch_service::begin(
            db,
            &job.distinct_id,
            &job.consent_revision,
        )
        .await
        else {
            continue;
        };
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
        let result = http.post(&url).json(&body).send().await;
        let accepted = result
            .as_ref()
            .is_ok_and(|response| response.status().is_success());
        if crate::services::analytics_dispatch_service::settle(db, &dispatch_id, accepted)
            .await
            .is_err()
        {
            tracing::warn!(
                "Analytics dispatch settlement unconfirmed; erasure reconciliation required"
            );
        }
        match result {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_app_config;

    async fn test_client_with_sender(tx: Sender<CaptureJob>) -> TelemetryClient {
        let db = crate::test_utils::connect_test_database("analytics_queue")
            .await
            .expect("MongoDB required");
        let actor = "11111111-2222-4333-8444-555555555555";
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                actor,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        crate::services::analytics_consent_service::set(&db, actor, true)
            .await
            .unwrap();
        await_responsive_consent_reads(&db, actor).await;
        TelemetryClient {
            admission_slots: Arc::new(Semaphore::new(CHANNEL_CAPACITY)),
            db: Some(db),
            dsn: "test-dsn".to_string(),
            host: "https://telemetry.example.com".to_string(),
            environment: "staging",
            app_version: "0.0-test",
            tx,
        }
    }

    /// These tests observe the production consent gate: a linearizable read
    /// that must answer within its 200 ms MongoDB / 250 ms overall deadline.
    /// That deadline is the assertion, so the precondition is a responsive
    /// server. A CI runner can still be stalled on the database's own I/O
    /// backlog (journal flushes, hundreds of per-test database drops) when the
    /// test starts, even with every test slot reserved. Wait, within the same
    /// bounded budget the test-database readiness probe uses, until the gate
    /// answers comfortably inside its deadline; each measured call below still
    /// runs exactly once against the unchanged production bounds.
    async fn await_responsive_consent_reads(db: &mongodb::Database, actor: &str) {
        const BUDGET: std::time::Duration = std::time::Duration::from_secs(90);
        const RESPONSIVE: std::time::Duration = std::time::Duration::from_millis(100);
        let started = std::time::Instant::now();
        loop {
            let probe = std::time::Instant::now();
            let permitted =
                crate::services::analytics_consent_service::permitted_revision(db, actor)
                    .await
                    .is_some();
            if permitted && probe.elapsed() <= RESPONSIVE {
                return;
            }
            assert!(
                started.elapsed() < BUDGET,
                "MongoDB consent reads were not responsive within {RESPONSIVE:?} for {BUDGET:?} \
                 (last probe: permitted={permitted}, took {:?})",
                probe.elapsed()
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    #[test]
    fn from_config_returns_none_when_telemetry_is_hard_off() {
        let cfg = test_app_config();

        assert!(TelemetryClient::from_config(&cfg, None).is_none());
    }

    #[tokio::test]
    async fn from_config_prefers_configured_dsn_and_normalizes_host() {
        let mut cfg = test_app_config();
        cfg.telemetry_dsn = Some("phc_self_hosted".to_string());
        cfg.telemetry_host = Some("https://telemetry.example.com/capture/".to_string());
        cfg.environment = "production".to_string();

        let client =
            TelemetryClient::from_config(&cfg, None).expect("configured DSN should enable");

        assert_eq!(client.dsn, "phc_self_hosted");
        assert_eq!(client.host, "https://telemetry.example.com");
        assert_eq!(client.environment, "production");
        assert_eq!(client.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn from_config_blank_dsn_uses_public_share_back_when_enabled() {
        let mut cfg = test_app_config();
        cfg.telemetry_dsn = Some("   ".to_string());
        cfg.telemetry_host = Some("https://ignored.example.com".to_string());
        cfg.share_analytics = true;
        cfg.environment = "staging".to_string();

        let client =
            TelemetryClient::from_config(&cfg, None).expect("share analytics should enable");

        assert_eq!(client.dsn, NYXID_PUBLIC_TELEMETRY_DSN);
        assert_eq!(client.host, NYXID_PUBLIC_TELEMETRY_HOST);
        assert_eq!(client.environment, "staging");
    }

    #[tokio::test]
    async fn from_config_defaults_unknown_environment_to_development() {
        let mut cfg = test_app_config();
        cfg.telemetry_dsn = Some("phc_test".to_string());
        cfg.telemetry_host = None;
        cfg.environment = "qa".to_string();

        let client =
            TelemetryClient::from_config(&cfg, None).expect("configured DSN should enable");

        assert_eq!(client.host, DEFAULT_HOST);
        assert_eq!(client.environment, "development");
    }

    #[tokio::test]
    async fn track_enqueues_scrubbed_capture_job_with_common_properties() {
        let (tx, mut rx) = mpsc::channel(1);
        let client = test_client_with_sender(tx).await;
        let ctx = TelemetryContext {
            surface: "cli",
            client_version: Some("1.2.3 alice@example.com".to_string()),
            analytics_allowed: true,
            analytics_account: Some("11111111-2222-4333-8444-555555555555".into()),
        };
        let api_key_id = "4b9d8f21-9b9d-41f4-9963-3c822f4fbbed";

        client
            .track(
                "11111111-2222-4333-8444-555555555555",
                TelemetryEvent::KeyCreated {
                    source: "catalog".to_string(),
                    catalog_slug: Some("openai".to_string()),
                    has_node_binding: true,
                },
                &ctx,
                Some(api_key_id),
            )
            .await;

        let job = rx.try_recv().expect("capture job should be queued");
        assert_eq!(job.distinct_id, "11111111-2222-4333-8444-555555555555");
        assert_eq!(job.event_name, "key.created");
        assert_eq!(job.properties["source"], "catalog");
        assert_eq!(job.properties["catalog_slug"], "openai");
        assert_eq!(job.properties["has_node_binding"], true);
        assert_eq!(job.properties["surface"], "cli");
        assert_eq!(job.properties["app_version"], "0.0-test");
        assert_eq!(job.properties["environment"], "staging");
        assert_eq!(job.properties["client_version"], "1.2.3 [EMAIL_REDACTED]");
        assert_eq!(
            job.properties["api_key_id"],
            sampling::hash_short_id(api_key_id)
        );
    }

    #[tokio::test]
    async fn emit_event_excludes_chrono_sandbox_successes() {
        let (tx, mut rx) = mpsc::channel(1);
        let client = test_client_with_sender(tx).await;

        emit_event(
            Some(&client),
            "11111111-2222-4333-8444-555555555555",
            None,
            &TelemetryContext {
                analytics_allowed: true,
                analytics_account: Some("11111111-2222-4333-8444-555555555555".into()),
                ..Default::default()
            },
            TelemetryEvent::ProxySuccess {
                service_slug: "chrono-sandbox".into(),
                method: "POST".into(),
                status: 200,
                latency_ms: 42,
                auth_kind: "service_account",
            },
        )
        .await;

        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn emit_event_keeps_sandbox_errors_and_other_service_successes() {
        let (tx, mut rx) = mpsc::channel(2);
        let client = test_client_with_sender(tx).await;

        for event in [
            TelemetryEvent::ProxyError {
                service_slug: "chrono-sandbox".into(),
                error_code: 8001,
                status: 503,
            },
            TelemetryEvent::ProxySuccess {
                service_slug: "chrono-storage-service".into(),
                method: "DELETE".into(),
                status: 200,
                latency_ms: 42,
                auth_kind: "service_account",
            },
        ] {
            emit_event(
                Some(&client),
                "11111111-2222-4333-8444-555555555555",
                None,
                &TelemetryContext {
                    analytics_allowed: true,
                    analytics_account: Some("11111111-2222-4333-8444-555555555555".into()),
                    ..Default::default()
                },
                event,
            )
            .await;
        }

        let error = rx.try_recv().expect("sandbox errors remain visible");
        assert_eq!(error.event_name, "proxy.error");
        assert_eq!(error.properties["service_slug"], "chrono-sandbox");
        let success = rx.try_recv().expect("other services remain tracked");
        assert_eq!(success.event_name, "proxy.success");
        assert_eq!(success.properties["service_slug"], "chrono-storage-service");
    }

    #[tokio::test]
    async fn track_silently_drops_when_channel_is_full() {
        let (tx, mut rx) = mpsc::channel(1);
        tx.try_send(CaptureJob {
            consent_revision: "first-revision".into(),
            distinct_id: "first".to_string(),
            event_name: "auth.logged_in",
            properties: json!({"queued": true}),
            timestamp: chrono::Utc::now(),
        })
        .expect("pre-fill telemetry queue");
        let client = test_client_with_sender(tx).await;
        assert!(
            crate::services::analytics_consent_service::permitted_revision(
                client.db.as_ref().unwrap(),
                "11111111-2222-4333-8444-555555555555"
            )
            .await
            .is_some()
        );

        client
            .track(
                "11111111-2222-4333-8444-555555555555",
                TelemetryEvent::AuthLoggedOut,
                &TelemetryContext {
                    analytics_allowed: true,
                    analytics_account: Some("11111111-2222-4333-8444-555555555555".into()),
                    ..Default::default()
                },
                None,
            )
            .await;

        let job = rx.try_recv().expect("original job should remain queued");
        assert_eq!(job.distinct_id, "first");
        assert_eq!(job.event_name, "auth.logged_in");
        assert_eq!(job.properties, json!({"queued": true}));
        assert!(
            rx.try_recv().is_err(),
            "full-channel send should be dropped"
        );
    }

    #[tokio::test]
    async fn emit_event_preserves_local_denial_when_overriding_agent_surface() {
        let (tx, mut rx) = mpsc::channel(2);
        let client = test_client_with_sender(tx).await;
        let actor = "11111111-2222-4333-8444-555555555555";
        emit_event(
            Some(&client),
            actor,
            Some("agent-key"),
            &TelemetryContext::default(),
            TelemetryEvent::AuthLoggedOut,
        )
        .await;
        assert!(rx.try_recv().is_err());
        emit_event(
            Some(&client),
            actor,
            Some("agent-key"),
            &TelemetryContext {
                analytics_allowed: true,
                analytics_account: Some("11111111-2222-4333-8444-555555555555".into()),
                ..Default::default()
            },
            TelemetryEvent::AuthLoggedOut,
        )
        .await;
        let permitted = rx.try_recv().expect("opted-in account and device");
        assert_eq!(permitted.properties["surface"], "agent");
        assert_eq!(
            permitted.properties["api_key_id"],
            sampling::hash_short_id("agent-key")
        );
    }

    #[tokio::test]
    async fn api_key_events_use_authenticated_human_owner_permission() {
        use crate::{models::api_key::ApiKey, services::analytics_consent_service as consent};
        let (tx, mut rx) = mpsc::channel(4);
        let client = test_client_with_sender(tx).await;
        let db = client.db.as_ref().unwrap();
        let owner = "11111111-2222-4333-8444-555555555555";
        let key_id = uuid::Uuid::new_v4().to_string();
        let key: ApiKey = mongodb::bson::from_document(mongodb::bson::doc! {
            "_id": &key_id, "user_id": owner, "name": "consent-test", "key_prefix": "test",
            "key_hash": "not-a-secret", "scopes": "proxy", "is_active": true,
            "created_at": mongodb::bson::DateTime::now()
        })
        .unwrap();
        let auth = crate::mw::auth::api_key_auth_user(db, &key, None, None, None)
            .await
            .unwrap();
        assert_eq!(auth.user_id.to_string(), owner);
        let ctx = TelemetryContext {
            analytics_allowed: true,
            analytics_account: Some(owner.into()),
            ..Default::default()
        };
        consent::set(db, owner, false).await.unwrap();
        // Even a consenting account whose UUID equals the key ID cannot grant
        // permission to the key's human owner.
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &key_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        consent::set(db, &key_id, true).await.unwrap();
        emit_event(
            Some(&client),
            &auth.user_id.to_string(),
            Some(&key_id),
            &ctx,
            TelemetryEvent::AuthLoggedOut,
        )
        .await;
        assert!(rx.try_recv().is_err());
        consent::set(db, &key_id, false).await.unwrap();
        consent::set(db, owner, true).await.unwrap();
        emit_event(
            Some(&client),
            &auth.user_id.to_string(),
            Some(&key_id),
            &ctx,
            TelemetryEvent::AuthLoggedOut,
        )
        .await;
        let job = rx
            .try_recv()
            .expect("owner's permission admits the API-key event");
        assert_eq!(job.distinct_id, owner);
        assert_eq!(job.properties["surface"], "agent");
        assert_eq!(
            job.properties["api_key_id"],
            sampling::hash_short_id(&key_id)
        );
        let denied = TelemetryContext {
            analytics_allowed: false,
            ..ctx
        };
        emit_event(
            Some(&client),
            &auth.user_id.to_string(),
            Some(&key_id),
            &denied,
            TelemetryEvent::AuthLoggedOut,
        )
        .await;
        assert!(rx.try_recv().is_err());
    }

    /// The worker is held by not starting it until after withdrawal/regrant.
    /// A valid sentinel follows rejected work so receipt proves the drain passed
    /// every earlier item without timing sleeps or an assumed empty network.
    #[tokio::test]
    async fn dispatch_checks_live_revision_across_independent_clients() {
        use crate::services::analytics_consent_service as consent;
        use axum::{Json, Router, http::StatusCode, routing::post};
        let (captures, mut received) = mpsc::channel::<serde_json::Value>(16);
        let app = Router::new().route(
            "/capture/",
            post(move |Json(body): Json<serde_json::Value>| {
                let captures = captures.clone();
                async move {
                    captures.send(body).await.unwrap();
                    StatusCode::OK
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (tx, rx) = mpsc::channel(32);
        let client = test_client_with_sender(tx).await;
        let db = client.db.as_ref().unwrap();
        let actor = "11111111-2222-4333-8444-555555555555";
        // Separate client/pool and database handle, as another replica uses.
        let uri = std::env::var("NYXID_TEST_DATABASE_URL").expect("isolated MongoDB URI required");
        let remote_db = mongodb::Client::with_uri_str(uri)
            .await
            .unwrap()
            .database(db.name());
        let mut first_state = Box::new(crate::test_utils::test_app_state(db.clone()));
        first_state.telemetry = Some(Arc::new(client.clone()));
        let (remote_tx, remote_rx) = mpsc::channel(1);
        let mut second_state = Box::new(crate::test_utils::test_app_state(remote_db.clone()));
        second_state.telemetry = Some(Arc::new(TelemetryClient {
            db: Some(remote_db.clone()),
            tx: remote_tx,
            admission_slots: Arc::new(Semaphore::new(CHANNEL_CAPACITY)),
            ..client.clone()
        }));
        let allowed = |actor: &str| TelemetryContext {
            analytics_allowed: true,
            analytics_account: Some(actor.into()),
            ..Default::default()
        };
        let admitted_revision = consent::read(db, actor).await.unwrap().revision;
        await_responsive_consent_reads(db, actor).await;
        let capacity = client.tx.capacity();
        first_state
            .telemetry
            .as_ref()
            .unwrap()
            .track(actor, TelemetryEvent::AuthLoggedOut, &allowed(actor), None)
            .await;
        assert_eq!(
            client.tx.capacity(),
            capacity - 1,
            "first replica must enqueue"
        );
        await_responsive_consent_reads(&remote_db, actor).await;
        second_state
            .telemetry
            .as_ref()
            .unwrap()
            .track(actor, TelemetryEvent::AuthLoggedOut, &allowed(actor), None)
            .await;
        assert_eq!(
            second_state.telemetry.as_ref().unwrap().tx.capacity(),
            0,
            "second replica must enqueue before withdrawal"
        );
        consent::set(&second_state.db, actor, false).await.unwrap();
        consent::set(&second_state.db, actor, true).await.unwrap();
        drop(first_state);
        drop(second_state);
        // A delayed client request must not obtain a new revision at server
        // admission after withdrawal/regrant has already happened.
        client
            .track(
                actor,
                TelemetryEvent::ClientOriginated {
                    event: serde_json::from_value(
                        json!({"name":"ui.docs_opened", "props":{"page":"encryption"}}),
                    )
                    .unwrap(),
                    revision: admitted_revision,
                },
                &allowed(actor),
                None,
            )
            .await;
        // Keep the regranted actor live and opted in through the drain:
        // removing revision equality would leak the first admitted event.
        let withdrawn = uuid::Uuid::new_v4().to_string();
        let deleted = uuid::Uuid::new_v4().to_string();
        for id in [&withdrawn, &deleted] {
            db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .insert_one(crate::test_utils::test_user(
                    id,
                    crate::models::user::UserType::Person,
                ))
                .await
                .unwrap();
            consent::set(&remote_db, id, true).await.unwrap();
            await_responsive_consent_reads(db, id).await;
            let capacity = client.tx.capacity();
            client
                .track(id, TelemetryEvent::AuthLoggedOut, &allowed(id), None)
                .await;
            assert_eq!(
                client.tx.capacity(),
                capacity - 1,
                "permission-change case must start admitted"
            );
        }
        consent::set(&remote_db, &withdrawn, false).await.unwrap();
        client
            .track(
                &withdrawn,
                TelemetryEvent::AuthLoggedOut,
                &allowed(&withdrawn),
                None,
            )
            .await;
        db.collection::<mongodb::bson::Document>(crate::models::user::COLLECTION_NAME)
            .delete_one(mongodb::bson::doc! {"_id": &deleted})
            .await
            .unwrap();
        // Absent, unknown, deleted, and default/DNT/pre-auth narrowing each deny.
        let sentinel = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &sentinel,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        client
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &allowed(&sentinel),
                None,
            )
            .await;
        let unknown = uuid::Uuid::new_v4().to_string();
        client
            .track(
                &unknown,
                TelemetryEvent::AuthLoggedOut,
                &allowed(&unknown),
                None,
            )
            .await;
        client
            .track(
                &deleted,
                TelemetryEvent::AuthLoggedOut,
                &allowed(&deleted),
                None,
            )
            .await;
        consent::set(&remote_db, &sentinel, true).await.unwrap();
        client
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &TelemetryContext::default(),
                None,
            )
            .await;
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-nyxid-analytics", "enabled".parse().unwrap());
        headers.insert("x-nyxid-analytics-account", sentinel.parse().unwrap());
        headers.insert("dnt", "1".parse().unwrap());
        client
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &TelemetryContext::from_request_headers(&headers),
                None,
            )
            .await;
        headers.remove("dnt");
        headers.insert("sec-gpc", "1".parse().unwrap());
        client
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &TelemetryContext::from_request_headers(&headers),
                None,
            )
            .await;
        headers.remove("sec-gpc");
        client
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &TelemetryContext::pre_auth(&headers),
                None,
            )
            .await;
        let mut no_db = client.clone();
        no_db.db = None;
        no_db
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &allowed(&sentinel),
                None,
            )
            .await;
        // Saturated admission drops even when the queue has space.
        let admission = client
            .admission_slots
            .acquire_many(CHANNEL_CAPACITY as u32)
            .await
            .unwrap();
        client
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &allowed(&sentinel),
                None,
            )
            .await;
        drop(admission);
        // Both accounts are globally enabled. A device choice from actor A
        // must not authorize a generic event authenticated as sentinel B.
        let capacity = client.tx.capacity();
        client
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &allowed(actor),
                None,
            )
            .await;
        assert_eq!(
            client.tx.capacity(),
            capacity,
            "cross-account device choice admitted"
        );
        // A database outage must deny rather than reuse prior permission.
        let mut options = mongodb::options::ClientOptions::parse("mongodb://127.0.0.1:1")
            .await
            .unwrap();
        options.server_selection_timeout = Some(Duration::from_millis(25));
        let mut unavailable = client.clone();
        unavailable.db = Some(
            mongodb::Client::with_options(options)
                .unwrap()
                .database("unavailable"),
        );
        unavailable
            .track(
                &sentinel,
                TelemetryEvent::AuthLoggedOut,
                &allowed(&sentinel),
                None,
            )
            .await;
        // Admit with a healthy DB, then make the dispatcher independently
        // unavailable (and absent) before it can process the queued job.
        for dispatch_db in [unavailable.db.clone(), None] {
            let (failure_tx, failure_rx) = mpsc::channel(1);
            let mut admitted = client.clone();
            admitted.tx = failure_tx;
            await_responsive_consent_reads(db, &sentinel).await;
            admitted
                .track(
                    &sentinel,
                    TelemetryEvent::AuthLoggedOut,
                    &allowed(&sentinel),
                    None,
                )
                .await;
            assert_eq!(admitted.tx.capacity(), 0, "outage case must start admitted");
            drop(admitted);
            drain_loop(
                failure_rx,
                Client::new(),
                "test-dsn".into(),
                host.clone(),
                dispatch_db,
            )
            .await;
        }
        await_responsive_consent_reads(db, &sentinel).await;
        let capacity = client.tx.capacity();
        client
            .track(
                &sentinel,
                TelemetryEvent::KeyCreated {
                    source: "catalog".into(),
                    catalog_slug: None,
                    has_node_binding: false,
                },
                &allowed(&sentinel),
                None,
            )
            .await;
        assert_eq!(client.tx.capacity(), capacity - 1, "sentinel must enqueue");
        // End the bounded stream after the sentinel; worker completion proves all
        // items were processed, including any erroneously admitted extra jobs.
        drop(no_db);
        drop(unavailable);
        let db = db.clone();
        drop(client);
        // Both replicas admitted work before the acknowledged withdrawal. Each
        // independently drains against the same authoritative account revision.
        drain_loop(
            remote_rx,
            Client::new(),
            "test-dsn".into(),
            host.clone(),
            Some(remote_db),
        )
        .await;
        drain_loop(rx, Client::new(), "test-dsn".into(), host, Some(db)).await;
        let capture = received.try_recv().expect("exactly one permitted capture");
        assert_eq!(capture["event"], "key.created");
        assert_eq!(capture["distinct_id"], sentinel);
        assert!(
            received.try_recv().is_err(),
            "withdrawn or denied work escaped"
        );
        server.abort();
    }
}
