use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::client_error_report::*,
    mw::auth::AuthUser,
    services::crash_report_service::{
        self as service,
        admin::{ReportQuery, SummaryQuery},
    },
};
use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, Path, Query, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestRequest {
    pub account_id: String,
    pub revision: String,
    pub report: CrashReportV1,
}
impl std::fmt::Debug for IngestRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IngestRequest([redacted])")
    }
}
#[derive(Serialize)]
struct IngestResponse {
    status: &'static str,
    report_id: String,
}
fn no_store(value: impl Serialize) -> Response {
    ([("cache-control", "no-store")], Json(value)).into_response()
}

pub async fn ingest(
    State(state): State<AppState>,
    auth: AuthUser,
    peer: Result<ConnectInfo<std::net::SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> AppResult<Response> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    if !service::available(&state.db, &actor).await {
        service::count(&state.db, None, "unavailable").await;
        return Err(AppError::CrashReportingUnavailable);
    }
    let ip = crate::mw::rate_limit::resolve_client_ip_for_rate_limit(
        &headers,
        peer.ok().map(|p| p.0),
        &state.config.trusted_proxy_ips,
    )
    .map(|ip| ip.to_string());
    if let Err(error) = service::rate_limits(&state.db, &actor, ip.as_deref()).await {
        service::count(&state.db, None, "rate_limited").await;
        return Err(error);
    }
    if headers
        .get_all("content-encoding")
        .iter()
        .any(|v| v != "identity")
    {
        service::count(&state.db, None, "invalid").await;
        return Ok((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            [("cache-control", "no-store")],
        )
            .into_response());
    }
    let bytes = body.map_err(|_| AppError::RequestBodyTooLarge {
        max_bytes: 65536,
        context: "crash report".into(),
    })?;
    let request: IngestRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => {
            service::count(&state.db, None, "invalid").await;
            return Err(AppError::ValidationError("Invalid crash report".into()));
        }
    };
    let platform = request.report.platform;
    let tele = crate::telemetry::context::TelemetryContext::from_request_headers(&headers);
    if request.account_id != actor
        || !tele.analytics_allowed
        || tele.analytics_account.as_deref() != Some(&actor)
        || request.revision.len() > 64
        || request.revision.is_empty()
    {
        service::count(&state.db, Some(platform), "consent_denied").await;
        return Err(AppError::CrashReportConsentRequired);
    }
    match service::insert(
        &state.db,
        &state.crash_reports,
        &state.config.environment,
        &actor,
        &request.revision,
        request.report,
    )
    .await
    {
        Ok((duplicate, report_id)) => {
            let status = if duplicate { "duplicate" } else { "accepted" };
            service::count(&state.db, Some(platform), status).await;
            let mut response = no_store(IngestResponse { status, report_id });
            *response.status_mut() = StatusCode::ACCEPTED;
            Ok(response)
        }
        Err(error) => {
            service::count(
                &state.db,
                Some(platform),
                if matches!(error, AppError::CrashReportConsentRequired) {
                    "consent_denied"
                } else {
                    "invalid"
                },
            )
            .await;
            Err(error)
        }
    }
}

// Dedicated admin projections deliberately have no owner or consent fields.
#[derive(Serialize)]
struct RuntimeResponse {
    name: &'static str,
    major: u16,
}
#[derive(Serialize)]
struct JsFrameResponse {
    asset: String,
    line: u32,
    column: u32,
}
#[derive(Serialize)]
struct NativeImageResponse {
    uuid: String,
    name: String,
    size: u64,
}
#[derive(Serialize)]
struct NativeFrameResponse {
    image: Option<u8>,
    offset: u64,
}
#[derive(Serialize)]
struct ReportResponse {
    id: String,
    event_id: String,
    captured_at: String,
    platform: &'static str,
    kind: &'static str,
    exception_class: &'static str,
    native_signal: Option<&'static str>,
    app_version: String,
    app_build: String,
    artifact_id: Option<String>,
    os_version: String,
    runtime: Option<RuntimeResponse>,
    js_frames: Vec<JsFrameResponse>,
    native_images: Vec<NativeImageResponse>,
    native_frames: Vec<NativeFrameResponse>,
    environment: String,
    severity: String,
    fingerprint: String,
    received_at: String,
    expires_at: String,
    export: ExportResponse,
}
#[derive(Serialize)]
struct ExportResponse {
    status: &'static str,
    destination_id: Option<String>,
    attempts: u32,
    last_error_code: Option<String>,
    exported_at: Option<String>,
    provider_event_id: String,
}
impl From<ClientErrorReport> for ReportResponse {
    fn from(row: ClientErrorReport) -> Self {
        let r = row.report;
        Self {
            id: row.id,
            event_id: row.event_id,
            captured_at: r.captured_at,
            platform: r.platform.as_str(),
            kind: r.kind.as_str(),
            exception_class: r.exception_class.as_str(),
            native_signal: r.native_signal.map(NativeSignal::as_str),
            app_version: r.app_version,
            app_build: r.app_build,
            artifact_id: r.artifact_id,
            os_version: r.os_version,
            runtime: r.runtime.map(|r| RuntimeResponse {
                name: r.name.as_str(),
                major: r.major,
            }),
            js_frames: r
                .js_frames
                .into_iter()
                .map(|f| JsFrameResponse {
                    asset: f.asset,
                    line: f.line,
                    column: f.column,
                })
                .collect(),
            native_images: r
                .native_images
                .into_iter()
                .map(|i| NativeImageResponse {
                    uuid: i.uuid,
                    name: i.name,
                    size: i.size,
                })
                .collect(),
            native_frames: r
                .native_frames
                .into_iter()
                .map(|f| NativeFrameResponse {
                    image: f.image,
                    offset: f.offset,
                })
                .collect(),
            environment: row.environment,
            severity: row.severity,
            fingerprint: row.fingerprint,
            received_at: row.received_at.to_rfc3339(),
            expires_at: row.expires_at.to_rfc3339(),
            export: ExportResponse {
                status: row.export.status.as_str(),
                destination_id: row.export.destination_id,
                attempts: row.export.attempts,
                last_error_code: row.export.last_error_code,
                exported_at: row.export.exported_at.map(|d| d.to_rfc3339()),
                provider_event_id: r.event_id.replace('-', ""),
            },
        }
    }
}
async fn require_admin(state: &AppState, auth: &AuthUser) -> AppResult<()> {
    super::login_client_context::require_first_party_human(auth)?;
    super::admin_helpers::require_admin_or_operator(state, auth, "admin.crash_reports.read").await
}
#[derive(Serialize)]
struct ListResponse {
    reports: Vec<ReportResponse>,
    cursor: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ReportQuery>,
) -> AppResult<Response> {
    require_admin(&state, &auth).await?;
    let (rows, cursor) = service::admin::list(&state.db, &query).await?;
    Ok(no_store(ListResponse {
        reports: rows.into_iter().map(Into::into).collect(),
        cursor,
    }))
}
pub async fn detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    require_admin(&state, &auth).await?;
    Ok(no_store(ReportResponse::from(
        service::admin::detail(&state.db, &id).await?,
    )))
}
#[derive(Serialize)]
struct VersionResponse {
    platform: String,
    environment: String,
    app_version: String,
    app_build: String,
    artifact_id: Option<String>,
    counts: std::collections::BTreeMap<String, i64>,
    total: i64,
    first_seen: String,
    last_seen: String,
}
#[derive(Serialize)]
struct HealthResponse {
    outcomes: std::collections::BTreeMap<String, i64>,
    export_statuses: std::collections::BTreeMap<String, i64>,
    destination_ids: Vec<String>,
    retention_days: u32,
    flag_enabled: bool,
}
#[derive(Serialize)]
struct SummaryResponse {
    health: HealthResponse,
    versions: Vec<VersionResponse>,
}
pub async fn summary(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<SummaryQuery>,
) -> AppResult<Response> {
    require_admin(&state, &auth).await?;
    let data = service::admin::summary(&state.db, &query).await?;
    let counts = |rows: Vec<bson::Document>| {
        rows.into_iter()
            .map(|d| {
                (
                    d.get_str("_id").unwrap_or("unknown").to_owned(),
                    number(&d, "count"),
                )
            })
            .collect()
    };
    let versions = data
        .versions
        .into_iter()
        .map(|d| {
            let id = d.get_document("_id").cloned().unwrap_or_default();
            VersionResponse {
                platform: string(&id, "platform"),
                environment: string(&id, "environment"),
                app_version: string(&id, "app_version"),
                app_build: string(&id, "app_build"),
                artifact_id: id.get_str("artifact_id").ok().map(str::to_owned),
                total: number(&d, "total"),
                first_seen: time(&d, "first_seen"),
                last_seen: time(&d, "last_seen"),
                counts: [
                    "native_crash",
                    "js_fatal",
                    "js_error",
                    "unhandled_rejection",
                    "render_error",
                    "recoverable_error",
                    "resource_error",
                ]
                .into_iter()
                .map(|k| (k.to_owned(), number(&d, k)))
                .collect(),
            }
        })
        .collect();
    Ok(no_store(SummaryResponse {
        health: HealthResponse {
            outcomes: counts(data.counters),
            export_statuses: counts(data.exports),
            destination_ids: state.crash_reports.destination_ids(),
            retention_days: state.crash_reports.retention_days,
            flag_enabled: data.flag_enabled,
        },
        versions,
    }))
}
fn number(d: &bson::Document, key: &str) -> i64 {
    d.get_i64(key)
        .unwrap_or_else(|_| i64::from(d.get_i32(key).unwrap_or(0)))
}
fn string(d: &bson::Document, key: &str) -> String {
    d.get_str(key).unwrap_or_default().to_owned()
}
fn time(d: &bson::Document, key: &str) -> String {
    d.get_datetime(key)
        .map(|d| d.to_chrono().to_rfc3339())
        .unwrap_or_default()
}
#[cfg(test)]
mod tests;
